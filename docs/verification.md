# Verification

This project includes a deterministic snapshot/export workflow for the terminal
renderer. Its purpose is to let humans and coding agents inspect concrete,
reproducible rendered output after every change, and to catch regressions via text
diffing and tests.

For hands-on exploration there are interactive viewers (not part of the
deterministic flow):

```bash
cargo run --release -p finchart -- browse    # static: pan h/l ←/→  zoom k/j +/-  reset r  quit q
cargo run --release -p finchart -- live   # live replay: follows the forming candle (--fps N); quit q
```

This doc has two deterministic snapshot flows: the **static chart** snapshot
(`finchart snapshot chart`, below) and the **live replay** snapshot (`finchart snapshot live`, see
"Live replay verification").

## Command

```bash
cargo run --release -p finchart -- snapshot chart artifacts/tui
```

The argument is the output directory (created if missing). `artifacts/tui` is the
conventional location; `artifacts/` is gitignored, so snapshots are regenerable
local output, never committed.

The snapshot uses **fixed, deterministic inputs** — no current time, no
terminal-size dependence:

| Input | Value |
|---|---|
| seed | 42 |
| candle count | 500 |
| viewport | indices `[300, 420)` → 120 visible candles |
| chart size | 120 columns × 30 rows |

These are defined in `finchart/src/bench/snapshot.rs`. Changing them changes the
golden output, so treat them as fixtures.

## Chart layout (V2)

`render_chart` composes a laid-out chart:

```text
row 0            title / status line  (label · candles → bars · price range · style)
rows 1..M        price gutter + main plot (OHLC bars or candles)
rows M..V        volume panel (block bars per displayed bar)
last row         time axis (UTC MM-DD HH:MM ticks)
```

When the visible candle count exceeds the available plot columns, the renderer
**aggregates** candles into OHLC bars (display-resolution level-of-detail) instead
of overplotting — open/high/low/close and summed volume are preserved per bar.

## Files produced

| File | What it is |
|---|---|
| `chart.ohlc.plain.txt` | Default OHLC-bar style. Uncolored, exact fixed dimensions, no escapes. Primary diff target. |
| `chart.candles.plain.txt` | Block-candle style, same fixed dimensions. |
| `chart.ansi.txt` | OHLC chart with ANSI green/red colors. View with `cat`/`less -R`. |
| `chart.html` | Self-contained dark-background monospace page (`<pre>`). Open in a browser to see colors without a terminal. No external assets. |
| `metrics.json` | Dataset/render stats (see below). |

`metrics.json` fields: `dataset_size`, `visible_candles`, `rendered_bars`,
`lod_active`, `style`, `chart_width`, `chart_height`, `generation_us`,
`culling_us`, `render_us`, `memory_bytes`.

## What to inspect

1. **`chart.ohlc.plain.txt`** — the primary artifact. Read it directly; diff it
   against a previous run to see exactly what a change altered.
2. **`chart.candles.plain.txt`** — confirm the alternate style still renders.
3. **`chart.html`** — open in a browser to confirm up/down colors and spacing.
4. **`chart.ansi.txt`** — `cat artifacts/tui/chart.ansi.txt` (or `less -R`) to
   confirm colors in a real terminal.
5. **`metrics.json`** — sanity-check sizes, and that `lod_active`/`rendered_bars`
   reflect the expected downsampling. Timings are informational, not pass/fail.

## Visual acceptance criteria

A snapshot is acceptable when:

- [ ] **Chart fits fixed dimensions.** Each `chart.*.plain.txt` is exactly 30
      lines, each 120 characters wide. Quick check:
      ```bash
      awk '{ n++ } END { print n " lines" }' artifacts/tui/chart.ohlc.plain.txt   # 30
      # character width (glyphs are multi-byte UTF-8, so count chars, not bytes):
      python3 -c "print(sorted({len(l.rstrip(chr(10))) for l in open('artifacts/tui/chart.ohlc.plain.txt')}))"  # [120]
      ```
- [ ] **Title/status line is informative.** Row 0 shows the label, candle→bar
      counts, price range, and style.
- [ ] **Price axis is readable.** The left gutter shows price labels (top = max,
      middle, bottom = min), right-aligned and not colliding with the plot.
