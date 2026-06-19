# Verification

This project includes a deterministic snapshot/export workflow for the terminal
renderer. Its purpose is to let humans and coding agents inspect concrete,
reproducible rendered output after every change, and to catch regressions via text
diffing and tests.

For hands-on exploration there are interactive viewers (not part of the
deterministic flow):

```bash
cargo run -p examples-tui --release    # static: pan h/l ←/→  zoom k/j +/-  reset r  quit q
cargo run -p examples-live --release   # live replay: follows the forming candle (--fps N); quit q
```

This doc has two deterministic snapshot flows: the **static chart** snapshot
(`examples-bench`, below) and the **live replay** snapshot (`examples-live`, see
"Live replay verification").

## Command

```bash
cargo run -p examples-bench --release -- --snapshot artifacts/tui
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

These are defined in `examples-bench/src/snapshot.rs`. Changing them changes the
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
      cargo run -q -p examples-bench --release -- --snapshot artifacts/tui
      diff /tmp/prev.txt artifacts/tui/chart.ohlc.plain.txt && echo "deterministic"
      ```
      Note: `metrics.json` contains wall-clock timings that vary per run, so it is
      intentionally **not** byte-deterministic. Only the `chart.*` files are.

## Live replay verification

Proves incremental candle updates: a forming candle updated from synthetic ticks,
its wick expanding, then closing and rolling into a new candle.

```bash
cargo run -p examples-live --release -- --snapshot artifacts/live
```

Fixed, deterministic inputs (no wall-clock, no terminal size): seed 42, 200
history candles, 60s interval, 30 ticks/candle, 60-candle follow window, 120×30
chart. Three frames are captured at fixed tick offsets around a candle boundary
(just opened / half formed / just after roll-over). Defined in
`examples-live/src/snapshot.rs`.

### Files produced

| File | What it is |
|---|---|
| `frame.before.plain.txt` | Forming candle just opened (narrow body/wick). |
| `frame.mid.plain.txt` | Same candle half formed — wick has expanded. |
| `frame.after.plain.txt` | The candle has closed; a new one just opened. |
| `frame.mid.html` | The mid frame as a self-contained HTML page (shared `backend_terminal::html`). |
| `metrics.json` | `history_candles`, `interval_secs`, `ticks_per_candle`, `total_ticks`, `frames`, `avg_update_us`, `render_us`, chart dims. |

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
- [ ] **Deterministic for the same seed** — frame text is byte-identical across runs:
      ```bash
      cp artifacts/live/frame.mid.plain.txt /tmp/prev.txt
      cargo run -q -p examples-live --release -- --snapshot artifacts/live
      diff /tmp/prev.txt artifacts/live/frame.mid.plain.txt && echo "deterministic"
      ```
      (`metrics.json` timings vary per run; the frame text does not.)

## Test command

```bash
cargo test
```

Runs the full workspace suite, including:

- OHLC **aggregation correctness** (`render-core`): single-bucket = overall OHLC,
  identity at `target == count`, invariants hold, volume conserved.
- **Viewport pan/zoom bounds** (`render-core` + `examples-tui`): clamping at edges,
  zoom respects `MIN_CANDLES` / saturates to full range, reset restores default.
- **Chart dimensions & style selection** (`backend-terminal`): exact width/height,
  OHLC ticks vs candle blocks.
- **Snapshot** (`examples-bench`): determinism, no ANSI in plain output, fixed
  dimensions per style, HTML self-contained/escaped/preformatted with up/down
  colors, and `metrics.json` includes the required fields (incl. `rendered_bars`,
  `lod_active`, `style`).
- **Live replay** (`market-core` + `examples-live`): candle-builder OHLCV
  invariants, ticks update the forming candle, interval boundary appends a new
  candle, same-seed tick streams and replays are deterministic, viewport follows
  the latest candle (`Viewport::tail`), and the live snapshot frames are
  byte-deterministic with the required `metrics.json` fields.

## Suggested loop after a renderer change

```bash
cargo test
cargo run -p examples-bench --release -- --snapshot artifacts/tui
# inspect artifacts/tui/chart.ohlc.plain.txt (and chart.html for color)
```
