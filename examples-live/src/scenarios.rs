//! Deterministic market-structure scenario snapshots.
//!
//! Each scenario drives a synthetic order book from a scripted
//! `market_core::scenario`, records a liquidity heatmap, builds a candle price
//! line, and renders a zoomed view with the heatmap layer + executed-trade
//! markers — making the heatmap interpretable. Fully deterministic.

use std::f64::consts::TAU;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use backend_terminal::{
    html, render_chart, ChartOptions, ColorMode, HeatmapView, Style, TradeMarker, TradeMarkers,
};
use market_core::{
    scenario, BookSide, Candle, CandleSeries, ChartState, MarketEvent, ScenarioConfig,
    ScenarioKind, Side,
};
use render_core::{cull, Viewport};

pub(crate) const CHART_WIDTH: usize = 120;
pub(crate) const CHART_HEIGHT: usize = 30;
const STEPS: usize = 60;

/// Which captured moment of a scenario to render.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Moment {
    Before,
    Mid,
    After,
}

impl Moment {
    /// How many steps to advance for this moment (Before/Mid straddle the
    /// halfway point where pulls happen).
    fn up_to(self) -> usize {
        match self {
            Moment::Before => STEPS / 3,
            Moment::Mid => 2 * STEPS / 3,
            Moment::After => STEPS,
        }
    }
}

/// Map a scenario name to its kind.
pub(crate) fn parse_scenario(name: &str) -> Option<ScenarioKind> {
    Some(match name {
        "ask-wall" => ScenarioKind::StaticAskWall,
        "bid-wall" => ScenarioKind::StaticBidWall,
        "ask-wall-pulled" => ScenarioKind::AskWallPulled,
        "bid-wall-pulled" => ScenarioKind::BidWallPulled,
        "sweep" => ScenarioKind::SweepAsk,
        "bid-reload" => ScenarioKind::BidReload,
        _ => return None,
    })
}

/// The display name for a scenario kind.
fn scenario_name(kind: ScenarioKind) -> &'static str {
    match kind {
        ScenarioKind::StaticAskWall => "ask-wall",
        ScenarioKind::StaticBidWall => "bid-wall",
        ScenarioKind::AskWallPulled => "ask-wall-pulled",
        ScenarioKind::BidWallPulled => "bid-wall-pulled",
        ScenarioKind::SweepAsk => "sweep",
        ScenarioKind::BidReload => "bid-reload",
    }
}

/// Fixed scenario config for snapshots.
fn config(kind: ScenarioKind) -> ScenarioConfig {
    ScenarioConfig {
        kind,
        seed: 42,
        tick_size: 0.05,
        levels: 12,
        base_size: 10.0,
        mid: 100.0,
        steps: STEPS,
        wall_size: 200.0,
        wall_offset: 5,
        start_ts: 1_600_000_000,
    }
}

/// Per-step candle close: a sine wave across the price band (so the band is in
/// view), except `SweepAsk`, which follows the ramping mid.
fn close_price(cfg: &ScenarioConfig, mid: f64, t: usize) -> f64 {
    match cfg.kind {
        ScenarioKind::SweepAsk => mid,
        _ => {
            let band = cfg.levels as f64 * cfg.tick_size;
            cfg.mid + band * 0.85 * (t as f64 / cfg.steps as f64 * TAU * 2.0).sin()
        }
    }
}

/// Build the full chart for a scenario advanced to `up_to` steps.
fn render(kind: ScenarioKind, up_to: usize, color: ColorMode) -> (String, ScenarioStats) {
    let cfg = config(kind);
    let steps = scenario(&cfg);
    let up_to = up_to.min(steps.len());

    // The candle price line (sine/ramp) is decorative history; the book and
    // heatmap are driven through ChartState via depth + heartbeat events.
    let mut candles = CandleSeries::new();
    let mut markers = Vec::new();
    let mut prev = cfg.mid;
    for (t, step) in steps.iter().take(up_to).enumerate() {
        let close = close_price(&cfg, step.mid, t);
        candles.push(Candle {
            ts: step.ts,
            open: prev,
            high: prev.max(close) + cfg.tick_size,
            low: prev.min(close) - cfg.tick_size,
            close,
            volume: 1.0,
        });
        prev = close;

        for trade in &step.trades {
            markers.push(TradeMarker {
                price: trade.price,
                col_from_right: (up_to - 1) - t,
                buy: trade.side == Some(Side::Buy),
            });
        }
    }

    let mut state = ChartState::from_history(candles, 1, cfg.tick_size, Some(cfg.steps), 0);
    for step in steps.iter().take(up_to) {
        state.apply(&MarketEvent::DepthSnapshot(step.snapshot.clone()));
        state.apply(&MarketEvent::Heartbeat { ts: step.ts });
    }
    let heatmap = state.heatmap().expect("heatmap enabled");

    let view = cull(state.series(), &Viewport::new(0, state.series().len()));
    let (lo, hi) = view
        .price_extent()
        .unwrap_or((cfg.mid - 1.0, cfg.mid + 1.0));
    let columns = heatmap
        .columns_in_range_sided(lo, hi)
        .into_iter()
        .map(|col| {
            col.into_iter()
                .map(|(p, l, side)| (p, l, side == BookSide::Bid))
                .collect()
        })
        .collect();

    let stats = ScenarioStats {
        time_buckets: heatmap.time_buckets(),
        price_buckets: heatmap.price_buckets(),
        visible_cells: heatmap.cells_in_price_range(lo, hi),
        max_liquidity: heatmap.max_liquidity(),
        trade_markers: markers.len(),
    };

    let frame = render_chart(
        &view,
        &ChartOptions {
            width: CHART_WIDTH,
            height: CHART_HEIGHT,
            color,
            style: Style::OhlcBars,
            show_volume: false,
            show_delta: false,
            depth: None,
            heatmap: Some(HeatmapView {
                columns,
                max_liquidity: heatmap.max_liquidity(),
            }),
            trades: Some(TradeMarkers { markers }),
            title: format!("scenario: {}", scenario_name(kind)),
        },
    );
    (frame, stats)
}

