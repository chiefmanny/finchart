# finchart MVP — Implementation Plan

> Scope is strictly [`docs/mvp.md`](./mvp.md): synthetic OHLCV → viewport cull →
> terminal candles → benchmark timing → `cargo test`. The README's broader crate
> layout (`backend-canvas`, `web/`, depth, trades) is **deliberately not** created
> yet — empty future crates are clutter. Structure is chosen so they slot in later
> without rework.

## Repo prerequisites / notes

- **Rust is not installed** (no `cargo`/`rustup`). Installing the toolchain is
  prerequisite step 0 before any `cargo test` / `cargo run`.
- **`.gitignore` currently ignores `Cargo.lock`.** For a workspace that ships
  binaries (the `examples-bench` runner), the convention is to *commit* `Cargo.lock`
  for reproducible benchmarks. Recommend removing that line.

## 1. Rust workspace structure

```text
finchart/
  Cargo.toml            # [workspace], resolver = "2", shared profile.bench/release
  crates/
    market-core/        # data types + columnar candle store + synthetic generator
    render-core/        # viewport, culling, price/time→coordinate scales (backend-agnostic)
    backend-terminal/   # ASCII/Unicode candlestick rasterizer (pure string output)
  examples-bench/       # binary crate: demo chart + 10k/100k/1M timing runner
```

Four crates, not seven. Rationale:

- `market-core` + `render-core` + `backend-terminal` mirror the README's eventual
  split, so the names are forward-compatible.
- `synthetic-data` is folded into `market-core` as a `synth` module for the MVP
  (one generator, deterministic). It can be promoted to its own crate when
  trades/depth generators arrive — no API break since it already produces
  `market-core` types.
- `bench` becomes a top-level binary crate `examples-bench` so the success-criteria
  "running the example" is literally `cargo run -p examples-bench --release`.

## 2. Crate responsibilities

| Crate | Responsibility | Depends on |
|---|---|---|
| `market-core` | Core data types (`Candle`, `CandleSeries` columnar store, `TimeRange`), deterministic synthetic generator. No rendering, no viewport. | std only |
| `render-core` | `Viewport` (visible index/price range), culling (`visible_range`), `PriceScale`/`TimeScale` mapping data→cell coords. Backend-agnostic; knows nothing about terminals. | `market-core` |
| `backend-terminal` | Takes a culled slice + scales + a target `(cols, rows)` and produces a `String` grid of candles + axes. Pure compute, returns text — caller prints. | `market-core`, `render-core` |
| `examples-bench` | Wires it together; generates datasets, times generate/cull/render phases, prints a chart and a timing table. | all three |

Keeping `backend-terminal` return a `String` (not writing to stdout) keeps it
testable and SSH-agnostic.

## 3. Core data types (first milestone only)

In `market-core`:

```text
Candle            { ts: i64, open: f64, high: f64, low: f64, close: f64, volume: f64 }   // convenience / tests
CandleSeries      { ts: Vec<i64>, open/high/low/close/volume: Vec<f64> }                  // columnar hot path
  - len(), is_empty(), get(i) -> Candle, push(Candle), iter
  - invariant: all columns equal length; ts strictly increasing
TimeRange         { start: i64, end: i64 }
```

