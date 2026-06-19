# Live Synthetic Market Replay + Incremental Candle Updates — Plan

> Status: proposed (planning only, no code yet).
>
> Goal: prove the engine can update a currently-forming candle from synthetic live
> ticks/trades, close that candle, append a new one, and render the terminal chart
> as if watching a live financial chart.
>
> Scope is synthetic data only. No gRPC, exchange APIs, browser Canvas, WebGPU,
> order-book heatmap, footprint charts, or plugin system. Dependencies stay
> minimal; no async — a simple deterministic loop is enough.

## Current-state review

- **`market-core`**: `Candle` (Copy struct) + columnar `CandleSeries` (append-only
  via `push`; column slices, `get`). `generate()` does a per-candle random walk
  where each open = prior close. `SplitMix64` RNG is `pub(crate)`. **No mutation
  API for the last candle yet** — the builder needs one.
- **`render-core`**: `Viewport` (+ `pan/zoom/full`), `cull` → `CulledView`,
  `aggregate` → LOD `Bar`s.
- **`backend-terminal`**: `render_chart` (pure). `examples-bench/snapshot.rs` holds
  a private `ansi_to_html` + HTML shell + hand-written JSON — reusable for live
  snapshots.
- **`examples-tui`**: thin crossterm loop over `Viewport` + `render_chart`;
  `default_viewport` = tail window.

The one real gap is mutation of the forming candle; everything else composes on
existing pieces (cull/aggregate/render_chart, tail viewport, snapshot export).

## Guiding decisions (require sign-off)

1. **Forming candle lives as the last element of `CandleSeries`.** The builder
   mutates it in place via a new `update_last`/`last` API. Rendering then culls the
   series directly (zero-copy) and the last bar updates tick-by-tick — no per-frame
   copy. (Alternative: keep it separate and splice at render; rejected — adds a copy
   and a special case.)
2. **Reuse the snapshot HTML/JSON machinery** by moving `ansi_to_html` + the
   HTML-document shell into `backend-terminal` as a small public `html` module, then
   have both `examples-bench` and `examples-live` use it (de-dupes). (Alternative:
   duplicate ~40 lines in `examples-live`.)
3. **`crossterm` added to `examples-live`** (same justification as `examples-tui`;
   core stays zero-dep).
4. **No async.** Synthetic tick timestamps drive all candle logic deterministically;
   wall-clock `sleep` only paces the visual frame rate. Snapshot mode has no sleep
   and is fully deterministic.
5. **`ts` stays epoch seconds**; live ticks continue from the preloaded history's
   last close/timestamp for visual continuity.

## 2. Neutral live event types (`market-core`, new `live` module)

```rust
pub enum Side { Buy, Sell }              // optional aggressor side
pub struct Tick { pub ts: i64, pub price: f64, pub size: f64, pub side: Option<Side> }
pub enum CandleUpdate {
    Opened,                    // first tick created the initial forming candle
    Updated,                   // mutated the current forming candle
    Rolled { closed: Candle }, // crossed an interval boundary: prior closed, new opened
}
```

Neutral — adaptable later to gRPC/WebSocket/file/backtest feeds (per
`future-integrations.md`).

## 3. `CandleBuilder` (`market-core::live`)

```rust
pub struct CandleBuilder { series: CandleSeries, interval_secs: i64, current_open_ts: i64 }
impl CandleBuilder {
    pub fn from_history(series: CandleSeries, interval_secs: i64) -> Self;  // current = last.ts + interval
    pub fn new(interval_secs: i64, start_ts: i64) -> Self;
    pub fn ingest(&mut self, tick: Tick) -> CandleUpdate;
    pub fn series(&self) -> &CandleSeries;       // for culling/rendering
    pub fn current(&self) -> Option<Candle>;     // the forming candle
}
```

- **Open semantics**: new candle `open = prior close` (continuity), then the tick
  sets `close = price`, `high = max(open, price)`, `low = min(open, price)`,
  `volume += size`.
- **Boundary**: when `tick.ts >= current_open_ts + interval`, push a new candle
  (advance `current_open_ts` by one interval) → `Rolled{closed}`, then apply the
  tick to the new candle.
- **Invariants preserved by construction**: `high ≥ max(open,close)`,
  `low ≤ min(open,close)`, `high ≥ low`, `volume` monotonic.
- Requires new `CandleSeries::last()` + `CandleSeries::update_last(Candle)`
  (minimal, invariant-neutral mutation).

## 4. Deterministic synthetic tick generation (`market-core::synth`)

```rust
pub struct LiveConfig { seed, start_ts, start_price, interval_secs, ticks_per_candle, volatility, drift, volume_per_tick }
pub fn ticks(cfg: &LiveConfig) -> impl Iterator<Item = Tick>   // or a TickStream struct
```

- Seeded `SplitMix64`; per-tick geometric step (`price *= 1 + drift + vol·(2u−1)`).
- `ts = start_ts + (k · interval_secs) / ticks_per_candle` → spreads ticks across
  each interval, so **wicks visibly expand** as ticks accumulate.
