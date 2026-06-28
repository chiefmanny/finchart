//! Deterministic synthetic OHLCV generator: a seeded geometric random walk on a
//! fixed time grid. Same `GenConfig` always produces the same `CandleSeries`.

use std::collections::VecDeque;

use crate::candle::{Candle, CandleSeries};
use crate::depth::{BookSide, DepthLevel, DepthSnapshot, DepthUpdate};
use crate::event::MarketEvent;
use crate::live::{Side, Tick};
use crate::rng::SplitMix64;

/// Parameters for [`generate`].
#[derive(Debug, Clone, PartialEq)]
pub struct GenConfig {
    /// PRNG seed. Same seed + same fields ⇒ identical output.
    pub seed: u64,
    /// Number of candles to produce.
    pub count: usize,
    /// Timestamp of the first candle (epoch seconds).
    pub start_ts: i64,
    /// Spacing between candles, in seconds (a regular grid — no market gaps yet).
    pub interval_secs: i64,
    /// Price of the first candle's open.
    pub start_price: f64,
    /// Per-candle relative move scale (e.g. 0.02 ≈ ±2% body + wick size).
    pub volatility: f64,
    /// Per-candle deterministic drift added to each return.
    pub drift: f64,
}

/// Generate a deterministic synthetic candle series from `cfg`.
///
/// Each candle's open is the previous candle's close (the first open is
/// `start_price`). The close is a bounded random step from the open; wicks
/// extend beyond the body so candles render with high/low whiskers.
pub fn generate(cfg: &GenConfig) -> CandleSeries {
    let mut series = CandleSeries::with_capacity(cfg.count);
    let mut rng = SplitMix64::new(cfg.seed);
    // Separate stream for flow so adding it leaves the OHLCV output unchanged.
    let mut flow_rng = SplitMix64::new(cfg.seed ^ 0x5A17_F10A_5A17_F10A);
    let mut open = cfg.start_price;

    const VOLUME_BASE: f64 = 1_000.0;

    for i in 0..cfg.count {
        // Body: bounded relative step in [drift - vol, drift + vol).
        let step = cfg.drift + cfg.volatility * (2.0 * rng.next_f64() - 1.0);
        let close = (open * (1.0 + step)).max(f64::MIN_POSITIVE);

        // Wicks extend past the body so high >= max(o,c) and low <= min(o,c).
        let body_high = open.max(close);
        let body_low = open.min(close);
        let high = body_high + open * cfg.volatility * rng.next_f64();
        let low = (body_low - open * cfg.volatility * rng.next_f64()).max(f64::MIN_POSITIVE);

        let volume = VOLUME_BASE * rng.next_f64();

        series.push(Candle {
            ts: cfg.start_ts + i as i64 * cfg.interval_secs,
            open,
            high,
            low,
            close,
            volume,
        });

        // Split volume into aggressive buy/sell, skewed by candle direction so
        // up candles are buy-heavy and down candles sell-heavy. buy + sell == volume.
        let bias = 0.1 + 0.3 * flow_rng.next_f64(); // [0.1, 0.4)
        let buy_fraction = if close >= open {
            0.5 + bias
        } else {
            0.5 - bias
        };
        let buy = volume * buy_fraction;
        series.add_flow_last(buy, volume - buy);

        open = close;
    }

    series
}

/// Parameters for the deterministic synthetic [`ticks`] stream.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveConfig {
    /// PRNG seed. Same config ⇒ identical tick sequence.
    pub seed: u64,
    /// Timestamp of the first tick (epoch seconds).
    pub start_ts: i64,
    /// Price the walk starts from.
    pub start_price: f64,
    /// Candle interval the ticks are spaced to fill.
    pub interval_secs: i64,
    /// Number of ticks per candle interval (≥ 1).
    pub ticks_per_candle: usize,
    /// Per-tick relative move scale.
    pub volatility: f64,
    /// Per-tick deterministic drift.
    pub drift: f64,
    /// Base traded size per tick.
    pub volume_per_tick: f64,
}

