//! Event record / replay: run the synthetic [`MarketEvent`] source, write the
//! stream to a text file, and replay a recorded file back through a
//! [`ChartState`] — proving the neutral event path is source-agnostic. A future
//! file/gRPC adapter plugs in here by producing the same events.

use std::fs;
use std::io;
use std::path::Path;

use backend_terminal::{render_chart, ChartOptions, ColorMode, DepthPanel, HeatmapView, Style};
use market_core::{
    events, parse_events, replay_events, serialize_events, BookSide, CandleSeries, ChartState,
    DepthConfig, EventConfig, LiveConfig, MarketEvent,
};
use render_core::{cull, Viewport};

const TICK_SIZE: f64 = 0.05;
const INTERVAL: i64 = 60;
const HEATMAP_CAP: usize = 160;
const WINDOW: usize = 60;
const DEPTH_LEVELS: usize = 6;
const RECORD_TRADES: usize = 400;
const CHART_WIDTH: usize = 120;
const CHART_HEIGHT: usize = 30;

/// A fixed, deterministic synthetic event source config.
fn feed_config() -> EventConfig {
    EventConfig {
        live: LiveConfig {
            seed: 42,
            start_ts: 1_600_000_000,
            start_price: 100.0,
            interval_secs: INTERVAL,
            ticks_per_candle: 30,
            volatility: 0.0015,
            drift: 0.0002,
            volume_per_tick: 40.0,
        },
        depth: DepthConfig {
            seed: 99,
            tick_size: TICK_SIZE,
            levels: 12,
            base_size: 50.0,
            size_jitter: 0.5,
            recenter_band: 0.5,
            updates_per_step: 6,
        },
        depth_every: 5,
    }
}

/// Collect events spanning `trades` trades (the seeding snapshot plus the
/// interleaved depth/heartbeat events come along for free).
fn collect_events(trades: usize) -> Vec<MarketEvent> {
    let mut out = Vec::new();
    let mut seen = 0;
    for ev in events(&feed_config()) {
        let is_trade = matches!(ev, MarketEvent::Trade(_));
        out.push(ev);
        if is_trade {
            seen += 1;
            if seen >= trades {
                break;
            }
        }
    }
    out
}

/// Fold an event slice into a fresh [`ChartState`].
fn fold(evs: &[MarketEvent]) -> ChartState {
    let mut state = ChartState::from_history(
        CandleSeries::new(),
        INTERVAL,
        TICK_SIZE,
        Some(HEATMAP_CAP),
        WINDOW,
    );
    replay_events(&mut state, evs);
    state
}

/// Render a follow-latest frame from a [`ChartState`] (heatmap on).
fn render_state(state: &ChartState, title: &str) -> String {
    let series = state.series();
    let view = cull(series, &Viewport::tail(series.len(), WINDOW));
    let (lo, hi) = view.price_extent().unwrap_or((0.0, 1.0));

    let heatmap = state.heatmap().map(|h| {
        let columns = h
            .columns_in_range_sided(lo, hi)
            .into_iter()
            .map(|col| {
                col.into_iter()
                    .map(|(p, l, side)| (p, l, side == BookSide::Bid))
                    .collect()
            })
            .collect();
        HeatmapView {
            columns,
            max_liquidity: h.max_liquidity(),
        }
    });

    let to_tuples = |side| {
        state
            .book()
            .top_n(side, DEPTH_LEVELS)
            .into_iter()
            .map(|l| (l.price, l.size))
            .collect()
    };
    let depth = DepthPanel {
        bids: to_tuples(BookSide::Bid),
        asks: to_tuples(BookSide::Ask),
        spread: state.spread(),
    };

    render_chart(
        &view,
        &ChartOptions {
            width: CHART_WIDTH,
            height: CHART_HEIGHT,
            color: ColorMode::Plain,
            style: Style::OhlcBars,
            show_volume: true,
            show_delta: true,
            depth: Some(depth),
            heatmap,
            trades: None,
            annotations: Vec::new(),
            title: title.to_string(),
        },
    )
}

/// Record the synthetic event stream to `path` (one event per line); returns the
/// number of events written.
pub(crate) fn record(path: &Path) -> io::Result<usize> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let evs = collect_events(RECORD_TRADES);
    fs::write(path, serialize_events(&evs))?;
    Ok(evs.len())
}

/// Replay a recorded event file into a fresh [`ChartState`] and render a frame.
pub(crate) fn replay(path: &Path) -> io::Result<String> {
    let text = fs::read_to_string(path)?;
    let state = fold(&parse_events(&text));
    Ok(render_state(&state, "finchart REPLAY"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_then_replayed_matches_direct_fold() {
        let evs = collect_events(300);
        let direct = fold(&evs);
        // Round-trip through the text format and re-fold.
        let parsed = market_core::parse_events(&market_core::serialize_events(&evs));
        let replayed = fold(&parsed);
        assert_eq!(direct, replayed);
    }

    #[test]
    fn rendered_frame_matches_after_record_replay() {
        let evs = collect_events(300);
        let direct = render_state(&fold(&evs), "finchart FEED");
        let parsed = market_core::parse_events(&market_core::serialize_events(&evs));
        let replayed = render_state(&fold(&parsed), "finchart FEED");
        assert_eq!(direct, replayed);
        assert_eq!(direct.lines().count(), CHART_HEIGHT);
    }

    #[test]
    fn record_writes_a_file_and_replay_reads_it() {
        let dir = std::env::temp_dir().join(format!("finchart_feed_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("events.txt");

        let n = record(&file).unwrap();
        assert!(n > 0, "no events recorded");
        assert!(file.exists(), "events file not written");

        let frame = replay(&file).unwrap();
        assert_eq!(frame.lines().count(), CHART_HEIGHT);
        for line in frame.lines() {
            assert_eq!(line.chars().count(), CHART_WIDTH);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_events_starts_with_seed_and_covers_requested_trades() {
        let evs = collect_events(50);
        assert!(matches!(evs[0], market_core::MarketEvent::DepthSnapshot(_)));
        let trades = evs
            .iter()
            .filter(|e| matches!(e, market_core::MarketEvent::Trade(_)))
            .count();
        assert_eq!(trades, 50);
    }
}
