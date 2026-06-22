# Depth V1 — Plan

> Status: proposed (planning only, no code yet).
>
> Goal: add data-source-agnostic Level 2 market depth support — neutral depth
> types, an order book, deterministic synthetic depth generation, and a small
> terminal depth panel — without building a full heatmap yet.
>
> Synthetic data only. No gRPC, exchange APIs, browser Canvas, WebGPU, order-book
> heatmap, footprint charts, or plugin system. Dependencies stay minimal.

## Current-state review

- **`market-core`**: `live.rs` (`Side`/`Tick`/`Flow`/`CandleBuilder`), `candle.rs`
  (columnar `CandleSeries` + flow columns), `synth.rs` (`generate`,
  `ticks`/`TickStream`, `LiveConfig`), `rng.rs` (`SplitMix64`, `pub(crate)`). All
  deterministic, std-only.
- **`render-core`**: `cull → CulledView → aggregate → Bar`. Single path; not
  involved in depth (depth is a separate live structure, not per-candle).
- **`backend-terminal`**: `render_chart` composes title → price plot → volume panel
  → delta panel → time axis via a row budget; `ChartOptions` is built as struct
  literals in ~5 places. Panels are horizontal/bottom.
- **`examples-live`**: `Replay` owns `CandleBuilder` + `TickStream`; `status()` is
  deterministic (no timings); snapshot captures before/mid/after frames +
  `metrics.json`.

## 2. Neutral depth types (`market-core`, new `depth` module)

```rust
pub enum BookSide { Bid, Ask }
pub struct DepthLevel { pub price: f64, pub size: f64 }
pub struct DepthSnapshot { pub ts: i64, pub bids: Vec<DepthLevel>, pub asks: Vec<DepthLevel> } // full replace
pub struct DepthUpdate  { pub ts: i64, pub side: BookSide, pub price: f64, pub size: f64 }      // size 0 = delete
pub struct BestBidAsk   { pub bid: Option<f64>, pub ask: Option<f64> }
impl BestBidAsk { pub fn spread(&self) -> Option<f64> } // ask - bid
```