/// An endless deterministic stream of synthetic [`Tick`]s. Use `.take(n)` to
/// bound it. Ticks are spaced so that `ticks_per_candle` of them fill each
/// `interval_secs` window, producing intra-candle movement (expanding wicks).
pub struct TickStream {
    rng: SplitMix64,
    start_ts: i64,
    interval_secs: i64,
    ticks_per_candle: i64,
    volatility: f64,
    drift: f64,
    volume_per_tick: f64,
    index: i64,
    price: f64,
}

/// Create a [`TickStream`] from `cfg`.
pub fn ticks(cfg: &LiveConfig) -> TickStream {
    TickStream {
        rng: SplitMix64::new(cfg.seed),
        start_ts: cfg.start_ts,
        interval_secs: cfg.interval_secs,
        ticks_per_candle: cfg.ticks_per_candle.max(1) as i64,
        volatility: cfg.volatility,
        drift: cfg.drift,
        volume_per_tick: cfg.volume_per_tick,
        index: 0,
        price: cfg.start_price,
    }
}

impl Iterator for TickStream {
    type Item = Tick;

    fn next(&mut self) -> Option<Tick> {
        let prev = self.price;
        let step = self.drift + self.volatility * (2.0 * self.rng.next_f64() - 1.0);
        let price = (self.price * (1.0 + step)).max(f64::MIN_POSITIVE);
        self.price = price;

        // Spread ticks evenly across each interval: ticks_per_candle ticks fill
        // [start_ts + c*interval, start_ts + (c+1)*interval).
        let ts = self.start_ts + (self.index * self.interval_secs) / self.ticks_per_candle;
        let size = self.volume_per_tick * (0.5 + self.rng.next_f64());
        let side = Some(if price >= prev { Side::Buy } else { Side::Sell });

        self.index += 1;
        Some(Tick {
            ts,
            price,
            size,
            side,
        })
    }
}

/// Parameters for the deterministic synthetic depth generator [`DepthGen`].
#[derive(Debug, Clone, PartialEq)]
pub struct DepthConfig {
    pub seed: u64,
    pub tick_size: f64,
    /// Levels per side in a ladder.
    pub levels: usize,
    /// Base resting size at each level.
    pub base_size: f64,
    /// Fractional size jitter in `[0, 1)`.
    pub size_jitter: f64,
    /// Mid drift (price units) before the ladder is re-centered.
    pub recenter_band: f64,
    /// Level updates emitted per non-recentering step.
    pub updates_per_step: usize,
}

/// Deterministic synthetic Level 2 depth: builds a non-crossing ladder around a
/// mid and emits liquidity-shifting updates. Same seed + same mid sequence ⇒
/// identical book state.
pub struct DepthGen {
    rng: SplitMix64,
    cfg: DepthConfig,
    /// Current ladder center; `None` until the first snapshot.
    center: Option<f64>,
}

impl DepthGen {
    pub fn new(cfg: DepthConfig) -> Self {
        Self {
            rng: SplitMix64::new(cfg.seed),
            cfg,
            center: None,
        }
    }

    /// A jittered resting size, always positive.
    fn jittered_size(&mut self) -> f64 {
        self.cfg.base_size * (1.0 + self.cfg.size_jitter * (2.0 * self.rng.next_f64() - 1.0))
    }

    /// Build a fresh non-crossing ladder around `mid` (bids below, asks above),
    /// and set it as the new center.
    pub fn snapshot(&mut self, ts: i64, mid: f64) -> DepthSnapshot {
        let tick = self.cfg.tick_size;
        let mut bids = Vec::with_capacity(self.cfg.levels);
        let mut asks = Vec::with_capacity(self.cfg.levels);
        for i in 1..=self.cfg.levels {
            let off = i as f64 * tick;
            bids.push(DepthLevel {
                price: mid - off,
                size: self.jittered_size(),
            });
            asks.push(DepthLevel {
                price: mid + off,
                size: self.jittered_size(),
            });
        }
        self.center = Some(mid);
        DepthSnapshot { ts, bids, asks }
    }

