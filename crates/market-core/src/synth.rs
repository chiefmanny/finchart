//! Deterministic synthetic OHLCV generator: a seeded geometric random walk on a
//! fixed time grid. Same `GenConfig` always produces the same `CandleSeries`.

use crate::candle::{Candle, CandleSeries};
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
}