Neutral and feed-agnostic — adaptable to gRPC L2 / WS / synthetic (per
`future-integrations.md`'s "depth snapshots, depth deltas").

## 3. Price keying: f64 at the API, scaled integer ticks internally (chosen)

Tradeoff:

- **f64 keys directly** — ergonomic but unsafe as map keys: float equality / `NaN`
  mean "same price level" can't be matched reliably across updates, risking
  duplicate/orphan levels; needs an `OrderedFloat`/`total_cmp` wrapper anyway.
- **Scaled integer ticks (`i64 = round(price / tick_size)`)** — exact level keys,
  **sorted sides for free** via `BTreeMap<i64, f64>`, no float-key hazards, fast
  O(log n) updates. Convert at the boundary (f64 in/out). This is how real matching
  engines (incl. the future C++ engine) key books.

**Choose integer ticks internally**, f64 externally.

## 4. `OrderBook` (`market-core::depth`)

```rust
pub struct OrderBook { tick_size: f64, bids: BTreeMap<i64, f64>, asks: BTreeMap<i64, f64> }
impl OrderBook {
  pub fn new(tick_size: f64) -> Self;
  pub fn apply_snapshot(&mut self, snap: &DepthSnapshot);  // clear + set (skip size <= 0)
  pub fn apply_update(&mut self, upd: &DepthUpdate);        // size <= 0 => remove level
  pub fn best_bid(&self) -> Option<f64>;   // max bid tick  (bids.keys().next_back())
  pub fn best_ask(&self) -> Option<f64>;   // min ask tick  (asks.keys().next())
  pub fn spread(&self) -> Option<f64>;
  pub fn best_bid_ask(&self) -> BestBidAsk;
  pub fn top_n(&self, side: BookSide, n: usize) -> Vec<DepthLevel>; // best-first, sorted
  pub fn level_counts(&self) -> (usize, usize); // (bid_levels, ask_levels)
}
```

`BTreeMap` keeps each side sorted automatically (the "sorted sides" invariant is
free). The book is a **faithful mirror** of the feed (no rejection); the generator
maintains non-crossing.

## 5. Deterministic synthetic depth (`market-core::synth`)

```rust
pub struct DepthConfig { seed, tick_size, levels, base_size, size_jitter, recenter_band, updates_per_step }
pub struct DepthGen { /* SplitMix64 + cfg + last_mid */ }
impl DepthGen {
  pub fn new(cfg) -> Self;
  pub fn snapshot(&mut self, ts, mid) -> DepthSnapshot;          // a fresh ladder around mid
  pub fn step(&mut self, book: &mut OrderBook, ts, mid);         // re-center if mid drifted; else jitter/delete a few levels
}
```

- **Ladder**: bids at `mid − i·tick`, asks at `mid + i·tick` for `i ∈ 1..=levels`;
  sizes from seeded RNG. **Non-crossing by construction** (best bid < mid < best
  ask).
- **Updates shift liquidity**: jitter sizes near the top, occasionally set size 0
  (delete) → exercises `apply_update` + deletion; re-center (snapshot) when `mid`
  drifts past `recenter_band`.
- Invariants maintained: bids < asks, non-negative sizes, sorted (via book). Fully
  deterministic (seed + mid sequence).

## 6. `examples-live` integration

- `ReplayConfig` gains depth params (`tick_size`, `levels`, `base_size`,
  `depth_every`).
- `Replay` holds an `OrderBook` + `DepthGen`. On `new()`: apply an initial snapshot
  around the starting mid. In `advance()`: every `depth_every` ticks, call
  `depth.step(&mut book, ts, mid)` with `mid` = latest tick price — deterministic
  from the tick index.
- `status()` appends `bid / ask / spread`.

## 7. Terminal render: compact right-side depth panel

- New `ChartOptions.depth: Option<DepthPanel>` where
  `DepthPanel { bids: Vec<(f64,f64)>, asks: Vec<(f64,f64)>, spread: Option<f64> }`
  — plain tuples so **backend-terminal stays decoupled** from `OrderBook`. (Adding
  the field ripples to the ~5 `ChartOptions` literals; default `None` keeps
  behavior.)
- When `Some` and width allows, reserve a fixed-width right strip (e.g. 18 cols):
  top-N **asks** (top, red) → `spread` row → top-N **bids** (bottom, green), each
  `price size` with a mini size bar. The price/volume/delta plot uses
  `width − gutter − depth_cols`; total dimensions stay exactly `width × height`
  (deterministic). **No heatmap.**

## 8. Snapshot verification

- Live `metrics.json` gains `best_bid`, `best_ask`, `spread`, `bid_depth_levels`,
  `ask_depth_levels`.
- Frames render with the depth panel on → deterministic, fixed dims, contain depth
  labels (`BID`/`ASK`).

## 9. Tests (mapped to scope)

- **OrderBook**: snapshot application, incremental update, level deletion (size 0),
  best bid/ask, spread, `top_n` sorting (best-first).
- **DepthGen**: deterministic (same seed+mid ⇒ identical book state), non-crossing
  after snapshot+updates, non-negative sizes.
- **Depth panel**: fixed dimensions with depth on; contains level/labels.
- **Live snapshot**: `metrics.json` includes the 5 depth fields; frames
  deterministic with the panel.

## Dependencies & invariants

- `BTreeMap` / `SplitMix64` are std — **no new dependencies**; core stays zero-dep,
  `crossterm` confined to interactive binaries.
- No gRPC/exchange/Canvas/WebGPU/**heatmap**/footprint/plugin. Existing
  examples/tests preserved (mechanical `depth: None` on `ChartOptions` literals;
  behavior unchanged).

## Implementation checklist (in order)

1. **`market-core::depth`** — neutral types + `OrderBook` (tick-keyed `BTreeMap`):
   snapshot/update/delete/best/spread/top_n. Test-first.
2. **`market-core::synth`** — `DepthConfig` + `DepthGen` (ladder snapshot +
   liquidity-shifting updates, deterministic, non-crossing). Test-first.
3. **`backend-terminal`** — `ChartOptions.depth` + `draw_depth_panel` right strip +
   layout budget; update `ChartOptions` literals. Test-first (dims, labels).
4. **`examples-live`** — `Replay` owns `OrderBook` + `DepthGen`; init snapshot +
   per-cadence steps; `status()` shows bid/ask/spread; render passes the depth
   panel. Test-first (deterministic book, status).
5. **`examples-live` snapshot** — depth fields in `metrics.json`; frames show the
   panel. Test-first.
6. **Preserve `examples-bench`/`examples-tui`** — `depth: None`; confirm static
   snapshot byte-identical.
7. **Docs** — `docs/verification.md`, `CLAUDE.md`, this plan.
8. **Full verification** — `cargo test`, clippy, fmt, run both snapshots, sanity-run
   `examples-live`; report artifacts.

## Open decisions to confirm before starting

- **(3)** Internal **integer-tick** keying (f64 at the API).
- **(7)** A **right-side depth panel** inside `render_chart` gated by
  `ChartOptions.depth` (default `None`).
