# Feed Adapter V1 — Source-Agnostic Market Event Ingestion — Plan

> Status: approved (planning when written; implementation in progress).
>
> Goal: refactor live replay so synthetic data, scenarios, future file replay, and
> future gRPC matching-engine streams can all feed the chart through the same
> neutral event path.
>
> Constraints: no gRPC yet, no exchange APIs, no browser Canvas, no WebGPU, no full
> plugin system. Keep core crates minimally dependent; preserve existing examples
> and snapshots.

## 1. Current-state review

- **`examples-live::Replay`** directly mutates `CandleBuilder` + `OrderBook` +
  `LiquidityHeatmap` in its `advance` loop: per tick → `builder.ingest`; every
  `depth_every` → `depth.step(&mut book, …)` + `heatmap.record(&book, …)`. Rendering
  reads those three structures.
- **`scenarios.rs`** drives the book directly from `ScenarioStep { snapshot, trades
  }` (applies `DepthSnapshot`, records heatmap).
- **`DepthGen::step(&mut book, ts, mid)`** mutates a book in place (recenter → full
  snapshot; else → N random `DepthUpdate`s). **`TickStream`** is
  `Iterator<Item = Tick>`. All deterministic.

The ingestion logic is duplicated across `Replay` and `scenarios`, and coupled to
concrete generators — exactly what a neutral event boundary fixes.

## 2. Neutral `MarketEvent` (`market-core`, new `event` module)

```rust
pub enum MarketEvent {
    Trade(Tick),                  // executed trade
    DepthSnapshot(DepthSnapshot), // full book replace
    DepthUpdate(DepthUpdate),     // one level (size 0 = delete)
    Heartbeat { ts: i64 },        // time tick: advances the heatmap / recency
}
```

`Heartbeat` is the neutral "end-of-bucket" signal that drives heatmap recording
cadence — sources emit one per time bucket. Future gRPC/file/WS adapters all map
naturally to these four.

## 3. `ChartState` (`market-core`, new `state` module)

```rust
pub struct ChartState { builder: CandleBuilder, book: OrderBook, heatmap: Option<LiquidityHeatmap>, recent_trades: VecDeque<Tick>, /* caps */ }
pub enum ApplyResult { Trade(CandleUpdate), Depth, Heartbeat, Ignored }
impl ChartState {
  pub fn from_history(series, interval_secs, tick_size, heatmap: Option<(tick,cap)>, trade_cap) -> Self;
  pub fn apply(&mut self, ev: &MarketEvent) -> ApplyResult;
  // read access: series(), book(), heatmap(), recent_trades(), current_flow(), cumulative_delta(), best_bid/ask/spread …
}
```

- `Trade` → `builder.ingest` + push to `recent_trades` (bounded).
- `DepthSnapshot`/`DepthUpdate` → `book.apply_snapshot`/`apply_update`.
- `Heartbeat` → if heatmap enabled, `record(&book, ts)`.
- The single ingestion path: `Replay` and `scenarios` stop mutating the three
  structures directly and only build `MarketEvent`s through `apply`.
- Add `PartialEq` to `CandleBuilder` + `ChartState` so tests can assert state
  equality.

## 4. Refactor `examples-live`

`Replay` owns a `ChartState` + a synthetic event source. `advance(n)` pulls events
and `apply`s them (no direct mutation). `render`/`status` read from `ChartState`.
Behavior preserved.

## 5. Deterministic synthetic event source (`market-core`, in `synth`)

Refactor **`DepthGen::step(&mut self, ts, mid) -> DepthDelta`** (`enum DepthDelta {
Snapshot(DepthSnapshot), Updates(Vec<DepthUpdate>) }`) — it stops taking a book (a
generator should produce deltas, not mutate external state). Then:

```rust
pub fn events(cfg: &EventConfig) -> EventSource;  // impl Iterator<Item = MarketEvent>
```