- `size = volume_per_tick·(0.5+u)`; `side` from the sign of the price move.
- Fully deterministic (same `LiveConfig` ⇒ identical tick sequence).

## 5. Follow-latest viewport (`render-core`)

```rust
impl Viewport { pub fn tail(series_len: usize, window: usize) -> Viewport }  // last `window` candles, clamped
```

Pure + testable; the live binary calls it each frame so the view tracks the newest
candle. (Can replace the ad-hoc `default_viewport` logic; `examples-tui` may adopt
it too.)

## 6. `examples-live` binary

```
cargo run -p examples-live --release
```

- Preload historical synthetic candles via `generate()` (e.g. 500); seed
  `CandleBuilder::from_history`.
- Drive a deterministic `ticks(...)` stream; each frame ingest `ticks_per_frame`
  ticks, then `cull(Viewport::tail(...))` → `render_chart` (OHLC + volume, ANSI),
  print.
- **Follows the latest candle** by default; **configurable frame rate** (`--fps`,
  default e.g. 12) via `crossterm::event::poll(frame_dur)` which also handles
  `q`/`Esc` to quit.
- **Status line**: forming candle `O/H/L/C`, tick count, last update time (µs), last
  render time (µs).
- Panic-safe `TerminalGuard` (raw mode / alt screen restore).
- Testable core extracted: a pure `Replay` struct (builder + tick stream +
  viewport) with `advance(n)`/`frame(width,height)` so logic is unit-tested without
  a TTY.

## 7. Verification integration

- **Deterministic non-interactive snapshot**:
  `cargo run -p examples-live --release -- --snapshot artifacts/live` (no terminal,
  no sleep). Replays a fixed tick budget and captures three frames:
  - `frame.before.plain.txt` — just after a candle opens (tiny body/wick),
  - `frame.mid.plain.txt` — candle partially formed (wick expanded),
  - `frame.after.plain.txt` — candle closed + next one opened.
  - Plus `frame.mid.html` (color, via the shared `html` module) and `metrics.json`.
- **`metrics.json`** (live): `history_candles`, `interval_secs`, `ticks_per_candle`,
  `total_ticks`, `frames`, `avg_update_us`, `render_us`, chart dims. (Timings vary
  per run; frame text + counts are deterministic.)
- Update `docs/verification.md` with the live flow + acceptance checks.

## 8. Tests (mapped to scope)

- **OHLCV invariants** (`market-core`): after arbitrary tick sequences the forming
  candle satisfies all OHLC relations + volume = Σsize.
- **Tick updates current candle** (`market-core`): known ticks → exact expected
  H/L/C/V.
- **Interval boundary appends** (`market-core`): a boundary-crossing tick →
  `series.len()+1`, prior candle closed with correct close, `Rolled{closed}`.
- **Same-seed replay deterministic** (`market-core`): identical tick sequence and
  identical resulting `CandleSeries`.
- **Viewport follows latest** (`render-core`): `tail` tracks the newest candle as
  length grows; clamps.
- **Live replay snapshot deterministic** (`examples-live`): the three frame strings
  are byte-identical across runs; `metrics.json` includes required fields.
- Plus `update_last`/`last` round-trip tests and the shared `ansi_to_html` test
  (moved with it).

## Dependencies

- Core crates: **still zero external deps** (RNG/builder/generator are std-only).
- `examples-live`: **`crossterm` only** (already used by `examples-tui`).
- No `serde_json`, no Canvas/WebGPU/gRPC/exchange/order-flow/plugins/async — all
  constraints honored.

## Implementation checklist (in order)

1. **`market-core`: `CandleSeries::last` + `update_last`** — mutation API,
   test-first.
2. **`market-core::live`: event types + `CandleBuilder`** — `ingest` with
   invariants/update/rollover, test-first.
3. **`market-core::synth`: `LiveConfig` + `ticks()`** — deterministic tick
   generator, test-first.
4. **`render-core`: `Viewport::tail`** — follow-latest, test-first.
5. **`backend-terminal`: `html` module** — move `ansi_to_html` + document shell out
   of `examples-bench`; refactor that snapshot to use it (keep its tests green),
   test-first for the moved fn.
6. **`examples-live` crate scaffold + pure `Replay`** — preload → ingest → tail →
   frame; test-first (deterministic replay, viewport follows).
7. **`examples-live` snapshot mode** — before/mid/after frames + `metrics.json`;
   test-first (deterministic frames, fields).
8. **`examples-live` interactive loop** — crossterm frame pacing + quit + terminal
   restore (thin glue).
9. **Docs** — `docs/verification.md` (live snapshot), `CLAUDE.md` (new
   crate/types/commands), this plan.
10. **Full verification** — `cargo test`, clippy, fmt, run the live snapshot,
    sanity-run `examples-live`; report artifact paths.

## Open decisions to confirm before starting

- **(1)** Forming candle as the last `CandleSeries` element (new
  `update_last`/`last`).
- **(2)** Moving `ansi_to_html` into a shared `backend-terminal::html` module
  (refactoring `examples-bench`).
- **(5)** The `Viewport::tail` follow model.
