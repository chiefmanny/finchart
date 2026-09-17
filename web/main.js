// finchart web player: loads the WASM chart, streams a recorded MarketEvent file
// through it, and paints ANSI frames into an xterm.js terminal.
//
// The WASM module (WasmChart) does all the charting; this file only owns the
// transport (fetch a scenario file) and the animation/interaction loop.

import init, { WasmChart } from './pkg/finchart_web.js';

// Each scenario is a recorded MarketEvent file plus the ChartState params it was
// recorded with (interval/tick must match for candles to bucket correctly).
//
// To showcase different market participants, record one session per client mix
// from the matching engine and add an entry here (see web/record-client.sh):
//   { id: 'market-maker', label: 'Market maker', file: 'scenarios/market-maker.txt',
//     desc: 'Two-sided quoting: tight spread, thick resting liquidity',
//     interval: 5, tick: 0.01, heatmap: 160, tradeCap: 64, window: 60 },
// Entries whose file is missing are skipped automatically, so it is safe to list
// clients before you have captured them.
const SCENARIOS = [
  { id: 'baseline', label: 'Baseline (synthetic)', file: 'scenarios/baseline.txt',
    desc: 'Synthetic feed — deterministic reference market',
    interval: 60, tick: 0.05, heatmap: 160, tradeCap: 64, window: 60 },
  // interval:1 so even short (seconds-long) captures render as a candle sequence;
  // longer captures simply yield more candles (LOD handles density).
  { id: 'market-maker', label: 'Market maker', file: 'scenarios/market-maker.txt',
    desc: 'Two-sided quoting: tight spread, thick resting liquidity',
    interval: 1, tick: 0.01, heatmap: 160, tradeCap: 64, window: 60 },
  { id: 'hft', label: 'HFT', file: 'scenarios/hft.txt',
    desc: 'Fast one-sided bursts, rapid book churn',
    interval: 1, tick: 0.01, heatmap: 160, tradeCap: 64, window: 60 },
  { id: 'retail', label: 'Retail', file: 'scenarios/retail.txt',
    desc: 'Sparse small market orders, little resting size',
    interval: 1, tick: 0.01, heatmap: 160, tradeCap: 64, window: 60 },
  { id: 'institution', label: 'Institution', file: 'scenarios/institution.txt',
    desc: 'Large blocks sweeping multiple levels',
    interval: 1, tick: 0.01, heatmap: 160, tradeCap: 64, window: 60 },
];

const PAN_STEP = 4;
const ZOOM_IN = 0.8;
const ZOOM_OUT = 1.25;

const els = {
  term: document.getElementById('term'),
  scenario: document.getElementById('scenario'),
  playpause: document.getElementById('playpause'),
  speed: document.getElementById('speed'),
  zoomin: document.getElementById('zoomin'),
  zoomout: document.getElementById('zoomout'),
  panl: document.getElementById('panl'),
  panr: document.getElementById('panr'),
  follow: document.getElementById('follow'),
  desc: document.getElementById('desc'),
};

// --- terminal ---------------------------------------------------------------
const term = new Terminal({
  fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
  fontSize: 13,
  theme: { background: '#0d1117', foreground: '#c9d1d9' },
  // render_chart joins rows with '\n'; xterm needs CRLF to return to column 0,
  // else every row cascades off the right edge (blank screen). convertEol maps
  // '\n' -> '\r\n' on write.
  convertEol: true,
  cursorBlink: false,
  disableStdin: true,
});
const fit = new FitAddon.FitAddon();
term.loadAddon(fit);
term.open(els.term);
// Fit after layout settles so cols/rows aren't computed against a zero-size box.
requestAnimationFrame(() => fit.fit());

// --- player state -----------------------------------------------------------
let chart = null;      // WasmChart
let lines = [];        // event lines of the current scenario
let cursor = 0;        // next line to apply
let playing = true;
let following = true;
let current = SCENARIOS[0];
let available = SCENARIOS;   // narrowed to scenarios with a present file at boot

function newChart(sc) {
  return new WasmChart(sc.interval, sc.tick, sc.heatmap, sc.tradeCap, sc.window, sc.id.toUpperCase());
}

