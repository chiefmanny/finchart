# Order-Flow Layer V1 — Plan

> Status: implemented. Extend the live synthetic engine from OHLCV-only into
> order-flow-native data — aggressive buy/sell volume, per-candle delta, and
> cumulative delta (CVD) — and render it in the terminal with deterministic
> snapshots.
>
> Synthetic data only. No gRPC, exchange APIs, browser Canvas, WebGPU, order-book
> heatmap, footprint charts, or plugin system. Dependencies stay minimal.

## Current-state review (at planning time)

- **`market-core`**: columnar `CandleSeries` (OHLCV columns + `push`/`update_last`/
  `last`). `CandleBuilder.apply()` folded ticks into the forming candle but
  ignored `tick.side`. `Tick` already carried `side: Option<Side>` (`Buy`/`Sell`);
  `synth::ticks()` already set side from price-move sign.
- **`render-core`**: `CulledView` (OHLCV slices), `aggregate` → `Bar`. Single data
  path: `cull → aggregate → Bar`.
- **`backend-terminal`**: `render_chart` composes title → price plot → optional
  volume panel → time axis; `draw_volume_panel` is the template for a delta panel.
- **`examples-live`**: `Replay`, deterministic `status()`, snapshot frames.

## Storage decision: columns on `CandleSeries` (chosen)

Add `buy_volume` + `sell_volume` columns to `CandleSeries`; derive
`delta = buy - sell` (don't store a redundant `delta` column that could desync).

Tradeoff:

- **Columns (chosen).** The engine is already columnar and single-path
  (`cull → aggregate → Bar → render`). Flow rides that path as two more
  columns/fields — no second dataset to align. Equal-length invariant enforced
  like the existing columns.
- **Sidecar `FlowSeries` keyed by index (rejected).** Keeps `CandleSeries` pure
  OHLCV but forces a parallel cull and a parallel aggregate (or a combined view),
  plus a second aligned input into `render_chart` — more moving parts and a
  standing alignment burden for no real benefit.

`Candle` (the OHLCV accessor struct) stays unchanged, so the many `Candle { … }`
literals in tests/`generate` are untouched. Flow lives only in the series columns,
updated via new builder methods.

## Trade / aggressor semantics

`Tick` is an executed trade. `side = Buy` ⇒ buyer-initiated (aggressive) → buy
volume; `side = Sell` ⇒ seller-initiated → sell volume; `None` ⇒ unsided (counts
toward total volume only). No type change needed.

## Live candle building (flow accumulation)

- `CandleSeries`: `buy_volume`/`sell_volume` columns; `push` appends `0.0` flow;
  `add_flow_last(buy, sell)` accumulates onto the last candle; `update_last`
  touches OHLCV only (flow intact). Accessors `buy_volume()`/`sell_volume()`.
- `CandleBuilder::apply()` routes `tick.size` by `tick.side`. Total volume still
  accumulates everything; `buy + sell == volume` when all trades are sided.
- `Flow { buy_volume, sell_volume }` with `delta()`; `CandleBuilder::current_flow()`
  and `cumulative_delta()` (CVD = running Σ(buy−sell)).
- OHLCV invariants untouched (flow is orthogonal).

## Deterministic synthetic trade generation

- `ticks()` already yields a deterministic `side`; the builder consumes it — same
  seed ⇒ same trades ⇒ same flow stats.
- `generate()` (history) fills per-candle flow from a **separate RNG stream**
  (`seed ^ salt`) so adding it leaves the OHLCV output byte-unchanged. Volume is
  split skewed by candle direction (up ⇒ buy-heavy, down ⇒ sell-heavy);
  `buy + sell == volume`.

## Render support (delta panel)

- `CulledView` gains `buy`/`sell` slices (filled in `cull`); `Bar` gains
  `buy_volume`/`sell_volume` (summed in `aggregate`) + `delta()`.
- `ChartOptions.show_delta: bool` (default `false` — static snapshots stay
  byte-identical). Price and volume panels unchanged.
- `draw_delta_panel`: per bar `delta = buy−sell` scaled by `max|delta|`; positive
  bars grow up (green), negative down (red) around a center `─` baseline; gutter
  labels max-|delta| at the top and `0` at the baseline. Layout budget:
  `plot_rows = usable − volume_rows − delta_rows` (price plot keeps ≥1 row; panel
  silently off when too short).
- Status line content (built by callers) can show current-candle delta and CVD.

## Snapshot support

- `examples-live --snapshot` frames render with the delta panel on (price + volume
  + delta).
- `metrics.json` gains `buy_volume`, `sell_volume`, `current_delta`,
  `cumulative_delta`. A separate `frame.delta.*` file was skipped — the standard
  frames already include the panel.

## Tests

- Buy/sell accumulation, delta correctness, rollover preserves closed flow
  (`market-core`).
- Same-seed determinism of trades + flow (`market-core`).
- Flow conserved through cull+aggregate; `Bar::delta()` (`render-core`).
- Delta panel fixed dimensions + pos/neg bars + baseline (`backend-terminal`).
- Snapshot order-flow fields + deterministic frames (`examples-live`).

## Implementation checklist (done in order)

1. `market-core::candle` — `buy_volume`/`sell_volume` columns + accessors +
   `add_flow_last`; `update_last` stays OHLCV-only.
2. `market-core::live` — `apply` routes size by side; `Flow`, `current_flow`,
   `cumulative_delta`; aggressor docs.
3. `market-core::synth` — `generate()` fills deterministic per-candle flow;
   confirm `ticks()` side semantics.
4. `render-core` — `CulledView` buy/sell slices in `cull`; `Bar` buy/sell +
   `delta()` summed in `aggregate`.
5. `backend-terminal` — `ChartOptions.show_delta` + `draw_delta_panel` + layout
   budget; update existing `ChartOptions` literals.
6. `examples-live` — `status()`/render show current delta + CVD; frames enable the
   delta panel.
7. `examples-live` snapshot — flow fields in `metrics.json`.
8. Preserve `examples-bench`/`examples-tui` (behavior unchanged; static snapshot
   byte-identical).
9. Docs — `docs/verification.md`, `CLAUDE.md`, this plan.
10. Full verification.

## Constraints honored

Synthetic data only; core crates stay zero-dependency (`crossterm` confined to the
interactive binaries); no gRPC/exchange/Canvas/WebGPU/heatmap/footprint/plugin.
Existing examples and tests preserved.
