# CLAUDE.md

Guidance for working in this repo.

## What this is

`finchart` is a renderer-agnostic financial charting engine. The current code
covers the **MVP** (synthetic OHLCV → viewport culling → terminal rendering →
benchmarks), **Terminal Renderer V2** (OHLC/candle styles, display-resolution LOD
aggregation, a laid-out chart with title/gutter/volume/time-axis, interactive
viewer), **Live Replay** (incremental candle building from synthetic ticks —
update the forming candle, roll over at interval boundaries, follow the latest
candle live), **Order-Flow V1** (aggressive buy/sell volume, per-candle delta,
cumulative delta, and a terminal delta panel), **Depth V1** (a Level 2 order book,
synthetic depth generation, and a right-side terminal depth panel), and
**Heatmap V1** (a bounded depth-history store and a coarse terminal liquidity
heatmap layer), and **Heatmap V1.1** (side-aware rendering, deterministic
market-structure scenarios, and executed-trade markers). See `docs/mvp.md`,
`docs/mvp-implementation-plan.md`, `docs/terminal-renderer-v2-plan.md`,
`docs/live-replay-plan.md`, `docs/order-flow-v1-plan.md`, `docs/depth-v1-plan.md`,
`docs/heatmap-v1-plan.md`, and `docs/heatmap-v1.1-plan.md`.

The broader vision (trades, order-book depth, footprint/heatmap, Canvas/WebGPU
backends, live feeds) is described in `README.md` / `docs/research.md` but is
**not implemented yet** and should not be added without an explicit milestone.

## Commands

```bash
cargo build                 # build the workspace
cargo test                  # run all unit tests (run after every change)
cargo run -p examples-tui --release     # interactive chart viewer (pan/zoom/reset/quit)
cargo run -p examples-live --release            # live synthetic replay (follows the forming candle; --fps N)
cargo run -p examples-live --release -- --snapshot artifacts/live  # deterministic before/mid/after frames
cargo run -p examples-live --release -- --scenario sweep --snapshot artifacts/scenario  # heatmap scenario (ask-wall, bid-wall, ask-wall-pulled, bid-wall-pulled, sweep, bid-reload)
cargo run -p examples-bench --release   # demo chart + 10k/100k/1M benchmark table
cargo run -p examples-bench --release -- --snapshot artifacts/tui  # export deterministic render snapshots
cargo clippy --workspace --all-targets  # lint (keep clean)
cargo fmt                   # format (keep clean; CI-style check: cargo fmt --check)
```

Rust toolchain: 1.96 (workspace `rust-version`). `Cargo.lock` is committed for
reproducible benchmarks (not gitignored).

## Workspace layout

```text
crates/
  market-core/       # Candle/CandleSeries, synth gen, live Tick/CandleBuilder/flow, depth OrderBook + LiquidityHeatmap, RNG
  render-core/       # Viewport (+ pan/zoom/tail), cull -> CulledView (zero-copy), aggregate -> LOD Bars, scales
  backend-terminal/  # render_chart: laid-out OHLC/candle chart (LOD, volume, axes); html export; pure
examples-bench/      # binary: demo chart + benchmark table + deterministic snapshot export
examples-tui/        # binary: interactive viewer (crossterm) — pan/zoom/reset/quit
examples-live/       # binary: live synthetic replay (crossterm) — incremental candles, follow latest, snapshot
```

Dependency direction: `backend-terminal` → `render-core` → `market-core`.
The example binaries depend on all three. `crossterm` is used **only** by
`examples-tui` and `examples-live`; the core crates stay zero-dependency.

## Architecture notes / conventions

- **Columnar hot path.** Candles are stored as parallel `Vec`s (one per OHLCV
  field) in `CandleSeries`, not as a `Vec<Candle>`. `Candle` is a convenience
  accessor type. `CulledView` borrows column *slices* — culling is zero-copy.
- **Index-based viewport + navigation.** Culling is by candle index
  (`[start, end)`), not by time. `Viewport::pan/zoom/full` are pure and clamped
  (`MIN_CANDLES` floor); the TUI's keys are thin wrappers over them. There is
  intentionally no `TimeRange` type yet — nothing consumes it.
