//! Candle data types: the per-candle `Candle` (ergonomic accessor / test type)
//! and the columnar `CandleSeries` that is the render hot-path structure.

/// A single OHLCV candle.
///
/// Convenience/accessor type. The hot path stores data column-wise in
/// [`CandleSeries`]; this exists for ergonomic `get`/`iter`/`push` and tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candle {
    /// Candle open time (epoch seconds in the MVP).
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Columnar OHLCV store: one packed array per field.
///
/// Invariant: every column has the same length (`== self.len()`). All mutation
/// goes through [`CandleSeries::push`], which preserves it.
///
/// Order-flow columns (delta, bid/ask volume) would be added here when that
/// milestone arrives — out of scope for the MVP.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandleSeries {
    ts: Vec<i64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
}

impl CandleSeries {
    /// Create an empty series.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty series with capacity reserved in every column.
    pub fn with_capacity(n: usize) -> Self {
        Self {
            ts: Vec::with_capacity(n),
            open: Vec::with_capacity(n),
            high: Vec::with_capacity(n),
            low: Vec::with_capacity(n),
            close: Vec::with_capacity(n),
            volume: Vec::with_capacity(n),
        }
    }

    /// Number of candles.
    pub fn len(&self) -> usize {
        self.ts.len()
    }

    /// Whether the series holds no candles.
    pub fn is_empty(&self) -> bool {
        self.ts.is_empty()
    }

    /// Append a candle, preserving the equal-column-length invariant.
    pub fn push(&mut self, c: Candle) {
        self.ts.push(c.ts);
        self.open.push(c.open);
        self.high.push(c.high);
        self.low.push(c.low);
        self.close.push(c.close);
        self.volume.push(c.volume);
    }

    /// Borrow candle `i`, or `None` if out of bounds.
    pub fn get(&self, i: usize) -> Option<Candle> {
        if i >= self.len() {
            return None;
        }
        Some(Candle {
            ts: self.ts[i],
            open: self.open[i],
            high: self.high[i],
            low: self.low[i],
            close: self.close[i],
            volume: self.volume[i],
        })
    }

    /// Borrow the most recent candle, or `None` if empty.
    pub fn last(&self) -> Option<Candle> {
        self.get(self.len().checked_sub(1)?)
    }

    /// Overwrite the most recent candle's columns in place (used to mutate a
    /// currently-forming candle from live ticks). No-op if the series is empty.
    /// Preserves the equal-column-length invariant; OHLCV validity is the
    /// caller's responsibility.
    pub fn update_last(&mut self, c: Candle) {
        let Some(i) = self.len().checked_sub(1) else {
            return;
        };
        self.ts[i] = c.ts;
        self.open[i] = c.open;
        self.high[i] = c.high;
        self.low[i] = c.low;
        self.close[i] = c.close;
        self.volume[i] = c.volume;
    }

    /// Iterate candles in index order.
    pub fn iter(&self) -> impl Iterator<Item = Candle> + '_ {
        (0..self.len()).map(move |i| {
            // Safe: index is in `0..len`.
            self.get(i).expect("index in bounds")
        })
    }

    /// Zero-copy borrow of the timestamp column.
    pub fn ts(&self) -> &[i64] {
        &self.ts
    }
    pub fn open(&self) -> &[f64] {
        &self.open
    }
    pub fn high(&self) -> &[f64] {
        &self.high
    }
    pub fn low(&self) -> &[f64] {
        &self.low
    }
    pub fn close(&self) -> &[f64] {
        &self.close
    }
    pub fn volume(&self) -> &[f64] {
        &self.volume
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ts: i64) -> Candle {
        Candle {
            ts,
            open: 10.0,
            high: 12.0,
            low: 9.0,
            close: 11.0,
            volume: 100.0,
        }
    }

    #[test]
    fn new_series_is_empty() {
        let s = CandleSeries::new();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
    }

    #[test]
    fn push_then_get_round_trips() {
        let mut s = CandleSeries::new();
        let c = sample(1);
        s.push(c);
        assert_eq!(s.len(), 1);
        assert!(!s.is_empty());
        assert_eq!(s.get(0), Some(c));
    }

    #[test]
    fn get_out_of_bounds_is_none() {
        let s = CandleSeries::new();
        assert_eq!(s.get(0), None);
    }

    #[test]
    fn columns_stay_equal_length_after_pushes() {
        let mut s = CandleSeries::new();
        for i in 0..5 {
            s.push(sample(i));
        }
        assert_eq!(s.ts().len(), 5);
        assert_eq!(s.open().len(), 5);
        assert_eq!(s.high().len(), 5);
        assert_eq!(s.low().len(), 5);
        assert_eq!(s.close().len(), 5);
        assert_eq!(s.volume().len(), 5);
    }

    #[test]
    fn iter_yields_pushed_candles_in_order() {
        let mut s = CandleSeries::new();
        let cs = [sample(1), sample(2), sample(3)];
        for c in cs {
            s.push(c);
        }
        let collected: Vec<Candle> = s.iter().collect();
        assert_eq!(collected, cs);
    }

    #[test]
    fn last_is_none_when_empty() {
        assert_eq!(CandleSeries::new().last(), None);
    }

    #[test]
    fn last_returns_most_recent_candle() {
        let mut s = CandleSeries::new();
        s.push(sample(1));
        s.push(sample(2));
        assert_eq!(s.last(), Some(s.get(1).unwrap()));
    }

    #[test]
    fn update_last_overwrites_only_the_last_candle() {
        let mut s = CandleSeries::new();
        s.push(sample(1));
        s.push(sample(2));

        let updated = Candle {
            ts: 2,
            open: 50.0,
            high: 60.0,
            low: 40.0,
            close: 55.0,
            volume: 999.0,
        };
        s.update_last(updated);

        assert_eq!(s.len(), 2);
        assert_eq!(s.last(), Some(updated));
        assert_eq!(s.get(0), Some(sample(1))); // earlier candle untouched
    }

    #[test]
    fn update_last_on_empty_is_a_noop() {
        let mut s = CandleSeries::new();
        s.update_last(sample(1));
        assert!(s.is_empty());
    }
}
