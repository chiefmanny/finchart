//! The pure (terminal-free) core of the live viewer: preload history, fold a
//! deterministic synthetic tick stream into candles, and render a follow-latest
//! frame. All logic here is unit-testable without a TTY.

use std::iter::Peekable;

use backend_terminal::{render_chart, ChartOptions, ColorMode, DepthPanel, HeatmapView, Style};
use market_core::{
    events, generate, BookSide, Candle, ChartState, DepthConfig, EventConfig, EventSource, Flow,
    GenConfig, LiquidityHeatmap, LiveConfig, MarketEvent,
};
use render_core::{cull, Viewport};

/// Where historical candles start (epoch seconds).
const HISTORY_START_TS: i64 = 1_600_000_000;

/// Configuration for a synthetic live replay.
#[derive(Debug, Clone)]
pub struct ReplayConfig {
    pub seed: u64,
    /// Historical candles to preload before live ticks begin.
    pub history: usize,
    pub interval_secs: i64,
    pub start_price: f64,
    /// Per-candle volatility for the preloaded history.
    pub candle_volatility: f64,
    /// Per-tick volatility for the live stream.
    pub tick_volatility: f64,
    pub drift: f64,
    pub ticks_per_candle: usize,
    pub volume_per_tick: f64,
    /// Visible candle window (follow-latest tail).
    pub window: usize,
    /// Order-book price tick size.
    pub tick_size: f64,
    /// Levels per side in the synthetic ladder.
    pub depth_levels: usize,
    /// Base resting size per depth level.
    pub depth_base_size: f64,
    /// Apply a depth step every this many ticks.
    pub depth_every: usize,
    /// Levels shown per side in the depth panel.
    pub depth_panel_levels: usize,
    /// Heatmap rolling-window capacity (time buckets).
    pub heatmap_time_buckets: usize,
    /// Draw the heatmap layer (off in the normal interactive viewer).
    pub show_heatmap: bool,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            history: 400,
            interval_secs: 60,
            start_price: 100.0,
            candle_volatility: 0.02,
            tick_volatility: 0.0015,
            drift: 0.0002,
            ticks_per_candle: 30,
            volume_per_tick: 40.0,
            window: 80,
            tick_size: 0.05,
            depth_levels: 12,
            depth_base_size: 50.0,
            depth_every: 5,
            depth_panel_levels: 6,
            heatmap_time_buckets: 160,
            show_heatmap: false,
        }
    }
}

/// A running live replay driven through the neutral [`ChartState`] ingestion
/// path: a synthetic [`EventSource`] produces `MarketEvent`s and `ChartState`
/// folds them into candles + order book + heatmap. The runner no longer mutates
/// those structures directly.
pub struct Replay {
    state: ChartState,
    events: Peekable<EventSource>,
    window: usize,
    total_ticks: usize,
    depth_panel_levels: usize,
    show_heatmap: bool,
}