- **Display-resolution LOD.** `render_core::aggregate(view, target_bars)` buckets
  visible candles into OHLC `Bar`s when candles exceed columns, so the renderer
  never overplots (preserves O/H/L/C + summed volume). `target_bars` comes from
  `plot_cols / style.cell_width`.
- **Incremental candle building (live).** `market_core::CandleBuilder` folds a
  `Tick` stream into a `CandleSeries`: the forming candle is the *last element*,
  mutated in place via `CandleSeries::update_last` (new open = prior close, then
  high/low/close/volume fold in per tick); an interval boundary pushes a new
  candle (`CandleUpdate::Rolled`). Rendering culls the series directly — the live
  candle is just the last bar, so `render_chart`/LOD/follow all work unchanged.
  `synth::ticks(LiveConfig)` is the deterministic synthetic tick source;
  `Viewport::tail` follows the newest candle.
- **Order flow.** `CandleSeries` carries `buy_volume`/`sell_volume` columns
  (delta = buy − sell, *derived* not stored); `add_flow_last` accumulates them and
  `update_last` leaves them intact. `CandleBuilder::apply` routes each tick's size
  by `tick.side` (`Side::Buy`/`Sell` = aggressor; `None` = volume only); it also
  exposes `current_flow()` and `cumulative_delta()` (CVD). Flow rides the same
  path: `CulledView` has `buy`/`sell` slices, `Bar` has `buy_volume`/`sell_volume`
  + `delta()`, and `render_chart`'s `show_delta` panel draws per-bar delta as
  green-up / red-down bars around a `─` baseline. `generate()` fills history flow
  from a separate RNG stream (so OHLCV bytes are unchanged), skewed by candle
  direction.
- **Level 2 depth.** `market_core::OrderBook` is keyed by integer price *ticks*
  (`round(price / tick_size)` → `BTreeMap<i64, f64>` per side), so levels match
  exactly across updates and each side stays sorted for free; prices are `f64` at
  the API. It's a faithful feed mirror: `apply_snapshot`/`apply_update`
  (`size == 0` deletes), `best_bid`/`best_ask`/`spread`/`best_bid_ask`, `top_n`
  (best-first), `level_counts`. Neutral events: `DepthSnapshot`, `DepthUpdate`,
  `DepthLevel`, `BookSide`, `BestBidAsk`. `synth::DepthGen` produces a
  deterministic non-crossing ladder around a mid plus liquidity-shifting updates.
  `render_chart`'s `depth: Option<DepthPanel>` draws a fixed-width right-side
  ladder (top asks / spread / top bids) — total dimensions stay `width × height`.
