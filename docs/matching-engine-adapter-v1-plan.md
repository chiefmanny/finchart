# Matching Engine Adapter V1 — Plan

> Status: approved (planning when written; implementation in progress).
>
> Goal: build an isolated adapter binary that connects to the external matching
> engine gRPC API, translates messages into `MarketEvent`, applies them through
> `ChartState`, and renders the existing terminal chart.
>
> Constraints: no exchange APIs (the matching engine is the sanctioned
> integration), no browser Canvas, no WebGPU, no footprint chart, no plugin system.
> Do not add gRPC/protobuf/tokio to `market-core`, `render-core`, or
> `backend-terminal` — confine them to the adapter crate. Do not mutate core
> architecture unless absolutely necessary. Preserve all existing synthetic
> examples and snapshots.

## External API (as documented)

1. `Subscribe(Symbol) -> stream FillNotification` — one message per trade
   execution; fields `price`, `size`, `buy`, `partial_fill`, `remaining_size`.
   The raw trade/tick feed for candle aggregation.
2. `GetLevel2(Level2Request) -> stream Level2Response` — request `symbol`,
   `depth`, `side`; currently returns one snapshot then closes; each response has
   `bids` and `asks`, ordered top-N `{price, size, rank}`. Poll periodically, treat
   each response as a full `DepthSnapshot`.
3. `WatchSymbol` — streams best bid/ask every 100ms. Optional for V1 (BBO
   fallback only).

## 1. Inspection findings

- No `.proto` files exist in the repo; no gRPC/tokio/prost anywhere — the
  workspace is std-only across 6 members. So the adapter must vendor a
  reconstructed `.proto` and confine all gRPC deps to a new crate.
- The Feed Adapter V1 boundary is the seam: `MarketEvent` → `ChartState::apply` →
  render, plus `serialize_events`/`parse_events`/`replay_events`. No core changes
  needed — the adapter is just another `MarketEvent` source.

## 2. New isolated crate: `examples-matching-engine` (binary)

Depends on `market-core` + `render-core` + `backend-terminal` (chart) plus
`tonic`, `prost`, `tokio`, `crossterm` (transport/TUI). Heavy deps live only here;
the three core crates stay zero-dependency.

```
examples-matching-engine/
  proto/matching_engine.proto   # vendored, reconstructed from the documented API
  build.rs                      # tonic-build codegen
  src/translate.rs              # proto msg -> MarketEvent (PURE, offline-testable)
  src/feed.rs                   # gRPC streams/poll/heartbeat -> mpsc<MarketEvent>
  src/render.rs                 # ChartState -> frame + status line (local helper)
  src/record.rs                 # record/replay wiring (reuses market-core format)
  src/main.rs                   # CLI + async runtime + sync render loop
```

Single crate over a lib+bin split for V1 (isolation already satisfied since it is
not a core crate).

**Build note:** `tonic-build` needs `protoc`; bundle a vendored protoc
(`protoc-bin-vendored`) so the build is self-contained (vs requiring a system
install).

## 3. `FillNotification` -> `MarketEvent::Trade`

Pure `fill_to_trade(fill, ts) -> MarketEvent::Trade`:

- `price -> Tick.price`, `size -> Tick.size`, `buy -> Tick.side` (via `fill_side`).
- `partial_fill` / `remaining_size` ignored for aggregation in V1 (each fill = one
  executed trade of `size`); documented, surfaceable later.

**Aggressor semantics (flagged ambiguity).** The real service is not runnable here,
so this cannot be empirically confirmed now.

- Centralize in `fill_side(buy: bool, invert: bool) -> Side`.
- Documented default: `buy=true -> Side::Buy` (buyer-initiated / aggressor lifts the
  ask).
- `--invert-trade-side` flag flips it in one place.
- A test pins the chosen mapping; reconcile against the live engine before trusting
  delta/CVD (e.g. verify CVD trends with price direction, or match a known trade).

## 4. `Level2Response` -> `MarketEvent::DepthSnapshot`

Pure `level2_to_snapshot(resp, ts) -> MarketEvent::DepthSnapshot`:

- `bids -> Vec<DepthLevel>{price,size}`, `asks -> Vec<DepthLevel>`, preserving
  best-first order.
- `rank` ignored for construction but used in a test to validate ordering (bids
  rank-ascending = price-descending; asks rank-ascending = price-ascending).
- Empty book: empty `bids`/`asks` -> empty `DepthSnapshot` (no panic;
  `apply_snapshot` clears).

## 5. Depth polling

`GetLevel2` returns one snapshot then closes: an async loop that, every
`--depth-interval-ms` (default 1000), opens `GetLevel2{symbol, depth, side=BOTH}`,
reads the single response, translates -> `DepthSnapshot`, pushes to the channel,
closes. A `depth_refresh` counter increments per poll.