impl Replay {
    /// Preload history and prepare the synthetic event source (continuous with the
    /// last historical candle), then apply its seeding depth snapshot.
    pub fn new(cfg: &ReplayConfig) -> Self {
        let history = generate(&GenConfig {
            seed: cfg.seed,
            count: cfg.history,
            start_ts: HISTORY_START_TS,
            interval_secs: cfg.interval_secs,
            start_price: cfg.start_price,
            volatility: cfg.candle_volatility,
            drift: cfg.drift,
        });

        // Live ticks continue from the last historical candle for continuity.
        let (live_ts, live_price) = history
            .last()
            .map(|c| (c.ts + cfg.interval_secs, c.close))
            .unwrap_or((HISTORY_START_TS, cfg.start_price));

        let event_cfg = EventConfig {
            live: LiveConfig {
                seed: cfg.seed ^ 0x9E37_79B9_7F4A_7C15,
                start_ts: live_ts,
                start_price: live_price,
                interval_secs: cfg.interval_secs,
                ticks_per_candle: cfg.ticks_per_candle,
                volatility: cfg.tick_volatility,
                drift: cfg.drift,
                volume_per_tick: cfg.volume_per_tick,
            },
            depth: DepthConfig {
                seed: cfg.seed ^ 0xD3F7_0000_D3F7_0000,
                tick_size: cfg.tick_size,
                levels: cfg.depth_levels,
                base_size: cfg.depth_base_size,
                size_jitter: 0.5,
                recenter_band: 0.5,
                updates_per_step: cfg.depth_levels / 2,
            },
            depth_every: cfg.depth_every.max(1),
        };

        // The heatmap is always recorded; `show_heatmap` only gates rendering.
        let state = ChartState::from_history(
            history,
            cfg.interval_secs,
            cfg.tick_size,
            Some(cfg.heatmap_time_buckets),
            cfg.window.max(1),
        );

        let mut replay = Self {
            state,
            events: events(&event_cfg).peekable(),
            window: cfg.window,
            total_ticks: 0,
            depth_panel_levels: cfg.depth_panel_levels,
            show_heatmap: cfg.show_heatmap,
        };
        // Apply the seeding events (initial depth snapshot + heartbeat) so the book
        // and heatmap are populated before the first tick.
        replay.drain_until_next_trade();
        replay
    }

    /// Apply pending non-trade events (depth deltas + heartbeats) up to the next
    /// trade, leaving the trade pending.
    fn drain_until_next_trade(&mut self) {
        while self
            .events
            .peek()
            .is_some_and(|ev| !matches!(ev, MarketEvent::Trade(_)))
        {
            if let Some(ev) = self.events.next() {
                self.state.apply(&ev);
            }
        }
    }

    /// Ingest the next `n` trades, applying each trade plus the depth/heartbeat
    /// events that follow it, through [`ChartState`].
    pub fn advance(&mut self, n: usize) {
        for _ in 0..n {
            match self.events.next() {
                Some(ev) => {
                    self.state.apply(&ev);
                    self.total_ticks += 1;
                }
                None => break,
            }
            self.drain_until_next_trade();
        }
    }

    /// The follow-latest viewport (most recent `window` candles).
    pub fn viewport(&self) -> Viewport {
        Viewport::tail(self.series_len(), self.window)
    }

    /// Render the current frame at the given size and color mode, using `title`
    /// as the status line. Callers pass a deterministic title (e.g. [`status`])
    /// for snapshots, or one augmented with live timings for the interactive view.
    ///
    /// [`status`]: Replay::status
    pub fn render(&self, width: usize, height: usize, color: ColorMode, title: &str) -> String {
        let series = self.state.series();
        let view = cull(series, &self.viewport());
        render_chart(
            &view,
            &ChartOptions {
                width,
                height,
                color,
                style: Style::OhlcBars,
                show_volume: true,
                show_delta: true,
                depth: Some(self.depth_panel()),
                heatmap: self.show_heatmap.then(|| self.heatmap_view()),
                trades: None,
                annotations: Vec::new(),
                title: title.to_string(),
            },
        )
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.state.best_bid()
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.state.best_ask()
    }

    pub fn spread(&self) -> Option<f64> {
        self.state.spread()
    }

    /// `(bid_levels, ask_levels)` resting in the book.
    pub fn depth_level_counts(&self) -> (usize, usize) {
        self.state.depth_level_counts()
    }

    /// The visible price band (from the follow-latest viewport), if any.
    fn visible_price_extent(&self) -> Option<(f64, f64)> {
        cull(self.state.series(), &self.viewport()).price_extent()
    }

    /// The replay's heatmap (always enabled).
    fn heatmap(&self) -> &LiquidityHeatmap {
        self.state.heatmap().expect("heatmap enabled in replay")
    }

    /// Largest single-cell liquidity in the heatmap window.
    pub fn max_liquidity(&self) -> f64 {
        self.heatmap().max_liquidity()
    }

    /// Time buckets currently held in the heatmap window.
    pub fn heatmap_time_buckets(&self) -> usize {
        self.heatmap().time_buckets()
    }

    /// Distinct price buckets across the heatmap window.
    pub fn heatmap_price_buckets(&self) -> usize {
        self.heatmap().price_buckets()
    }

