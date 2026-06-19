//! The pure (terminal-free) core of the live viewer: preload history, fold a
//! deterministic synthetic tick stream into candles, and render a follow-latest
//! frame. All logic here is unit-testable without a TTY.

use backend_terminal::{render_chart, ChartOptions, ColorMode, Style};
use market_core::{generate, ticks, Candle, CandleBuilder, GenConfig, LiveConfig, TickStream};
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
        }
    }
}

/// A running live replay: closed history + a forming candle, fed by ticks.
pub struct Replay {
    builder: CandleBuilder,
    stream: TickStream,
    window: usize,
    total_ticks: usize,
}

impl Replay {
    /// Preload history and prepare the live tick stream (continuous with the
    /// last historical candle).
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

        let builder = CandleBuilder::from_history(history, cfg.interval_secs);
        let stream = ticks(&LiveConfig {
            seed: cfg.seed ^ 0x9E37_79B9_7F4A_7C15,
            start_ts: live_ts,
            start_price: live_price,
            interval_secs: cfg.interval_secs,
            ticks_per_candle: cfg.ticks_per_candle,
            volatility: cfg.tick_volatility,
            drift: cfg.drift,
            volume_per_tick: cfg.volume_per_tick,
        });

        Self {
            builder,
            stream,
            window: cfg.window,
            total_ticks: 0,
        }
    }

    /// Ingest the next `n` ticks.
    pub fn advance(&mut self, n: usize) {
        for _ in 0..n {
            if let Some(tick) = self.stream.next() {
                self.builder.ingest(tick);
                self.total_ticks += 1;
            }
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
        let series = self.builder.series();
        let view = cull(series, &self.viewport());
        render_chart(
            &view,
            &ChartOptions {
                width,
                height,
                color,
                style: Style::OhlcBars,
                show_volume: true,
                title: title.to_string(),
            },
        )
    }

    /// A deterministic status string: the forming candle's OHLC and tick count
    /// (no timings, so snapshot frames built from it are reproducible).
    pub fn status(&self) -> String {
        match self.current() {
            Some(c) => format!(
                "finchart LIVE · O {:.2} H {:.2} L {:.2} C {:.2} · ticks {}",
                c.open,
                c.high,
                c.low,
                c.close,
                self.total_ticks()
            ),
            None => "finchart LIVE · warming up".to_string(),
        }
    }

    pub fn series_len(&self) -> usize {
        self.builder.series().len()
    }

    pub fn current(&self) -> Option<Candle> {
        self.builder.current()
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
}
