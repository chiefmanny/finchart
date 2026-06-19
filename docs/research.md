# High-Performance Financial Charting Engine Research

## North Star

Build a high-performance financial charting engine with both GUI and terminal backends, built specifically for live OHLC, trades, order-book, and order-flow visualization.

The key wedge is not simply another candlestick chart. The opportunity is a charting engine where candles, trades, volume, order-book depth, footprint cells, heatmaps, indicators, drawings, and terminal rendering are first-class architectural concerns.

## Core Thesis

Most existing charting libraries optimize one lane:

- TradingView Lightweight Charts: finance-friendly and lightweight, but browser Canvas-first and not order-flow-native.
- uPlot: extremely fast and minimal for time series, but not a full financial/order-flow charting product.
- SciChart and LightningChart: high-performance GPU/WASM commercial engines, but GUI-first and commercial.
- Highcharts Stock: polished financial analytics and enterprise-friendly, but performance mode is a special path with tradeoffs.
- ECharts: powerful and open-source, but broad/generic rather than trading-native.
- Terminal charting libraries: sparse, immature, and mostly not built for real-time order-flow.

The whitespace is a developer-facing, open, performance-oriented financial charting engine that supports both GUI and terminal use cases from a shared data/rendering core.

## What Existing Software Teaches Us

### TradingView Lightweight Charts

What it does well:

- Finance-specific API and UX.
- Small footprint.
- Fast Canvas-based rendering.
- Incremental update path for real-time bars.
- Recent data conflation support for large datasets when zoomed out.

Lessons:

- Replacing the entire series on every update is expensive. Real-time charts need append/update semantics.
- Finance-specific defaults matter: time scale, candles, histograms, crosshair, price scale, volume overlay.
- Conflation/downsampling is not optional for large datasets.

Limitations / wedge:

- Order-book heatmap and deeper order-flow views are not core.
- Plugin ecosystem can extend it, but order-flow visualization is bolted on rather than native.
- Canvas/CPU path may hit limits with dense heatmaps or very large live order-flow scenes.

### uPlot

What it does well:

- Extremely lean Canvas 2D time-series rendering.
- Minimal memory and abstraction overhead.
- Strong performance with large line/time-series datasets.
- Simple, hacker-friendly design.

Lessons:

- Doing less is a serious performance strategy.
- Object allocation and abstraction overhead matter.
- Canvas can go very far when paired with tight loops and a narrow feature set.

Limitations / wedge:

- Financial charting is not the core product.
- Complex layering and plugin ordering can become awkward.
- Finance-specific time handling, gaps, sessions, off-hours, and market calendars need deeper native support.

### SciChart JS

What it does well:

- Uses WebGL + WebAssembly.
- Built for real-time and million-point workloads.
- Has financial demos: candlestick/OHLC, multi-pane stock charts, market depth, order-book heatmap.
- Supports FIFO/ring-buffer-style data modes.
- Throttles redraws to frame rate.

Lessons:

- The data model is as important as the renderer.
- Preallocated buffers and bounded live windows matter.
- Redraw frequency should be controlled explicitly.
- WebGL/WASM becomes valuable when datasets and update frequency are high.

Limitations / wedge:

- Commercial.
- GUI-first.
- Not positioned as an open dual-backend engine.

### LightningChart JS

What it does well:

- GPU/WebGL-first.
- Targets massive real-time rendering.
- Has financial/trader-oriented chart types including candlesticks.

Lessons:

- GPU acceleration is the right ceiling for dense order-flow visuals.
- Rendering pipelines should be designed for large data from the beginning.

Limitations / wedge:

- Commercial.
- More raw rendering platform than open financial charting core.
- Terminal use case is not the point.

### Highcharts Stock + Boost

What it does well:

- Mature enterprise stock chart product.
- Navigator, range selector, indicators, annotations.
- Boost module uses WebGL on canvas for supported high-density series.

Lessons:

- SVG is great for polish and interactivity but weak for huge data.
- A separate high-performance path can help, but feature tradeoffs appear quickly.

Limitations / wedge:

