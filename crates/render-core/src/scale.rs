//! Linear scales mapping data values to terminal cell coordinates. Row 0 is the
//! top of the grid; column 0 is the left. Both clamp into range and guard their
//! degenerate cases (flat price range, single/zero visible candle).

/// Maps a price to a row index in `[0, rows)`. Higher prices map to smaller row
/// indices (top of the chart).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceScale {
    price_min: f64,
    price_max: f64,
    rows: usize,
}

impl PriceScale {
    pub fn new(price_min: f64, price_max: f64, rows: usize) -> Self {
        Self {
            price_min,
            price_max,
            rows,
        }
    }

    /// Map `price` to a row, clamped to `[0, rows - 1]`. A flat range
    /// (`price_max <= price_min`) maps every price to the middle row instead of
    /// dividing by zero.
    pub fn price_to_row(&self, price: f64) -> i32 {
        if self.rows == 0 {
            return 0;
        }
        let last = self.rows as i32 - 1;
        let span = self.price_max - self.price_min;
        if span <= 0.0 {
            return last / 2;
        }
        // 0.0 at price_max (top), 1.0 at price_min (bottom).
        let frac = (self.price_max - price) / span;
        let row = (frac * last as f64).round() as i32;
        row.clamp(0, last)
    }
}

/// Maps an absolute candle index in `[start_index, start_index + count)` to a
/// column in `[0, cols)`, stretching the visible candles across the width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeScale {
    start_index: usize,
    count: usize,
    cols: usize,
}

impl TimeScale {
    pub fn new(start_index: usize, count: usize, cols: usize) -> Self {
        Self {
            start_index,
            count,
            cols,
        }
    }

    /// Map absolute candle index `i` to a column, clamped to `[0, cols - 1]`.
    /// With a single visible candle (or zero width) the first column is used.
    pub fn index_to_col(&self, i: usize) -> i32 {
        if self.cols == 0 {
            return 0;
        }
        let last = self.cols as i32 - 1;
        if self.count <= 1 {
            return 0;
        }
        let rel = i.saturating_sub(self.start_index);
        let frac = rel as f64 / (self.count - 1) as f64;
        let col = (frac * last as f64).round() as i32;
        col.clamp(0, last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_max_maps_to_top_row() {
        let s = PriceScale::new(0.0, 100.0, 11);
        assert_eq!(s.price_to_row(100.0), 0);
    }

    #[test]
    fn price_min_maps_to_bottom_row() {
        let s = PriceScale::new(0.0, 100.0, 11);
        assert_eq!(s.price_to_row(0.0), 10);
    }

    #[test]
    fn price_midpoint_maps_to_middle_row() {
        let s = PriceScale::new(0.0, 100.0, 11);
        assert_eq!(s.price_to_row(50.0), 5);
    }

    #[test]
    fn price_above_extent_clamps_to_top() {
        let s = PriceScale::new(0.0, 100.0, 11);
        assert_eq!(s.price_to_row(150.0), 0);
    }

    #[test]
    fn price_below_extent_clamps_to_bottom() {
        let s = PriceScale::new(0.0, 100.0, 11);
        assert_eq!(s.price_to_row(-50.0), 10);
    }

    #[test]
    fn flat_series_does_not_panic_and_centers() {
        let s = PriceScale::new(42.0, 42.0, 11);
        assert_eq!(s.price_to_row(42.0), 5);
    }

    #[test]
    fn first_visible_index_maps_to_first_column() {
        let s = TimeScale::new(100, 50, 80);
        assert_eq!(s.index_to_col(100), 0);
    }

    #[test]
    fn last_visible_index_maps_to_last_column() {
        let s = TimeScale::new(100, 50, 80);
        assert_eq!(s.index_to_col(149), 79);
    }

    #[test]
    fn single_visible_candle_maps_to_first_column() {
        let s = TimeScale::new(7, 1, 80);
        assert_eq!(s.index_to_col(7), 0);
    }

    #[test]
    fn middle_visible_index_maps_to_middle_column() {
        let s = TimeScale::new(0, 11, 11);
        assert_eq!(s.index_to_col(5), 5);
    }
}
