//! `render-core`: backend-agnostic viewport, culling, and price/time-to-coordinate
//! scales. Knows about `market-core` data but nothing about any specific renderer
//! (terminal, canvas, etc.).

mod aggregate;
mod scale;
mod viewport;

pub use aggregate::{aggregate, Bar};
pub use scale::{PriceScale, TimeScale};
pub use viewport::{cull, CulledView, Viewport, MIN_CANDLES};