    /// Populated heatmap cells within the visible price band.
    pub fn visible_heatmap_cells(&self) -> usize {
        let (lo, hi) = self.visible_price_extent().unwrap_or((0.0, 1.0));
        self.heatmap().cells_in_price_range(lo, hi)
    }

    /// A render-ready side-aware heatmap view over the visible price band.
    pub fn heatmap_view(&self) -> HeatmapView {
        let (lo, hi) = self.visible_price_extent().unwrap_or((0.0, 1.0));
        let columns = self
            .heatmap()
            .columns_in_range_sided(lo, hi)
            .into_iter()
            .map(|col| {
                col.into_iter()
                    .map(|(p, l, side)| (p, l, side == BookSide::Bid))
                    .collect()
            })
            .collect();
        HeatmapView {
            columns,
            max_liquidity: self.max_liquidity(),
        }
    }

    /// A render-ready depth view: top-`depth_panel_levels` bids/asks + spread.
    pub fn depth_panel(&self) -> DepthPanel {
        let n = self.depth_panel_levels;
        let to_tuples = |side| {
            self.state
                .book()
                .top_n(side, n)
                .into_iter()
                .map(|l| (l.price, l.size))
                .collect()
        };
        DepthPanel {
            bids: to_tuples(BookSide::Bid),
            asks: to_tuples(BookSide::Ask),
            spread: self.spread(),
        }
    }

    /// The forming candle's order flow, or `None` before the first tick.
    pub fn current_flow(&self) -> Option<Flow> {
        self.state.current_flow()
    }

    /// Cumulative volume delta (CVD) across the whole series.
    pub fn cumulative_delta(&self) -> f64 {
        self.state.cumulative_delta()
    }

    /// A deterministic status string: the forming candle's OHLC, order-flow delta,
    /// CVD, and tick count (no timings, so snapshot frames built from it are
    /// reproducible).
    pub fn status(&self) -> String {
        match (self.current(), self.current_flow()) {
            (Some(c), Some(f)) => format!(
                "finchart LIVE · O {:.2} H {:.2} L {:.2} C {:.2} · Δ {:+.0} · CVD {:+.0} · {} · liq {:.0} cells {} · ticks {}",
                c.open,
                c.high,
                c.low,
                c.close,
                f.delta(),
                self.cumulative_delta(),
                self.book_status(),
                self.max_liquidity(),
                self.visible_heatmap_cells(),
                self.total_ticks()
            ),
            _ => "finchart LIVE · warming up".to_string(),
        }
    }

