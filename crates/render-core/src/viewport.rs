//! Viewport selection and culling: pick a visible index window over a
//! `CandleSeries` and borrow just those columns (zero copy), plus the visible
//! price extent that the price scale autoscales to.

use market_core::CandleSeries;

/// Smallest window the user can zoom into, so candles never collapse to nothing.
pub const MIN_CANDLES: usize = 8;

/// A visible window over a series, as a half-open index range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub start_index: usize,
    pub end_index: usize,
}

impl Viewport {
    pub fn new(start_index: usize, end_index: usize) -> Self {
        Self {
            start_index,
            end_index,
        }
    }

    /// The window covering the whole series (used for reset).
    pub fn full(series_len: usize) -> Viewport {
        Viewport::new(0, series_len)
    }

    /// The window showing the most recent `window` candles, right-anchored to the
    /// newest candle (clamped to the series length). Use this to follow live data.
    pub fn tail(series_len: usize, window: usize) -> Viewport {
        let w = window.min(series_len);
        Viewport::new(series_len - w, series_len)
    }

    /// Number of candles in the window (0 if the range is inverted).
    pub fn candle_count(&self) -> usize {
        self.end_index.saturating_sub(self.start_index)
    }

    /// Clamp the window to a series of `series_len` candles: `end` is capped to
    /// the length and `start` is capped to the (clamped) end, so the result is
    /// always a valid, in-bounds range.
    pub fn clamp(self, series_len: usize) -> Viewport {
        let end = self.end_index.min(series_len);
        let start = self.start_index.min(end);
        Viewport {
            start_index: start,
            end_index: end,
        }
    }

    /// Shift the window by `delta` candles, preserving its width and staying
    /// within `[0, series_len]`. Panning past either edge stops at the edge.
    pub fn pan(self, delta: isize, series_len: usize) -> Viewport {
        let vp = self.clamp(series_len);
        let width = vp.candle_count();
        if series_len == 0 || width == 0 {
            return vp;
        }
        let max_start = (series_len - width) as isize;
        let start = (vp.start_index as isize + delta).clamp(0, max_start) as usize;
        Viewport::new(start, start + width)
    }

    /// Scale the window width by `factor` about its center (`factor < 1` zooms in,
    /// `> 1` zooms out). Width is clamped to `[MIN_CANDLES, series_len]` and the
    /// window is kept within bounds.
    pub fn zoom(self, factor: f64, series_len: usize) -> Viewport {
        let vp = self.clamp(series_len);
        if series_len == 0 {
            return vp;
        }
        let width = vp.candle_count().max(1);
        let center = vp.start_index as f64 + width as f64 / 2.0;

        let min_width = MIN_CANDLES.min(series_len);
        let new_width = ((width as f64 * factor).round() as usize).clamp(min_width, series_len);

        let max_start = (series_len - new_width) as isize;
        let start = (center - new_width as f64 / 2.0).round() as isize;
        let start = start.clamp(0, max_start) as usize;
        Viewport::new(start, start + new_width)
    }
}

/// A zero-copy borrow of the visible candle columns.
#[derive(Debug, Clone, Copy)]
pub struct CulledView<'a> {
    pub ts: &'a [i64],
    pub open: &'a [f64],
    pub high: &'a [f64],
    pub low: &'a [f64],
    pub close: &'a [f64],
    pub volume: &'a [f64],
}

impl CulledView<'_> {
    /// Number of visible candles.
    pub fn len(&self) -> usize {
        self.ts.len()
    }

    /// Whether the view is empty.
    pub fn is_empty(&self) -> bool {
        self.ts.is_empty()
    }

    /// The `(min_low, max_high)` price extent over the visible candles, or
    /// `None` if the view is empty. This is what the price scale autoscales to,
    /// so the axis tracks the viewport rather than the whole dataset.
    pub fn price_extent(&self) -> Option<(f64, f64)> {
        if self.is_empty() {
            return None;
        }
        let min = self.low.iter().copied().fold(f64::INFINITY, f64::min);
        let max = self.high.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some((min, max))
    }
}

