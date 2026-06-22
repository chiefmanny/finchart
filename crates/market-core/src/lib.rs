//! `market-core`: finance-native data types and the deterministic synthetic
//! generator. No rendering and no viewport logic live here.
//!
//! Covers candles / OHLCV, live tick ingestion + order flow, and Level 2 market
//! depth (an order book + neutral depth events).

mod candle;
mod depth;
mod live;
mod rng;
mod synth;

pub use candle::{Candle, CandleSeries};
pub use depth::{BestBidAsk, BookSide, DepthLevel, DepthSnapshot, DepthUpdate, OrderBook};
pub use live::{CandleBuilder, CandleUpdate, Flow, Side, Tick};
pub use synth::{generate, ticks, DepthConfig, DepthGen, GenConfig, LiveConfig, TickStream};
