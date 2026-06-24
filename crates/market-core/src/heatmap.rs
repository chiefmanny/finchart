//! A bounded time × price liquidity heatmap.
//!
//! Each `record` snapshots an [`OrderBook`]'s combined (bid + ask) resting
//! liquidity into a new time-bucket column, keyed by integer price *tick* (so the
//! price axis is absolute and handles drift). The window is a rolling deque capped
//! at `capacity`; price→row bucketing for rendering happens later against the live
//! price extent.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::depth::{BookSide, OrderBook};

/// One time bucket: combined liquidity per price tick.
#[derive(Debug, Clone, PartialEq)]
struct HeatmapColumn {
    ts: i64,
    liquidity: BTreeMap<i64, f64>,
}

/// A rolling window of liquidity columns (oldest → newest).
#[derive(Debug, Clone, PartialEq)]
pub struct LiquidityHeatmap {
    tick_size: f64,
    capacity: usize,
    columns: VecDeque<HeatmapColumn>,
}

impl LiquidityHeatmap {
    pub fn new(tick_size: f64, capacity: usize) -> Self {
        Self {
            tick_size,
            capacity: capacity.max(1),
            columns: VecDeque::new(),
        }
    }

    fn price_to_tick(&self, price: f64) -> i64 {
        (price / self.tick_size).round() as i64
    }

    /// The internal price-bucket key (tick) for a price.
    pub fn price_to_bucket(&self, price: f64) -> i64 {
        self.price_to_tick(price)
    }

    /// Snapshot `book`'s combined liquidity into a new time bucket, evicting the
    /// oldest column once the window exceeds `capacity`.
    pub fn record(&mut self, book: &OrderBook, ts: i64) {
        let mut liquidity: BTreeMap<i64, f64> = BTreeMap::new();
        for side in [BookSide::Bid, BookSide::Ask] {
            for lvl in book.levels(side) {
                *liquidity
                    .entry(self.price_to_tick(lvl.price))
                    .or_insert(0.0) += lvl.size;
            }
        }
        self.columns.push_back(HeatmapColumn { ts, liquidity });
        while self.columns.len() > self.capacity {
            self.columns.pop_front();
        }
    }

    /// Number of time buckets currently held.
    pub fn time_buckets(&self) -> usize {
        self.columns.len()
    }

    /// Distinct price ticks present across the window (price-axis extent).
    pub fn price_buckets(&self) -> usize {
        let mut ticks = BTreeSet::new();
        for col in &self.columns {
            ticks.extend(col.liquidity.keys().copied());
        }
        ticks.len()
    }

    /// Largest single-cell liquidity over the window.
    pub fn max_liquidity(&self) -> f64 {
        self.columns
            .iter()
            .flat_map(|c| c.liquidity.values().copied())
            .fold(0.0, f64::max)
    }

    /// Populated cells whose price falls within `[min, max]`.
    pub fn cells_in_price_range(&self, min: f64, max: f64) -> usize {
        let (lo, hi) = self.tick_range(min, max);
        self.columns
            .iter()
            .map(|c| c.liquidity.range(lo..=hi).count())
            .sum()
    }

    /// Render export: per column (oldest → newest), the `(price, liquidity)`
    /// levels within `[min, max]`.
    pub fn columns_in_range(&self, min: f64, max: f64) -> Vec<Vec<(f64, f64)>> {
        let (lo, hi) = self.tick_range(min, max);
        self.columns
            .iter()
            .map(|c| {
                c.liquidity
                    .range(lo..=hi)
                    .map(|(&t, &l)| (t as f64 * self.tick_size, l))
                    .collect()
            })
            .collect()
    }

    fn tick_range(&self, min: f64, max: f64) -> (i64, i64) {
        let (a, b) = (self.price_to_tick(min), self.price_to_tick(max));
        (a.min(b), a.max(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depth::{DepthLevel, DepthSnapshot, OrderBook};

    fn book() -> OrderBook {
        let mut b = OrderBook::new(0.05);
        let lvl = |price, size| DepthLevel { price, size };
        b.apply_snapshot(&DepthSnapshot {
            ts: 0,
            bids: vec![lvl(99.95, 5.0), lvl(99.90, 3.0), lvl(99.85, 2.0)],
            asks: vec![lvl(100.05, 4.0), lvl(100.10, 6.0), lvl(100.15, 1.0)],
        });
        b
    }

    #[test]
    fn record_appends_a_column() {
        let mut h = LiquidityHeatmap::new(0.05, 8);
        assert_eq!(h.time_buckets(), 0);
        h.record(&book(), 1);
        assert_eq!(h.time_buckets(), 1);
        h.record(&book(), 2);
        assert_eq!(h.time_buckets(), 2);
    }

    #[test]
    fn rolling_window_evicts_oldest_past_capacity() {
        let mut h = LiquidityHeatmap::new(0.05, 3);
        for ts in 0..5 {
            h.record(&book(), ts);
        }
        assert_eq!(h.time_buckets(), 3);
    }

    #[test]
    fn price_buckets_and_max_liquidity_reflect_recorded_levels() {
        let mut h = LiquidityHeatmap::new(0.05, 8);
        h.record(&book(), 1);
        assert_eq!(h.price_buckets(), 6); // 3 bids + 3 asks, distinct ticks
        assert_eq!(h.max_liquidity(), 6.0); // largest level size
    }

    #[test]
    fn price_to_bucket_steps_by_one_tick() {
        let h = LiquidityHeatmap::new(0.05, 8);
        assert_eq!(
            h.price_to_bucket(100.10) - h.price_to_bucket(100.05),
            1,
            "one tick apart should be one bucket apart"
        );
    }

    #[test]
    fn cells_in_price_range_filters_by_band() {
        let mut h = LiquidityHeatmap::new(0.05, 8);
        h.record(&book(), 1);
        assert_eq!(h.cells_in_price_range(99.0, 101.0), 6); // whole book
        assert_eq!(h.cells_in_price_range(99.88, 99.96), 2); // 99.90 + 99.95
    }

    #[test]
    fn columns_in_range_exports_levels_within_band() {
        let mut h = LiquidityHeatmap::new(0.05, 8);
        h.record(&book(), 1);
        let cols = h.columns_in_range(99.0, 101.0);
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].len(), 6);
        let narrow = h.columns_in_range(99.88, 99.96);
        assert_eq!(narrow[0].len(), 2);
    }

    #[test]
    fn heatmap_is_deterministic() {
        let mut a = LiquidityHeatmap::new(0.05, 8);
        let mut b = LiquidityHeatmap::new(0.05, 8);
        for ts in 0..5 {
            a.record(&book(), ts);
            b.record(&book(), ts);
        }
        assert_eq!(a, b);
    }
}
