//! Live market events and incremental candle building.
//!
//! Neutral types (`Tick`, `Side`, `CandleUpdate`) plus a [`CandleBuilder`] that
//! folds a stream of ticks into a `CandleSeries`: it mutates the currently
//! forming candle in place and rolls over to a new candle on each interval
//! boundary. Data-source agnostic — synthetic, file, or (later) network feeds can
//! all produce `Tick`s.

use crate::candle::{Candle, CandleSeries};

/// Aggressor side of a trade, when known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

/// A single trade / price tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    /// Event time (epoch seconds).
    pub ts: i64,
    pub price: f64,
    /// Traded size; added to the forming candle's volume.
    pub size: f64,
    pub side: Option<Side>,
}

/// What ingesting a tick did to the candle series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CandleUpdate {
    /// The first forming candle was created.
    Opened,
    /// The currently-forming candle was mutated.
    Updated,
    /// An interval boundary was crossed: the prior candle closed (returned here)
    /// and a new forming candle opened.
    Rolled { closed: Candle },
}

/// Folds ticks into a `CandleSeries`. The forming candle is the last element of
/// the series, mutated in place via [`CandleSeries::update_last`].
pub struct CandleBuilder {
    series: CandleSeries,
    interval_secs: i64,
    /// Open time of the forming candle (or the next candle to open).
    current_open_ts: i64,
    /// Whether the last series element is a live forming candle (vs closed
    /// history before the first tick).
    forming: bool,
}

impl CandleBuilder {
    /// Start from preloaded history. The next candle opens one interval after the
    /// last historical candle.
    pub fn from_history(series: CandleSeries, interval_secs: i64) -> Self {
        let current_open_ts = series.last().map(|c| c.ts + interval_secs).unwrap_or(0);
        Self {
            series,
            interval_secs,
            current_open_ts,
            forming: false,
        }
    }

    /// Start empty; the first tick opens a candle at (or after) `start_ts`.
    pub fn new(interval_secs: i64, start_ts: i64) -> Self {
        Self {
            series: CandleSeries::new(),
            interval_secs,
            current_open_ts: start_ts,
            forming: false,
        }
    }

    /// The closed candles plus the forming candle (as the last element).
    pub fn series(&self) -> &CandleSeries {
        &self.series
    }

    /// The currently-forming candle, or `None` before the first tick.
    pub fn current(&self) -> Option<Candle> {
        if self.forming {
            self.series.last()
        } else {
            None
        }
    }

    /// Ingest a tick, mutating the forming candle (or rolling to a new one).
    pub fn ingest(&mut self, tick: Tick) -> CandleUpdate {
        if !self.forming {
            self.align_to(tick.ts);
            let open = self.series.last().map(|c| c.close).unwrap_or(tick.price);
            self.open_candle(open);
            self.apply(tick);
            self.forming = true;
            return CandleUpdate::Opened;
        }

        if tick.ts >= self.current_open_ts + self.interval_secs {
            let closed = self.series.last().expect("forming candle exists");
            self.align_to(tick.ts);
            self.open_candle(closed.close);
            self.apply(tick);
            return CandleUpdate::Rolled { closed };
        }

        self.apply(tick);
        CandleUpdate::Updated
    }

    /// Advance `current_open_ts` to the interval containing `ts` (skips empty
    /// intervals; synthetic streams don't produce gaps, but this stays correct).
    fn align_to(&mut self, ts: i64) {
        while ts >= self.current_open_ts + self.interval_secs {
            self.current_open_ts += self.interval_secs;
        }
    }

    /// Push a fresh candle seeded at `open` (high = low = close = open, vol 0).
    fn open_candle(&mut self, open: f64) {
        self.series.push(Candle {
            ts: self.current_open_ts,
            open,
            high: open,
            low: open,
            close: open,
            volume: 0.0,
        });
    }