- [ ] **Candles / OHLC bars are not visually overlapping.** OHLC bars show a
      high-low line with open (`╴`) / close (`╶`) ticks; candles show `█`/`▒`
      bodies with `─` dojis. LOD keeps bars from smearing into an unreadable block.
- [ ] **Volume panel** shows one block bar per displayed bar, aligned under the
      price bars, with a max-volume gutter label.
- [ ] **Time axis** shows UTC `MM-DD HH:MM` ticks (left/right, plus a centered one
      when there's room).
- [ ] **LOD is active when expected.** With 120 visible candles in ~113 plot
      columns at OHLC cell width 3, the chart draws ~37 bars. Check:
      ```bash
      grep -E '"rendered_bars"|"visible_candles"|"lod_active"' artifacts/tui/metrics.json
      # rendered_bars < visible_candles, lod_active: true
      ```
- [ ] **No benchmark/debug clutter** in the snapshot artifacts unless explicitly
      requested. (The benchmark table is part of the plain `cargo run` demo output,
      not the snapshot files.)
- [ ] **Deterministic output for the same seed.** The four `chart.*` files are
      byte-identical across runs. Verify:
      ```bash
      cp artifacts/tui/chart.ohlc.plain.txt /tmp/prev.txt
      cargo run -q --release -p finchart -- snapshot chart artifacts/tui
      diff /tmp/prev.txt artifacts/tui/chart.ohlc.plain.txt && echo "deterministic"
      ```
      Note: `metrics.json` contains wall-clock timings that vary per run, so it is
      intentionally **not** byte-deterministic. Only the `chart.*` files are.

## Live replay verification

Proves incremental candle updates, order flow, Level 2 depth, and a liquidity
heatmap: a forming candle updated from synthetic ticks, its wick expanding, then
closing and rolling into a new candle, with aggressive buy/sell volume tracked into
a delta panel, a synthetic order book shown in a right-side depth panel, and the
book's liquidity accumulated over time into a background heatmap layer.

```bash
cargo run --release -p finchart -- snapshot live artifacts/live
```

Fixed, deterministic inputs (no wall-clock, no terminal size): seed 42, 200
history candles, 60s interval, 30 ticks/candle, 60-candle follow window, 120×30
chart. Three frames are captured at fixed tick offsets around a candle boundary
(just opened / half formed / just after roll-over). Defined in
`finchart/src/live/snapshot.rs`.

### Files produced

| File | What it is |
|---|---|
| `frame.before.plain.txt` | Forming candle just opened (narrow body/wick). |
| `frame.mid.plain.txt` | Same candle half formed — wick has expanded. |
| `frame.after.plain.txt` | The candle has closed; a new one just opened. |
| `frame.mid.html` | The mid frame as a self-contained HTML page (shared `backend_terminal::html`). |
| `metrics.json` | Replay stats + order flow + depth + heatmap: `history_candles`, `interval_secs`, `ticks_per_candle`, `total_ticks`, `frames`, `avg_update_us`, `render_us`, chart dims, `buy_volume`, `sell_volume`, `current_delta`, `cumulative_delta`, `best_bid`, `best_ask`, `spread`, `bid_depth_levels`, `ask_depth_levels`, `heatmap_time_buckets`, `heatmap_price_buckets`, `visible_heatmap_cells`, `max_liquidity`. |

The frames render with the **delta panel** (price + volume + order-flow delta), a
right-side **depth panel** (top bid/ask ladder + spread), and a **liquidity heatmap**
background layer (coarse `· ░ ▒ ▓ █` intensity under the candles); the status line
carries the current candle's `Δ`/`CVD`, the book's `bid`/`ask`/`spr`, and the
heatmap's `liq`/`cells`.

### Acceptance criteria

- [ ] **Frames fit fixed dimensions** — each `frame.*.plain.txt` is 30 lines × 120
      chars, no ANSI escapes.
- [ ] **The candle visibly forms.** The status line (row 0) of each frame shows the
      forming candle's `O/H/L/C` and tick count. Compare:
      ```bash
      for f in before mid after; do head -1 artifacts/live/frame.$f.plain.txt; done
      # before: narrow H–L; mid: wider H–L (wick expanded); after: open == prior close, narrow again
      ```
- [ ] **Roll-over happened.** The `after` frame's forming candle `open` equals the
      prior candle's close, and its tick count crossed an interval boundary.
- [ ] **Order flow is tracked.** The status line shows `Δ` and `CVD`, the frames
      draw a delta panel (a long `─` baseline with green-up / red-down bars), and
      `metrics.json` reports the flow fields with `current_delta == buy − sell`:
      ```bash
      grep -E '"buy_volume"|"sell_volume"|"current_delta"|"cumulative_delta"' artifacts/live/metrics.json
      ```
- [ ] **Depth is tracked.** The status line shows `bid`/`ask`/`spr`, the frames draw
      a right-side depth panel (`ASK`/`BID` headers, a non-crossing ladder, `spr`
      row), and `metrics.json` reports the book fields:
      ```bash
      grep -E '"best_bid"|"best_ask"|"spread"|"bid_depth_levels"|"ask_depth_levels"' artifacts/live/metrics.json
      # best_bid < best_ask; spread == best_ask - best_bid
      ```
- [ ] **Liquidity accumulates into a heatmap.** The frames draw a coarse, side-aware
      `· ░ ▒ ▓` background layer under the candles (bid green / ask red, no full
      blocks), the status line shows `liq`/`cells`, and `metrics.json` reports the
      heatmap fields:
      ```bash
      grep -E '"heatmap_time_buckets"|"heatmap_price_buckets"|"visible_heatmap_cells"|"max_liquidity"' artifacts/live/metrics.json
      ```
      (The heatmap is **off by default in the interactive viewer**; it is on in this
      snapshot and in scenarios.)
- [ ] **Deterministic for the same seed** — frame text is byte-identical across runs:
      ```bash
      cp artifacts/live/frame.mid.plain.txt /tmp/prev.txt
      cargo run -q --release -p finchart -- snapshot live artifacts/live
      diff /tmp/prev.txt artifacts/live/frame.mid.plain.txt && echo "deterministic"
      ```
      (`metrics.json` timings vary per run; the frame text does not.)

## Scenario verification

Makes the heatmap interpretable by rendering deterministic market-structure
fixtures with the side-aware heatmap + executed-trade markers.

```bash
cargo run --release -p finchart -- scenario <name> artifacts/scenario
# names: ask-wall, bid-wall, ask-wall-pulled, bid-wall-pulled, sweep, bid-reload
```

Each scenario emits `frame.before/mid/after.plain.txt`, `frame.mid.html`, and a
`metrics.json` with `scenario`, `steps`, heatmap stats, and `trade_markers`. The
runner zooms the price view to the book so structure is legible. Fixtures and
config are in `finchart/src/live/scenarios.rs` and `market-core`'s `scenario`
module.

### Acceptance criteria

- [ ] **Frames fit fixed dimensions** — each `frame.*.plain.txt` is 30 lines × 120
      chars, no ANSI escapes.
- [ ] **Walls are visible.** A wall scenario renders a bright `▓` band at the wall
      price (it dominates the faint `·` ladder); `metrics.json` `max_liquidity`
      equals the wall size:
      ```bash
      cargo run -q --release -p finchart -- scenario ask-wall artifacts/scenario
      grep -E '"scenario"|"max_liquidity"' artifacts/scenario/metrics.json   # ask-wall, 200
      grep -o '▓' artifacts/scenario/frame.mid.plain.txt | wc -l             # wall band cells
      ```
- [ ] **Wall pull changes the frame.** For `ask-wall-pulled`, the `before` (wall
      present) and `after` (wall gone) frames differ.
- [ ] **Sweep shows trade markers.** `sweep` renders climbing `▲` buy markers
      through the consumed ask liquidity; `metrics.json` `trade_markers` > 0.
- [ ] **Deterministic for the same seed** — frame text is byte-identical across runs.

## Feed adapter (record / replay)

All ingestion flows through one neutral path — `MarketEvent` → `ChartState::apply`
— so synthetic data, scenarios, and (future) file/gRPC sources share it. Events
serialize to a tiny line-based text format for deterministic record/replay.

```bash
cargo run --release -p finchart -- record artifacts/feed/events.txt
cargo run --release -p finchart -- replay artifacts/feed/events.txt          # interactive player
cargo run --release -p finchart -- replay artifacts/feed/events.txt --at 5:00   # one frame
```

`replay` plays interactively on a terminal; with `--at TIME`, or when its output
is piped, it prints one frame (the final one unless `--at` says otherwise).

### Acceptance criteria

- [ ] **Record writes a readable event stream.** After a `# finchart interval=60
      tick=0.05` header (parsers skip `#` lines; `replay` reads it), each line is
      one event (`T`/`S`/`U`/`H`), starting with a seeding `S` (snapshot) + `H`
      (heartbeat):
      ```bash
      cargo run -q --release -p finchart -- record artifacts/feed/events.txt
      head -3 artifacts/feed/events.txt    # # finchart …   then   S … | …   then   H …
      ```
- [ ] **Replay reproduces the state.** Replaying a recorded file folds back into the
      same `ChartState` and renders a fixed-dimension frame (verified in tests via
      `ChartState: PartialEq` over record → serialize → parse → replay).
- [ ] **Replay is deterministic** — the printed (piped) frame is byte-identical across runs:
      ```bash
      cargo run -q --release -p finchart -- replay artifacts/feed/events.txt > /tmp/a.txt
      cargo run -q --release -p finchart -- replay artifacts/feed/events.txt > /tmp/b.txt
      diff /tmp/a.txt /tmp/b.txt && echo "deterministic"
      ```

## Matching engine adapter (offline)

`examples-matching-engine` is the isolated gRPC adapter. Its live mode needs a
running matching engine, but everything else — translation, record/replay, render —
runs offline and reuses the neutral event format, so a `finchart record` recording
replays straight through the adapter.

```bash
cargo run --release -p finchart -- record artifacts/feed/events.txt
cargo run -p examples-matching-engine -- --replay artifacts/feed/events.txt
```

### Acceptance criteria

- [ ] **gRPC deps are confined.** The core crates pull in no gRPC/async stack:
      ```bash
      cargo tree -p market-core -e normal | grep -Ei 'tonic|tokio|prost' || echo "market-core clean"
      cargo tree -p backend-terminal -e normal | grep -Ei 'tonic|tokio|prost' || echo "backend-terminal clean"
      ```
- [ ] **Cross-crate replay.** A `finchart record` recording folds through the
      adapter into a fixed-dimension frame with a `source=replay` status line:
      ```bash
      cargo run -q -p examples-matching-engine -- --replay artifacts/feed/events.txt | head -1   # source=replay · …
      ```
- [ ] **Unreachable endpoint fails gracefully** (exit 1 + hint), never hangs:
      ```bash
      cargo run -q -p examples-matching-engine -- --endpoint http://127.0.0.1:1   # "could not connect …"
      ```
- [ ] **Replay is deterministic** — the printed frame is byte-identical across runs.

## Web showcase (WASM terminal)

`finchart-web` compiles the pure pipeline to WebAssembly; `web/` is the static
browser demo. The logic is host-testable without a browser or wasm toolchain; the
compiled artifact is validated with a wasm build + a Node smoke render.

```bash
cargo test -p finchart-web                         # host unit tests (rlib)
./web/build.sh                                      # wasm-pack build → web/pkg
python3 -m http.server --directory web              # open http://localhost:8000
```

### Acceptance criteria

- [ ] **Host logic is correct** (`cargo test -p finchart-web`): render is exactly
      `rows`×`cols`, plain has no ANSI / Ansi carries escapes, trades build+count
      candles, `zoom` shrinks the visible window, `reset` clears state.
- [ ] **`wasm-bindgen` is confined.** The cores pull in no wasm stack:
      ```bash
      cargo tree -p market-core -e normal | grep -i 'wasm' || echo "market-core clean"
      cargo tree -p render-core -e normal | grep -i 'wasm' || echo "render-core clean"
      cargo tree -p backend-terminal -e normal | grep -i 'wasm' || echo "backend-terminal clean"
      ```
- [ ] **WASM builds and renders.** `./web/build.sh` produces `web/pkg/*.wasm`; a
      `--target nodejs` smoke build folds `web/scenarios/baseline.txt` and renders a
      30-row ANSI frame (14 candles / 400 trades) — the compiled module, not the rlib.
- [ ] **Static assets serve.** `index.html`, `main.js`, `pkg/*.wasm`
      (`application/wasm`), and `scenarios/*.txt` all return 200 over a static server.
- [ ] **Recorded, no backend.** The demo replays `scenarios/*.txt` entirely
      in-browser; determinism means each visit renders identically.

## Test command

```bash
cargo test
```

Runs the full workspace suite, including:

- OHLC **aggregation correctness** (`render-core`): single-bucket = overall OHLC,
  identity at `target == count`, invariants hold, volume conserved.
- **Viewport pan/zoom bounds** (`render-core` + `finchart browse`): clamping at edges,
  zoom respects `MIN_CANDLES` / saturates to full range, reset restores default.
- **Chart dimensions & style selection** (`backend-terminal`): exact width/height,
  OHLC ticks vs candle blocks.
- **Snapshot** (`finchart bench`): determinism, no ANSI in plain output, fixed
  dimensions per style, HTML self-contained/escaped/preformatted with up/down
  colors, and `metrics.json` includes the required fields (incl. `rendered_bars`,
  `lod_active`, `style`).
- **Live replay** (`market-core` + `finchart`): candle-builder OHLCV
  invariants, ticks update the forming candle, interval boundary appends a new
  candle, same-seed tick streams and replays are deterministic, viewport follows
  the latest candle (`Viewport::tail`), and the live snapshot frames are
  byte-deterministic with the required `metrics.json` fields.
- **Order flow** (`market-core` + `render-core` + `backend-terminal` +
  `finchart`): buy/sell volume accumulate by aggressor side, `delta = buy −
  sell`, rollover preserves a closed candle's flow, history flow sums to volume and
  skews with direction, flow is conserved through cull+aggregate, the delta panel
  keeps fixed dimensions with a baseline, and the live `metrics.json` carries the
  order-flow fields.
- **Depth** (`market-core` + `backend-terminal` + `finchart`): order-book
  snapshot/update/delete, best bid/ask, spread, `top_n` sorting; deterministic
  synthetic depth that stays non-crossing with positive sizes; the depth panel
  keeps fixed dimensions; the replay book stays non-crossing and the live
  `metrics.json` carries the depth fields.
- **Heatmap** (`market-core` + `backend-terminal` + `finchart`): rolling-window
  liquidity store (record, eviction at capacity, price-bucket mapping,
  `cells_in_price_range`, side-aware export, determinism); the heatmap renders as a
  fixed-dimension side-coloured background layer with no full blocks, candles drawn
  on top; the replay records liquidity and the live `metrics.json` carries the
  heatmap fields.
- **Scenarios & markers** (`market-core` + `backend-terminal` + `finchart`):
  deterministic scenario step generation (walls at expected prices, wall pulls,
  sweep buy trades, bid reload); trade markers render at the expected price/recency;
  the heatmap is off in the default interactive view; scenario frames are
  deterministic with the required `metrics.json` fields.
- **Matching engine adapter** (`examples-matching-engine`, offline): pure
  translation `FillNotification → Trade` (fields + side from `aggressor_is_buy`,
  legacy `buy` ignored) and `Level2Response → DepthSnapshot` (best-first, rank
  validates order, empty book); translated events fold through `ChartState` (fill →
  candle/flow/CVD, level2 → BBO, heartbeat → heatmap column) with no rendering; the
  channel-drain seam; record → replay reproduces the `ChartState`; and CLI arg
  parsing. The live gRPC path needs a server (integration/manual).
- **Feed adapter** (`market-core` + `finchart`): `MarketEvent` text round-trips
  (each variant, empty sides, blank lines, garbage rejected); `ChartState::apply`
  folds trades→candles/flow, depth→book, heartbeat→heatmap with the expected
  `ApplyResult`; the same event stream yields the same final `ChartState`;
  recorded → serialized → parsed → replayed reproduces the state exactly; the
  synthetic `events()` source is deterministic and seeds with snapshot+heartbeat;
  `DepthGen::step` returns snapshot-first-then-updates; and `--record`/`--replay`
  round-trips through a file.

## Suggested loop after a renderer change

```bash
cargo test
cargo run --release -p finchart -- snapshot chart artifacts/tui
# inspect artifacts/tui/chart.ohlc.plain.txt (and chart.html for color)
```
