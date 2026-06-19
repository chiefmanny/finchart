//! `market-core`: finance-native data types and the deterministic synthetic
//! generator. No rendering and no viewport logic live here.
//!
//! MVP scope only: candles / OHLCV. Order-flow columns (delta, bid/ask volume)
//! and trades/depth types are deliberately out of scope for the first milestone.

mod candle;
mod live;
mod rng;
mod synth;

pub use candle::{Candle, CandleSeries};
pub use live::{CandleBuilder, CandleUpdate, Side, Tick};
pub use synth::{generate, ticks, GenConfig, LiveConfig, TickStream};
