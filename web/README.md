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

## Showing off different clients

The feed carries no participant identity, so the way to make a client type legible
is to **record a session where that client dominates**. Run the engine with (say)
only the market-maker client active, capture it, and its signature shows up on the
chart: a market maker holds a tight spread with thick resting liquidity (dense
two-sided heatmap around the mid); an HFT shows fast one-sided bursts and rapid
book churn; retail is sparse small market orders; an institution sweeps several
levels at once (a streak of same-side trade prints and a visible dent in depth).

Capture one file per client mix with the helper:

```bash
# with the engine running the client mix you want to show:
./web/record-client.sh market-maker AAPL          # → web/scenarios/market-maker.txt
./web/record-client.sh hft AAPL
./web/record-client.sh institution AAPL
```

Press `q` in the chart to stop; the file flushes on quit. `main.js` already lists
`market-maker` / `hft` / `retail` / `institution` entries — **missing files are
skipped automatically**, so each appears in the switcher as soon as you record it.
To add a different id, copy an entry in the `SCENARIOS` array in `main.js`.

Scenario files are ordinary recorded `MarketEvent` streams — deterministic on
replay, so each renders identically every visit. A synthetic baseline (no engine
needed) comes from:

```bash
cargo run -p examples-live --release -- --record web/scenarios/baseline.txt
```

The recorded prices/sizes are replay-parameter agnostic; only a `SCENARIOS`
entry's `interval` / `tick` / `heatmap` / `window` affect how the browser buckets
candles. Note the `examples-live --scenario` structures (walls/sweeps) are
snapshot-only — their candle line is decorative, not trade-driven — so they don't
translate to browser event replay; use engine captures instead.

## Embedding on a site

`web/` is self-contained static files (WASM + JS + CDN xterm.js). Host it on
GitHub Pages / Vercel / Netlify, or drop it into an existing site as a subdirectory
or an `<iframe>`. No backend — the demo replays recorded files entirely in-browser.

[xterm.js]: https://xtermjs.org/
