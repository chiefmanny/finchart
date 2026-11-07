//! The pure (terminal-free) core of the live viewer: preload history, fold a
//! deterministic synthetic tick stream into candles, and render a follow-latest
//! frame. All logic here is unit-testable without a TTY.

use std::iter::Peekable;

use backend_terminal::{
    layout, render_chart, ChartOptions, ColorMode, DepthPanel, HeatmapView, Layout, Style,
};
use crossterm::event::MouseEvent;
use market_core::{
    events, generate, BookSide, Candle, ChartState, DepthConfig, EventConfig, EventSource, Flow,
    GenConfig, LiquidityHeatmap, LiveConfig, MarketEvent,
};
use render_core::{cull, Viewport};

use super::Action;
use crate::nav::{Navigator, ZOOM_IN, ZOOM_OUT};

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
    /// Follow-or-hold viewport plus the crosshair.
    nav: Navigator,
    /// The terminal size frames are drawn at, for mapping the crosshair and mouse.
    size: (usize, usize),
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
            nav: Navigator::default(),
            size: (120, 30),
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
        self.nav.viewport(self.series_len(), self.window)
    }

    /// Apply a key action; returns `false` when the view should quit.
    pub fn act(&mut self, action: Action) -> bool {
        let (len, window) = (self.series_len(), self.window);
        match action {
            Action::Quit => return false,
            Action::Escape => return self.nav.pointer.hide(),
            Action::ToggleCrosshair => self.nav.pointer.toggle(self.layout().as_ref()),
            Action::StepBack => self.nav.pointer.step(-1, self.layout().as_ref()),
            Action::StepForward => self.nav.pointer.step(1, self.layout().as_ref()),
            Action::PanLeft => self.nav.pan(-self.nav.pan_step(len, window), len, window),
            Action::PanRight => self.nav.pan(self.nav.pan_step(len, window), len, window),
            Action::ZoomIn => self.nav.zoom(ZOOM_IN, len, window),
            Action::ZoomOut => self.nav.zoom(ZOOM_OUT, len, window),
            Action::Follow => self.nav.follow(),
            Action::None => {}
        }
        true
    }

    /// Hover moves the crosshair, the wheel zooms, a left drag pans.
    pub fn mouse(&mut self, ev: MouseEvent) {
        let layout = self.layout();
        let (len, window) = (self.series_len(), self.window);
        self.nav.mouse(ev, layout.as_ref(), len, window);
    }

    /// Remember the terminal size frames are drawn at.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.size = (width, height);
    }

    fn layout(&self) -> Option<Layout> {
        let view = cull(self.state.series(), &self.viewport());
        let (w, h) = self.size;
        layout(&view, &self.options(w, h, ColorMode::Plain, ""))
    }

    /// Render the current frame at the given size and color mode, using `title`
    /// as the status line. Callers pass a deterministic title (e.g. [`status`])
    /// for snapshots, or one augmented with live timings for the interactive view.
    ///
    /// [`status`]: Replay::status
    pub fn render(&self, width: usize, height: usize, color: ColorMode, title: &str) -> String {
        let series = self.state.series();
        let view = cull(series, &self.viewport());
        render_chart(&view, &self.options(width, height, color, title))
    }

    fn options(&self, width: usize, height: usize, color: ColorMode, title: &str) -> ChartOptions {
        ChartOptions {
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
            crosshair: self.nav.pointer.crosshair,
            title: title.to_string(),
        }
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
        HeatmapView::from_heatmap(self.heatmap(), lo, hi)
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
        // The seeding heartbeat opened the first live interval: flat, no volume yet.
        assert_eq!(r.current().map(|c| c.volume), Some(0.0));
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
    fn current_flow_accumulates_from_the_first_tick() {
        let mut r = Replay::new(&cfg());
        let f = r.current_flow().expect("seeding heartbeat opened a candle");
        assert_eq!(f.buy_volume + f.sell_volume, 0.0);
        r.advance(50);
        let f = r.current_flow().unwrap();
        assert!(f.buy_volume + f.sell_volume > 0.0);
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

    // --- interaction: crosshair, pan/zoom, follow ---------------------------

    fn interactive() -> Replay {
        let mut r = Replay::new(&cfg());
        r.resize(120, 30);
        r.advance(400);
        r
    }

    fn status(r: &Replay) -> String {
        r.render(120, 30, ColorMode::Plain, &r.status())
            .lines()
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn the_crosshair_reads_out_a_bar_and_escape_hides_it_before_quitting() {
        let mut r = interactive();
        r.act(Action::ToggleCrosshair);
        let newest = status(&r);
        assert!(newest.starts_with("⌖"), "{newest}");
        r.act(Action::StepBack);
        assert_ne!(status(&r), newest);
        assert!(r.act(Action::Escape), "first Esc hides the crosshair");
        assert!(status(&r).starts_with("finchart LIVE"));
        assert!(!r.act(Action::Escape), "second Esc quits");
    }

    #[test]
    fn panning_holds_the_view_while_candles_keep_arriving() {
        let mut r = interactive();
        r.act(Action::PanLeft);
        let held = r.viewport();
        assert!(held.end_index < r.series_len());
        r.advance(300); // more candles arrive
        assert_eq!(r.viewport(), held, "a held view does not move");
        r.act(Action::Follow);
        assert_eq!(r.viewport().end_index, r.series_len());
    }

    #[test]
    fn the_wheel_zooms_out_of_follow_mode() {
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let mut r = interactive();
        let before = r.viewport().candle_count();
        r.mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 60,
            row: 8,
            modifiers: KeyModifiers::NONE,
        });
        assert!(r.viewport().candle_count() < before);
    }
}
