# Future Integrations

The charting engine should remain data-source agnostic.

One intended future integration is an existing C++17 limit order matching engine that exposes market data through gRPC/protobuf streaming APIs. That system includes:

- Price-time priority limit order book.
- Live bid/ask and Level 2 market depth streams.
- Order submission and cancellation APIs.
- Mean-reversion simulated trading client.
- Python FastAPI bridge and React frontend prototype.

This project should not depend on that system directly.

Instead, the charting engine should define neutral market data types and ingestion interfaces that could later be adapted to:

- gRPC market data streams.
- Exchange WebSocket streams.
- Historical files.
- Synthetic data generators.
- Backtest/replay engines.

The first MVP should use only synthetic data. The matching engine integration is a future adapter, not a core dependency.

## Matching Engine Adapter V1 (implemented)

A first adapter now exists as the isolated `examples-matching-engine` binary crate.
It connects to the matching engine's gRPC API, translates messages into
`market_core::MarketEvent`, applies them through `ChartState`, and renders the
existing terminal chart — the same neutral ingestion path the synthetic examples
use. All gRPC/async/protobuf dependencies (`tonic`, `prost`, `tokio`) live **only**
in that crate; `market-core`, `render-core`, and `backend-terminal` stay
zero-dependency (verified via `cargo tree`).

Translation (`examples-matching-engine/src/translate.rs`, pure + offline-tested):

- `FillNotification` → `MarketEvent::Trade` — `price`/`size`/side; `partial_fill`
  and `remaining_size` are ignored for candle aggregation in V1.
- `Level2Response` → `MarketEvent::DepthSnapshot` — `bids`/`asks` → `DepthLevel`s,
  best-first order preserved, `rank` dropped (the book is price-tick-keyed).
- A fixed-cadence `Heartbeat` drives heatmap / depth-history recording.

Known limitations / assumptions:

- **Aggressor semantics** *(resolved)*. `fill_side` reads
  `FillNotification.aggressor_is_buy` (tag 10, "true when the buyer lifted the ask").
  The legacy `buy` field (tag 4) means "bid order fully consumed", not the aggressor,
  and is ignored; the earlier `--invert-trade-side` escape hatch was removed.
- **Timestamps.** The documented `FillNotification`/`Level2Response` carry no source
  timestamp, so events are stamped with adapter *receive time*. Live candle
  bucketing therefore reflects arrival timing, not exchange time; live mode is
  non-deterministic (a live feed). Recorded event files replay deterministically.
- **`GetLevel2`** returns one snapshot then closes, so depth is **polled** on an
  interval (`--depth-interval-ms`, default 1000). `WatchSymbol` (BBO stream) is
  defined in the proto but unused in V1.

The vendored `proto/matching_engine.proto` is now a **faithful copy of the real
service's proto** (`package matchingengine`; `FillNotification` carries `symbol`
and integer `size`/`remaining_size`; `Level2Request` is `side=2, depth=3` with a
nested `Side` enum where `BOTH=2`; depth levels are `PriceLevelData`). The adapter
only calls `Subscribe` + `GetLevel2`, but the message field numbers and scalar types
now line up on the wire. Record/replay reuses the neutral event text format, so
`--replay <file>` works offline (no engine) and event files are interchangeable with
`finchart replay`. See `docs/matching-engine-adapter-v1-plan.md`.

The engine's gRPC port (`cpp-server:50051`) is **internal to its Docker network by
default**; expose it with a `ports: ["50051:50051"]` entry on the `cpp-server`
service before connecting. It's an insecure channel (`http://`, no TLS), and
`Subscribe` only emits on a trade cross — a quiet book yields no fills, so keep the
mean-reversion client running to generate flow.