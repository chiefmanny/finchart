# Financial Charting Engine

A high-performance financial charting engine for developers building live trading and research tools, with native support for OHLC candles, trades, order-book depth, order-flow views, and both GUI and terminal rendering.

The long-term goal is not to clone TradingView. The goal is to build a renderer-agnostic market visualization engine where the data model, aggregation pipeline, and rendering layers are designed around live financial data from the start.

## North Star

Build a high-performance financial charting engine with shared core logic and multiple render backends:

- Browser GUI rendering for rich charts, high-density visuals, and future WebGPU acceleration.
- Terminal rendering for SSH-native quant, backtesting, and market-monitoring workflows.
- A finance-native core for OHLCV, trades, market depth, footprint charts, heatmaps, and volume/delta views.

## Why This Exists

Most charting libraries optimize for one lane:

- Generic dashboard charts.
- Basic candlestick charts.
- Commercial trading terminals.
- Scientific high-performance rendering.
- Simple terminal plots.

This project aims at the gap between them: an open, developer-facing, high-performance financial charting engine that treats order-flow and terminal workflows as first-class features instead of plugins or afterthoughts.

## MVP Scope

The first milestone is deliberately small and measurable:

- Generate synthetic OHLCV data.
- Render candles in a terminal.
- Cull data to the visible viewport.
- Measure render time, update time, and memory.
- Keep the architecture simple enough to evolve.

The first MVP should prove that one shared market-data core can drive multiple render backends.

## Initial Non-Goals

Do not build these first:

- WebGPU renderer / Render candles in a browser Canvas view.
- Exchange or broker integrations.
- Trading execution.
- Full indicator library.
- Full plugin system.
- Drawing tools.
- TradingView-style UI shell.
- Order-flow heatmap perfection.

Those come after the core data and rendering model has a pulse.

## Proposed Workspace Structure

```text
crates/
  market-core/
    Finance-native data types and storage.

  render-core/
    Viewport, scales, culling, layout, and render abstractions.

  synthetic-data/
    Deterministic generators for candles, trades, and depth data.

  backend-terminal/
    Unicode/Braille/block terminal renderer.

  backend-canvas/
    Browser Canvas 2D renderer.

  bench/
    Benchmarks and example scenarios.

web/
  canvas-demo/
    Minimal browser demo.

docs/
  architecture.md
  mvp.md
  non-goals.md
  benchmark-targets.md
  financial-charting-engine-research.md
```

## Core Design Principles

### Data First

The chart should be built around a finance-native data model, not around a specific renderer.

Core data types should include:

- Candles / OHLCV.
- Trades.
- Depth levels.
- Depth updates.
- Time ranges.
- Viewport ranges.

### Columnar Hot Path

The hot path should avoid object-heavy models where possible.

Prefer packed arrays for render-critical data:

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

### Render Only What Is Visible

The engine should never blindly render every candle or tick. It should calculate visible ranges and draw only what the viewport needs, with a small overscan buffer when useful.

### Incremental Updates

Live updates should mutate or append small pieces of data:

- Update current candle.
- Append completed candle.
- Append trade.
- Apply depth delta.
- Mark affected layers dirty.

Avoid replacing full datasets during live updates.

### Layered Rendering

Different visual layers need different redraw rules:

- Candles.
- Volume.
- Trades.
- Order-book heatmap.
- Footprint cells.
- Indicators.
- Drawings.
- Crosshair.
- Axes and labels.

The architecture should eventually allow these layers to update independently.

### Terminal Is First-Class

Terminal rendering should not be a toy export path. It should be a real backend with:

- Configurable width and height.
- Keyboard-friendly interaction.
- SSH-friendly output.
- Unicode/Braille/block rendering first.
- Optional pixel protocol support later.

## First Implementation Target

Build the smallest useful vertical slice:

```text
synthetic candles -> viewport culling -> terminal candle grid -> benchmark timing
```

Then add:

```text
same synthetic candles -> Canvas 2D browser renderer
```

Only after this should the project add trades, depth, order-flow, or WebGPU.

## Benchmark Targets

Initial benchmark scenarios:

- Generate 10k candles.
- Generate 100k candles.
- Generate 1M candles.
- Render a visible viewport, not the entire dataset.
- Measure culling time.
- Measure render time.
- Measure memory usage where practical.

Later benchmark scenarios:

- 1k simulated trades per second.
- Current-candle live updates.
- Order-book depth updates.
- Heatmap tile rendering.
- Terminal rendering at 120x40, 160x50, and 240x80.

## Example Future API

The public API is not the first task, but this is the shape the project is aiming toward:

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

Terminal usage might look like:

```bash
finchart watch BTCUSDT --backend terminal --layers candles,volume,heatmap
```

## Development Philosophy

Keep the first version boring and measurable.

Prefer:

- Simple data structures.
- Deterministic tests.
- Minimal dependencies.
- Visible output early.
- Benchmarks before abstractions.

Avoid:

- Premature plugin systems.
- Renderer-specific data models.
- Full trading terminal UI work.
- WebGPU before Canvas proves the model.
- Exchange integrations before synthetic data works.

## Success Criteria For The First Sprint

The first sprint is successful if the repo can:

1. Generate deterministic synthetic OHLCV data.
2. Select a visible viewport from a much larger dataset.
3. Render a readable candlestick chart in the terminal.
4. Run tests for generation, culling, and coordinate mapping.
5. Print basic timing results for 10k, 100k, and 1M candle scenarios.

Ugly output is acceptable. Unmeasured architecture is not.

## Research

The companion research brief is:

```text
docs/financial-charting-engine-research.md
```

It covers:

- Existing financial charting libraries.
- What makes them performant.
- Architectural gaps to exploit.
- New rendering/data technologies.
- GUI vs TUI tradeoffs.
- Proposed build plan.