- **Liquidity heatmap.** `market_core::LiquidityHeatmap` is a bounded rolling
  window (`VecDeque`) of time-bucket columns, each holding **separate bid/ask**
  `BTreeMap<i64, f64>` liquidity per price *tick* — absolute keys, so price drift
  is handled and price→row bucketing happens at render time. `record(&book, ts)`
  snapshots the book each depth step; accessors give `time_buckets`,
  `price_buckets`, `max_liquidity`, `cells_in_price_range`, combined
  `columns_in_range`, and **side-aware `columns_in_range_sided`** (render export).
  `render_chart`'s `heatmap: Option<HeatmapView>` draws a coarse `· ░ ▒ ▓`
  intensity **background layer** (newest column → rightmost) *before* the bars, so
  candles draw on top; dimensions stay `width × height`. It is **side-aware**
  (cells carry `is_bid`; bid green / ask red), **normalized by the max rendered row
  total** (cells don't saturate), the ramp is **capped at `▓`** (no full block),
  and a `· heatmap bid/ask` legend appears in the status line.
  `ChartOptions.trades: Option<TradeMarkers>` overlays executed trades (`▲` buy /
  `▼` sell) above the heatmap, below candles. The heatmap is **off by default in
  the interactive viewer** (`ReplayConfig.show_heatmap`); it is on in the live
  snapshot and scenarios. Not a footprint chart.
- **Scenarios.** `market_core::scenario(ScenarioConfig)` produces deterministic
  market-structure step sequences (`ScenarioKind`: static/pulled ask/bid walls, ask
  sweep, bid reload) — each step a book snapshot + executed trades. The
  `examples-live --scenario <name> --snapshot <dir>` runner renders them with the
  heatmap + trade markers so structure (walls, pulls, sweeps) is legible.
- **Autoscale is viewport-scoped.** `CulledView::price_extent()` gives the
  `(min_low, max_high)` of the *visible* candles; the price scale uses that, so
  the axis never reflects the whole dataset.
- **Scales clamp and guard.** `PriceScale` handles a flat price range
  (centers instead of dividing by zero). Both scales clamp into range.
- **`render_chart` is the single rendering API** (the old `render`/`render_with`
  were removed in V2). It's pure (returns a `String`) and composes a status line,
  price gutter, OHLC/candle plot, optional volume panel, and a UTC time axis.
  `Style::OhlcBars` (default, `╴`/`╶` ticks) and `Style::Candles` (`█`/`▒`/`─`);
  direction is in the glyph so plain output reads without color, ANSI reinforces.
  Only data cells are colored — gutter/axis/title stay default. `rendered_bar_count`
  reports the post-LOD bar count using the same gutter/cell math.
- **Time axis is dependency-free & deterministic.** `backend-terminal::timefmt`
  formats epoch→UTC (`MM-DD HH:MM`) via Howard Hinnant's `civil_from_days` — no
  chrono, no "now", no locale.
- **No external runtime dependencies in the core.** RNG, ANSI, JSON, HTML export,
  and the date formatter are hand-rolled (std only). `crossterm` is confined to the
  interactive binaries (`examples-tui`, `examples-live`). Prefer minimal deps (see
  `docs/mvp-implementation-plan.md` §10).

## Snapshot / verification workflow

`cargo run -p examples-bench --release -- --snapshot <dir>` exports deterministic
render artifacts from fixed inputs (seed/count/viewport/size — no time or
terminal-size dependence) for inspection and diffing after each change:

- `chart.ohlc.plain.txt` — default OHLC style, uncolored, fixed dims (120×30), no escapes.
- `chart.candles.plain.txt` — candle style, same.
- `chart.ansi.txt` — OHLC chart with ANSI colors.
- `chart.html` — self-contained dark-bg monospace page (`<pre>`, up/down spans).
- `metrics.json` — dataset/visible/`rendered_bars`/`lod_active`/`style`/size +
  gen/cull/render timings + memory estimate.

The four `chart.*` files are byte-deterministic (good golden files);
`metrics.json` timings vary per run (its other fields are deterministic). Snapshot
logic lives in `examples-bench/src/snapshot.rs`; JSON is hand-written (no
`serde_json`) since the record is a flat fixed set of numbers.

`cargo run -p examples-live --release -- --snapshot <dir>` is the **live** analogue:
fixed config + fixed advance points capture a forming candle just opened
(`frame.before.plain.txt`), half formed with an expanded wick
(`frame.mid.plain.txt`), and after roll-over (`frame.after.plain.txt`), plus
`frame.mid.html` and `metrics.json`. The frames render the **delta panel** and a
right-side **depth panel** (the status line shows `Δ`/`CVD` and `bid`/`ask`/`spr`);
a **heatmap** background layer; `metrics.json` adds order-flow fields
(`buy_volume`, `sell_volume`, `current_delta`, `cumulative_delta`), depth fields
(`best_bid`, `best_ask`, `spread`, `bid_depth_levels`, `ask_depth_levels`), and
heatmap fields (`heatmap_time_buckets`, `heatmap_price_buckets`,
`visible_heatmap_cells`, `max_liquidity`) on top of update/render timings. The
three frame texts are byte-deterministic; timings are not. Logic lives in
`examples-live/src/snapshot.rs`; HTML uses the shared `backend_terminal::html`.

See `docs/verification.md` for the full workflow and acceptance criteria.

## Testing

- TDD is the workflow here: every behavior was written test-first. Add a failing
  test before production code; keep `cargo test` and `cargo clippy` clean.
- Determinism is a hard requirement of the generator: same `GenConfig` ⇒
  byte-identical `CandleSeries`. There are tests guarding this — don't break it.
- Benchmark timings are not asserted (nondeterministic); only structural
  invariants (counts, memory estimate) are tested.

## Out of scope for the MVP

Do not add without an explicit new milestone: WebGPU, browser Canvas, exchange/
broker APIs, gRPC/protobuf, async/networking, indicators, plugin systems,
order-book heatmaps, footprint charts. The existing C++ matching engine
(`docs/future-integrations.md`) is a *future adapter*, not a dependency — the
core stays data-source agnostic.
