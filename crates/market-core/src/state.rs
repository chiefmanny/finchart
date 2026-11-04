//! [`ChartState`] — the single ingestion sink that folds [`MarketEvent`]s into a
//! candle series, an order book, and (optionally) a liquidity heatmap.
//!
//! Every data source (synthetic generator, scenarios, file replay, future gRPC)
//! drives the chart through `apply`, so the rendering layer reads one place and the
//! ingestion logic lives here, not duplicated in each runner.

use std::collections::VecDeque;

use crate::candle::CandleSeries;
use crate::depth::OrderBook;
use crate::event::MarketEvent;
use crate::heatmap::LiquidityHeatmap;
use crate::live::{CandleBuilder, CandleUpdate, Flow, Tick};

/// Fold a recorded slice of events into `state` (record/replay).
pub fn replay_events(state: &mut ChartState, events: &[MarketEvent]) {
    for ev in events {
        state.apply(ev);
    }
}

/// What an [`ChartState::apply`] call did.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplyResult {
    /// A trade folded into the forming candle.
    Trade(CandleUpdate),
    /// The order book changed (snapshot or update).
    Depth,
    /// A time tick: the candle series advanced to its time (flat candles for
    /// quiet intervals) and a heatmap column was recorded if enabled.
    Heartbeat,
}

/// Owns the chart's evolving state and folds events into it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartState {
    builder: CandleBuilder,
    book: OrderBook,
    heatmap: Option<LiquidityHeatmap>,
    recent_trades: VecDeque<Tick>,
    trade_cap: usize,
}

impl ChartState {
    /// Build from preloaded candle history. `heatmap_capacity` enables a rolling
    /// heatmap window (sharing `tick_size`); `trade_cap` bounds the recent-trade
    /// ring used for trade markers.
    pub fn from_history(
        series: CandleSeries,
        interval_secs: i64,
        tick_size: f64,
        heatmap_capacity: Option<usize>,
        trade_cap: usize,
    ) -> Self {
        Self {
            builder: CandleBuilder::from_history(series, interval_secs),
            book: OrderBook::new(tick_size),
            heatmap: heatmap_capacity.map(|cap| LiquidityHeatmap::new(tick_size, cap)),
            recent_trades: VecDeque::new(),
            trade_cap,
        }
    }

    /// Fold one event into the state.
    pub fn apply(&mut self, ev: &MarketEvent) -> ApplyResult {
        match ev {
            MarketEvent::Trade(t) => {
                let update = self.builder.ingest(*t);
                self.recent_trades.push_back(*t);
                while self.recent_trades.len() > self.trade_cap {
                    self.recent_trades.pop_front();
                }
                ApplyResult::Trade(update)
            }
            MarketEvent::DepthSnapshot(s) => {
                self.book.apply_snapshot(s);
                ApplyResult::Depth
            }
            MarketEvent::DepthUpdate(u) => {
                self.book.apply_update(u);
                ApplyResult::Depth
            }
            MarketEvent::Heartbeat { ts } => {
                // Time passes even when nothing trades: keep the series time-linear.
                self.builder.advance_to(*ts);
                if let Some(heatmap) = &mut self.heatmap {
                    heatmap.record(&self.book, *ts);
                }
                ApplyResult::Heartbeat
            }
        }
    }

    /// Closed candles plus the forming candle (last element).
    pub fn series(&self) -> &CandleSeries {
        self.builder.series()
    }

    /// The currently-forming candle, or `None` before the first trade.
    pub fn current(&self) -> Option<crate::candle::Candle> {
        self.builder.current()
    }

    /// The order book.
    pub fn book(&self) -> &OrderBook {
        &self.book
    }

    /// The liquidity heatmap, if enabled.
    pub fn heatmap(&self) -> Option<&LiquidityHeatmap> {
        self.heatmap.as_ref()
    }

    /// Recent executed trades (oldest → newest), bounded by `trade_cap`.
    pub fn recent_trades(&self) -> &VecDeque<Tick> {
        &self.recent_trades
    }