struct ScenarioStats {
    time_buckets: usize,
    price_buckets: usize,
    visible_cells: usize,
    max_liquidity: f64,
    trade_markers: usize,
}

/// Render a scenario frame at the given moment.
pub(crate) fn scenario_frame(kind: ScenarioKind, moment: Moment, color: ColorMode) -> String {
    render(kind, moment.up_to(), color).0
}

/// Scenario metrics JSON (hand-written; deterministic counts, no timings).
pub(crate) fn scenario_metrics_json(kind: ScenarioKind) -> String {
    let stats = render(kind, Moment::After.up_to(), ColorMode::Plain).1;
    format!(
        "{{\n  \
\"scenario\": \"{}\",\n  \
\"steps\": {},\n  \
\"heatmap_time_buckets\": {},\n  \
\"heatmap_price_buckets\": {},\n  \
\"visible_heatmap_cells\": {},\n  \
\"max_liquidity\": {:.3},\n  \
\"trade_markers\": {},\n  \
\"chart_width\": {},\n  \
\"chart_height\": {}\n}}\n",
        scenario_name(kind),
        STEPS,
        stats.time_buckets,
        stats.price_buckets,
        stats.visible_cells,
        stats.max_liquidity,
        stats.trade_markers,
        CHART_WIDTH,
        CHART_HEIGHT,
    )
}

/// Write the before/mid/after frames, a mid HTML page, and metrics into `dir`.
pub(crate) fn write_scenario(kind: ScenarioKind, dir: &Path) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    let files = [
        (
            "frame.before.plain.txt",
            scenario_frame(kind, Moment::Before, ColorMode::Plain),
        ),
        (
            "frame.mid.plain.txt",
            scenario_frame(kind, Moment::Mid, ColorMode::Plain),
        ),
        (
            "frame.after.plain.txt",
            scenario_frame(kind, Moment::After, ColorMode::Plain),
        ),
        (
            "frame.mid.html",
            html::to_document(&scenario_frame(kind, Moment::Mid, ColorMode::Ansi)),
        ),
        ("metrics.json", scenario_metrics_json(kind)),
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
    fn parse_known_and_unknown_scenarios() {
        assert_eq!(
            parse_scenario("ask-wall"),
            Some(ScenarioKind::StaticAskWall)
        );
        assert_eq!(
            parse_scenario("bid-wall"),
            Some(ScenarioKind::StaticBidWall)
        );
        assert_eq!(
            parse_scenario("ask-wall-pulled"),
            Some(ScenarioKind::AskWallPulled)
        );
        assert_eq!(
            parse_scenario("bid-wall-pulled"),
            Some(ScenarioKind::BidWallPulled)
        );
        assert_eq!(parse_scenario("sweep"), Some(ScenarioKind::SweepAsk));
        assert_eq!(parse_scenario("bid-reload"), Some(ScenarioKind::BidReload));
        assert_eq!(parse_scenario("bogus"), None);
    }

    #[test]
    fn scenario_frames_are_deterministic() {
        for kind in [ScenarioKind::StaticAskWall, ScenarioKind::SweepAsk] {
            assert_eq!(
                scenario_frame(kind, Moment::Mid, ColorMode::Plain),
                scenario_frame(kind, Moment::Mid, ColorMode::Plain)
            );
        }
    }

    #[test]
    fn scenario_frames_keep_fixed_dimensions() {
        let f = scenario_frame(ScenarioKind::StaticBidWall, Moment::After, ColorMode::Plain);
        assert_eq!(f.lines().count(), CHART_HEIGHT);
        for line in f.lines() {
            assert_eq!(line.chars().count(), CHART_WIDTH);
        }
        assert!(!f.contains('\u{1b}'), "plain frame has escapes");
    }

    #[test]
    fn scenario_frame_shows_heatmap_and_trade_markers() {
        // The sweep emits buy trades, so a buy marker plus heatmap shades appear.
        let f = scenario_frame(ScenarioKind::SweepAsk, Moment::After, ColorMode::Plain);
        assert!(
            f.chars().any(|c| matches!(c, '░' | '▒' | '▓')),
            "no heatmap shade glyphs"
        );
        assert!(f.contains('▲'), "no buy trade markers");
    }

    #[test]
    fn pulled_wall_changes_the_frame() {
        let before = scenario_frame(
            ScenarioKind::AskWallPulled,
            Moment::Before,
            ColorMode::Plain,
        );
        let after = scenario_frame(ScenarioKind::AskWallPulled, Moment::After, ColorMode::Plain);
        assert_ne!(before, after);
    }

    #[test]
    fn scenario_metrics_includes_required_fields() {
        let json = scenario_metrics_json(ScenarioKind::SweepAsk);
        for field in [
            "scenario",
            "steps",
            "heatmap_time_buckets",
            "heatmap_price_buckets",
            "visible_heatmap_cells",
            "max_liquidity",
            "trade_markers",
        ] {
            assert!(json.contains(field), "metrics missing field: {field}");
        }
        assert!(json.contains("sweep"), "scenario name not recorded");
    }
}
