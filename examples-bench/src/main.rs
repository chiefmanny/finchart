//! `examples-bench`: wires the crates together to print a demo candlestick chart
//! and (later) a 10k/100k/1M generate/cull/render timing table.

use std::hint::black_box;
use std::time::{Duration, Instant};

use backend_terminal::{render_chart, ChartOptions, ColorMode, Style};
use market_core::{generate, CandleSeries, GenConfig};
use render_core::{cull, Viewport};

mod snapshot;

/// Visible window (candles) culled from each dataset for the benchmark.
const VISIBLE: usize = 200;
/// Chart size the benchmark renders the visible window into.
const RENDER_W: usize = 120;
const RENDER_H: usize = 40;

/// Bytes one candle occupies in the columnar store: one `i64` + five `f64`.
const fn bytes_per_candle() -> usize {
    std::mem::size_of::<i64>() + 5 * std::mem::size_of::<f64>()
}

/// Deterministic series used for the on-screen demo.
fn demo_series() -> CandleSeries {
    generate(&GenConfig {
        seed: 42,
        count: 240,
        start_ts: 1_600_000_000,
        interval_secs: 60,
        start_price: 100.0,
        volatility: 0.02,
        drift: 0.0003,
    })
}

/// The visible window for the demo: the most recent 120 candles.
fn demo_viewport(series_len: usize) -> Viewport {
    Viewport::new(series_len.saturating_sub(120), series_len)
}

/// Render the demo chart at the given size and color mode.
fn demo_chart(width: usize, height: usize, color: ColorMode) -> String {
    let series = demo_series();
    let view = cull(&series, &demo_viewport(series.len()));
    render_chart(
        &view,
        &ChartOptions {
            width,
            height,
            color,
            style: Style::OhlcBars,
            show_volume: true,
            show_delta: false,
            depth: None,
            heatmap: None,
            trades: None,
            annotations: Vec::new(),
            title: "finchart · SYNTH".to_string(),
        },
    )
}

/// Average per-phase timings (and the dataset's columnar footprint) for one
/// benchmark scenario.
struct BenchResult {
    size: usize,
    visible: usize,
    bytes: usize,
    gen: Duration,
    cull: Duration,
    render: Duration,
}

/// Benchmark one dataset `size`: generate (averaged over `gen_reps`), then cull
/// and render the visible window (each averaged over `hot_reps`).
fn bench_one(size: usize, gen_reps: usize, hot_reps: usize) -> BenchResult {
    let cfg = GenConfig {
        seed: 1,
        count: size,
        start_ts: 1_600_000_000,
        interval_secs: 60,
        start_price: 100.0,
        volatility: 0.02,
        drift: 0.0001,
    };

    // Generate: time the full O(n) build, keeping the last series for the rest.
    let mut gen_total = Duration::ZERO;
    let mut series = CandleSeries::new();
    for _ in 0..gen_reps.max(1) {
        let t = Instant::now();
        series = generate(&cfg);
        gen_total += t.elapsed();
    }

    let vp = Viewport::new(size.saturating_sub(VISIBLE), size);

    // Cull: time selecting the visible columns (zero-copy slice).
    let mut cull_total = Duration::ZERO;
    for _ in 0..hot_reps.max(1) {
        let t = Instant::now();
        let view = cull(&series, &vp);
        cull_total += t.elapsed();
        black_box(&view);
    }

    let view = cull(&series, &vp);

    // Render: time rasterizing the visible window to text.
    let render_opts = ChartOptions {
        width: RENDER_W,
        height: RENDER_H,
        color: ColorMode::Plain,
        style: Style::OhlcBars,
        show_volume: false,
        show_delta: false,
        depth: None,
        heatmap: None,
        trades: None,
        annotations: Vec::new(),
        title: String::new(),
    };
    let mut render_total = Duration::ZERO;
    for _ in 0..hot_reps.max(1) {
        let t = Instant::now();
        let out = render_chart(&view, &render_opts);
        render_total += t.elapsed();
        black_box(&out);
    }

    BenchResult {
        size,
        visible: view.len(),
        bytes: size * bytes_per_candle(),
        gen: gen_total / gen_reps.max(1) as u32,
        cull: cull_total / hot_reps.max(1) as u32,
        render: render_total / hot_reps.max(1) as u32,
    }
}

fn print_bench_table() {
    println!("benchmark — generate, cull {VISIBLE}, render {RENDER_W}x{RENDER_H} (averaged)\n");
    println!(
        "{:>12}  {:>8}  {:>10}  {:>10}  {:>11}  {:>10}",
        "dataset", "visible", "gen(ms)", "cull(µs)", "render(µs)", "mem"
    );
    for &size in &[10_000usize, 100_000, 1_000_000] {
        let r = bench_one(size, 5, 200);
        println!(
            "{:>12}  {:>8}  {:>10.3}  {:>10.3}  {:>11.3}  {:>8.2} MiB",
            fmt_count(r.size),
            r.visible,
            r.gen.as_secs_f64() * 1e3,
            r.cull.as_secs_f64() * 1e6,
            r.render.as_secs_f64() * 1e6,
            r.bytes as f64 / (1024.0 * 1024.0),
        );
    }
}

/// Format an integer with thousands separators (e.g. `1000000` -> `1,000,000`).
fn fmt_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--snapshot") {
        let dir = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("artifacts/tui");
        match snapshot::write_all(std::path::Path::new(dir)) {
            Ok(paths) => {
                println!("wrote {} snapshot artifacts:", paths.len());
                for p in paths {
                    println!("  {}", p.display());
                }
            }
            Err(e) => {
                eprintln!("snapshot failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    println!("finchart demo — synthetic candles, last 120 of 240\n");
    println!("{}", demo_chart(120, 30, ColorMode::Ansi));
    println!();
    print_bench_table();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_chart_has_requested_height() {
        let out = demo_chart(80, 20, ColorMode::Plain);
        assert_eq!(out.lines().count(), 20);
    }

    #[test]
    fn demo_chart_is_deterministic() {
        assert_eq!(
            demo_chart(80, 20, ColorMode::Plain),
            demo_chart(80, 20, ColorMode::Plain)
        );
    }

    #[test]
    fn demo_chart_draws_candles() {
        let out = demo_chart(80, 20, ColorMode::Plain);
        assert!(
            out.chars().any(|c| matches!(c, '│' | '█' | '▒' | '─')),
            "expected candle glyphs in demo output"
        );
    }

    #[test]
    fn bench_one_reports_size_visible_and_memory() {
        let r = bench_one(1_000, 1, 1);
        assert_eq!(r.size, 1_000);
        assert_eq!(r.visible, 200);
        assert_eq!(r.bytes, 1_000 * (8 + 5 * 8));
    }

    #[test]
    fn bench_one_visible_is_clamped_for_tiny_dataset() {
        let r = bench_one(50, 1, 1);
        assert_eq!(r.visible, 50);
        assert_eq!(r.size, 50);
    }
}