    /// Advance one step, returning the depth delta to apply: a re-centering full
    /// snapshot if `mid` has drifted past the band, otherwise a deterministic batch
    /// of level updates (size jitter, occasional deletion) around the current
    /// center. Bids stay below center and asks above, so the book never crosses.
    /// Pure: the caller (e.g. [`ChartState`]) owns the book.
    ///
    /// [`ChartState`]: crate::ChartState
    pub fn step(&mut self, ts: i64, mid: f64) -> DepthDelta {
        let drift = self.center.map_or(f64::INFINITY, |c| (mid - c).abs());
        if drift > self.cfg.recenter_band {
            return DepthDelta::Snapshot(self.snapshot(ts, mid));
        }

        let center = self.center.expect("center set when not recentering");
        let tick = self.cfg.tick_size;
        let mut updates = Vec::with_capacity(self.cfg.updates_per_step);
        for _ in 0..self.cfg.updates_per_step {
            let side = if self.rng.next_f64() < 0.5 {
                BookSide::Bid
            } else {
                BookSide::Ask
            };
            let i =
                (1 + (self.rng.next_f64() * self.cfg.levels as f64) as usize).min(self.cfg.levels);
            let off = i as f64 * tick;
            let price = match side {
                BookSide::Bid => center - off,
                BookSide::Ask => center + off,
            };
            // Occasionally remove a level; otherwise refresh its size.
            let size = if self.rng.next_f64() < 0.1 {
                0.0
            } else {
                self.jittered_size()
            };
            updates.push(DepthUpdate {
                ts,
                side,
                price,
                size,
            });
        }
        DepthDelta::Updates(updates)
    }
}

/// The depth change produced by one [`DepthGen::step`]: a full re-centering
/// snapshot, or a batch of incremental level updates.
#[derive(Debug, Clone, PartialEq)]
pub enum DepthDelta {
    Snapshot(DepthSnapshot),
    Updates(Vec<DepthUpdate>),
}

/// Parameters for the synthetic [`events`] source.
#[derive(Debug, Clone, PartialEq)]
pub struct EventConfig {
    /// Tick (trade) stream config.
    pub live: LiveConfig,
    /// Depth generator config.
    pub depth: DepthConfig,
    /// Run a depth step every this many trades (≥ 1).
    pub depth_every: usize,
}

/// A deterministic synthetic [`MarketEvent`] stream: an initial seeding
/// `DepthSnapshot` + `Heartbeat`, then a `Trade` per tick, with a depth delta
/// (snapshot/updates) and a `Heartbeat` every `depth_every` trades. Endless — use
/// `.take(n)`. Folding it into a [`ChartState`] reproduces the live replay.
///
/// [`ChartState`]: crate::ChartState
pub struct EventSource {
    ticks: TickStream,
    depth: DepthGen,
    depth_every: usize,
    start_ts: i64,
    start_price: f64,
    tick_count: usize,
    seeded: bool,
    pending: VecDeque<MarketEvent>,
}

/// Create an [`EventSource`] from `cfg`.
pub fn events(cfg: &EventConfig) -> EventSource {
    EventSource {
        ticks: ticks(&cfg.live),
        depth: DepthGen::new(cfg.depth.clone()),
        depth_every: cfg.depth_every.max(1),
        start_ts: cfg.live.start_ts,
        start_price: cfg.live.start_price,
        tick_count: 0,
        seeded: false,
        pending: VecDeque::new(),
    }
}

impl EventSource {
    fn refill(&mut self) {
        if !self.seeded {
            self.seeded = true;
            let snap = self.depth.snapshot(self.start_ts, self.start_price);
            self.pending.push_back(MarketEvent::DepthSnapshot(snap));
            self.pending
                .push_back(MarketEvent::Heartbeat { ts: self.start_ts });
            return;
        }
        let Some(tick) = self.ticks.next() else {
            return;
        };
        let (ts, price) = (tick.ts, tick.price);
        self.tick_count += 1;
        self.pending.push_back(MarketEvent::Trade(tick));

        if self.tick_count.is_multiple_of(self.depth_every) {
            match self.depth.step(ts, price) {
                DepthDelta::Snapshot(s) => self.pending.push_back(MarketEvent::DepthSnapshot(s)),
                DepthDelta::Updates(us) => self
                    .pending
                    .extend(us.into_iter().map(MarketEvent::DepthUpdate)),
            }
            self.pending.push_back(MarketEvent::Heartbeat { ts });
        }
    }
}