/// Select the visible candles from `series` for `vp`, borrowing their columns.
/// The viewport is clamped to the series bounds first, so out-of-range or
/// inverted windows yield a valid (possibly empty) view.
pub fn cull<'a>(series: &'a CandleSeries, vp: &Viewport) -> CulledView<'a> {
    let vp = vp.clamp(series.len());
    let r = vp.start_index..vp.end_index;
    CulledView {
        ts: &series.ts()[r.clone()],
        open: &series.open()[r.clone()],
        high: &series.high()[r.clone()],
        low: &series.low()[r.clone()],
        close: &series.close()[r.clone()],
        volume: &series.volume()[r],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_core::{generate, GenConfig};

    fn series(count: usize) -> market_core::CandleSeries {
        generate(&GenConfig {
            seed: 1,
            count,
            start_ts: 0,
            interval_secs: 60,
            start_price: 100.0,
            volatility: 0.02,
            drift: 0.0,
        })
    }

    #[test]
    fn candle_count_is_range_width() {
        let vp = Viewport::new(10, 25);
        assert_eq!(vp.candle_count(), 15);
    }

    #[test]
    fn candle_count_saturates_when_inverted() {
        let vp = Viewport::new(30, 10);
        assert_eq!(vp.candle_count(), 0);
    }

    #[test]
    fn clamp_caps_end_to_series_len() {
        let vp = Viewport::new(5, 1_000).clamp(100);
        assert_eq!(vp.start_index, 5);
        assert_eq!(vp.end_index, 100);
    }

    #[test]
    fn clamp_caps_start_to_end() {
        let vp = Viewport::new(500, 1_000).clamp(100);
        assert_eq!(vp.start_index, 100);
        assert_eq!(vp.end_index, 100);
        assert_eq!(vp.candle_count(), 0);
    }

    #[test]
    fn cull_returns_exact_requested_range() {
        let s = series(1_000);
        let view = cull(&s, &Viewport::new(100, 150));
        assert_eq!(view.len(), 50);
        assert_eq!(view.ts[0], s.ts()[100]);
        assert_eq!(view.ts[49], s.ts()[149]);
    }

    #[test]
    fn cull_clamps_out_of_bounds_range() {
        let s = series(100);
        let view = cull(&s, &Viewport::new(80, 1_000));
        assert_eq!(view.len(), 20);
        assert_eq!(view.ts[19], s.ts()[99]);
    }

    #[test]
    fn cull_empty_viewport_is_empty() {
        let s = series(100);
        let view = cull(&s, &Viewport::new(50, 50));
        assert!(view.is_empty());
        assert_eq!(view.price_extent(), None);
    }

    #[test]
    fn price_extent_tracks_visible_slice_only() {
        let mut s = market_core::CandleSeries::new();
        // Tall candle outside the viewport; short candles inside it.
        s.push(market_core::Candle {
            ts: 0,
            open: 1.0,
            high: 999.0,
            low: 0.5,
            close: 1.0,
            volume: 0.0,
        });
        s.push(market_core::Candle {
            ts: 1,
            open: 10.0,
            high: 12.0,
            low: 8.0,
            close: 11.0,
            volume: 0.0,
        });
        s.push(market_core::Candle {
            ts: 2,
            open: 11.0,
            high: 15.0,
            low: 7.0,
            close: 9.0,
            volume: 0.0,
        });

        let view = cull(&s, &Viewport::new(1, 3));
        assert_eq!(view.price_extent(), Some((7.0, 15.0)));
    }

    #[test]
    fn full_covers_whole_series() {
        assert_eq!(Viewport::full(100), Viewport::new(0, 100));
        assert_eq!(Viewport::full(0), Viewport::new(0, 0));
    }

    #[test]
    fn pan_shifts_preserving_width() {
        assert_eq!(Viewport::new(10, 20).pan(5, 100), Viewport::new(15, 25));
        assert_eq!(Viewport::new(10, 20).pan(-5, 100), Viewport::new(5, 15));
    }

    #[test]
    fn pan_right_clamps_at_end_preserving_width() {
        let vp = Viewport::new(90, 100).pan(50, 100);
        assert_eq!(vp, Viewport::new(90, 100));
        assert_eq!(vp.candle_count(), 10);
    }

    #[test]
    fn pan_left_clamps_at_start_preserving_width() {
        let vp = Viewport::new(5, 15).pan(-50, 100);
        assert_eq!(vp, Viewport::new(0, 10));
        assert_eq!(vp.candle_count(), 10);
    }

    #[test]
    fn zoom_keeps_center() {
        assert_eq!(Viewport::new(40, 60).zoom(0.5, 100), Viewport::new(45, 55));
        assert_eq!(Viewport::new(45, 55).zoom(2.0, 100), Viewport::new(40, 60));
    }

    #[test]
    fn zoom_out_saturates_to_full_range() {
        assert_eq!(Viewport::new(40, 60).zoom(10.0, 100), Viewport::new(0, 100));
    }

    #[test]
    fn zoom_in_respects_min_candles() {
        let vp = Viewport::new(0, 100).zoom(0.01, 100);
        assert_eq!(vp.candle_count(), MIN_CANDLES);
        // Centered on the old midpoint (50).
        assert_eq!(
            vp,
            Viewport::new(50 - MIN_CANDLES / 2, 50 + MIN_CANDLES / 2)
        );
    }

    #[test]
    fn zoom_in_on_small_series_caps_at_series_len() {
        // Series shorter than MIN_CANDLES: can't widen past the data.
        assert_eq!(Viewport::new(0, 5).zoom(0.1, 5), Viewport::new(0, 5));
    }

    #[test]
    fn tail_shows_the_last_window_candles() {
        assert_eq!(Viewport::tail(100, 20), Viewport::new(80, 100));
        assert_eq!(Viewport::tail(100, 20).candle_count(), 20);
    }

    #[test]
    fn tail_clamps_window_to_series_length() {
        assert_eq!(Viewport::tail(10, 50), Viewport::new(0, 10));
    }

    #[test]
    fn tail_follows_the_latest_candle_as_series_grows() {
        // End always pins to the series length (the newest candle).
        assert_eq!(Viewport::tail(100, 20).end_index, 100);
        assert_eq!(Viewport::tail(150, 20), Viewport::new(130, 150));
    }

    #[test]
    fn tail_handles_empty_and_zero_window() {
        assert_eq!(Viewport::tail(0, 20), Viewport::new(0, 0));
        assert_eq!(Viewport::tail(100, 0), Viewport::new(100, 100));
    }
}
