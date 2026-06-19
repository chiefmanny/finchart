# Terminal Renderer V2 — Plan

> Status: proposed (planning only, no code yet). Goal: make the terminal backend
> readable, navigable, and verifiable — a usable synthetic terminal chart viewer
> that proves the TUI backend can be a real product surface.
>
> Scope is constrained to synthetic data only. No gRPC, exchange APIs, browser
> Canvas, WebGPU, order-book heatmap, footprint charts, or plugin system.
> Dependencies stay minimal; `crossterm` is allowed for interactive input (see §6).

## Current state review

- **`render-core`**: `Viewport` (index window + `clamp`), `cull` → `CulledView`
  (zero-copy slices + `price_extent`), `PriceScale`/`TimeScale`. No aggregation,
  no pan/zoom.
- **`backend-terminal`**: `render_with(view, w, h, ColorMode)` draws one candle per
  column via `TimeScale` — **when visible candles > columns it overplots
  (last-writer-wins), losing data**. Single block-candle style (`█`/`▒`/`│`/`─`).
  No title, no volume, no real time labels. Color via per-cell `CellColor` +
  run-length ANSI; HTML export derives from ANSI.
- **`examples-bench`**: deterministic snapshot (`--snapshot`) →
  `chart.{plain,ansi}.txt`, `chart.html`, `metrics.json`; tests for determinism /
  no-ANSI / dims / HTML / metrics.
- Zero external deps so far (hand-rolled RNG, ANSI, JSON). `Cargo.lock` committed;
  `artifacts/` gitignored.

The overplot-on-zoom-out behavior is the main correctness gap; the missing
layout/navigation is the main product gap. V2 addresses both.

## Guiding decisions (require sign-off)

1. **New default style = OHLC bars; keep candles as opt-in.** OHLC bars render
   cleaner at terminal density and are the classic quant view.
2. **Replace `render_with` with a single `render_chart(view, &ChartOptions)`
   API.** The current `render`/`render_with` can't grow a title/volume/time-axis
   layout without dims drift. Migrate the snapshot, demo, and tests to the new API
   and delete the old one (cleaner single path). The internal `draw_*`/`Grid`
   primitives and their cell-placement tests stay.
3. **LOD aggregation lives in `render-core`** (display-resolution is a
   backend-agnostic concern, per `research.md`).
4. **`crossterm` only in the new `examples-tui` crate** — never in core crates or
   the deterministic snapshot/test path.

## 2. Cleaner default OHLC bar style

Each bar occupies a **cell of N columns** (style-defined width):

- **OHLC bars** (default, cell width 3): col0 = open tick at the open row (`╴`),
  col1 = vertical high→low line (`│`), col2 = close tick at the close row (`╶`).
  Direction by color (green up / red down) and reinforced by tick side.
- **Candles** (opt-in, cell width 1, optional 1-col gap): existing wick `│` + body
  `█`/`▒`, doji `─`.

Both share the price scale and the same bucket input, so the only difference is
the per-bar glyph routine — that is what `Style` selects.

## 3. Keep block candle style

Retained as `Style::Candles`. The existing `draw_candles` cell-placement tests
remain valid against the candle routine.

## 4. Display-resolution LOD (the core correctness win)

New `render-core` module `aggregate`:

```rust
pub struct Bar { pub start_ts: i64, pub open, high, low, close, volume: f64 }
pub fn aggregate(view: &CulledView, target_bars: usize) -> Vec<Bar>
```

- Partition the N visible candles into `min(N, target_bars)` contiguous,
  evenly-split buckets.
- Per bucket: `open` = first.open, `close` = last.close, `high` = max(high),
  `low` = min(low), `volume` = Σvolume, `start_ts` = first.ts.
- `target_bars = plot_cols / cell_width` (≥1). When N ≤ target, it is an identity
  map (1 candle/bar) — no aggregation artifacts.
- Renderer draws one bar per bucket at `col = bucket_index * cell_width`. This
  **replaces** the overplotting `TimeScale` stretch.

## 5. Chart layout

A `ChartLayout` computed from `(width, height, show_volume)`:

```text
row 0            title/status line          (full width)
rows 1..M        main panel: [price gutter][OHLC/candle plot]
rows M..V        volume panel (optional):  [vol gutter][volume bars]
last row         time axis                 (aligned under plot)
```

- **Title/status**: e.g. `finchart · SYNTH · 120 candles → 37 bars · $90.61–$113.36 · OHLC`
  (deterministic; surfaces LOD + range).
- **Price gutter**: existing label logic (max/mid/min), reused for both panels'
  alignment.
- **Volume panel**: per-bucket bars from the panel bottom, height ∝ vol/max-vol,
  colored by bar direction; small `vol` + max label in its gutter. Height
  ≈ `max(4, main_area/5)` when enabled.
- **Time axis**: 2–3 UTC tick labels (left/mid/right) from bucket `start_ts`, via a
  tiny dependency-free epoch→UTC formatter (`civil_from_days`), format
  `MM-DD HH:MM`. No "now", no locale → deterministic.

## 6. Interactive example binary `examples-tui`