    /// Fold `tick` into the forming candle.
    fn apply(&mut self, tick: Tick) {
        let mut c = self.series.last().expect("forming candle exists");
        c.high = c.high.max(tick.price);
        c.low = c.low.min(tick.price);
        c.close = tick.price;
        c.volume += tick.size;
        self.series.update_last(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(close: f64) -> CandleSeries {
        let mut s = CandleSeries::new();
        s.push(Candle {
            ts: 0,
            open: close,
            high: close,
            low: close,
            close,
            volume: 0.0,
        });
        s
    }

    fn tick(ts: i64, price: f64, size: f64) -> Tick {
        Tick {
            ts,
            price,
            size,
            side: None,
        }
    }

    #[test]
    fn from_history_has_no_forming_candle_until_first_tick() {
        let b = CandleBuilder::from_history(history(100.0), 60);
        assert_eq!(b.current(), None);
        assert_eq!(b.series().len(), 1);
    }

    #[test]
    fn first_tick_opens_a_candle_seeded_from_prior_close() {
        let mut b = CandleBuilder::from_history(history(100.0), 60);
        let upd = b.ingest(tick(60, 101.0, 10.0));
        assert_eq!(upd, CandleUpdate::Opened);
        assert_eq!(b.series().len(), 2);
        assert_eq!(
            b.current(),
            Some(Candle {
                ts: 60,
                open: 100.0, // prior close
                high: 101.0,
                low: 100.0,
                close: 101.0,
                volume: 10.0,
            })
        );
    }

    #[test]
    fn ticks_update_current_candle_ohlcv() {
        let mut b = CandleBuilder::from_history(history(100.0), 60);
        assert_eq!(b.ingest(tick(60, 101.0, 10.0)), CandleUpdate::Opened);
        assert_eq!(b.ingest(tick(63, 99.0, 5.0)), CandleUpdate::Updated);
        assert_eq!(b.ingest(tick(66, 102.0, 2.0)), CandleUpdate::Updated);

        let c = b.current().unwrap();
        assert_eq!(c.open, 100.0);
        assert_eq!(c.high, 102.0); // max tick price (and open)
        assert_eq!(c.low, 99.0); // min tick price
        assert_eq!(c.close, 102.0); // last tick price
        assert_eq!(c.volume, 17.0); // sum of sizes
    }

    #[test]
    fn crossing_interval_boundary_closes_and_appends() {
        let mut b = CandleBuilder::from_history(history(100.0), 60);
        b.ingest(tick(60, 101.0, 10.0));
        b.ingest(tick(63, 99.0, 5.0));
        b.ingest(tick(66, 102.0, 2.0));

        let upd = b.ingest(tick(120, 103.0, 1.0));
        let closed = Candle {
            ts: 60,
            open: 100.0,
            high: 102.0,
            low: 99.0,
            close: 102.0,
            volume: 17.0,
        };
        assert_eq!(upd, CandleUpdate::Rolled { closed });

        assert_eq!(b.series().len(), 3); // history + closed + new forming
        assert_eq!(b.series().get(1), Some(closed)); // closed candle persisted
        assert_eq!(
            b.current(),
            Some(Candle {
                ts: 120,
                open: 102.0, // prior close
                high: 103.0,
                low: 102.0,
                close: 103.0,
                volume: 1.0,
            })
        );
    }

    #[test]
    fn empty_builder_seeds_first_candle_from_first_tick() {
        let mut b = CandleBuilder::new(60, 60);
        assert_eq!(b.ingest(tick(60, 50.0, 3.0)), CandleUpdate::Opened);
        let c = b.current().unwrap();
        assert_eq!(c.open, 50.0); // no prior close -> first tick price
        assert_eq!(c.close, 50.0);
        assert_eq!(c.volume, 3.0);
    }

    #[test]
    fn ohlcv_invariants_hold_over_a_tick_sequence() {
        let mut b = CandleBuilder::from_history(history(100.0), 60);
        // Deterministic zig-zag of prices across several intervals.
        let mut price: f64 = 100.0;
        for k in 0..600i64 {
            price += if k % 3 == 0 { 1.5 } else { -1.0 };
            b.ingest(tick(60 + k, price.max(1.0), 1.0));
        }
        for c in b.series().iter() {
            assert!(c.high >= c.open, "high<open: {c:?}");
            assert!(c.high >= c.close, "high<close: {c:?}");
            assert!(c.low <= c.open, "low>open: {c:?}");
            assert!(c.low <= c.close, "low>close: {c:?}");
            assert!(c.high >= c.low, "high<low: {c:?}");
            assert!(c.volume >= 0.0, "negative volume: {c:?}");
        }
    }
}
