//! `market-core`: finance-native data types and the deterministic synthetic
//! generator. No rendering and no viewport logic live here.
//!
//! Covers candles / OHLCV, live tick ingestion + order flow, and Level 2 market
//! depth (an order book + neutral depth events).

mod candle;
mod depth;
mod event;
mod heatmap;
mod live;
mod rng;
mod scenario;
mod state;
mod synth;

pub use candle::{Candle, CandleSeries};
pub use depth::{BestBidAsk, BookSide, DepthLevel, DepthSnapshot, DepthUpdate, OrderBook};
pub use event::{parse_event, parse_events, serialize_event, serialize_events, MarketEvent};
pub use heatmap::LiquidityHeatmap;
pub use live::{CandleBuilder, CandleUpdate, Flow, Side, Tick, MAX_GAP_FILL};
pub use scenario::{scenario, ScenarioConfig, ScenarioKind, ScenarioStep};
pub use state::{replay_events, ApplyResult, ChartState};
pub use synth::{
    events, generate, ticks, DepthConfig, DepthDelta, DepthGen, EventConfig, EventSource,
    GenConfig, LiveConfig, TickStream,
};
