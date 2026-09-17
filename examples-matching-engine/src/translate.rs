//! Pure translation from matching-engine protobuf messages to neutral
//! `market_core::MarketEvent`s. No gRPC, no async, no rendering — every function
//! takes plain values (and an injected `ts`) so it is deterministic and unit-
//! testable offline.

use market_core::{DepthLevel, DepthSnapshot, MarketEvent, Side, Tick};

use crate::pb::{FillNotification, Level2Response};

/// Map the aggressor flag to a trade side.
///
/// Documented assumption: `buy == true` means the trade was **buyer-initiated**
/// (the aggressor lifted the ask). This must be reconciled against the real
/// service — some feeds encode `buy` as "the bid side was consumed", which is the
/// opposite. `invert` (from `--invert-trade-side`) flips it in one place.
pub fn fill_side(buy: bool, invert: bool) -> Side {
    if buy ^ invert {
        Side::Buy
    } else {
        Side::Sell
    }
}

/// Translate a `FillNotification` into a `MarketEvent::Trade` at `ts`.
///
/// `partial_fill` / `remaining_size` are informational and ignored for candle
/// aggregation in V1 — each fill is one executed trade of `size` at `price`.
pub fn fill_to_trade(fill: &FillNotification, ts: i64, invert: bool) -> MarketEvent {
    MarketEvent::Trade(Tick {
        ts,
        price: fill.price,
        size: fill.size as f64,
        side: Some(fill_side(fill.buy, invert)),
    })
}

/// Translate a `Level2Response` into a full-book `MarketEvent::DepthSnapshot` at
/// `ts`, preserving the response's best-first ordering. `rank` is not needed to
/// build the book (it is keyed by price tick) and is dropped.
pub fn level2_to_snapshot(resp: &Level2Response, ts: i64) -> MarketEvent {
    let levels = |src: &[crate::pb::PriceLevelData]| -> Vec<DepthLevel> {
        src.iter()
            .map(|l| DepthLevel {
                price: l.price,
                size: l.size as f64,
            })
            .collect()
    };
    MarketEvent::DepthSnapshot(DepthSnapshot {
        ts,
        bids: levels(&resp.bids),
        asks: levels(&resp.asks),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pb::{FillNotification, Level2Response, PriceLevelData};
    use market_core::{MarketEvent, Side};

    fn fill(buy: bool) -> FillNotification {
        FillNotification {
            symbol: "AAPL".to_string(),
            price: 100.25,
            size: 3,
            buy,
            partial_fill: true,
            remaining_size: 5,
        }
    }

    #[test]
    fn fill_side_default_is_buyer_initiated() {
        // Documented default: buy=true means buyer-initiated (aggressor buys).
        assert_eq!(fill_side(true, false), Side::Buy);
        assert_eq!(fill_side(false, false), Side::Sell);
    }

    #[test]
    fn fill_side_invert_flips_the_mapping() {
        assert_eq!(fill_side(true, true), Side::Sell);
        assert_eq!(fill_side(false, true), Side::Buy);
    }

    #[test]
    fn fill_to_trade_maps_all_fields() {
        let ev = fill_to_trade(&fill(true), 42, false);
        match ev {
            MarketEvent::Trade(t) => {
                assert_eq!(t.ts, 42);
                assert_eq!(t.price, 100.25);
                assert_eq!(t.size, 3.0);
                assert_eq!(t.side, Some(Side::Buy));
            }
            other => panic!("expected Trade, got {other:?}"),
        }
    }

    #[test]
    fn fill_to_trade_respects_invert() {
        let ev = fill_to_trade(&fill(true), 1, true);
        match ev {
            MarketEvent::Trade(t) => assert_eq!(t.side, Some(Side::Sell)),
            other => panic!("expected Trade, got {other:?}"),
        }
    }

    fn lvl(price: f64, size: i32, rank: i32) -> PriceLevelData {
        PriceLevelData { price, size, rank }
    }

    #[test]
    fn level2_maps_bids_and_asks_preserving_order() {
        let resp = Level2Response {
            bids: vec![lvl(100.00, 5, 0), lvl(99.90, 3, 1)],
            asks: vec![lvl(100.10, 4, 0), lvl(100.20, 2, 1)],
        };
        match level2_to_snapshot(&resp, 7) {
            MarketEvent::DepthSnapshot(s) => {
                assert_eq!(s.ts, 7);
                assert_eq!(s.bids.len(), 2);
                assert_eq!(s.asks.len(), 2);
                assert_eq!((s.bids[0].price, s.bids[0].size), (100.00, 5.0));
                assert_eq!(s.bids[1].price, 99.90);
                assert_eq!(s.asks[0].price, 100.10);
                assert_eq!(s.asks[1].price, 100.20);
            }
            other => panic!("expected DepthSnapshot, got {other:?}"),
        }
    }

    #[test]
    fn level2_ordering_is_best_first_by_rank() {
        // rank 0 = best; bids descend, asks ascend in price.
        let resp = Level2Response {
            bids: vec![lvl(100.00, 5, 0), lvl(99.90, 3, 1)],
            asks: vec![lvl(100.10, 4, 0), lvl(100.20, 2, 1)],
        };
        match level2_to_snapshot(&resp, 0) {
            MarketEvent::DepthSnapshot(s) => {
                assert!(
                    s.bids.windows(2).all(|w| w[0].price > w[1].price),
                    "bids not best-first"
                );
                assert!(
                    s.asks.windows(2).all(|w| w[0].price < w[1].price),
                    "asks not best-first"
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn level2_handles_an_empty_book() {
        let resp = Level2Response {
            bids: vec![],
            asks: vec![],
        };
        match level2_to_snapshot(&resp, 3) {
            MarketEvent::DepthSnapshot(s) => {
                assert_eq!(s.ts, 3);
                assert!(s.bids.is_empty());
                assert!(s.asks.is_empty());
            }
            _ => unreachable!(),
        }
    }
}