`EventSource` wraps `TickStream` + `DepthGen`, emitting (per its cadence): `Trade`
per tick, the depth delta as `DepthSnapshot`/`DepthUpdate` events at the cadence,
then a `Heartbeat`. Same RNG draw order ⇒ identical book/heatmap ⇒ snapshots stay
byte-stable.

## 6. Event record / replay

Hand-rolled deterministic line-based text format (no deps — same posture as the
JSON metrics), using f64's shortest round-trip `Display`:

```
T <ts> <price> <size> <B|S|->         S <ts> <p:s;…> | <p:s;…>
U <ts> <B|A> <price> <size>           H <ts>
```

`serialize_event`/`parse_event` + `replay_events(&mut ChartState, events)`.
Replaying recorded events into a fresh `ChartState` yields the same final state.

## 7. Binary modes

Extend **`examples-live`** (vs a new crate — fewer crates, reuses snapshot infra)
with `--record <file>` (run the synthetic source, apply through `ChartState`, write
events) and `--replay <file>` (read events → `ChartState` → render/snapshot). The
record/replay logic lives in `market-core`; the binary just wires files.

## 8. Tests

- **Event application order** (`market-core`): a fixed `[Trade, DepthSnapshot,
  DepthUpdate, Heartbeat]` sequence produces expected `ApplyResult`s and state.
- **Trade updates candle + flow**: after `apply(Trade)`, `current_flow`/series
  reflect it.
- **Depth snapshot/update updates book**: BBO/levels change as expected.
- **Heatmap records on heartbeat** (when enabled): `Heartbeat` adds a time bucket;
  disabled → no-op.
- **Same event stream ⇒ same final `ChartState`** (`PartialEq`).
- **Recorded/replayed ⇒ same final state** (serialize → parse → apply equals direct
  apply).
- Plus: `DepthGen` delta determinism; synthetic `events()` determinism; round-trip
  serialize/parse.

## Constraints & invariants

- `VecDeque`/`BTreeMap` std-only — no new deps; `crossterm` stays in interactive
  binaries; no gRPC/exchange/Canvas/WebGPU/plugin. Existing examples/tests preserved
  (the refactor keeps RNG order, so snapshots should stay byte-identical; even if
  bytes shift, tests assert determinism/structure, not golden bytes — diff against
  pre-refactor artifacts to confirm).

## Implementation checklist (in order)

1. **`market-core::event`** — `MarketEvent`, `ApplyResult`, serialize/parse
   (round-trip). Test-first.
2. **`market-core::state`** — `ChartState` + `apply`; `PartialEq` on
   `CandleBuilder`/`ChartState`. Test-first (trade→candle/flow, depth→book,
   heartbeat→heatmap, same-stream→same-state).
3. **`market-core::synth`** — `DepthGen::step -> DepthDelta` (drop book param) +
   `events()` synthetic `EventSource`. Update `DepthGen` tests. Test-first (delta +
   source determinism; folds into `ChartState`).
4. **Record/replay** — `replay_events` + serialize a stream; recorded/replayed
   equals original final state. Test-first.
5. **`examples-live::Replay`** — own `ChartState` + `EventSource`; `advance` applies
   events; render/status read state. Preserve behavior.
6. **`examples-live::scenarios`** — emit `MarketEvent`s from scenario steps into
   `ChartState`. Preserve scenario snapshots.
7. **`examples-live` `--record`/`--replay`** modes. Test-first (replay snapshot
   matches).
8. **Preserve `examples-bench`/`examples-tui`** + live/scenario snapshots; diff
   artifacts byte-for-byte where possible.
9. **Docs** — `docs/verification.md` (feed + record/replay), `CLAUDE.md`, this plan.
10. **Full verification** — `cargo test`, clippy, fmt, run live + scenario +
    record/replay, sanity-run `examples-live`; report artifacts.

## Confirmed decisions

- **(2)** `MarketEvent` + `ChartState` in market-core, with `Heartbeat` driving
  heatmap cadence.
- **(5)** `DepthGen::step` returns a `DepthDelta` (no book param).
- **(6/7)** Hand-rolled text record/replay format; extend `examples-live` (no new
  crate).
