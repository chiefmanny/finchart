# Heatmap V1 — Plan

> Status: approved (planning only, no code yet).
>
> Goal: prove the engine can accumulate order-book liquidity over time and render
> a time × price heatmap layer in the terminal, using only synthetic depth data.
>
> Synthetic data only. No gRPC, exchange APIs, browser Canvas, WebGPU, footprint
> chart, or plugin system. Dependencies stay minimal; existing tests/examples
> preserved.

## Current-state review

- **`market-core::depth`**: `OrderBook` is tick-keyed (`BTreeMap<i64, f64>` per
  side, private), exposing `apply_snapshot`/`apply_update`/`best_*`/`spread`/
  `top_n`/`level_counts`. No "all levels" accessor yet — the heatmap needs to read
  the whole book per snapshot.
- **`synth::DepthGen`**: deterministic non-crossing ladder + liquidity-shifting
  `step`.
- **`render-core`**: `cull → CulledView → aggregate → Bar` (candles only; depth and
  heatmap are separate live structures, not per-candle).
- **`backend-terminal`**: `render_chart` builds a full `width×height` `Grid`, draws
  price bars into `[gutter, gutter+plot_cols) × [plot_top, plot_top+plot_rows)`,
  then volume/delta panels, a right-side `DepthPanel`, and the time axis.
  `ChartOptions` literals exist in ~5 places.
- **`examples-live`**: `Replay` owns `CandleBuilder` + `TickStream` + `OrderBook` +
  `DepthGen`; deterministic `status()`; snapshot frames + `metrics.json`.

## 2. `LiquidityHeatmap` structure (`market-core`, new `heatmap` module)

A bounded rolling window of time buckets, each a tick-keyed liquidity column:

```rust
pub struct LiquidityHeatmap {
    tick_size: f64,
    capacity: usize,                  // max time buckets (rolling window)
    columns: VecDeque<HeatmapColumn>, // oldest -> newest
}
struct HeatmapColumn { ts: i64, liquidity: BTreeMap<i64, f64> } // combined liq per price tick
impl LiquidityHeatmap {
  pub fn new(tick_size, capacity) -> Self;
  pub fn record(&mut self, book: &OrderBook, ts: i64); // snapshot book -> new column; evict oldest past capacity
  pub fn time_buckets(&self) -> usize;       // columns.len()
  pub fn price_buckets(&self) -> usize;      // distinct ticks across the window
  pub fn max_liquidity(&self) -> f64;        // max cell over the window
  pub fn cells_in_price_range(&self, min, max) -> usize; // populated cells within a price band
  pub fn price_to_bucket(&self, price) -> i64;           // the internal tick key
  pub fn columns_in_range(&self, min, max) -> Vec<Vec<(f64, f64)>>; // render export: (price, liq) per column
}
```

Records **combined liquidity** (bid + ask) per price tick — simpler than splitting
sides and sufficient for a V1 liquidity map; a bid/ask split is a future
enhancement. Requires a new `OrderBook::levels(side) -> Vec<DepthLevel>` (all
levels) so `record` can fold both sides.

## 3. Storage decision: rolling per-time-bucket tick→liquidity maps (chosen)

Tradeoff vs. a pre-aggregated fixed price × time matrix:

- **Pre-aggregated 2D cells** — compact and render-ready, but forces a *fixed price
  grid chosen up front*; as price drifts the grid goes stale and needs
  re-centering/rebasing (extra state + correctness hazard).