- Boost mode is not a universal magic switch.
- Financial/order-flow rendering is not fundamentally rethought.
- Commercial constraints for many use cases.

### Apache ECharts

What it does well:

- Open-source and powerful.
- Supports Canvas and SVG.
- Progressive rendering and large-data features.
- Candlestick support and broad visualization capability.

Lessons:

- Renderer choice should be explicit.
- Canvas is generally better for large numbers of visual elements.
- Progressive rendering is useful, but UX can suffer during zoom/hover if not carefully designed.

Limitations / wedge:

- Broad general-purpose architecture.
- Finance-specific aggregation, sessions, footprints, and order-flow are not first-class.

### finplot / PyQtGraph

What it does well:

- Python desktop financial plotting.
- Built on PyQtGraph, which uses NumPy and Qt GraphicsView for speed.
- Designed for backtesting charts with hundreds of thousands of datapoints.

Lessons:

- A finance-specific API on top of a performant rendering substrate can be very productive.
- Backtesting and research users value fast local charting.

Limitations / wedge:

- Desktop/Python-specific.
- Not web or terminal-native.
- Not an order-flow-native engine.

### Terminal Libraries

Relevant tools:

- plotext: Python terminal plotting with candlestick support.
- tui-candlestick-chart / cli-candlestick-chart: Rust terminal candlestick experiments.
- termichart-charts: Rust terminal charting SDK with candlestick modules.
- Ratatui / Textual / Blessed / Bubble Tea: general TUI frameworks, not financial chart engines.

Lessons:

- Terminal financial charting exists, but it is sparse.
- There is room for a polished, finance-native TUI charting layer.
- Terminal cells are limiting, so backend design matters.

Limitations / wedge:

- Most terminal options are not built for live order-flow.
- Most are not designed around shared GUI/TUI data cores.
- Order-book heatmaps and footprint charts are especially underserved.

## What Actually Makes Financial Charts Fast

### Avoid DOM/SVG Per Mark

One DOM/SVG node per candle, wick, volume bar, marker, heatmap cell, or footprint cell does not scale.

Fast engines use Canvas, WebGL, or tightly batched rendering for the hot path.

### Draw Only Visible Data

The renderer should maintain viewport bounds and draw only:

- Visible candles.
- Visible price levels.
- Visible trades.
- Visible heatmap tiles.
- A small overscan buffer.

### Use Incremental Updates

Live market data should not rebuild the chart.

Desired operations:

- Append trade.
- Update current candle.
- Append completed candle.
- Apply order-book depth delta.
- Update heatmap tile.
- Mark dirty region/layer.

### Separate Layers

Different chart layers have different redraw requirements:

- Heatmap: frequent but can be tiled.
- Candles: update current candle, append new candle.
- Volume: update with candle.
- Trades: append events.
- Crosshair: redraw on mouse movement.
- Axis labels: redraw rarely.
- Drawings: redraw when edited or transformed.
- Indicators: redraw when dependent data changes.

These should not all repaint together.

### Use Packed Columnar Data

Prefer:

```text
timestamps: i64[]
open: f64[]
high: f64[]
low: f64[]
close: f64[]
volume: f64[]
delta: f64[]
bid_volume: f64[]
ask_volume: f64[]
```

Avoid object-per-candle models in the hot path.

### Aggregate By Pixel / Zoom Level

If one screen pixel represents 50 candles, drawing all 50 is wasted.

Use zoom-aware aggregation:

- OHLC aggregation for candles.
- Min/max for line ranges.
- Volume summation.
- Bid/ask delta aggregation.
- Heatmap tile aggregation.
- Trade bubble clustering.

### Treat Finance Time As Its Own Domain

Financial time is not ordinary continuous time.

The engine should support:

- Trading sessions.
- Off-hours.
- Weekends.
- Holidays.
- Missing data gaps.
- Continuous crypto sessions.
- Futures sessions.
- Custom calendar compression.

### Keep Text Sparse

Text is often more expensive than geometric primitives.

Axis labels, price labels, tooltips, and footprint text should be drawn carefully and sparsely. For dense footprint views, text should degrade into color/shape until zoomed in enough.

