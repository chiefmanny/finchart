# Heatmap V1.1 — Explainability, Scenarios, Trade Overlay — Plan

> Status: approved (planning only when written; implementation in progress).
>
> Goal: make the terminal heatmap interpretable by adding deterministic
> market-structure scenarios, side-aware rendering, and executed trade markers.
> No new data sources.
>
> Synthetic data only. No gRPC, exchange APIs, browser Canvas, WebGPU, footprint
> chart, or plugin system. Dependencies stay minimal; existing tests/examples
> preserved.

## 1. Current-state review

- **`LiquidityHeatmap`**: rolling `VecDeque` of columns, each a `BTreeMap<i64,f64>`
  of **combined** (bid+ask) liquidity per tick. `record` merges both sides;
  `columns_in_range` exports `(price, liquidity)`; `max_liquidity` is the
  **per-cell** max.
- **`draw_heatmap`** (backend-terminal): per plot column, sums every level's
  liquidity into its price **row**, then `glyph = heat_glyph(row_liq /
  max_liquidity)` with ramp `· ░ ▒ ▓ █`.
- **`DepthGen.step`**: random jitter/delete around a center; no scripted structure.
- **`examples-live`**: `Replay` always renders the heatmap; the live snapshot
  asserts heatmap fields/glyphs.

## 2. Why the current heatmap is visually ambiguous

1. **Wrong normalization → saturation.** A rendered row aggregates *many* ticks
   (the ±0.6 ladder collapses into ~1–2 rows of a ~12-wide price view), but it is
   divided by the **per-cell** max. So `frac ≫ 1` → clamps to `█` everywhere
   populated → solid white blocks.
2. **No side information.** Bid and ask liquidity are merged, so a bid wall and an
   ask wall look identical — the heatmap cannot communicate book structure.
3. **Wide price range vs narrow ladder.** The populated rows track the drifting mid
   over time → the image reads as **diagonal bands** (where the book has *been*),
   not market structure.
4. **`█` dominance.** The top ramp glyph is a full block that overpowers candles in
   non-bar columns.

## 3. Heatmap off by default in the normal viewer

Add a `show_heatmap` toggle (`ReplayConfig`, default **false**). The interactive
`examples-live` viewer renders with it **off**; the deterministic **live snapshot
keeps it on** (it is a verification artifact, not "the viewer"), and the new
**scenario snapshots** turn it on. This satisfies "off by default in the normal
live viewer" while preserving existing live-snapshot tests.

## 4. Deterministic scenario fixtures (`market-core`, new `scenario` module)

A `ScenarioKind` enum + generator producing a deterministic `Vec<ScenarioStep>`
(`{ ts, mid, snapshot: DepthSnapshot, trades: Vec<Tick> }`), one per time bucket:

- **`StaticAskWall`** — a large persistent ask size at `mid + offset` across all
  steps (+ normal ladder).
- **`StaticBidWall`** — large persistent bid at `mid − offset`.
- **`AskWallPulled`** — ask wall present, then set to size 0 at the pull step.
- **`BidWallPulled`** — bid wall present, then removed.
- **`SweepAsk`** — mid ramps up; ask levels below the new mid are consumed
  (size→0); buy-initiated trades emitted at the consumed prices.
- **`BidReload`** — a bid level repeatedly drained then refilled (iceberg-like);
  sell trades hitting it.

Structure is fully scripted (walls/pulls deterministic); a seeded RNG only adds
small size jitter. The runner applies `snapshot` per step (so the book/heatmap show
clean structure).

## 5. Side-aware export — split bid/ask in the store (chosen)

Tradeoff:

- **Combined store + infer side from a stored mid/BBO** — keeps one map but needs
  per-column best-bid/ask and tick-vs-mid inference (edge cases at the spread).
- **Split bid/ask maps per column (chosen)** — `HeatmapColumn { ts, bid: BTreeMap,
  ask: BTreeMap }`. Side is explicit and unambiguous; combined values are just
  `bid+ask` (existing accessors unchanged). Minimal extra memory (two small maps).
  Simpler and robust.

Add `columns_in_range_sided(min,max) -> Vec<Vec<(f64 price, f64 liq, BookSide)>>`;
keep combined accessors (`max_liquidity`, `cells_in_price_range`, `price_buckets`)
aggregating both sides so existing tests hold.

## 6. Terminal rendering changes (legibility)

- **Side color**: bid cells green, ask cells red (dim), accumulated per row per
  side; color by the dominant side.
