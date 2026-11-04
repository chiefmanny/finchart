# finchart web showcase (WASM terminal)

An interactive "terminal on a page": the finchart engine compiled to WebAssembly,
replaying a recorded market feed through the live candle / order-flow / depth /
heatmap pipeline and painting ANSI frames into an [xterm.js] terminal. Same
zero-dependency Rust core as the terminal binaries — just a different front-end.

## Layout

```
web/
  index.html        # the page: terminal window + controls
  main.js           # loads the WASM, streams a scenario file, animation + input loop
  build.sh          # wasm-pack build → web/pkg/
  scenarios/*.txt   # recorded MarketEvent files (static assets, deterministic)
  pkg/              # generated WASM bundle (gitignored; run build.sh)
```

The chart itself lives in the `finchart-web` crate (`../finchart-web`), a thin
`wasm-bindgen` wrapper over `market-core` + `render-core` + `backend-terminal`.

## Build & run

One-time:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

Then:

```bash
./build.sh                                   # → web/pkg/
python3 -m http.server --directory .         # serve (WASM won't load from file://)
# open http://localhost:8000
```

Controls: scenario switcher, play/pause, speed. Pan `← →`, zoom `+ −` / mouse
wheel, `f` to follow the live edge.

## Scenarios

| File | Source | What to watch |
|---|---|---|
| `baseline.txt` | `finchart record` (seeded synthetic feed) | a steady two-sided reference market |
| `normal_market.txt` | matching-engine recording, 3 min | an ordinary session for comparison |
| `large_buy_sweep.txt` | matching-engine recording, 3 min | a 10,000-share buy ~70 s in; the book refills, the price does not return |
| `liquidity_crisis.txt` | matching-engine recording, 3 min | resting depth withdraws, then buying runs through the gap and the ask side empties |

The three engine sessions were recorded from a separate C++ price-time-priority
matching engine with simulated participants (symbol `AAPL`, 1 s candles, 0.01
tick) through `examples-matching-engine --record`. They are ordinary
`MarketEvent` files, so they replay identically on every visit.

To add your own session, run the engine with the client mix you want and capture
it with the helper (needs a reachable engine):

```bash
./web/record-client.sh my-session AAPL          # → web/scenarios/my-session.txt
```

Press `q` in the chart to stop; the file flushes on quit. Then add an entry to the
`SCENARIOS` array in `main.js` (entries whose file is missing are skipped). The
recorded prices/sizes are replay-parameter agnostic; only an entry's `interval` /
`tick` / `heatmap` / `window` affect how the browser buckets candles. The
`finchart scenario` structures (walls/sweeps) are snapshot-only — their
candle line is decorative, not trade-driven — so they are not browser scenarios.

A fresh synthetic baseline comes from:

```bash
cargo run --release -p finchart -- record web/scenarios/baseline.txt
```

## Embedding on a site

`web/` is self-contained static files (WASM + JS + CDN xterm.js). Host it on
GitHub Pages / Vercel / Netlify, or drop it into an existing site as a subdirectory
or an `<iframe>`. No backend — the demo replays recorded files entirely in-browser.

[xterm.js]: https://xtermjs.org/