## New Technologies To Exploit

### WebGPU

Best long-term GUI performance bet.

Useful for:

- Order-book heatmap rendering.
- Footprint grids.
- GPU-side aggregation.
- Tile compositing.
- Large scatter/trade bubble layers.
- Shader-based color mapping.
- Parallel transformations.

Why it matters:

- WebGPU gives modern GPU compute and rendering access.
- It is a successor to WebGL with better support for compute-style workloads.
- Order-flow heatmaps are closer to image/texture processing than traditional chart rendering.

### OffscreenCanvas

Useful for moving Canvas/WebGL rendering into a worker.

Benefits:

- Keeps main thread responsive.
- Lets pan/zoom/input avoid fighting render work.
- Good fallback before WebGPU backend is ready.

### SharedArrayBuffer + Atomics

Useful for live data ingestion.

Use cases:

- Shared-memory tick ring buffer.
- Shared-memory trade buffer.
- Shared order-book update queue.
- Worker-to-renderer communication without constant copies.

This matters because high-frequency charting often bottlenecks on data movement, not drawing alone.

### WASM + SIMD

Best for:

- Aggregation.
- Viewport indexing.
- Session calendar compression.
- Footprint bucketing.
- Order-book delta processing.
- Min/max scans.
- Typed-array transformations.

Rust/WASM is especially attractive for a shared core that can run in browser, Node, CLI, and potentially native desktop.

### Terminal Pixel Protocols

Portable TUI should use Unicode cells first.

Advanced terminal graphics can support:

- Kitty graphics protocol.
- Sixel.
- iTerm image protocol.

Benefits:

- High-resolution terminal candles.
- Real heatmaps instead of coarse cell blocks.
- Better footprint density.

Tradeoff:

- Terminal compatibility becomes harder.
- Need graceful fallback.

## Architectural Flaws To Improve On

### Flaw 1: Renderer-First Design

Many libraries start from a renderer and shape the data model around it.

Better:

Start with a finance-native data engine, then attach render backends.

### Flaw 2: Candles As The Center Of Everything

Most financial libraries treat candles as the primary object and bolt on everything else.

Better:

Treat candles, trades, depth, volume, footprint, and indicators as equal data layers.

### Flaw 3: Single Redraw Pipeline

Many systems redraw too much too often.

Better:

Per-layer invalidation and redraw cadence.

### Flaw 4: Weak Order-Flow Model

Order-flow is often a plugin, image, or external product feature.

Better:

Native order-flow primitives:

- Trades.
- Bid/ask aggression.
- Delta.
- Volume profile.
- Footprint.
- Market depth.
- Liquidity heatmap.
- Imbalances.
- Stacked imbalances.
- Liquidation markers.

### Flaw 5: No Shared GUI/TUI Core

Terminal tools and web tools are usually separate.

Better:

One data engine and layout model, multiple renderers.

### Flaw 6: Poor Finance Time Semantics

Generic time-series libraries often struggle with market calendars and gaps.

Better:

Market-time axis as a core abstraction.

### Flaw 7: Plugin Systems Without Strong Layer Contracts

Plugins can cause drawing-order, scale, hit-test, and autoscale issues.

Better:

Explicit layer contracts:

```text
z-index
autoscale contribution
hit-test behavior
dirty bounds
render frequency
backend compatibility
data dependencies
```

## Proposed Architecture

### Core Packages

```text
market-data-core
  OHLCV
  trades
  depth snapshots
  depth deltas
  market calendars
  tick aggregation
  footprint aggregation

render-core
  viewport
  scales
  layout
  layer graph
  culling
  level-of-detail
  dirty-region tracking

backend-canvas
  Canvas 2D renderer

backend-webgpu
  WebGPU renderer

backend-terminal
  Unicode/Braille/block terminal renderer

backend-terminal-pixels
  Kitty/Sixel/iTerm renderer

bindings-js
  TypeScript browser/Node API

cli
  Terminal charting tool
```

### Layer Model