    /// Aggressive buy/sell volume of the forming candle.
    pub fn current_flow(&self) -> Option<Flow> {
        self.builder.current_flow()
    }

    /// Cumulative volume delta over the whole series.
    pub fn cumulative_delta(&self) -> f64 {
        self.builder.cumulative_delta()
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.book.best_bid()
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.book.best_ask()
    }

    pub fn spread(&self) -> Option<f64> {
        self.book.spread()
    }

    /// `(bid_levels, ask_levels)` resting in the book.
    pub fn depth_level_counts(&self) -> (usize, usize) {
        self.book.level_counts()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candle::CandleSeries;
    use crate::depth::{BookSide, DepthLevel, DepthSnapshot, DepthUpdate};
    use crate::live::{Side, Tick};

    fn state(heatmap_cap: Option<usize>) -> ChartState {
        ChartState::from_history(CandleSeries::new(), 60, 0.05, heatmap_cap, 16)
    }

    fn trade(ts: i64, price: f64, size: f64, side: Option<Side>) -> MarketEvent {
        MarketEvent::Trade(Tick {
            ts,
            price,
            size,
            side,
        })
    }

    fn snapshot(ts: i64) -> MarketEvent {
        MarketEvent::DepthSnapshot(DepthSnapshot {
            ts,
            bids: vec![
                DepthLevel {
                    price: 99.95,
                    size: 5.0,
                },
                DepthLevel {
                    price: 99.90,
                    size: 3.0,
                },
            ],
            asks: vec![DepthLevel {
                price: 100.05,
                size: 4.0,
            }],
        })
    }

    #[test]
    fn trade_updates_candle_and_flow() {
        let mut s = state(None);
        let r = s.apply(&trade(60, 100.0, 2.0, Some(Side::Buy)));
        assert!(matches!(r, ApplyResult::Trade(_)));
        assert_eq!(s.series().len(), 1);
        let f = s.current_flow().unwrap();
        assert_eq!(f.buy_volume, 2.0);
        assert_eq!(f.sell_volume, 0.0);
        assert_eq!(s.recent_trades().len(), 1);
    }

    fn approx(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|v| (v - b).abs() < 1e-6)
    }

    #[test]
    fn depth_snapshot_and_update_change_book() {
        let mut s = state(None);
        assert!(matches!(s.apply(&snapshot(1)), ApplyResult::Depth));
        assert!(approx(s.best_bid(), 99.95));
        assert!(approx(s.best_ask(), 100.05));
        // size 0 deletes the best bid.
        let del = MarketEvent::DepthUpdate(DepthUpdate {
            ts: 2,
            side: BookSide::Bid,
            price: 99.95,
            size: 0.0,
        });
        assert!(matches!(s.apply(&del), ApplyResult::Depth));
        assert!(approx(s.best_bid(), 99.90));
    }

    #[test]
    fn heartbeat_records_heatmap_when_enabled() {
        let mut s = state(Some(8));
        s.apply(&snapshot(1));
        assert_eq!(s.heatmap().unwrap().time_buckets(), 0);
        assert!(matches!(
            s.apply(&MarketEvent::Heartbeat { ts: 1 }),
            ApplyResult::Heartbeat
        ));
        assert_eq!(s.heatmap().unwrap().time_buckets(), 1);
    }

    #[test]
    fn heartbeat_is_noop_without_heatmap() {
        let mut s = state(None);
        assert!(s.heatmap().is_none());
        assert!(matches!(
            s.apply(&MarketEvent::Heartbeat { ts: 1 }),
            ApplyResult::Heartbeat
        ));
        assert!(s.heatmap().is_none());
    }

    #[test]
    fn apply_results_match_event_kinds() {
        let mut s = state(Some(8));
        let seq = [
            trade(60, 100.0, 1.0, Some(Side::Sell)),
            snapshot(60),
            MarketEvent::DepthUpdate(DepthUpdate {
                ts: 60,
                side: BookSide::Ask,
                price: 100.10,
                size: 2.0,
            }),
            MarketEvent::Heartbeat { ts: 60 },
        ];
        let results: Vec<_> = seq.iter().map(|e| s.apply(e)).collect();
        assert!(matches!(results[0], ApplyResult::Trade(_)));
        assert_eq!(results[1], ApplyResult::Depth);
        assert_eq!(results[2], ApplyResult::Depth);
        assert_eq!(results[3], ApplyResult::Heartbeat);
    }