    /// Best bid / ask / spread fragment for the status line.
    fn book_status(&self) -> String {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => format!("bid {b:.2} ask {a:.2} spr {:.2}", a - b),
            _ => "bid — ask —".to_string(),
        }
    }

    pub fn series_len(&self) -> usize {
        self.state.series().len()
    }

    pub fn current(&self) -> Option<Candle> {
        self.state.current()
    }

    pub fn total_ticks(&self) -> usize {
        self.total_ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend_terminal::ColorMode;

    fn cfg() -> ReplayConfig {
        ReplayConfig {
            history: 100,
            window: 40,
            ..Default::default()
        }
    }

    #[test]
    fn replay_is_deterministic_for_same_config() {
        let mut a = Replay::new(&cfg());
        let mut b = Replay::new(&cfg());
        a.advance(500);
        b.advance(500);
        assert_eq!(
            a.render(120, 30, ColorMode::Plain, &a.status()),
            b.render(120, 30, ColorMode::Plain, &b.status())
        );
    }

    #[test]
    fn advancing_ticks_builds_new_candles() {
        let mut r = Replay::new(&cfg());
        let before = r.series_len();
        assert!(r.current().is_none()); // no forming candle before first tick
        r.advance(cfg().ticks_per_candle * 3 + 5);
        assert!(r.series_len() > before, "expected new candles");
        assert!(r.current().is_some(), "expected a forming candle");
    }

    #[test]
    fn viewport_follows_latest_candle() {
        let mut r = Replay::new(&cfg());
        r.advance(3_000);
        let vp = r.viewport();
        assert_eq!(vp.end_index, r.series_len()); // pinned to the newest candle
        assert!(vp.candle_count() <= cfg().window);
    }

    #[test]
    fn total_ticks_counts_ingested() {
        let mut r = Replay::new(&cfg());
        r.advance(123);
        assert_eq!(r.total_ticks(), 123);
    }

    #[test]
    fn current_flow_appears_after_first_tick() {
        let mut r = Replay::new(&cfg());
        assert!(r.current_flow().is_none());
        r.advance(50);
        assert!(r.current_flow().is_some());
    }

    #[test]
    fn status_includes_delta_and_cvd() {
        let mut r = Replay::new(&cfg());
        r.advance(500);
        let s = r.status();
        assert!(s.contains('Δ'), "status missing delta: {s}");
        assert!(s.contains("CVD"), "status missing cvd: {s}");
    }

    #[test]
    fn book_stays_non_crossing_during_replay() {
        let mut r = Replay::new(&cfg());
        r.advance(2_000);
        let (b, a) = (r.best_bid(), r.best_ask());
        assert!(b.is_some() && a.is_some(), "no book after replay");
        assert!(b.unwrap() < a.unwrap(), "crossed: {b:?} >= {a:?}");
    }

    #[test]
    fn status_includes_bid_ask_spread() {
        let mut r = Replay::new(&cfg());
        r.advance(500);
        let s = r.status();
        assert!(s.contains("bid"), "missing bid: {s}");
        assert!(s.contains("ask"), "missing ask: {s}");
        assert!(s.contains("spr"), "missing spread: {s}");
    }

    #[test]
    fn depth_panel_levels_are_sorted_best_first() {
        let mut r = Replay::new(&cfg());
        r.advance(500);
        let d = r.depth_panel();
        assert!(!d.bids.is_empty() && !d.asks.is_empty());
        assert!(
            d.bids.windows(2).all(|w| w[0].0 > w[1].0),
            "bids not descending"
        );
        assert!(
            d.asks.windows(2).all(|w| w[0].0 < w[1].0),
            "asks not ascending"
        );
    }

    #[test]
    fn heatmap_is_populated_after_replay() {
        let mut r = Replay::new(&cfg());
        r.advance(500);
        assert!(r.max_liquidity() > 0.0, "heatmap has no liquidity");
        assert!(r.visible_heatmap_cells() > 0, "no visible heatmap cells");
    }

    #[test]
    fn status_includes_liquidity_and_cells() {
        let mut r = Replay::new(&cfg());
        r.advance(500);
        let s = r.status();
        assert!(s.contains("liq"), "missing liq: {s}");
        assert!(s.contains("cells"), "missing cells: {s}");
    }

    #[test]
    fn heatmap_view_has_columns() {
        let mut r = Replay::new(&cfg());
        r.advance(500);
        let hv = r.heatmap_view();
        assert!(!hv.columns.is_empty(), "no heatmap columns");
        assert!(hv.max_liquidity > 0.0);
    }

    #[test]
    fn interactive_view_disables_heatmap_by_default() {
        // ReplayConfig::default() has show_heatmap = false (the normal viewer).
        let mut r = Replay::new(&ReplayConfig::default());
        r.advance(500);
        let out = r.render(120, 30, ColorMode::Plain, &r.status());
        assert!(
            !out.chars().any(|c| matches!(c, '░' | '▒' | '▓')),
            "heatmap rendered when disabled"
        );
        assert_eq!(out.lines().count(), 30);
    }

    #[test]
    fn heatmap_renders_when_enabled() {
        let mut c = cfg();
        c.show_heatmap = true;
        let mut r = Replay::new(&c);
        r.advance(500);
        let out = r.render(120, 30, ColorMode::Plain, &r.status());
        assert!(
            out.chars().any(|ch| matches!(ch, '░' | '▒' | '▓')),
            "heatmap not rendered when enabled"
        );
    }
}
