//! Adapter-level integration: translated protobuf messages fold through
//! `ChartState` with **no rendering** (scope 10: translation must not require
//! rendering, and the fill feed must drive candle aggregation).

use crate::pb::{FillNotification, Level2Response, PriceLevelData};
use crate::translate::{fill_to_trade, level2_to_snapshot};
use market_core::{CandleSeries, ChartState, MarketEvent};

fn state() -> ChartState {
    ChartState::from_history(CandleSeries::new(), 60, 0.05, Some(32), 16)
}

fn fill(price: f64, size: i32, aggressor_is_buy: bool) -> FillNotification {
    FillNotification {
        symbol: "AAPL".to_string(),
        price,
        size,
        buy: aggressor_is_buy,
        partial_fill: false,
        remaining_size: 0,
        timestamp_ns: 0,
        maker_order_id: 0,
        taker_order_id: 0,
        aggressor_is_buy,
    }
}

fn book() -> Level2Response {
    Level2Response {
        bids: vec![PriceLevelData {
            price: 99.95,
            size: 5,
            rank: 0,
        }],
        asks: vec![PriceLevelData {
            price: 100.05,
            size: 4,
            rank: 0,
        }],
    }
}

#[test]
fn translated_buy_fill_builds_a_candle_and_buy_flow() {
    let mut s = state();
    s.apply(&fill_to_trade(&fill(100.0, 4, true), 60));
    assert_eq!(s.series().len(), 1, "fill did not open a candle");
    let f = s.current_flow().unwrap();
    assert_eq!(f.buy_volume, 4.0);
    assert_eq!(f.sell_volume, 0.0);
    assert!(
        s.cumulative_delta() > 0.0,
        "buyer-initiated should lift CVD"
    );
}

#[test]
fn translated_sell_fill_records_sell_flow() {
    let mut s = state();
    s.apply(&fill_to_trade(&fill(100.0, 3, false), 60));
    let f = s.current_flow().unwrap();
    assert_eq!(f.sell_volume, 3.0);
    assert_eq!(f.buy_volume, 0.0);
    assert!(
        s.cumulative_delta() < 0.0,
        "seller-initiated should lower CVD"
    );
}

#[test]
fn translated_level2_updates_the_book() {
    let mut s = state();
    s.apply(&level2_to_snapshot(&book(), 1));
    assert!(s.best_bid().is_some_and(|b| (b - 99.95).abs() < 1e-6));
    assert!(s.best_ask().is_some_and(|a| (a - 100.05).abs() < 1e-6));
}

#[test]
fn heartbeat_after_depth_records_a_heatmap_column() {
    let mut s = state();
    s.apply(&level2_to_snapshot(&book(), 1));
    assert_eq!(s.heatmap().unwrap().time_buckets(), 0);
    s.apply(&MarketEvent::Heartbeat { ts: 1 });
    assert_eq!(s.heatmap().unwrap().time_buckets(), 1);
}

#[test]
fn translation_yields_events_without_a_chartstate_or_render() {
    // Pure translation needs neither a book nor a renderer.
    assert!(matches!(
        fill_to_trade(&fill(100.0, 1, true), 1),
        MarketEvent::Trade(_)
    ));
    let empty = Level2Response {
        bids: vec![],
        asks: vec![],
    };
    assert!(matches!(
        level2_to_snapshot(&empty, 1),
        MarketEvent::DepthSnapshot(_)
    ));
}