- **Per-time-bucket tick-keyed maps (chosen)** — prices are *absolute integer
  ticks*, so drift is handled naturally; price→row bucketing happens at render
  time against the live price extent. Columns are small (bounded levels), so memory
  is fine. This is the simpler robust path. (Effectively "compact full snapshots
  per time bucket," but keyed by tick, not full `DepthLevel`s.)

## 4. Terminal heatmap rendering (`backend-terminal`)

- New `ChartOptions.heatmap: Option<HeatmapView>` where
  `HeatmapView { columns: Vec<Vec<(f64, f64)>>, max_liquidity: f64 }`
  (oldest→newest; plain tuples keep the backend decoupled — same pattern as
  `DepthPanel`).
- **Background pass before drawing bars**: for each plot column, pick the aligned
  heatmap column (newest → rightmost, so it follows latest), accumulate each
  level's liquidity into its price *row* via the existing `PriceScale`, and write a
  coarse intensity glyph: `' '` (zero) then `· ░ ▒ ▓ █` by `liq / max_liquidity`.
- **Candles / volume / delta / axes / depth draw after** → they overwrite, so bars
  sit *on top* of the heatmap.
- Total output stays exactly `width × height`. The right-side depth ladder stays
  independently optional.

## 5. `examples-live` integration

- `ReplayConfig` gains `heatmap_time_buckets` (window capacity).
- `Replay` holds a `LiquidityHeatmap`; `record(&book, ts)` on every depth step
  (same cadence as depth). Render builds a `HeatmapView` from `columns_in_range`
  over the *visible price extent*; the viewport already follows latest.
- `status()` may append `maxliq` / visible heatmap cells.

## 6. Snapshot verification

- Live `metrics.json` gains: `heatmap_time_buckets`, `heatmap_price_buckets`,
  `visible_heatmap_cells` (cells within the visible price band), `max_liquidity`.
- Frames render with the heatmap layer on → deterministic, fixed dims, contain
  intensity glyphs.

## 7. Tests (mapped to scope)

- **Heatmap store**: `record` adds a column; rolling-window eviction at capacity;
  `price_to_bucket` mapping; deterministic accumulation; `max_liquidity`;
  `cells_in_price_range`.
- **Render**: dimensions unchanged with heatmap on; intensity glyphs present;
  candles render over heatmap (both candle glyphs and heatmap glyphs present, the
  candle wins its column).
- **Live snapshot**: `metrics.json` includes the 4 heatmap fields; frames
  deterministic.

## Dependencies & invariants

- `VecDeque` / `BTreeMap` are std — no new dependencies; core stays zero-dep,
  `crossterm` confined to interactive binaries.
- No gRPC/exchange/Canvas/WebGPU/footprint/plugin. Existing examples/tests
  preserved (mechanical `heatmap: None` on `ChartOptions` literals; behavior
  unchanged).

## Implementation checklist (in order)

1. **`market-core::depth`** — add `OrderBook::levels(side) -> Vec<DepthLevel>` (all
   levels). Test-first.
2. **`market-core::heatmap`** — `LiquidityHeatmap` (rolling window, tick-keyed
   combined liquidity) + accessors + render export. Test-first.
3. **`backend-terminal`** — `HeatmapView` + `ChartOptions.heatmap` + background
   intensity render before bars; update `ChartOptions` literals. Test-first (dims,
   glyphs, candles-on-top).
4. **`examples-live`** — `Replay` owns the heatmap; record each depth step; render
   passes `HeatmapView`; status shows max liq / cells. Test-first (deterministic,
   populated).
5. **`examples-live` snapshot** — 4 heatmap fields in `metrics.json`; frames include
   the layer. Test-first.
6. **Preserve `examples-bench`/`examples-tui`** — `heatmap: None`; confirm static
   snapshot byte-identical.
7. **Docs** — `docs/verification.md`, `CLAUDE.md`, this plan.
8. **Full verification** — `cargo test`, clippy, fmt, run both snapshots, sanity-run
   `examples-live`; report artifacts.

## Confirmed decisions

- **(3)** Rolling **tick-keyed per-time-bucket** store (vs a fixed pre-aggregated
  matrix).
- **(4)** Render the heatmap as a **background layer inside the price panel**
  (candles drawn on top), gated by `ChartOptions.heatmap` (default `None`).
