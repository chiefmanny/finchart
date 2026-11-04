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
heatmap layer), **Heatmap V1.1** (side-aware rendering, deterministic
market-structure scenarios, and executed-trade markers), **Feed Adapter V1**
(a source-agnostic `MarketEvent` ingestion boundary + `ChartState` sink, a
synthetic event source, and deterministic event record/replay), **Matching
Engine Adapter V1** (an isolated `examples-matching-engine` gRPC adapter that
translates the external matching engine's streams into `MarketEvent`s), and
**Web Showcase V1** (the engine compiled to WebAssembly via the `finchart-web`
crate, replaying recorded scenarios in a browser xterm.js terminal — the `web/`
static front-end). See
`docs/mvp.md`, `docs/mvp-implementation-plan.md`,
`docs/terminal-renderer-v2-plan.md`, `docs/live-replay-plan.md`,
`docs/order-flow-v1-plan.md`, `docs/depth-v1-plan.md`, `docs/heatmap-v1-plan.md`,
`docs/heatmap-v1.1-plan.md`, `docs/feed-adapter-v1-plan.md`,
`docs/matching-engine-adapter-v1-plan.md`, and `docs/web-showcase-v1-plan.md`.

The broader vision (trades, order-book depth, footprint/heatmap, Canvas/WebGPU
backends, live feeds) is described in `README.md` / `docs/research.md` but is
**not implemented yet** and should not be added without an explicit milestone.

## Commands

```bash
cargo build                 # build the workspace
cargo test                  # run all unit tests (run after every change)
cargo install --path finchart           # install the `finchart` CLI (or: cargo run --release -p finchart -- <cmd>)
finchart help                           # list commands; `finchart help <cmd>` for options
finchart live                           # live synthetic replay (follows the forming candle; --fps N)
finchart browse                         # interactive viewer over 5,000 candles (pan/zoom/reset/quit)
finchart replay web/scenarios/large_buy_sweep.txt --start 0:56 --speed 4   # interactive player (space ←→ ↓↑ [] hjkl f m ?)
finchart replay FILE --at 1:28          # print one frame (also when piped: plain text, final frame)
finchart snapshot live artifacts/live   # deterministic before/mid/after frames
finchart scenario sweep artifacts/scenario  # heatmap scenario (ask-wall, bid-wall, ask-wall-pulled, bid-wall-pulled, sweep, bid-reload)
finchart record artifacts/feed/events.txt   # record the synthetic MarketEvent stream (with a `# finchart interval= tick=` header)
cargo run -p examples-matching-engine -- --replay artifacts/feed/events.txt   # offline: replay an event file through the gRPC adapter (no engine)
cargo run -p examples-matching-engine -- --endpoint http://host:50051 --symbol BTCUSD  # live: connect to the matching engine (needs a running server)
docs/media/render.sh                    # re-record the README GIFs/PNGs from docs/media/*.tape (needs vhs + ffmpeg)
cargo test -p finchart-web              # host unit tests for the WASM chart binding (no wasm needed)
./web/build.sh                          # build the WASM bundle -> web/pkg (needs wasm-pack + wasm32 target)
python3 -m http.server --directory web  # serve the browser demo at http://localhost:8000
finchart bench                          # demo chart + 10k/100k/1M benchmark table
finchart snapshot chart artifacts/tui   # export deterministic render snapshots
cargo clippy --workspace --all-targets  # lint (keep clean)
cargo fmt                   # format (keep clean; CI-style check: cargo fmt --check)
```

Rust toolchain: 1.96 (workspace `rust-version`). `Cargo.lock` is committed for
reproducible benchmarks (not gitignored).

## Workspace layout

```text
crates/
  market-core/       # Candle/CandleSeries, synth gen, live Tick/CandleBuilder/flow, depth OrderBook + LiquidityHeatmap, MarketEvent + ChartState, RNG
  render-core/       # Viewport (+ pan/zoom/tail), cull -> CulledView (zero-copy), aggregate -> LOD Bars, scales
  backend-terminal/  # render_chart: laid-out OHLC/candle chart (LOD, volume, axes); html export; pure
finchart/            # binary `finchart` (crossterm): cli.rs (pure hand-rolled parser), term.rs (Screen), frame.rs, live/ (live, snapshot live, scenario, record), browse.rs, player.rs (replay), bench/ (bench, snapshot chart)
examples-matching-engine/  # binary: gRPC matching-engine adapter — translate streams -> MarketEvent -> ChartState (tonic/tokio confined here)
finchart-web/        # lib (cdylib+rlib): wasm-bindgen wrapper — WasmChart folds MarketEvent lines -> ChartState -> render_chart (ANSI); wasm-bindgen confined here
web/                 # static browser demo: index.html + main.js (xterm.js) + build.sh + scenarios/*.txt; pkg/ is generated (gitignored)
```

Dependency direction: `backend-terminal` → `render-core` → `market-core`.
The binaries depend on all three. The core crates stay **zero-dependency**:
`crossterm` is confined to `finchart`/`examples-matching-engine`, all
gRPC/async deps (`tonic`, `prost`, `tokio`) live **only** in
`examples-matching-engine`, and `wasm-bindgen` lives **only** in `finchart-web`
(all verified via `cargo tree`). The adapter's vendored proto is compiled by
`build.rs` (`tonic-build` + a bundled `protoc`).

## Architecture notes / conventions

- **One CLI.** `finchart` absorbed the old `examples-live`/`-tui`/`-bench`
  binaries (same code, now subcommands; their snapshot output is byte-identical).
  `cli::parse` is pure and unit-tested (no clap); `term::Screen` owns raw mode +
  the alternate screen and repaints in place (full clear only on resize);
  `frame::render` is the standard ChartState frame (plot + heatmap + volume +
  delta + ladder). `player::Player` is the `replay` core: a clock over the
  recording's own timestamps that folds events up to it (seeking back refolds
  from the start, so any second shows the exact state then), mpv-style keys,
  pan/zoom/follow; terminal-free and tested. Settings resolve flags > the file's
  `#` header > defaults (1 s, 0.01 — the engine recordings). The gRPC adapter
  stays a separate binary so `finchart` never builds tonic/tokio.
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
  candle (`CandleUpdate::Rolled`). The series is **time-linear**: quiet intervals
  become flat zero-volume candles at the last close, both when the next tick lands
  and via `advance_to(ts)`, which `ChartState` calls on every `Heartbeat` so a quiet
  market keeps advancing (no candle before the first price; gaps longer than
  `MAX_GAP_FILL` intervals are skipped, not filled). Rendering culls the series
  directly — the live candle is just the last bar, so `render_chart`/LOD/follow all
  work unchanged.
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
  `columns_in_range`, and **side-aware `columns_in_range_sided`** (render export,
  each column with its `ts`; `HeatmapView::from_heatmap` converts it).
  `render_chart`'s `heatmap: Option<HeatmapView>` draws a coarse `· ░ ▒ ▓`
  intensity **background layer** *before* the bars, so candles draw on top;
  dimensions stay `width × height`. **Time-aligned:** each column goes under the
  bar whose time span contains its `ts` (filling that bar's cell; columns sharing a
  bar are averaged); columns before the first visible candle or after the last
  candle's interval are dropped. Trade markers and annotations use the same
  timestamp→bar mapping (`BarSpans` in `render.rs`). It is **side-aware**
  (cells carry `is_bid`; bid green / ask red), **normalized by the max rendered row
  total** (cells don't saturate), the ramp is **capped at `▓`** (no full block),
  and a `· heatmap bid/ask` legend appears in the status line.
  `ChartOptions.trades: Option<TradeMarkers>` overlays executed trades (`▲` buy /
  `▼` sell, each with a `ts`) in the right-hand column of their bar — drawn on top
  of multi-column bars (that column only holds the close tick), under one-column
  ones. The heatmap is **off by default in
  the interactive viewer** (`ReplayConfig.show_heatmap`); it is on in the live
  snapshot and scenarios. Not a footprint chart.
- **Scenarios.** `market_core::scenario(ScenarioConfig)` produces deterministic
  market-structure step sequences (`ScenarioKind`: static/pulled ask/bid walls, ask
  sweep, bid reload) — each step a book snapshot + executed trades. The
  `finchart scenario <name> [dir]` runner renders them with the
  heatmap + trade markers so structure (walls, pulls, sweeps) is legible.
- **Neutral feed boundary.** All ingestion goes through one path:
  `market_core::MarketEvent` (`Trade`/`DepthSnapshot`/`DepthUpdate`/`Heartbeat`) →
  `ChartState::apply` → `ApplyResult`. `ChartState` owns the `CandleBuilder` +
  `OrderBook` + optional `LiquidityHeatmap` + a bounded recent-trades ring;
  `Trade` folds a candle, depth events update the book, and `Heartbeat` is the
  neutral "end-of-bucket" tick that advances the candle clock (flat candles for
  quiet intervals) and records a heatmap column (so cadence is source-controlled,
  not wired into the book). Both runners (`finchart live`'s
  `Replay` and `scenarios`) build `MarketEvent`s and apply them — they no longer
  mutate the three structures directly. `DepthGen::step(ts, mid)` is **pure**: it
  returns a `DepthDelta` (`Snapshot`/`Updates`) instead of mutating a book.
  `synth::events(EventConfig)` is the deterministic `MarketEvent` source
  (`EventSource: Iterator`): a seeding snapshot + heartbeat, then a `Trade` per
  tick with a depth delta + heartbeat every `depth_every` trades — folding it into
  a `ChartState` reproduces the live replay. This is the adapter seam for future
  file/gRPC/exchange sources (see `docs/future-integrations.md`); none are
  implemented.
- **Event record/replay.** A hand-rolled line-based text format (no deps; f64
  shortest round-trip) via `serialize_event(s)`/`parse_event(s)`;
  `replay_events(&mut ChartState, &[MarketEvent])` folds a recorded slice. Replaying
  recorded events into a fresh `ChartState` reproduces the original state exactly
  (`ChartState: PartialEq`). `finchart record <file>` writes a stream (after a
  `# finchart interval=… tick=…` header; parsers skip `#` lines) and `finchart
  replay <file>` plays one; `MarketEvent::ts()` gives any event's time.
- **Matching-engine adapter (isolated).** `examples-matching-engine` is the first
  real feed adapter: pure `translate` fns map gRPC `FillNotification →
  MarketEvent::Trade` and `Level2Response → DepthSnapshot`, async tokio producers
  (`Subscribe` stream, `GetLevel2` poll, `Heartbeat` ticker) push onto an `mpsc`
  channel, and a sync render loop drains it into a `ChartState` + `render_chart`.
  All gRPC/async deps are confined to this crate; the cores stay zero-dep. It reuses
  the neutral event format, so `--replay <file>` runs offline and its recordings
  play in `finchart replay`. The vendored `proto/matching_engine.proto` is a **faithful
  copy of the real service** (`package matchingengine`; `FillNotification` has
  `symbol` + integer `size`; `Level2Request` is `side=2, depth=3`, nested `Side` enum
  `BOTH=2`; levels are `PriceLevelData`) — wire-compatible. Aggressor side comes
  from `FillNotification.aggressor_is_buy` (tag 10); the legacy `buy` field (tag 4,
  "bid order fully consumed") is ignored, and `--invert-trade-side` was removed.
  `--headless` + `--duration <secs>` record unattended (no TUI), and the recorder
  flushes on every exit path. One remaining assumption: events use adapter
  **receive time** (the proto has no source timestamp), so live candle bucketing is
  arrival-timed and non-deterministic (replay is deterministic). The engine's `cpp-server:50051` is
  Docker-internal by default — expose it (`ports: ["50051:50051"]`) to connect. Not a
  new core boundary — just a `MarketEvent` source.
- **Web showcase (WASM, isolated).** `finchart-web` compiles the same pure pipeline
  to WebAssembly: `WasmChart` (`#[wasm_bindgen]`) folds serialized `MarketEvent`
  lines (`parse_event`) through a `ChartState` and calls `render_chart` to return an
  ANSI `String`; `pan`/`zoom`/`follow` drive a `Viewport` (tail when following). The
  render assembly (depth + side-aware heatmap + volume/delta + status line) is
  duplicated locally so the crate depends only on the three cores + `wasm-bindgen`
  (never on `examples-matching-engine`); trade markers are off in V1. The `web/`
  static front-end loads the `wasm-pack --target web` bundle + xterm.js (which
  interprets the ANSI natively), fetches a recorded `web/scenarios/*.txt`, and runs a
  batched player loop with pan/zoom/follow. **Recorded replay only — no backend, no
  live feed, no Canvas/WebGPU** (the terminal backend shown in a browser emulator).
  Cores stay pure and WASM-safe; nothing existing changed. The logic is host-testable
  via the crate's `rlib` (`cargo test -p finchart-web`). `WasmChart` also offers
  `render_html` (frames as `<pre>`-ready HTML via `html::ansi_to_html`, used by the
  personal site instead of xterm.js), `set_dense_bars` (one-column `Style::Candles`
  so wide charts show more bars), and `add_annotation`/`clear_annotations`.
- **Autoscale is viewport-scoped.** `CulledView::price_extent()` gives the
  `(min_low, max_high)` of the *visible* candles; the price scale uses that, so
  the axis never reflects the whole dataset.
- **Scales clamp and guard.** `PriceScale` handles a flat price range
  (centers instead of dividing by zero). Both scales clamp into range.
- **`render_chart` is the single rendering API** (the old `render`/`render_with`
  were removed in V2). It's pure (returns a `String`) and composes a status line,
  price gutter, OHLC/candle plot, optional volume panel, and a UTC time axis.
  `ChartOptions.annotations` (`Annotation { ts, label }`) draws a dotted rule + label
  at the bar containing `ts` (ANSI 33; skipped when outside the visible candles);
  the terminal binaries pass none.
  `Style::OhlcBars` (default, `╴`/`╶` ticks) and `Style::Candles` (`█`/`▒`/`─`);
  direction is in the glyph so plain output reads without color, ANSI reinforces.
  Only data cells are colored — gutter/axis/title stay default. `rendered_bar_count`
  reports the post-LOD bar count using the same gutter/cell math.
- **Time axis is dependency-free & deterministic.** `backend-terminal::timefmt`
  formats epoch→UTC (`MM-DD HH:MM`) via Howard Hinnant's `civil_from_days` — no
  chrono, no "now", no locale.
- **No external runtime dependencies in the core.** RNG, ANSI, JSON, HTML export,
  and the date formatter are hand-rolled (std only). `crossterm` is confined to the
  binaries (`finchart`, `examples-matching-engine`). Prefer minimal deps (see
  `docs/mvp-implementation-plan.md` §10).

## Snapshot / verification workflow

`finchart snapshot chart <dir>` exports deterministic
render artifacts from fixed inputs (seed/count/viewport/size — no time or
terminal-size dependence) for inspection and diffing after each change:

- `chart.ohlc.plain.txt` — default OHLC style, uncolored, fixed dims (120×30), no escapes.
- `chart.candles.plain.txt` — candle style, same.
- `chart.ansi.txt` — OHLC chart with ANSI colors.
- `chart.html` — self-contained dark-bg monospace page (`<pre>`, up/down spans).
- `metrics.json` — dataset/visible/`rendered_bars`/`lod_active`/`style`/size +
  gen/cull/render timings + column memory (`CandleSeries::column_bytes`).

The four `chart.*` files are byte-deterministic (good golden files);
`metrics.json` timings vary per run (its other fields are deterministic). Snapshot
logic lives in `finchart/src/bench/snapshot.rs`; JSON is hand-written (no
`serde_json`) since the record is a flat fixed set of numbers.

`finchart snapshot live <dir>` is the **live** analogue:
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
`finchart/src/live/snapshot.rs`; HTML uses the shared `backend_terminal::html`.

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
broker APIs, indicators, plugin systems, footprint charts. gRPC/protobuf and
async/networking are permitted **only inside `examples-matching-engine`** (the
matching-engine adapter, `docs/matching-engine-adapter-v1-plan.md`), and
`wasm-bindgen` **only inside `finchart-web`** (the web showcase,
`docs/web-showcase-v1-plan.md`) — never in
`market-core`/`render-core`/`backend-terminal`, which stay data-source agnostic and
zero-dependency. The web showcase reuses the existing terminal renderer via WASM
(xterm.js displays its ANSI); it is **not** a Canvas/WebGPU backend, and V1 is
recorded-replay only (no live WebSocket bridge). The external C++ matching engine
remains an *adapter target*, not a core dependency; the adapter maps its streams to
neutral `MarketEvent`s.