Columnar `CandleSeries` is the load-bearing structure (matches README "Columnar Hot
Path"). Per-candle `Candle` exists only as an ergonomic accessor/return type and for
test readability. **No** `delta`/`bid_volume`/`ask_volume` columns yet — those are
order-flow, explicitly out of scope. Leave a one-line comment noting where they'd be
added.

## 4. Synthetic data generator design

`market-core::synth`:

```text
GenConfig { seed: u64, count: usize, start_ts: i64, interval_secs: i64,
            start_price: f64, volatility: f64, drift: f64 }
fn generate(cfg: &GenConfig) -> CandleSeries
```

- **Deterministic**: seeded PRNG, no `rand` crate — a small inline SplitMix64/xorshift
  (~10 lines). Same seed ⇒ byte-identical series. This is the testable backbone and
  avoids a dependency.
- **Realistic-enough candles**: geometric random walk for close-to-close; each
  candle's open = prior close; high/low extend beyond open/close by a noise term so
  wicks render. Volume = positive noise around a baseline.
- Timestamps are a fixed `interval_secs` grid starting at `start_ts` (regular grid —
  no market-calendar gaps; that's a future concern called out in research).
- Generation is O(n), single pass, pre-allocated `Vec`s with capacity — this is what
  the 1M benchmark measures.

## 5. Viewport / culling design

`render-core`:

```text
Viewport { start_index: usize, end_index: usize }   // half-open [start, end) over the series
  - fn candle_count(&self)
  - fn clamp(self, series_len) -> Viewport
fn cull(series: &CandleSeries, vp: &Viewport) -> CulledView<'_>
```

- Because the MVP grid is index-based (one candle per column, no time-zoom-to-pixel
  aggregation yet), culling is a **slice by index range** — O(1) to compute,
  O(visible) to scan for price min/max. This is the honest "render only what's
  visible" win and is trivially benchmarkable against full-dataset scans.
- `CulledView` borrows column slices (`&[i64]`, `&[f64]`) — zero copy.
- Price extent (min low / max high) over the visible slice is computed here and fed
  to the price scale, so autoscale tracks the viewport, not the whole dataset.
- Overscan is **not** added yet (no panning in MVP) — noted as a future hook.

## 6. Price/time → terminal-coordinate mapping

`render-core::scale`:

```text
PriceScale { price_min, price_max, rows }   // value → row, row 0 = top
  - fn price_to_row(&self, price: f64) -> i32
TimeScale  { start_index, count, cols }      // candle index → column
  - fn index_to_col(&self, i: usize) -> i32
```

- Linear mapping only (log scale is a future toggle). `price_to_row` inverts the axis
  (higher price = smaller row index). Guards the degenerate `price_max == price_min`
  case (flat series) to avoid div-by-zero by giving it a 1-unit synthetic span.
- Mapping is pure arithmetic, no allocation — unit-testable with exact expected cells
  (e.g. price_min→bottom row, price_max→top row, midpoint→middle).
- For the index-based MVP, `TimeScale` is essentially "one column per visible candle,
  possibly with a fixed candle width." Support an integer candle width (e.g. 1–3 cols)
  so candles are readable.

## 7. Terminal candlestick rendering approach

`backend-terminal`:

- Allocate a `cols × rows` `Vec<char>` cell grid, fill with spaces.
- For each visible candle: compute body top/bottom rows from `open`/`close` via
  `PriceScale`, wick top/bottom from `high`/`low`.
  - Wick: vertical line char (`│`) from high-row to low-row in the candle's center
    column.
  - Body: block char — `█` (filled) for the open↔close span. Direction (up/down)
    distinguished by **color via ANSI** (green up / red down), with a `--no-color`
    plain mode for tests/SSH. A zero-height body collapses to a doji char (`─`).
- Render axes last: a left price-axis gutter (a few labeled rows: top=max, bottom=min,
  plus mid) and a bottom time axis (first/last timestamp). Text kept sparse per
  research guidance.
- Output assembled into a single `String` (rows joined by `\n`). Renderer is pure; the
  example prints it. Color is opt-in so the plain grid is deterministic and
  snapshot-testable.

Boring and correct first: ASCII/Unicode block cells, no Braille sub-cell packing yet
(Braille is a clean later upgrade behind the same interface).

## 8. Benchmark / example runner design

`examples-bench` (binary, `cargo run -p examples-bench --release`):

- **Visual demo**: generate a small series (e.g. 200 candles), pick a viewport
  (~120×30), render, print the chart — satisfies "visible terminal candle chart."
- **Benchmark table**: for each of 10k / 100k / 1M candles, time three phases with
  `std::time::Instant`:
  - generate, cull-to-viewport, render-viewport.
  - Print a table: dataset size | gen ms | cull µs | render µs | approx bytes
    (`len * bytes_per_candle`, since columns are `Vec`s — practical memory estimate
    without a memory-profiler dependency).
- A `--warmup`/repeat-N loop (simple averaging) to dampen noise; no Criterion needed
  for MVP (kept boring, zero dep). Criterion is the natural upgrade if statistical
  rigor is wanted later.

## 9. Unit tests to add

Per success criteria ("generation, culling, coordinate mapping"):

- **market-core**
  - `generate` is deterministic: same seed ⇒ identical series (compare full columns).
  - generated `CandleSeries` invariants: equal column lengths, `count` candles,
    strictly increasing ts on the fixed grid, `high ≥ max(open,close)`,
    `low ≤ min(open,close)`, `volume ≥ 0`.
  - `CandleSeries::get`/`push` round-trip.
- **render-core**
  - `cull` returns exactly the requested index range; clamps out-of-bounds; empty
    viewport handled.
  - visible price min/max correct over a known slice.
  - `PriceScale`: max→top row, min→bottom row, midpoint→middle; flat-series guard
    doesn't panic.
  - `TimeScale`: first index→col 0, last index→last col.
- **backend-terminal**
  - grid has exact `cols × rows` dimensions.
  - a single known candle places wick/body at expected cells (render in `--no-color`
    mode, assert characters at coordinates).
  - up vs down candle produces expected body char/marker.
- **examples-bench**: a smoke test that the end-to-end pipeline runs on a tiny dataset
  without panic (kept light).

## 10. Dependencies (minimal)

Recommendation: **zero external runtime dependencies for the MVP.**

| Need | Choice | Justification |
|---|---|---|
| RNG | hand-rolled SplitMix64 (std only) | Determinism is a hard requirement and easy to guarantee with ~10 lines; avoids `rand`'s API churn and gives reproducible bytes across platforms/versions. |
| ANSI color | hand-written escape codes (std only) | Two escape sequences (red/green/reset); a crate (`crossterm`/`owo-colors`) is overkill for the MVP and pulls a tree. |
| Timing | `std::time::Instant` | Sufficient for "basic benchmark timing." |
| Benchmarking | none (manual loop) for MVP | Criterion is a dev-dependency upgrade if/when we want statistical distributions; not needed to meet success criteria. |

This honors "minimal dependencies" / "boring." If ergonomic terminal handling is
wanted now, `crossterm` (color + size detection) is the one to add — but default to
none unless decided otherwise.

## Proposed first implementation checklist (in order)

0. **Prereq**: install Rust toolchain (`rustup`/`cargo`); decide on committing
   `Cargo.lock` (recommend yes).
1. Scaffold workspace `Cargo.toml` + four crate skeletons; `cargo build` green.
2. `market-core`: `Candle`, `CandleSeries` columnar store + accessors + invariant tests.
3. `market-core::synth`: SplitMix64 + `generate`; determinism + invariant tests.
4. `render-core`: `Viewport` + `cull` + visible price extent; tests.
5. `render-core::scale`: `PriceScale` / `TimeScale` mapping; tests (incl. flat-series guard).
6. `backend-terminal`: cell grid rasterizer (plain mode) + axes; cell-placement tests.
7. `backend-terminal`: ANSI color (up/down) behind `--no-color` toggle.
8. `examples-bench`: visual demo render of a small series.
9. `examples-bench`: 10k/100k/1M timing table (gen/cull/render + memory estimate).
10. Full `cargo test` green; run example; (optional) seed `CLAUDE.md` with build/run/test
    commands + architecture notes.