```text
Chart
  TimeScale
  PriceScale
  Layers:
    HeatmapLayer
    CandleLayer
    VolumeLayer
    TradeBubbleLayer
    FootprintLayer
    IndicatorLayer
    DrawingLayer
    CrosshairLayer
    AxisLabelLayer
```

Each layer declares:

```text
data dependencies
z-index
autoscale behavior
hit-test behavior
redraw frequency
visible bounds
backend support
```

### Live Data Model

```text
tick_ring_buffer
trade_ring_buffer
depth_update_ring_buffer
candle_builder
footprint_builder
liquidity_heatmap_store
```

### Suggested Public API

TypeScript:

```ts
const chart = createChart(container, {
  backend: "canvas",
});

chart.addCandles({ symbol: "BTCUSDT", timeframe: "1m" });
chart.addVolume();
chart.addTrades({ mode: "bubbles" });
chart.addOrderBookHeatmap({ levels: 200 });
chart.addFootprint({ mode: "bidAskDelta" });

chart.feed.appendTrade(trade);
chart.feed.appendDepthUpdate(update);
chart.feed.appendCandle(candle);
```

CLI:

```bash
finchart watch BTCUSDT --backend terminal --layers candles,volume,heatmap
```

## GUI vs TUI Strategy

### GUI Pros

- Highest performance ceiling.
- Canvas, WebGL, WebGPU, and WASM available.
- Better interaction model for zoom, crosshair, drawings, heatmaps, footprint text.
- Larger market of potential users.
- Better for order-flow visualization.

### GUI Cons

- Crowded market.
- Hard to beat established chart UX.
- Browser graphics and DPI handling are complex.
- WebGPU adds power but also complexity.

### TUI Pros

- Much less competition.
- Strong fit for quants, backtesters, infra engineers, and terminal-native developers.
- Works over SSH.
- Differentiated story: live order-flow in the terminal.
- Great wedge into developer audience.

### TUI Cons

- Terminal cell resolution is limiting.
- Mouse/input support varies.
- Fonts/glyphs/colors vary.
- Pixel protocols are not universally supported.
- Harder to render rich footprint/heatmap views portably.

### Best Strategy

Do both, but sequence carefully:

1. Build shared Rust data/render core.
2. Build Canvas 2D backend for visual correctness and browser demo.
3. Build terminal Unicode backend early for differentiation.
4. Add WebGPU for heatmap/footprint performance.
5. Add terminal pixel backend later for high-resolution terminal rendering.

## Benchmark Plan

Compare against:

- TradingView Lightweight Charts.
- uPlot.
- Apache ECharts candlestick / heatmap.
- finplot / PyQtGraph.
- plotext.
- Ratatui candlestick crates.
- SciChart / LightningChart demos as commercial reference points.

Benchmark scenarios:

- 1 million historical candles.
- 50k visible candles.
- 1k tick updates per second into current candle.
- 100k trades plotted as bubbles/dots.
- Order-book heatmap: time x price x liquidity.
- Footprint chart: bid/ask volume per price level per candle.
- Terminal rendering at 120x40, 160x50, and 240x80.
- Pan latency.
- Zoom latency.
- Crosshair latency.
- Memory usage.
- First render time.
- Sustained live update CPU/GPU usage.

## MVP Plan

### Phase 1: Research Prototype

Goal:

Build benchmark harness and small rendering prototypes.

Deliverables:

- Synthetic OHLCV generator.
- Synthetic trade stream generator.
- Synthetic order-book depth stream generator.
- Canvas candle renderer.
- Terminal candle renderer.
- Basic benchmark runner.

### Phase 2: Core Data Engine

Goal:

Build the finance-native data model.

Deliverables:

- Columnar OHLCV store.
- Ring buffer for trades.
- Ring buffer for depth updates.
- Candle builder.
- Multi-resolution aggregation.
- Market-time axis abstraction.
- Viewport indexing.

### Phase 3: Layered Renderer

Goal:

Build the reusable rendering architecture.

Deliverables:

- Chart layout.
- Price scale.
- Time scale.
- Layer graph.
- Dirty-region tracking.
- Candle layer.
- Volume layer.
- Trade bubble layer.
- Crosshair layer.

