//! Level 2 market depth: neutral feed events and an order book.
//!
//! The book is keyed internally by integer price *ticks* (`round(price /
//! tick_size)`), so levels match exactly across updates and each side stays
//! sorted for free via `BTreeMap`. Prices are `f64` at the API boundary. The book
//! is a faithful mirror of whatever feed drives it — synthetic, file, or network.

use std::collections::BTreeMap;

/// Which side of the book a level/update belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookSide {
    Bid,
    Ask,
}

/// A single price level: a resting size at a price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthLevel {
    pub price: f64,
    pub size: f64,
}

/// A full-book snapshot (replaces book state).
#[derive(Debug, Clone, PartialEq)]
pub struct DepthSnapshot {
    pub ts: i64,
    pub bids: Vec<DepthLevel>,
    pub asks: Vec<DepthLevel>,
}

/// An incremental level update. `size == 0` deletes the level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthUpdate {
    pub ts: i64,
    pub side: BookSide,
    pub price: f64,
    pub size: f64,
}

/// Best bid / best ask, either possibly absent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BestBidAsk {
    pub bid: Option<f64>,
    pub ask: Option<f64>,
}

impl BestBidAsk {
    /// `ask - bid`, or `None` if either side is empty.
    pub fn spread(&self) -> Option<f64> {
        Some(self.ask? - self.bid?)
    }
}

/// A two-sided order book keyed by integer price ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderBook {
    tick_size: f64,
    bids: BTreeMap<i64, f64>,
    asks: BTreeMap<i64, f64>,
}

impl OrderBook {
    pub fn new(tick_size: f64) -> Self {
        Self {
            tick_size,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    fn price_to_tick(&self, price: f64) -> i64 {
        (price / self.tick_size).round() as i64
    }

    fn tick_to_price(&self, tick: i64) -> f64 {
        tick as f64 * self.tick_size
    }

    /// Replace the whole book from a snapshot (non-positive sizes are skipped).
    pub fn apply_snapshot(&mut self, snap: &DepthSnapshot) {
        self.bids.clear();
        self.asks.clear();
        for lvl in &snap.bids {
            if lvl.size > 0.0 {
                self.bids.insert(self.price_to_tick(lvl.price), lvl.size);
            }
        }
        for lvl in &snap.asks {
            if lvl.size > 0.0 {
                self.asks.insert(self.price_to_tick(lvl.price), lvl.size);
            }
        }
    }

    /// Apply one level update; `size <= 0` removes the level.
    pub fn apply_update(&mut self, upd: &DepthUpdate) {
        let tick = self.price_to_tick(upd.price);
        let side = match upd.side {
            BookSide::Bid => &mut self.bids,
            BookSide::Ask => &mut self.asks,
        };
        if upd.size > 0.0 {
            side.insert(tick, upd.size);
        } else {
            side.remove(&tick);
        }
    }

    /// Highest bid price, or `None` if there are no bids.
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.keys().next_back().map(|&t| self.tick_to_price(t))
    }

    /// Lowest ask price, or `None` if there are no asks.
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.keys().next().map(|&t| self.tick_to_price(t))
    }

    /// `best_ask - best_bid`, or `None` if either side is empty.
    pub fn spread(&self) -> Option<f64> {
        self.best_bid_ask().spread()
    }

    pub fn best_bid_ask(&self) -> BestBidAsk {
        BestBidAsk {
            bid: self.best_bid(),
            ask: self.best_ask(),
        }
    }

    /// The `n` best levels of `side`, best-first (bids descending, asks ascending).
    pub fn top_n(&self, side: BookSide, n: usize) -> Vec<DepthLevel> {
        let to_level = |(&tick, &size): (&i64, &f64)| DepthLevel {
            price: self.tick_to_price(tick),
            size,
        };
        match side {
            BookSide::Bid => self.bids.iter().rev().take(n).map(to_level).collect(),
            BookSide::Ask => self.asks.iter().take(n).map(to_level).collect(),
        }
    }