impl Iterator for EventSource {
    type Item = MarketEvent;

    fn next(&mut self) -> Option<MarketEvent> {
        if self.pending.is_empty() {
            self.refill();
        }
        self.pending.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GenConfig {
        GenConfig {
            seed: 42,
            count: 1_000,
            start_ts: 1_600_000_000,
            interval_secs: 60,
            start_price: 100.0,
            volatility: 0.02,
            drift: 0.0,
        }
    }

    #[test]
    fn same_config_is_deterministic() {
        let a = generate(&cfg());
        let b = generate(&cfg());
        assert_eq!(a, b);
    }

    #[test]
    fn different_seed_changes_output() {
        let a = generate(&cfg());
        let b = generate(&GenConfig { seed: 43, ..cfg() });
        assert_ne!(a, b);
    }

    #[test]
    fn produces_requested_count() {
        let s = generate(&cfg());
        assert_eq!(s.len(), 1_000);
    }

    #[test]
    fn timestamps_lie_on_fixed_grid() {
        let c = cfg();
        let s = generate(&c);
        for i in 0..s.len() {
            assert_eq!(s.ts()[i], c.start_ts + i as i64 * c.interval_secs);
        }
    }

    #[test]
    fn first_open_is_start_price() {
        let c = cfg();
        let s = generate(&c);
        assert_eq!(s.get(0).unwrap().open, c.start_price);
    }

    #[test]
    fn open_equals_previous_close() {
        let s = generate(&cfg());
        for i in 1..s.len() {
            assert_eq!(s.open()[i], s.close()[i - 1]);
        }
    }

    #[test]
    fn ohlc_invariants_hold() {
        let s = generate(&cfg());
        for c in s.iter() {
            assert!(c.high >= c.open, "high < open: {c:?}");
            assert!(c.high >= c.close, "high < close: {c:?}");
            assert!(c.low <= c.open, "low > open: {c:?}");
            assert!(c.low <= c.close, "low > close: {c:?}");
            assert!(c.high >= c.low, "high < low: {c:?}");
            assert!(c.volume >= 0.0, "negative volume: {c:?}");
            assert!(c.low > 0.0, "non-positive price: {c:?}");
        }
    }

    #[test]
    fn zero_count_yields_empty_series() {
        let s = generate(&GenConfig { count: 0, ..cfg() });
        assert!(s.is_empty());
    }

    fn live_cfg() -> LiveConfig {
        LiveConfig {
            seed: 7,
            start_ts: 1_000,
            start_price: 100.0,
            interval_secs: 60,
            ticks_per_candle: 20,
            volatility: 0.002,
            drift: 0.0,
            volume_per_tick: 5.0,
        }
    }

    #[test]
    fn tick_stream_is_deterministic() {
        let a: Vec<_> = ticks(&live_cfg()).take(200).collect();
        let b: Vec<_> = ticks(&live_cfg()).take(200).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn different_seed_changes_tick_stream() {
        let a: Vec<_> = ticks(&live_cfg()).take(50).collect();
        let b: Vec<_> = ticks(&LiveConfig {
            seed: 8,
            ..live_cfg()
        })
        .take(50)
        .collect();
        assert_ne!(a, b);
    }

    #[test]
    fn ticks_are_grouped_into_candle_intervals() {
        let cfg = live_cfg();
        let collected: Vec<_> = ticks(&cfg).take(cfg.ticks_per_candle + 1).collect();
        // The first `ticks_per_candle` ticks fall inside the first interval.
        for t in &collected[..cfg.ticks_per_candle] {
            assert!(t.ts >= cfg.start_ts);
            assert!(t.ts < cfg.start_ts + cfg.interval_secs);
        }
        // The next tick crosses into the second interval.
        assert_eq!(
            collected[cfg.ticks_per_candle].ts,
            cfg.start_ts + cfg.interval_secs
        );
    }

    #[test]
    fn ticks_produce_intra_candle_movement() {
        let cfg = live_cfg();
        let first: Vec<f64> = ticks(&cfg)
            .take(cfg.ticks_per_candle)
            .map(|t| t.price)
            .collect();
        let min = first.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = first.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!(max > min, "no intra-candle movement: {first:?}");
    }

    #[test]
    fn ticks_have_positive_price_size_and_a_side() {
        for t in ticks(&live_cfg()).take(500) {
            assert!(t.price > 0.0, "non-positive price: {t:?}");
            assert!(t.size > 0.0, "non-positive size: {t:?}");
            assert!(t.side.is_some(), "missing side: {t:?}");
        }
    }

    #[test]
    fn tick_side_matches_price_movement() {
        let cfg = live_cfg();
        let mut prev = cfg.start_price;
        for t in ticks(&cfg).take(200) {
            let expected = if t.price >= prev {
                Side::Buy
            } else {
                Side::Sell
            };
            assert_eq!(t.side, Some(expected), "side mismatch at price {}", t.price);
            prev = t.price;
        }
    }

    #[test]
    fn generated_history_has_flow_summing_to_volume() {
        let s = generate(&cfg());
        let mut any_flow = false;
        for i in 0..s.len() {
            let (buy, sell, vol) = (s.buy_volume()[i], s.sell_volume()[i], s.volume()[i]);
            assert!((buy + sell - vol).abs() < 1e-6, "buy+sell != volume at {i}");
            if buy + sell > 0.0 {
                any_flow = true;
            }
        }
        assert!(any_flow, "history has no flow");
    }

    #[test]
    fn flow_skews_with_candle_direction() {
        let s = generate(&cfg());
        for i in 0..s.len() {
            if s.volume()[i] <= 0.0 {
                continue;
            }
            let (buy, sell) = (s.buy_volume()[i], s.sell_volume()[i]);
            if s.close()[i] > s.open()[i] {
                assert!(buy > sell, "up candle not buy-heavy at {i}");
            } else if s.close()[i] < s.open()[i] {
                assert!(sell > buy, "down candle not sell-heavy at {i}");
            }
        }
    }

    #[test]
    fn generated_flow_is_deterministic() {
        assert_eq!(generate(&cfg()).buy_volume(), generate(&cfg()).buy_volume());
        assert_eq!(
            generate(&cfg()).sell_volume(),
            generate(&cfg()).sell_volume()
        );
    }

    // --- synthetic depth -----------------------------------------------------

    use crate::depth::OrderBook;

    fn depth_cfg() -> DepthConfig {
        DepthConfig {
            seed: 5,
            tick_size: 0.1,
            levels: 8,
            base_size: 10.0,
            size_jitter: 0.5,
            recenter_band: 0.5,
            updates_per_step: 6,
        }
    }

    /// Apply a depth delta to a book (the consumer's job now that `step` is pure).
    fn apply(book: &mut OrderBook, delta: DepthDelta) {
        match delta {
            DepthDelta::Snapshot(s) => book.apply_snapshot(&s),
            DepthDelta::Updates(us) => {
                for u in us {
                    book.apply_update(&u);
                }
            }
        }
    }

    #[test]
    fn step_is_snapshot_first_then_updates() {
        let mut g = DepthGen::new(depth_cfg());
        assert!(matches!(g.step(0, 100.0), DepthDelta::Snapshot(_)));
        // A small move stays within the band → incremental updates.
        assert!(matches!(g.step(1, 100.01), DepthDelta::Updates(_)));
    }

    #[test]
    fn step_recenter_returns_snapshot() {
        let mut g = DepthGen::new(depth_cfg());
        let _ = g.step(0, 100.0);
        // A jump past the band → a fresh snapshot.
        assert!(matches!(g.step(1, 105.0), DepthDelta::Snapshot(_)));
    }

    #[test]
    fn depth_snapshot_is_a_non_crossing_ladder() {
        let mut g = DepthGen::new(depth_cfg());
        let snap = g.snapshot(0, 100.0);
        assert_eq!(snap.bids.len(), 8);
        assert_eq!(snap.asks.len(), 8);
        let max_bid = snap
            .bids
            .iter()
            .map(|l| l.price)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_ask = snap
            .asks
            .iter()
            .map(|l| l.price)
            .fold(f64::INFINITY, f64::min);
        assert!(max_bid < min_ask, "crossed: {max_bid} >= {min_ask}");
        assert!(snap.bids.iter().all(|l| l.size > 0.0));
        assert!(snap.asks.iter().all(|l| l.size > 0.0));
    }

    #[test]
    fn depth_book_stays_non_crossing_over_steps() {
        let mut g = DepthGen::new(depth_cfg());
        let mut book = OrderBook::new(0.1);
        let mut mid = 100.0;
        for k in 0..200 {
            mid += if k % 2 == 0 { 0.03 } else { -0.02 };
            apply(&mut book, g.step(k, mid));
            if let (Some(b), Some(a)) = (book.best_bid(), book.best_ask()) {
                assert!(b < a, "crossed at step {k}: {b} >= {a}");
            }
        }
    }

    #[test]
    fn depth_generation_is_deterministic() {
        let run = || {
            let mut g = DepthGen::new(depth_cfg());
            let mut book = OrderBook::new(0.1);
            let mut mid = 100.0;
            for k in 0..100 {
                mid += 0.01;
                apply(&mut book, g.step(k, mid));
            }
            book
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn depth_recenters_when_mid_drifts_past_the_band() {
        let mut g = DepthGen::new(depth_cfg());
        let mut book = OrderBook::new(0.1);
        apply(&mut book, g.step(0, 100.0)); // initial snapshot around 100
        apply(&mut book, g.step(1, 105.0)); // jump > band -> recenter around 105
        let bb = book.best_bid().unwrap();
        let ba = book.best_ask().unwrap();
        assert!(
            bb < 105.0 && ba > 105.0,
            "not recentered around 105: {bb}/{ba}"
        );
    }

    #[test]
    fn depth_sizes_stay_positive_in_the_book() {
        let mut g = DepthGen::new(depth_cfg());
        let mut book = OrderBook::new(0.1);
        let mut mid = 100.0;
        for k in 0..150 {
            mid += 0.005;
            apply(&mut book, g.step(k, mid));
        }
        for side in [BookSide::Bid, BookSide::Ask] {
            for lvl in book.top_n(side, 8) {
                assert!(lvl.size > 0.0, "non-positive resting size: {lvl:?}");
            }
        }
    }

    // --- synthetic event source ----------------------------------------------

    use crate::event::MarketEvent;
    use crate::state::ChartState;
    use crate::CandleSeries;

    fn event_cfg() -> EventConfig {
        EventConfig {
            live: live_cfg(),
            depth: DepthConfig {
                tick_size: 0.05,
                ..depth_cfg()
            },
            depth_every: 5,
        }
    }

    #[test]
    fn events_are_deterministic() {
        let a: Vec<_> = events(&event_cfg()).take(300).collect();
        let b: Vec<_> = events(&event_cfg()).take(300).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn events_start_with_a_seeding_snapshot_and_heartbeat() {
        let first: Vec<_> = events(&event_cfg()).take(2).collect();
        assert!(matches!(first[0], MarketEvent::DepthSnapshot(_)));
        assert!(matches!(first[1], MarketEvent::Heartbeat { .. }));
    }

    #[test]
    fn event_stream_contains_trades_and_heartbeats() {
        let evs: Vec<_> = events(&event_cfg()).take(200).collect();
        assert!(evs.iter().any(|e| matches!(e, MarketEvent::Trade(_))));
        assert!(
            evs.iter()
                .filter(|e| matches!(e, MarketEvent::Heartbeat { .. }))
                .count()
                >= 2
        );
    }

    #[test]
    fn events_fold_into_a_populated_chartstate() {
        let cfg = event_cfg();
        let mut state = ChartState::from_history(
            CandleSeries::new(),
            cfg.live.interval_secs,
            0.05,
            Some(64),
            32,
        );
        for ev in events(&cfg).take(400) {
            state.apply(&ev);
        }
        assert!(!state.series().is_empty(), "no candles");
        assert!(state.best_bid().is_some(), "book empty");
        assert!(state.heatmap().unwrap().time_buckets() > 0, "heatmap empty");

        // Same stream folds to the same state.
        let mut other = ChartState::from_history(
            CandleSeries::new(),
            cfg.live.interval_secs,
            0.05,
            Some(64),
            32,
        );
        for ev in events(&cfg).take(400) {
            other.apply(&ev);
        }
        assert_eq!(state, other);
    }
}