- **Fix normalization**: scale by the **max rendered row-liquidity** computed
  within `draw_heatmap` (two-pass), not the per-cell max → cells spread across the
  ramp instead of saturating.
- **No pure-white default**: ramp capped at `▓` → `· ░ ▒ ▓` (drop `█`); reduces
  dominance.
- **Legend/status label**: a short `heatmap: bid░/ask░` legend in the status line.
  Candles/OHLC remain drawn on top.
- `HeatmapView` becomes side-aware: `columns: Vec<Vec<(f64, f64, bool /*is_bid*/)>>`,
  `max_liquidity` kept for reference; renderer computes its own row scale.

## 7. Executed trade markers

- Render input `TradeMarkers { markers: Vec<TradeMarker { price, col_from_right,
  buy }> }` (`col_from_right` aligns to heatmap/time columns; decoupled tuples).
  `ChartOptions.trades: Option<TradeMarkers>`.
- Draw order: **heatmap → trade markers → candles → panels** — markers sit *above
  the heatmap, below candles* (candles overwrite on overlap). Glyphs `▲` (buy,
  green) / `▼` (sell, red).
- The scenario runner collects recent trades into markers; the normal viewer
  leaves `trades: None`.

## 8. Scenario snapshot command

`cargo run -p examples-live --release -- --scenario <name> --snapshot <dir>` (with
`--snapshot` only ⇒ existing live snapshot). Emits `frame.before/mid/after.plain.txt`,
`frame.mid.html`, `metrics.json` (with a `scenario` field + heatmap/wall stats),
captured at scenario-specific steps (e.g. wall present → pulled). Renders a
**zoomed price view** (tight around the book) with heatmap + markers so structure
is legible.

## 9. Tests (mapped to scope)

- **Scenario determinism**: same kind ⇒ identical steps + identical frames.
- **Wall at expected price**: after a wall scenario, the book/heatmap has the large
  size at `mid ± offset`.
- **Wall pull removes liquidity**: the wall price's size is 0 after the pull step.
- **Trade markers at expected row/col**: render with a known marker ⇒ glyph at the
  expected cell.
- **Heatmap dimensions unchanged**: render with side-aware heatmap + markers ⇒
  exact `width × height`.
- **Normal live view works with heatmap disabled**: default `Replay` renders no
  heatmap shade glyphs and valid dims.

## Constraints & invariants

- `VecDeque`/`BTreeMap` std-only — no new deps; `crossterm` stays in interactive
  binaries. Synthetic only; no gRPC/exchange/Canvas/WebGPU/footprint/plugin.
  Existing examples/tests preserved (mechanical `trades: None` on `ChartOptions`
  literals; side-aware `HeatmapView` update is localized).

## Implementation checklist (in order)

1. **`market-core::heatmap`** — split bid/ask storage + `columns_in_range_sided`;
   keep combined accessors. Test-first.
2. **`market-core::scenario`** — `ScenarioKind` + deterministic step generator
   (snapshots + trades + mid). Test-first (determinism, wall present, wall pulled,
   sweep consumes).
3. **`backend-terminal`** — side-aware `HeatmapView` + row-max normalization +
   `· ░ ▒ ▓` ramp + bid/ask color + `TradeMarkers` overlay + legend; update
   `ChartOptions` literals. Test-first (dims, markers, side colors, no `█`).
4. **`examples-live`** — `show_heatmap` (default off) for the interactive viewer;
   build sided heatmap + markers; keep live snapshot heatmap on. Test-first (normal
   view heatmap-free; snapshot still has it).
5. **`examples-live` scenarios** — `--scenario <name>` + scenario snapshot
   (before/mid/after + metrics incl. `scenario`/wall fields). Test-first
   (determinism, files, wall metrics).
6. **Preserve `examples-bench`/`examples-tui`** + existing live-snapshot tests;
   confirm static snapshot byte-stable.
7. **Docs** — `docs/verification.md` (scenarios), `CLAUDE.md`, this plan.
8. **Full verification** — `cargo test`, clippy, fmt, run live + scenario snapshots,
   sanity-run `examples-live`; report artifacts.

## Confirmed decisions

- **(5)** Split bid/ask storage (vs infer-from-mid).
- **(6)** Ramp capped at `▓` with row-max normalization + bid/ask coloring.
- **(3)** Heatmap off by default in the interactive viewer, kept on in the live
  snapshot (preserves existing tests).