### Phase 4: Order-Flow Features

Goal:

Make the engine meaningfully differentiated.

Deliverables:

- Order-book heatmap layer.
- Footprint layer.
- Delta layer.
- Volume profile layer.
- Imbalance highlighting.
- Trade clustering.

### Phase 5: WebGPU Backend

Goal:

Accelerate the dense visual layers.

Deliverables:

- WebGPU heatmap renderer.
- GPU color mapping.
- GPU tile compositing.
- Optional GPU aggregation experiments.

### Phase 6: Terminal Product Demo

Goal:

Prove the terminal wedge.

Deliverables:

- `finchart watch BTCUSDT`.
- Candles.
- Volume.
- Trades.
- Basic heatmap.
- Keyboard pan/zoom.
- Optional exchange WebSocket integration.

## Recommended Stack

Core:

- Rust.

Web bindings:

- TypeScript.
- WASM.

Initial GUI backend:

- Canvas 2D.

High-performance GUI backend:

- WebGPU.

Terminal backend:

- Ratatui.

Advanced terminal pixels:

- Kitty graphics protocol.
- Sixel fallback.

Data movement:

- Typed arrays.
- SharedArrayBuffer where browser security headers allow it.
- Ring buffers.

## Killer Demo

Build a live crypto order-flow chart with the same core powering:

1. Browser chart.
2. Terminal chart.

Data source:

- Binance, OKX, or Bybit public streams.

Views:

- Candles.
- Volume.
- Live trades.
- Order-book heatmap.
- Delta.
- Footprint drilldown.

This demo proves the thesis immediately: the engine is not just a chart. It is a live market visualization engine that can run in a browser or terminal.

## Positioning

Possible one-liner:

> A high-performance financial charting engine for developers building live trading and research tools, with native support for candles, trades, order-book depth, footprint charts, heatmaps, and terminal rendering.

More technical:

> Renderer-agnostic market visualization core with Canvas, WebGPU, and terminal backends for OHLC, tick, trade, and depth data.

Open-source wedge:

> The open-source order-flow-native alternative to generic chart libraries and commercial trading chart SDKs.

## Source References

- TradingView Lightweight Charts: https://github.com/tradingview/lightweight-charts
- TradingView Lightweight Charts docs: https://tradingview.github.io/lightweight-charts/docs
- TradingView Lightweight Charts release notes: https://tradingview.github.io/lightweight-charts/docs/release-notes
- uPlot: https://github.com/leeoniya/uPlot
- SciChart JS performance tips: https://www.scichart.com/documentation/js/v5/2d-charts/performance-tips/performance-tips-and-tricks/
- SciChart DataSeries realtime updates: https://www.scichart.com/documentation/js/current/DataSeries_RealtimeUpdates.html
- Highcharts Boost module: https://www.highcharts.com/docs/advanced-chart-features/boost-module
- Apache ECharts: https://echarts.apache.org/
- Apache ECharts Canvas vs SVG: https://apache.github.io/echarts-handbook/en/best-practices/canvas-vs-svg/
- finplot: https://github.com/highfestiva/finplot
- PyQtGraph: https://www.pyqtgraph.org/
- plotext: https://github.com/piccolomo/plotext
- Ratatui: https://ratatui.rs/
- WebGPU API: https://developer.mozilla.org/en-US/docs/Web/API/WebGPU_API
- OffscreenCanvas: https://developer.mozilla.org/en-US/docs/Web/API/OffscreenCanvas
- SharedArrayBuffer: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer
- WebAssembly SIMD: https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/SIMD
- Kitty graphics protocol: https://sw.kovidgoyal.net/kitty/graphics-protocol/
- Bookmap heatmap overview: https://bookmap.com/blog/heatmap-in-trading-the-complete-guide-to-market-depth-visualization
- Sierra Chart Market Depth Historical Graph: https://www.sierrachart.com/index.php?ID=375&page=doc%2FStudiesReference.php
- Exocharts: https://exocharts.com/
- ATAS footprint settings: https://help.atas.net/en/support/solutions/articles/72000606631-cluster-settings