    /// `(bid_levels, ask_levels)`.
    pub fn level_counts(&self) -> (usize, usize) {
        (self.bids.len(), self.asks.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lvl(price: f64, size: f64) -> DepthLevel {
        DepthLevel { price, size }
    }

    fn approx(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|x| (x - b).abs() < 1e-9)
    }

    fn book() -> OrderBook {
        let mut b = OrderBook::new(0.1);
        b.apply_snapshot(&DepthSnapshot {
            ts: 0,
            bids: vec![lvl(99.9, 5.0), lvl(99.8, 3.0), lvl(99.7, 2.0)],
            asks: vec![lvl(100.1, 4.0), lvl(100.2, 6.0), lvl(100.3, 1.0)],
        });
        b
    }

    #[test]
    fn snapshot_sets_best_bid_and_ask() {
        let b = book();
        assert!(approx(b.best_bid(), 99.9));
        assert!(approx(b.best_ask(), 100.1));
        assert_eq!(b.level_counts(), (3, 3));
    }

    #[test]
    fn empty_book_has_no_bbo_or_spread() {
        let b = OrderBook::new(0.1);
        assert_eq!(b.best_bid(), None);
        assert_eq!(b.best_ask(), None);
        assert_eq!(b.spread(), None);
    }

    #[test]
    fn spread_is_ask_minus_bid() {
        assert!((book().spread().unwrap() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn update_inserts_then_modifies_a_level() {
        let mut b = book();
        // 100.0 is on the 0.1 tick grid and sits between the best bid (99.9) and
        // best ask (100.1).
        b.apply_update(&DepthUpdate {
            ts: 1,
            side: BookSide::Bid,
            price: 100.0,
            size: 10.0,
        });
        assert!(approx(b.best_bid(), 100.0)); // new best bid

        b.apply_update(&DepthUpdate {
            ts: 2,
            side: BookSide::Bid,
            price: 100.0,
            size: 1.0,
        });
        let top = b.top_n(BookSide::Bid, 1);
        assert!((top[0].price - 100.0).abs() < 1e-9);
        assert_eq!(top[0].size, 1.0); // size replaced, not added
    }

    #[test]
    fn update_with_zero_size_deletes_the_level() {
        let mut b = book();
        let (bids_before, _) = b.level_counts();
        b.apply_update(&DepthUpdate {
            ts: 1,
            side: BookSide::Bid,
            price: 99.9,
            size: 0.0,
        });
        assert!(approx(b.best_bid(), 99.8)); // best bid removed
        assert_eq!(b.level_counts().0, bids_before - 1);
    }

    #[test]
    fn top_n_is_sorted_best_first() {
        let b = book();
        let bids: Vec<f64> = b.top_n(BookSide::Bid, 2).iter().map(|l| l.price).collect();
        let asks: Vec<f64> = b.top_n(BookSide::Ask, 2).iter().map(|l| l.price).collect();
        assert!(bids
            .iter()
            .zip([99.9, 99.8])
            .all(|(a, b)| (a - b).abs() < 1e-9));
        assert!(asks
            .iter()
            .zip([100.1, 100.2])
            .all(|(a, b)| (a - b).abs() < 1e-9));
    }

    #[test]
    fn top_n_clamps_to_available_levels() {
        assert_eq!(book().top_n(BookSide::Bid, 10).len(), 3);
    }

    #[test]
    fn best_bid_ask_struct_carries_spread() {
        let bba = book().best_bid_ask();
        assert!(approx(bba.bid, 99.9));
        assert!(approx(bba.ask, 100.1));
        assert!((bba.spread().unwrap() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn snapshot_skips_non_positive_sizes() {
        let mut b = OrderBook::new(0.1);
        b.apply_snapshot(&DepthSnapshot {
            ts: 0,
            bids: vec![lvl(99.0, 0.0), lvl(98.0, 5.0)],
            asks: vec![],
        });
        assert!(approx(b.best_bid(), 98.0));
        assert_eq!(b.level_counts(), (1, 0));
    }
}