## 6. Heartbeats

A `tokio::time::interval` ticker emits `MarketEvent::Heartbeat{ts}` every
`--heartbeat-ms` (default 1000) into the channel — this drives heatmap-column
recording / depth-history cadence in `ChartState`.

## 7. Timestamp handling

The documented `FillNotification`/`Level2Response` fields carry no source
timestamp, so V1 uses adapter receive time (wall clock -> epoch seconds) as `ts`.
Documented limitation: candle bucketing and event ordering reflect adapter receive
timing, not exchange time; live mode is inherently non-deterministic (a live feed).
Translation fns take `ts` as a parameter so tests inject fixed values and
`--replay` uses recorded timestamps (both deterministic). If `WatchSymbol` or a
later proto exposes a timestamp, switch to source time.

## 8. Render loop

An async runtime (tokio) runs the three producers -> an `mpsc` channel; the main
thread runs a sync render loop (crossterm): drain the channel, `state.apply(&event)`
for each, then `render_chart` from a local `render.rs` (reuse `ChartState` +
`backend-terminal`, same pattern as `examples-live/feed.rs`). Status line shows
`source=matching-engine`, `symbol`, trade count, depth refresh count,
`bid/ask/spr`, `CVD`, and update/render timing. `WatchSymbol` is optional/deferred
(BBO fallback only).

## 9. Record / replay

- `--record <file>`: as events are drained/applied, append their `serialize_event`
  lines (reusing the existing format) — capturing the translated stream.
- `--replay <file>`: read the file, `parse_events` -> `replay_events` into a fresh
  `ChartState`, render — no engine connection required (offline, deterministic).
  Reuses `market-core` verbatim.

## 10. Tests (offline — no server, no rendering)

- `fill_to_trade`: price/size/ts mapping.
- `fill_side`: buyer-initiated default (`true->Buy`, `false->Sell`) + inverted
  variants (pins the chosen semantics).
- `level2_to_snapshot`: bids/asks -> `DepthLevel`s, best-first preserved, rank
  validates ordering.
- Empty book: empty response -> empty snapshot, no panic.
- Record/replay: translate a fixed message sequence -> serialize -> parse -> replay
  -> `ChartState` equals direct application (`PartialEq`).
- Translation independent of rendering: translation tests construct proto structs
  and assert `MarketEvent`s with no `ChartState`/render involved.

## Constraints honored

No exchange APIs (matching engine is the sanctioned integration); no
Canvas/WebGPU/footprint/plugins. Zero changes to `market-core`/`render-core`/
`backend-terminal`. All existing synthetic examples/snapshots preserved (new crate
is additive). gRPC deps confined to the new crate.

## Implementation checklist (in order)

1. **Scaffold crate** — `examples-matching-engine` + vendored
   `proto/matching_engine.proto` + `build.rs` (tonic-build, vendored protoc); add
   to workspace members; empty `main` builds.
2. **`translate.rs`** — `fill_side`, `fill_to_trade`, `level2_to_snapshot` (pure,
   `ts` injected). Test-first (mapping, side semantics, empty book, rank ordering).
3. **`feed.rs`** — async producers (Subscribe stream, GetLevel2 poll, heartbeat
   ticker) -> `mpsc<MarketEvent>`, behind a small `MarketFeed` seam so the render
   loop is transport-agnostic.
4. **`render.rs`** — local `render_state(&ChartState, …)` + status line
   (source/symbol/counts/BBO/CVD/timings).
5. **`record.rs` + replay** — `--record` appends `serialize_event`; `--replay`
   folds a file via `replay_events` (no engine). Test-first (record->replay equals
   direct apply).
6. **`main.rs` + CLI** — `--endpoint`, `--symbol`, `--tick-size`,
   `--interval-secs`, `--depth`, `--depth-interval-ms`, `--heartbeat-ms`,
   `--invert-trade-side`, `--record`, `--replay`; wire runtime + render loop;
   graceful connect-failure message.
7. **Tests** — the full offline suite from scope 10.
8. **Preserve** — confirm existing workspace tests + live/scenario/static snapshots
   byte-identical (new crate is additive; `Cargo.lock` grows).
9. **Docs** — update `docs/future-integrations.md`, `CLAUDE.md`,
   `docs/verification.md`; save this plan.
10. **Full verification** — `cargo test`, clippy, fmt; build the adapter; run
    `--replay` on a recorded fixture (offline); report artifacts.

## Confirmed decisions

- New single binary crate `examples-matching-engine` (not a lib+bin split).
- Vendor a reconstructed `.proto`; bundle protoc via `protoc-bin-vendored`.
- Aggressor default `buy=true -> Side::Buy` with `--invert-trade-side` + a
  reconciliation note.
- Receive-time timestamps with the documented non-determinism limitation.