    #[test]
    fn same_event_stream_gives_same_final_state() {
        let events = vec![
            snapshot(1),
            trade(60, 100.0, 2.0, Some(Side::Buy)),
            MarketEvent::Heartbeat { ts: 60 },
            trade(61, 100.1, 1.0, Some(Side::Sell)),
            MarketEvent::Heartbeat { ts: 61 },
        ];
        let mut a = state(Some(8));
        let mut b = state(Some(8));
        for e in &events {
            a.apply(e);
        }
        for e in &events {
            b.apply(e);
        }
        assert_eq!(a, b);
    }

    #[test]
    fn recent_trades_are_bounded_by_cap() {
        let mut s = ChartState::from_history(CandleSeries::new(), 60, 0.05, None, 3);
        for i in 0..10 {
            s.apply(&trade(60 + i, 100.0, 1.0, Some(Side::Buy)));
        }
        assert_eq!(s.recent_trades().len(), 3);
    }

    // --- record / replay -----------------------------------------------------

    use crate::event::{parse_events, serialize_events};
    use crate::synth::{events, DepthConfig, EventConfig, LiveConfig};

    fn ev_cfg() -> EventConfig {
        EventConfig {
            live: LiveConfig {
                seed: 7,
                start_ts: 1_000,
                start_price: 100.0,
                interval_secs: 60,
                ticks_per_candle: 20,
                volatility: 0.002,
                drift: 0.0,
                volume_per_tick: 5.0,
            },
            depth: DepthConfig {
                seed: 5,
                tick_size: 0.05,
                levels: 8,
                base_size: 10.0,
                size_jitter: 0.5,
                recenter_band: 0.5,
                updates_per_step: 6,
            },
            depth_every: 5,
        }
    }

    fn fresh() -> ChartState {
        ChartState::from_history(CandleSeries::new(), 60, 0.05, Some(64), 32)
    }

    #[test]
    fn replay_events_folds_a_slice() {
        let evs: Vec<_> = events(&ev_cfg()).take(120).collect();
        let mut s = fresh();
        replay_events(&mut s, &evs);
        assert!(!s.series().is_empty());
        assert!(s.best_bid().is_some());
    }

    #[test]
    fn recorded_then_replayed_matches_direct_application() {
        let evs: Vec<_> = events(&ev_cfg()).take(400).collect();

        let mut direct = fresh();
        for e in &evs {
            direct.apply(e);
        }

        // Record to text, parse back, replay into a fresh state.
        let parsed = parse_events(&serialize_events(&evs));
        let mut replayed = fresh();
        replay_events(&mut replayed, &parsed);

        assert_eq!(direct, replayed);
    }

    #[test]
    fn heartbeat_advances_a_quiet_series_to_its_time() {
        let mut s = state(Some(8));
        s.apply(&trade(60, 100.0, 2.0, Some(Side::Buy)));
        // Three quiet minutes: the heartbeat opens flat candles up to 240.
        s.apply(&MarketEvent::Heartbeat { ts: 245 });
        let ts: Vec<i64> = s.series().ts().to_vec();
        assert_eq!(ts, vec![60, 120, 180, 240]);
        assert_eq!(s.current().unwrap().volume, 0.0);
        assert_eq!(s.cumulative_delta(), 2.0); // flat candles add no flow
        assert_eq!(s.heatmap().unwrap().time_buckets(), 1);
    }

    #[test]
    fn heartbeat_before_any_trade_opens_no_candle() {
        let mut s = state(Some(8));
        s.apply(&snapshot(1));
        s.apply(&MarketEvent::Heartbeat { ts: 600 });
        assert!(s.series().is_empty());
    }
}
