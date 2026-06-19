# MVP

The first milestone is not a full charting library. It is a measurable proof-of-engine.

## Goal

Build a Rust workspace that can:

1. Generate deterministic synthetic OHLCV data.
2. Select/cull a visible viewport from a larger candle dataset.
3. Render a candlestick chart to the terminal.
4. Benchmark rendering a viewport from 10k, 100k, and 1M generated candles.

## In Scope

- Rust workspace setup
- Market data types
- Synthetic candle generator
- Viewport selection
- Price/time mapping
- Terminal renderer
- Basic benchmark/example runner
- Unit tests

## Out of Scope

- WebGPU
- Browser Canvas renderer
- Exchange APIs
- Real-time networking
- Indicators
- Plugin system
- Order-book heatmap
- Footprint charts
- Trading/broker integration

## Success Criteria

Running the example should produce:

- A visible terminal candle chart.
- Benchmark output for 10k, 100k, and 1M candle datasets.
- Tests passing with `cargo test`.