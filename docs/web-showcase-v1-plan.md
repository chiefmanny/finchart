# Web Showcase V1 — Interactive WASM Terminal — Plan

> Status: implemented. Embed an interactive "terminal on a page" that runs the
> **finchart engine compiled to WebAssembly**, replaying recorded market scenarios
> with pan/zoom and a scenario switcher — no backend.
>
> Recorded replay only (no live feed in V1). No Canvas/WebGPU renderer — the
> existing terminal backend's ANSI output is displayed in a browser terminal
> emulator (xterm.js). The zero-dependency cores stay untouched and zero-dep;
> `wasm-bindgen` is confined to the new `finchart-web` crate.

## Context

The most compelling personal-site showcase is the engine itself running live in the
browser, not a screenshot. Exploration confirmed it was low-risk: `market-core`,
`render-core`, and `backend-terminal` are zero-dependency, pure computation, and
WASM-safe with no modification (every system dep — tokio, gRPC, `SystemTime`, `fs`,
channels — is confined to `examples-matching-engine`). The full pipeline is already
pure: text `MarketEvent` lines (`parse_event`) → `ChartState::apply` → `cull` →
`render_chart(&CulledView, &ChartOptions) -> String` (ANSI). `render_chart` takes
`width`/`height` as pure inputs and `Viewport::{tail,pan,zoom}` are pure, so a
browser drives size + interactivity locally. Record/replay is deterministic and
already ships as plain `.txt` files — perfect static assets.

CLAUDE.md requires an explicit milestone to add "web"; this is that milestone.

## Architecture

```
recorded .txt (static asset)                 [browser]
  │  fetch + split into event lines
  ▼
main.js player ──apply_line(line)──►  finchart-web (WASM)
  │                                      │  parse_event → ChartState::apply
  │  ◄──────render(cols,rows)───────────┤  cull(viewport) → render_chart → ANSI
  ▼                                      │
xterm.js  term.write('\x1b[H\x1b[2J'+frame)   pan/zoom/follow → Viewport
```

JS owns the transport (fetch + animation loop); the WASM stays pure/sync.

## What was built

- **`finchart-web/` crate** (repo root; `crate-type = ["cdylib","rlib"]`). Depends
  on the three cores + `wasm-bindgen` only. Exposes `WasmChart` via `#[wasm_bindgen]`:
  `new(interval_secs, tick_size, heatmap_capacity, trade_cap, window, symbol)`,
  `apply_line`, `apply_events`, `reset`, `render(cols, rows) -> String` (ANSI),
  `pan`, `zoom`, `follow`, `series_len`, `visible_candles`, `trade_count`. `render`
  rebuilds `ChartOptions` from `ChartState` accessors each call and culls with the
  live tail (`Viewport::tail`) when following, else the manual viewport. The render
  assembly (depth panel + side-aware heatmap + volume/delta + status line) is
  **duplicated locally** (Open Decision A) so the crate needn't depend on
  `examples-matching-engine` (crossterm/gRPC); trade markers are off in V1 (their
  live column mapping is a follow-up), matching the adapter's `render_state`.
- **`web/` static front-end.** `index.html` (styled terminal window + controls),
  `main.js` (loads the `wasm-pack --target web` bundle + xterm.js/FitAddon from CDN,
  fetches a scenario file, runs the batched player loop, wires pan/zoom/follow/speed
  and resize), `build.sh`, `README.md`.
- **`web/scenarios/baseline.txt`** — a deterministic synthetic recording
  (`examples-live --record`, 905 events / 14 candles). More scenarios (structure
  scenarios; real engine captures) drop in the same way.

## Scope / constraints honored

- Cores stay **zero-dependency**; `finchart-web` adds `wasm-bindgen` only (verified
  via `cargo tree`). Nothing in the cores or existing binaries changed.
- **No Canvas/WebGPU** — the existing terminal backend's ANSI is shown in xterm.js.
- **No live feed / WebSocket bridge** in V1 (recorded replay = no backend). Future:
  a `--serve` broadcast mode on `examples-matching-engine` + a live toggle in JS.
- **No participant tagging** — V1 visualizes the aggregate tape; per-participant
  attribution would need a field on `MarketEvent::Trade` (later milestone).
- All existing snapshots/tests preserved.

## Verification

- Host unit tests (`cargo test -p finchart-web`): frame dimensions (30×120), no
  ANSI in plain / present in Ansi, trades build+count candles, `zoom` shrinks the
  visible window, `reset` clears state.
- `wasm-pack build finchart-web --target web` succeeds; a `--target nodejs` smoke
  build rendered the baseline file end-to-end (14 candles, 400 trades, 30-row ANSI
  frame) — proving the compiled WASM, not just the host rlib.
- Static server serves `index.html` / `main.js` / `pkg/*.wasm` (`application/wasm`)
  / `scenarios/*.txt` with 200s.
- Full workspace `cargo test` / `clippy` / `fmt` clean; `cargo tree` shows 0 wasm
  deps in the cores.

## Future extensions

- Live mode: `examples-matching-engine --serve <addr>` broadcasting serialized
  `MarketEvent` lines over WebSocket; a "Connect live" toggle in `main.js`.
- Trade markers (`▲`/`▼`) once live trade→column mapping is worked out.
- Per-participant coloring/labels (needs a participant field on trades).
- Structure scenarios and real engine captures per client mix.