New binary crate (`examples-tui/`), depends on the three core crates + `crossterm`.

- `cargo run -p examples-tui --release`
- Generates a deterministic synthetic series; holds a `Viewport`; re-renders via
  `render_chart` on each key.
- **Keys**: `←/h` pan left, `→/l` pan right, `+/k` zoom in, `-/j` zoom out, `r`
  reset, `q`/`Esc` quit.
- Uses crossterm raw mode + alternate screen; restores terminal on exit (incl.
  panic guard).
- Thin glue only — all logic (pan/zoom, aggregation, layout, render) is in tested
  library code.
- **`crossterm` justification**: raw-mode key input + alternate screen are exactly
  its job, cross-platform; hand-rolling termios is platform-specific and fragile.
  Scoped to this one binary so core stays zero-dep. Pin `crossterm = "0.28"`,
  default features.

### Pan/zoom model (`render-core`, on `Viewport`)

```rust
const MIN_CANDLES: usize = 8;
pub fn pan(self, delta: isize, series_len) -> Viewport   // shift, preserve width, clamp to [0,len]
pub fn zoom(self, factor: f64, series_len) -> Viewport   // scale width about center, clamp [MIN, len]
pub fn full(series_len) -> Viewport                      // reset → whole series (or a default window)
```

Pure and clamped — this is where the bounds tests live.

## 7. Verification / artifact integration

- Migrate `examples-bench` snapshot to `render_chart` with V2 default options
  (OHLC + volume + title), so artifacts reflect the product surface and stay
  deterministic (no crossterm).
- Emit **both styles** for inspection: `chart.ohlc.plain.txt` and
  `chart.candles.plain.txt`, plus `chart.ansi.txt` (default style) and
  `chart.html`. Extend `metrics.json` with deterministic fields: `rendered_bars`,
  `lod_active`, `style`.
- Update `docs/verification.md`: new layout description, style files, new fields,
  and an acceptance check that LOD reduces bars when candles > columns.

## 8. Tests (mapped to plan)

- **OHLC aggregation correctness** (`render-core`): `aggregate(_,1)` = overall
  O/H/L/C/Σvol; `N==target` identity; OHLC invariants hold; volume sum preserved;
  `start_ts` monotonic; even partitioning.
- **Viewport pan/zoom bounds** (`render-core`): pan clamps at both edges preserving
  width; zoom-out saturates to full range; zoom-in respects `MIN_CANDLES`; zoom
  keeps center; `full()` = whole series.
- **Chart dimensions** (`backend-terminal`): `render_chart` plain output = exactly
  `height` lines × `width` chars, with title + (optional) volume + time axis
  present.
- **No ANSI in plain snapshot** (`examples-bench`): retained/extended for both
  style files.
- **Style selection** (`backend-terminal`): OHLC output contains tick glyphs
  (`╴`/`╶`) and no body blocks; candle output contains `█`/`▒` and no ticks; the
  two differ.
- **HTML escaped/preformatted** (`examples-bench`): retained — `<pre>` +
  `white-space: pre`, up/down spans, entity-escaped, no raw ESC, no external
  assets.

## Dependencies summary

- Core crates: **still zero external deps** (+ tiny internal epoch formatter).
- `examples-tui`: **`crossterm` only**, justified above.
- **No** `serde_json` (hand-written metrics unchanged), no Canvas/WebGPU/gRPC/
  exchange/order-flow/plugins — all constraints honored.

## Implementation checklist (in order)

1. **`render-core::aggregate`** — `Bar` + `aggregate`, test-first (aggregation
   correctness suite).
2. **`render-core` Viewport navigation** — `pan`/`zoom`/`full` + `MIN_CANDLES`,
   test-first (bounds suite).
3. **`backend-terminal` time formatter** — `civil_from_days` epoch→UTC, test-first
   (known-date vectors).
4. **`backend-terminal` `ChartOptions` + `Style` + `render_chart`** — layout
   (title/gutter/plot/time axis), OHLC bar routine, candle routine reuse;
   test-first (dimensions + style selection). Wire LOD via `aggregate`.
5. **Volume panel** — add to `render_chart` under `show_volume`; dimension test
   with volume on.
6. **Migrate + delete old API** — replace `render`/`render_with` with
   `render_chart`; update internal tests; keep `Grid`/`draw_candles` primitives.
7. **Update `examples-bench`** — demo + snapshot use `render_chart`; emit
   both-style files + new metrics fields; update/extend snapshot tests.
8. **New `examples-tui` crate** — add to workspace, add `crossterm`; interactive
   loop (pan/zoom/reset/quit) with terminal restore + panic guard.
9. **Docs** — update `docs/verification.md` (V2 layout/files/fields) and
   `CLAUDE.md` (new crate, commands, style notes).
10. **Full verification** — `cargo test`, `cargo clippy --workspace --all-targets`,
    `cargo fmt --check`, run snapshot, manually run `examples-tui`; report artifact
    paths.

## Open decisions to confirm before starting

- **(2)** Removing the old `render`/`render_with` in favor of `render_chart`.
- **OHLC default style** with `╴`/`╶` tick glyphs.
- Emitting **both-style snapshot files**.