function paint() {
  const { cols, rows } = term;
  const frame = chart.render(cols, rows);
  // Home + clear-below, then the fresh frame (full repaint each tick).
  term.write('\x1b[H\x1b[2J' + frame);
}

async function loadScenario(sc) {
  current = sc;
  if (els.desc) els.desc.textContent = sc.desc || '';
  let text;
  try {
    const res = await fetch(sc.file);
    if (!res.ok) throw new Error(`${res.status}`);
    text = await res.text();
  } catch (e) {
    lines = [];
    chart = newChart(sc);
    term.write(`\x1b[H\x1b[2J\r\n  could not load ${sc.file} (${e.message}).\r\n` +
      `  record it from the engine, e.g.:\r\n\r\n` +
      `    ./web/record-client.sh ${sc.id} <symbol>\r\n`);
    return;
  }
  lines = text.split('\n').filter((l) => l.length > 0);
  cursor = 0;
  chart = newChart(sc);
  setFollow(true);
  paint();
}

// --- animation loop ---------------------------------------------------------
function tick() {
  if (chart && playing) {
    const batch = parseInt(els.speed.value, 10) || 6;
    for (let i = 0; i < batch && cursor < lines.length; i++) {
      chart.apply_line(lines[cursor++]);
    }
    if (cursor >= lines.length) {
      // Loop the demo: rebuild and replay from the top.
      cursor = 0;
      chart = newChart(current);
      setFollow(following);
    }
    paint();
  }
  requestAnimationFrame(tick);
}

// --- interaction ------------------------------------------------------------
function setFollow(on) {
  following = on;
  if (on && chart) chart.follow();
  els.follow.classList.toggle('active', on);
}
function setPlaying(on) {
  playing = on;
  els.playpause.textContent = on ? '⏸ Pause' : '▶ Play';
}
function pan(delta) { if (!chart) return; chart.pan(delta); setFollow(false); paint(); }
function zoom(factor) { if (!chart) return; chart.zoom(factor); setFollow(false); paint(); }

els.playpause.onclick = () => setPlaying(!playing);
els.zoomin.onclick = () => zoom(ZOOM_IN);
els.zoomout.onclick = () => zoom(ZOOM_OUT);
els.panl.onclick = () => pan(-PAN_STEP);
els.panr.onclick = () => pan(PAN_STEP);
els.follow.onclick = () => { setFollow(true); paint(); };
els.scenario.onchange = () => loadScenario(available[els.scenario.selectedIndex]);

// Keyboard (captured by the terminal element).
term.attachCustomKeyEventHandler((e) => {
  if (e.type !== 'keydown') return true;
  switch (e.key) {
    case 'ArrowLeft': pan(-PAN_STEP); return false;
    case 'ArrowRight': pan(PAN_STEP); return false;
    case '+': case '=': zoom(ZOOM_IN); return false;
    case '-': case '_': zoom(ZOOM_OUT); return false;
    case 'f': case 'F': setFollow(true); paint(); return false;
    default: return true;
  }
});
els.term.addEventListener('wheel', (e) => {
  e.preventDefault();
  zoom(e.deltaY < 0 ? ZOOM_IN : ZOOM_OUT);
}, { passive: false });

window.addEventListener('resize', () => { fit.fit(); if (chart) paint(); });

// --- boot -------------------------------------------------------------------
(async function boot() {
  await init();
  // Keep only scenarios whose recording is actually present (HEAD 200), so
  // listed-but-not-yet-captured clients don't show up as dead options.
  const present = await Promise.all(
    SCENARIOS.map((sc) =>
      fetch(sc.file, { method: 'HEAD' }).then((r) => r.ok).catch(() => false))
  );
  available = SCENARIOS.filter((_, i) => present[i]);
  if (available.length === 0) available = [SCENARIOS[0]]; // show the load error
  available.forEach((sc) => {
    const opt = document.createElement('option');
    opt.textContent = sc.label;
    els.scenario.appendChild(opt);
  });
  await loadScenario(available[0]);
  setPlaying(true);
  requestAnimationFrame(tick);
})();
