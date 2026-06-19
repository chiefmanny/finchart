//! Display-resolution level-of-detail: aggregate a visible window of candles
//! into a fixed number of OHLC bars when there are more candles than render
//! columns. Open/high/low/close/volume are preserved per bucket so no data is
//! lost to overplotting.

use crate::CulledView;

/// One aggregated OHLC(V) bar covering a contiguous run of candles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// Timestamp of the first candle in the bucket.
    pub start_ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Aggregate the visible candles into `min(view.len(), target_bars)` contiguous,
/// evenly-split OHLC buckets. `target_bars` is clamped to at least 1; an empty
/// view yields no bars.
///
/// Per bucket: `open` is the first candle's open, `close` the last candle's
/// close, `high`/`low` the extremes, `volume` the sum, and `start_ts` the first
/// candle's timestamp.
pub fn aggregate(view: &CulledView, target_bars: usize) -> Vec<Bar> {
    let n = view.len();
    if n == 0 {
        return Vec::new();
    }
    let buckets = target_bars.max(1).min(n);
    let mut bars = Vec::with_capacity(buckets);

    for b in 0..buckets {
        // Even split with the remainder spread across the leading buckets.
        let start = b * n / buckets;
        let end = (b + 1) * n / buckets;

        let mut high = view.high[start];
        let mut low = view.low[start];
        let mut volume = 0.0;
        for i in start..end {
            high = high.max(view.high[i]);
            low = low.min(view.low[i]);
            volume += view.volume[i];
        }

        bars.push(Bar {
            start_ts: view.ts[start],
            open: view.open[start],
            high,
            low,
            close: view.close[end - 1],
            volume,
        });
    }

    bars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cull, Viewport};
    use market_core::{generate, Candle, CandleSeries, GenConfig};

    fn c(ts: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Candle {
        Candle {
            ts,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    fn series_of(candles: &[Candle]) -> CandleSeries {
        let mut s = CandleSeries::new();
        for x in candles {
            s.push(*x);
        }
        s
    }

    fn gen(count: usize) -> CandleSeries {
        generate(&GenConfig {
            seed: 7,
            count,
            start_ts: 1_000,
            interval_secs: 60,
            start_price: 100.0,
            volatility: 0.02,
            drift: 0.0,
        })
    }

    #[test]
    fn single_bucket_is_overall_ohlc() {
        let s = series_of(&[
            c(0, 10.0, 15.0, 8.0, 12.0, 100.0),
            c(1, 12.0, 18.0, 11.0, 9.0, 50.0),
            c(2, 9.0, 14.0, 7.0, 13.0, 25.0),
        ]);
        let v = cull(&s, &Viewport::new(0, s.len()));
        let bars = aggregate(&v, 1);

        assert_eq!(bars.len(), 1);
        let b = bars[0];
        assert_eq!(b.start_ts, 0);
        assert_eq!(b.open, 10.0); // first open
        assert_eq!(b.close, 13.0); // last close
        assert_eq!(b.high, 18.0); // max high
        assert_eq!(b.low, 7.0); // min low
        assert_eq!(b.volume, 175.0); // sum
    }

    #[test]
    fn target_equal_to_count_is_identity() {
        let candles = [
            c(0, 10.0, 15.0, 8.0, 12.0, 100.0),
            c(1, 12.0, 18.0, 11.0, 9.0, 50.0),
            c(2, 9.0, 14.0, 7.0, 13.0, 25.0),
        ];
        let s = series_of(&candles);
        let v = cull(&s, &Viewport::new(0, s.len()));
        let bars = aggregate(&v, candles.len());

        assert_eq!(bars.len(), candles.len());
        for (b, cand) in bars.iter().zip(candles.iter()) {
            assert_eq!(b.start_ts, cand.ts);
            assert_eq!(b.open, cand.open);
            assert_eq!(b.high, cand.high);
            assert_eq!(b.low, cand.low);
            assert_eq!(b.close, cand.close);
            assert_eq!(b.volume, cand.volume);
        }
    }

    #[test]
    fn aggregated_bars_preserve_ohlc_invariants() {
        let s = gen(1_000);
        let v = cull(&s, &Viewport::new(0, s.len()));
        for target in [1usize, 7, 50, 333, 1_000] {
            for b in aggregate(&v, target) {
                assert!(b.high >= b.open, "high<open @ {target}");
                assert!(b.high >= b.close, "high<close @ {target}");
                assert!(b.low <= b.open, "low>open @ {target}");
                assert!(b.low <= b.close, "low>close @ {target}");
                assert!(b.high >= b.low, "high<low @ {target}");
            }
        }
    }

    #[test]
    fn volume_is_conserved_across_aggregation() {
        let s = gen(1_000);
        let v = cull(&s, &Viewport::new(0, s.len()));
        let total: f64 = v.volume.iter().sum();
        for target in [1usize, 13, 100, 1_000] {
            let agg: f64 = aggregate(&v, target).iter().map(|b| b.volume).sum();
            assert!((agg - total).abs() < 1e-6, "volume lost @ {target}");
        }
    }

    #[test]
    fn start_timestamps_are_strictly_increasing() {
        let s = gen(500);
        let v = cull(&s, &Viewport::new(0, s.len()));
        let bars = aggregate(&v, 37);
        for w in bars.windows(2) {
            assert!(w[1].start_ts > w[0].start_ts);
        }
    }

    #[test]
    fn bucket_count_is_min_of_count_and_target() {
        let s = gen(10);
        let v = cull(&s, &Viewport::new(0, s.len()));
        assert_eq!(aggregate(&v, 3).len(), 3); // downsample
        assert_eq!(aggregate(&v, 20).len(), 10); // can't exceed candle count
    }

    #[test]
    fn downsample_covers_full_range_end_to_end() {
        let s = gen(10);
        let v = cull(&s, &Viewport::new(0, s.len()));
        let bars = aggregate(&v, 3);
        assert_eq!(bars.first().unwrap().open, v.open[0]);
        assert_eq!(bars.last().unwrap().close, v.close[9]);
    }

    #[test]
    fn zero_target_is_clamped_to_one() {
        let s = gen(10);
        let v = cull(&s, &Viewport::new(0, s.len()));
        assert_eq!(aggregate(&v, 0).len(), 1);
    }

    #[test]
    fn empty_view_yields_no_bars() {
        let s = CandleSeries::new();
        let v = cull(&s, &Viewport::new(0, 0));
        assert!(aggregate(&v, 10).is_empty());
    }
}
