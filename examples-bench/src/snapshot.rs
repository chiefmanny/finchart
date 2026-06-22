//! Deterministic snapshot/export workflow for the terminal renderer.
//!
//! Given fixed inputs (seed, count, viewport, chart size — no time or
//! terminal-size dependence), this emits four artifacts for inspection and
//! diffing: a plain-text chart, an ANSI-colored chart, a self-contained HTML
//! page, and a metrics JSON file.
//!
//! JSON is hand-written rather than via `serde_json`: the metrics record is a
//! flat, fixed set of numbers, so a dependency-free writer keeps the project's
//! zero-dependency posture without losing anything.

use std::fs;
use std::hint::black_box;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use backend_terminal::{render_chart, rendered_bar_count, ChartOptions, ColorMode, Style};
use market_core::{generate, CandleSeries, GenConfig};
use render_core::{cull, Viewport};

// --- Fixed, deterministic inputs ---------------------------------------------

const SEED: u64 = 42;
pub(crate) const DATASET_SIZE: usize = 500;
const VIEWPORT_START: usize = 300;
const VIEWPORT_END: usize = 420; // 120 visible candles
pub(crate) const CHART_WIDTH: usize = 120;
pub(crate) const CHART_HEIGHT: usize = 30;

fn snapshot_series() -> CandleSeries {
    generate(&GenConfig {
        seed: SEED,
        count: DATASET_SIZE,
        start_ts: 1_600_000_000,
        interval_secs: 60,
        start_price: 100.0,
        volatility: 0.02,
        drift: 0.0003,
    })
}

fn snapshot_viewport() -> Viewport {
    Viewport::new(VIEWPORT_START, VIEWPORT_END)
}

// --- Chart rendering ----------------------------------------------------------

/// Fixed snapshot chart options for a given style and color mode.
fn snapshot_opts(style: Style, color: ColorMode) -> ChartOptions {
    ChartOptions {
        width: CHART_WIDTH,
        height: CHART_HEIGHT,
        color,
        style,
        show_volume: true,
        show_delta: false,
        depth: None,
        title: "finchart · SYNTH".to_string(),
    }
}

/// Render the fixed snapshot chart in the given style and color mode.
pub(crate) fn chart(style: Style, color: ColorMode) -> String {
    let series = snapshot_series();
    let view = cull(&series, &snapshot_viewport());
    render_chart(&view, &snapshot_opts(style, color))
}

/// Render the fixed chart as a self-contained dark-background HTML page that
/// preserves spacing and shows up/down candle colors. Uses the default OHLC
/// style. Delegates to the shared `backend_terminal::html` exporter.
pub(crate) fn chart_html() -> String {
    backend_terminal::html::to_document(&chart(Style::OhlcBars, ColorMode::Ansi))
}

// --- Metrics ------------------------------------------------------------------

pub(crate) struct Metrics {
    pub dataset_size: usize,
    pub visible_candles: usize,
    pub rendered_bars: usize,
    pub lod_active: bool,
    pub style: &'static str,
    pub chart_width: usize,
    pub chart_height: usize,
    pub generation: Duration,
    pub culling: Duration,
    pub render: Duration,
    pub memory_bytes: usize,
}

/// Measure generation/culling/render timings for the fixed snapshot inputs.
/// Timings vary run-to-run; the chart artifacts (not this) are the deterministic
/// outputs.
pub(crate) fn measure() -> Metrics {
    let cfg = GenConfig {
        seed: SEED,
        count: DATASET_SIZE,
        start_ts: 1_600_000_000,
        interval_secs: 60,
        start_price: 100.0,
        volatility: 0.02,
        drift: 0.0003,
    };

    let t = Instant::now();
    let series = generate(&cfg);
    let generation = t.elapsed();

    let vp = snapshot_viewport();
    let t = Instant::now();
    let view = cull(&series, &vp);
    let culling = t.elapsed();
    black_box(&view);

    let view = cull(&series, &vp);
    let render_opts = snapshot_opts(Style::OhlcBars, ColorMode::Plain);
    let t = Instant::now();
    let out = render_chart(&view, &render_opts);
    let render = t.elapsed();
    black_box(&out);

    let rendered_bars = rendered_bar_count(&view, &render_opts);

    Metrics {
        dataset_size: DATASET_SIZE,
        visible_candles: view.len(),
        rendered_bars,
        lod_active: rendered_bars < view.len(),
        style: render_opts.style.label(),
        chart_width: CHART_WIDTH,
        chart_height: CHART_HEIGHT,
        generation,
        culling,
        render,
        memory_bytes: DATASET_SIZE * (std::mem::size_of::<i64>() + 5 * std::mem::size_of::<f64>()),
    }
}

