//! A bounded time × price liquidity heatmap.
//!
//! Each `record` snapshots an [`OrderBook`]'s combined (bid + ask) resting
//! liquidity into a new time-bucket column, keyed by integer price *tick* (so the
//! price axis is absolute and handles drift). The window is a rolling deque capped
//! at `capacity`; price→row bucketing for rendering happens later against the live
//! price extent.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::depth::{BookSide, OrderBook};

/// One time bucket: resting liquidity per price tick, kept per side so the
/// renderer can color bids and asks distinctly. Combined liquidity is `bid + ask`.
#[derive(Debug, Clone, PartialEq)]
struct HeatmapColumn {
    ts: i64,
    bid: BTreeMap<i64, f64>,
    ask: BTreeMap<i64, f64>,
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
        let mut bid: BTreeMap<i64, f64> = BTreeMap::new();
        for lvl in book.levels(BookSide::Bid) {
            *bid.entry(self.price_to_tick(lvl.price)).or_insert(0.0) += lvl.size;
        }
        let mut ask: BTreeMap<i64, f64> = BTreeMap::new();
        for lvl in book.levels(BookSide::Ask) {
            *ask.entry(self.price_to_tick(lvl.price)).or_insert(0.0) += lvl.size;
        }
        self.columns.push_back(HeatmapColumn { ts, bid, ask });
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
            ticks.extend(col.bid.keys().copied());
            ticks.extend(col.ask.keys().copied());
        }
        ticks.len()
    }

    /// Largest single-cell liquidity over the window.
    pub fn max_liquidity(&self) -> f64 {
        self.columns
            .iter()
            .flat_map(|c| c.bid.values().chain(c.ask.values()).copied())
            .fold(0.0, f64::max)
    }

    /// Populated cells whose price falls within `[min, max]`.
    pub fn cells_in_price_range(&self, min: f64, max: f64) -> usize {
        let (lo, hi) = self.tick_range(min, max);
        self.columns
            .iter()
            .map(|c| c.bid.range(lo..=hi).count() + c.ask.range(lo..=hi).count())
            .sum()
    }

    /// Render export: per column (oldest → newest), the `(price, liquidity)`
    /// levels (both sides combined) within `[min, max]`, price-sorted.
    pub fn columns_in_range(&self, min: f64, max: f64) -> Vec<Vec<(f64, f64)>> {
        self.columns_in_range_sided(min, max)
            .into_iter()
            .map(|(_, col)| col.into_iter().map(|(p, l, _)| (p, l)).collect())
            .collect()
    }

    /// Side-aware render export: per column (oldest → newest), its recording time
    /// and the `(price, liquidity, side)` levels within `[min, max]`, price-sorted
    /// (bids and asks interleaved by price). The timestamp lets a renderer place
    /// each column under the candle whose interval contains it.
    #[allow(clippy::type_complexity)]
    pub fn columns_in_range_sided(
        &self,
        min: f64,
        max: f64,
    ) -> Vec<(i64, Vec<(f64, f64, BookSide)>)> {
        let (lo, hi) = self.tick_range(min, max);
        self.columns
            .iter()
            .map(|c| {
                let mut cells: Vec<(i64, f64, BookSide)> = c
                    .bid
                    .range(lo..=hi)
                    .map(|(&t, &l)| (t, l, BookSide::Bid))
                    .chain(c.ask.range(lo..=hi).map(|(&t, &l)| (t, l, BookSide::Ask)))
                    .collect();
                cells.sort_by_key(|(t, _, _)| *t);
                let cells = cells
                    .into_iter()
                    .map(|(t, l, s)| (t as f64 * self.tick_size, l, s))
                    .collect();
                (c.ts, cells)
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

    #[test]
    fn columns_in_range_sided_keeps_each_columns_timestamp() {
        let mut h = LiquidityHeatmap::new(0.05, 8);
        h.record(&book(), 1);
        h.record(&book(), 5);
        let ts: Vec<i64> = h
            .columns_in_range_sided(99.0, 101.0)
            .iter()
            .map(|(ts, _)| *ts)
            .collect();
        assert_eq!(ts, vec![1, 5]);
    }

    #[test]
    fn columns_in_range_sided_tags_each_cell_with_its_side() {
        let mut h = LiquidityHeatmap::new(0.05, 8);
        h.record(&book(), 1);
        let cols = h.columns_in_range_sided(99.0, 101.0);
        assert_eq!(cols.len(), 1);
        let cells = &cols[0].1;
        assert_eq!(cells.len(), 6);

        let bids: Vec<f64> = cells
            .iter()
            .filter(|(_, _, s)| *s == BookSide::Bid)
            .map(|(p, _, _)| *p)
            .collect();
        let asks: Vec<f64> = cells
            .iter()
            .filter(|(_, _, s)| *s == BookSide::Ask)
            .map(|(p, _, _)| *p)
            .collect();
        assert_eq!(bids.len(), 3);
        assert_eq!(asks.len(), 3);

        // Bids sit below asks (non-crossing).
        let max_bid = bids.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min_ask = asks.iter().cloned().fold(f64::INFINITY, f64::min);
        assert!(max_bid < min_ask, "{max_bid} >= {min_ask}");
    }
}
