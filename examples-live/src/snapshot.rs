//! Deterministic non-interactive replay snapshot: capture before / mid / after
//! frames of a forming candle plus a metrics file, for inspection and diffing.
//! Fixed inputs and fixed advance points → reproducible frame text.

use std::fs;
use std::hint::black_box;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use backend_terminal::{html, ColorMode};

use crate::replay::{Replay, ReplayConfig};

pub(crate) const CHART_WIDTH: usize = 120;
pub(crate) const CHART_HEIGHT: usize = 30;

/// Fixed, deterministic replay config for snapshots.
fn snap_config() -> ReplayConfig {
    ReplayConfig {
        seed: 42,
        history: 200,
        interval_secs: 60,
        start_price: 100.0,
        candle_volatility: 0.02,
        tick_volatility: 0.0015,
        drift: 0.0002,
        ticks_per_candle: 30,
        volume_per_tick: 40.0,
        window: 60,
    }
}

/// Tick counts for the three captured moments, relative to a candle boundary:
/// just opened, half formed, and just after the next candle opens.
fn advance_points() -> (usize, usize, usize) {
    let t = snap_config().ticks_per_candle;
    let base = 5 * t; // start a few live candles in
    (base + 1, base + t / 2, base + t + 1)
}

/// Render a fresh replay advanced by `advance` ticks.
fn frame_at(advance: usize, color: ColorMode) -> String {
    let mut replay = Replay::new(&snap_config());
    replay.advance(advance);
    replay.render(CHART_WIDTH, CHART_HEIGHT, color, &replay.status())
}

pub(crate) fn before_frame() -> String {
    frame_at(advance_points().0, ColorMode::Plain)
}

pub(crate) fn mid_frame_plain() -> String {
    frame_at(advance_points().1, ColorMode::Plain)
}

pub(crate) fn after_frame() -> String {
    frame_at(advance_points().2, ColorMode::Plain)
}

pub(crate) fn mid_html() -> String {
    html::to_document(&frame_at(advance_points().1, ColorMode::Ansi))
}

/// Replay statistics with update/render timings (timings vary per run).
pub(crate) struct LiveMetrics {
    history_candles: usize,
    interval_secs: i64,
    ticks_per_candle: usize,
    total_ticks: usize,
    frames: usize,
    avg_update_us: f64,
    render_us: f64,
    chart_width: usize,
    chart_height: usize,
}

/// Measure per-tick update time and a single render time for the snapshot run.
pub(crate) fn measure() -> LiveMetrics {
    let cfg = snap_config();
    let total_ticks = advance_points().2;

    let mut replay = Replay::new(&cfg);
    let t = Instant::now();
    replay.advance(total_ticks);
    let update_total = t.elapsed();

    let t = Instant::now();
    let frame = replay.render(
        CHART_WIDTH,
        CHART_HEIGHT,
        ColorMode::Plain,
        &replay.status(),
    );
    let render_us = t.elapsed().as_secs_f64() * 1e6;
    black_box(&frame);

    LiveMetrics {
        history_candles: cfg.history,
        interval_secs: cfg.interval_secs,
        ticks_per_candle: cfg.ticks_per_candle,
        total_ticks,
        frames: 3,
        avg_update_us: update_total.as_secs_f64() * 1e6 / total_ticks.max(1) as f64,
        render_us,
        chart_width: CHART_WIDTH,
        chart_height: CHART_HEIGHT,
    }
}

/// Serialize metrics as a small, valid JSON object (hand-written; no `serde_json`).
pub(crate) fn metrics_json(m: &LiveMetrics) -> String {
    format!(
        "{{\n  \
\"history_candles\": {},\n  \
\"interval_secs\": {},\n  \
\"ticks_per_candle\": {},\n  \
\"total_ticks\": {},\n  \
\"frames\": {},\n  \
\"avg_update_us\": {:.4},\n  \
\"render_us\": {:.3},\n  \
\"chart_width\": {},\n  \
\"chart_height\": {}\n}}\n",
        m.history_candles,
        m.interval_secs,
        m.ticks_per_candle,
        m.total_ticks,
        m.frames,
        m.avg_update_us,
        m.render_us,
        m.chart_width,
        m.chart_height,
    )
}

/// Write all snapshot artifacts into `dir`, returning the paths written.
pub(crate) fn write_all(dir: &Path) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    let files = [
        ("frame.before.plain.txt", before_frame()),
        ("frame.mid.plain.txt", mid_frame_plain()),
        ("frame.after.plain.txt", after_frame()),
        ("frame.mid.html", mid_html()),
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

    #[test]
    fn frames_are_deterministic() {
        assert_eq!(before_frame(), before_frame());
        assert_eq!(mid_frame_plain(), mid_frame_plain());
        assert_eq!(after_frame(), after_frame());
        assert_eq!(mid_html(), mid_html());
    }

    #[test]
    fn plain_frames_have_no_ansi_and_fixed_dimensions() {
        for f in [before_frame(), mid_frame_plain(), after_frame()] {
            assert!(!f.contains('\u{1b}'), "plain frame has escapes");
            assert_eq!(f.lines().count(), CHART_HEIGHT);
            for line in f.lines() {
                assert_eq!(line.chars().count(), CHART_WIDTH);
            }
        }
    }

    #[test]
    fn frames_differ_as_the_candle_forms() {
        // The forming candle grows between "just opened" and "half formed".
        assert_ne!(before_frame(), mid_frame_plain());
    }

    #[test]
    fn mid_html_is_self_contained() {
        let h = mid_html();
        assert!(h.contains("<!DOCTYPE html>"), "missing doctype");
        assert!(h.contains("<pre"), "missing <pre>");
        assert!(!h.contains('\u{1b}'), "raw escape leaked");
    }

    #[test]
    fn metrics_json_includes_required_fields() {
        let json = metrics_json(&measure());
        for field in [
            "history_candles",
            "interval_secs",
            "ticks_per_candle",
            "total_ticks",
            "frames",
            "avg_update_us",
            "render_us",
            "chart_width",
            "chart_height",
        ] {
            assert!(json.contains(field), "metrics.json missing field: {field}");
        }
    }
}