/// Serialize metrics as a small, valid JSON object (hand-written; see module docs).
pub(crate) fn metrics_json(m: &Metrics) -> String {
    format!(
        "{{\n  \
\"dataset_size\": {},\n  \
\"visible_candles\": {},\n  \
\"rendered_bars\": {},\n  \
\"lod_active\": {},\n  \
\"style\": \"{}\",\n  \
\"chart_width\": {},\n  \
\"chart_height\": {},\n  \
\"generation_us\": {:.3},\n  \
\"culling_us\": {:.3},\n  \
\"render_us\": {:.3},\n  \
\"memory_bytes\": {}\n}}\n",
        m.dataset_size,
        m.visible_candles,
        m.rendered_bars,
        m.lod_active,
        m.style,
        m.chart_width,
        m.chart_height,
        m.generation.as_secs_f64() * 1e6,
        m.culling.as_secs_f64() * 1e6,
        m.render.as_secs_f64() * 1e6,
        m.memory_bytes,
    )
}

// --- Writing artifacts --------------------------------------------------------

/// Write all four snapshot artifacts into `dir`, creating it if needed. Returns
/// the paths of the files written.
pub(crate) fn write_all(dir: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    fs::create_dir_all(dir)?;
    let files = [
        (
            "chart.ohlc.plain.txt",
            chart(Style::OhlcBars, ColorMode::Plain),
        ),
        (
            "chart.candles.plain.txt",
            chart(Style::Candles, ColorMode::Plain),
        ),
        ("chart.ansi.txt", chart(Style::OhlcBars, ColorMode::Ansi)),
        ("chart.html", chart_html()),
        ("metrics.json", metrics_json(&measure())),
    ];
    let mut written = Vec::with_capacity(files.len());
    for (name, contents) in files {
        let path = dir.join(name);
        fs::write(&path, contents)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend_terminal::ColorMode;

    #[test]
    fn plain_chart_is_deterministic() {
        assert_eq!(
            chart(Style::OhlcBars, ColorMode::Plain),
            chart(Style::OhlcBars, ColorMode::Plain)
        );
        assert_eq!(
            chart(Style::Candles, ColorMode::Plain),
            chart(Style::Candles, ColorMode::Plain)
        );
    }

    #[test]
    fn ansi_and_html_are_deterministic() {
        assert_eq!(
            chart(Style::OhlcBars, ColorMode::Ansi),
            chart(Style::OhlcBars, ColorMode::Ansi)
        );
        assert_eq!(chart_html(), chart_html());
    }

    #[test]
    fn plain_chart_has_no_ansi_escapes() {
        assert!(!chart(Style::OhlcBars, ColorMode::Plain).contains('\u{1b}'));
        assert!(!chart(Style::Candles, ColorMode::Plain).contains('\u{1b}'));
    }

    #[test]
    fn plain_chart_has_fixed_dimensions() {
        for style in [Style::OhlcBars, Style::Candles] {
            let plain = chart(style, ColorMode::Plain);
            assert_eq!(plain.lines().count(), CHART_HEIGHT);
            for line in plain.lines() {
                assert_eq!(line.chars().count(), CHART_WIDTH);
            }
        }
    }

    #[test]
    fn styles_produce_distinct_output() {
        let ohlc = chart(Style::OhlcBars, ColorMode::Plain);
        let candles = chart(Style::Candles, ColorMode::Plain);
        assert_ne!(ohlc, candles);
        assert!(ohlc.contains('╴'), "OHLC bars use open ticks");
        assert!(!candles.contains('╴'), "candles do not use ticks");
    }

    #[test]
    fn lod_is_active_for_snapshot_inputs() {
        let m = measure();
        assert!(
            m.rendered_bars < m.visible_candles,
            "expected downsampling: {} bars from {} candles",
            m.rendered_bars,
            m.visible_candles
        );
        assert!(m.lod_active);
        assert_eq!(m.style, "OHLC");
    }

    #[test]
    fn html_is_self_contained_and_preformatted() {
        let html = chart_html();
        assert!(html.contains("<!DOCTYPE html>"), "missing doctype");
        assert!(html.contains("<pre"), "missing <pre> block");
        assert!(html.contains("white-space: pre"), "spacing not preserved");
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "external asset"
        );
        assert!(!html.contains('\u{1b}'), "raw escape leaked into html");
        // Up and down candle colors are both rendered.
        assert!(html.contains("class=\"up\""), "no up-colored candles");
        assert!(html.contains("class=\"down\""), "no down-colored candles");
        // Chart glyphs survive into the HTML body.
        assert!(html.contains('█') || html.contains('▒'), "no candle glyphs");
    }

    #[test]
    fn metrics_json_includes_required_fields() {
        let json = metrics_json(&measure());
        for field in [
            "dataset_size",
            "visible_candles",
            "rendered_bars",
            "lod_active",
            "style",
            "chart_width",
            "chart_height",
            "generation_us",
            "culling_us",
            "render_us",
            "memory_bytes",
        ] {
            assert!(json.contains(field), "metrics.json missing field: {field}");
        }
    }

    #[test]
    fn measured_metrics_match_fixed_inputs() {
        let m = measure();
        assert_eq!(m.dataset_size, DATASET_SIZE);
        assert_eq!(m.chart_width, CHART_WIDTH);
        assert_eq!(m.chart_height, CHART_HEIGHT);
        assert_eq!(m.memory_bytes, DATASET_SIZE * (8 + 5 * 8));
    }
}
