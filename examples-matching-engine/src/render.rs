//! Render a [`ChartState`] to a terminal frame + build the adapter status line.
//! Reuses `backend-terminal`'s `render_chart` (same pattern as
//! `finchart/src/frame.rs`); kept local so `backend-terminal` stays decoupled from
//! `ChartState`.

use backend_terminal::{render_chart, ChartOptions, ColorMode, DepthPanel, HeatmapView, Style};
use market_core::{BookSide, ChartState};
use render_core::{cull, Viewport};

/// Static layout settings for the adapter chart.
pub struct RenderConfig {
    pub width: usize,
    pub height: usize,
    pub window: usize,
    pub depth_levels: usize,
    pub show_heatmap: bool,
}

/// Live counters + timings surfaced in the status line.
pub struct Stats {
    pub symbol: String,
    pub trades: usize,
    pub depth_refreshes: usize,
    pub update_us: f64,
    pub render_us: f64,
}

/// Render a follow-latest frame from `state`.
pub fn render_state(
    state: &ChartState,
    cfg: &RenderConfig,
    color: ColorMode,
    title: &str,
) -> String {
    let series = state.series();
    let view = cull(series, &Viewport::tail(series.len(), cfg.window));
    let (lo, hi) = view.price_extent().unwrap_or((0.0, 1.0));

    let heatmap = if cfg.show_heatmap {
        state
            .heatmap()
            .map(|h| HeatmapView::from_heatmap(h, lo, hi))
    } else {
        None
    };

    let to_tuples = |side| {
        state
            .book()
            .top_n(side, cfg.depth_levels)
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
            width: cfg.width,
            height: cfg.height,
            color,
            style: Style::OhlcBars,
            show_volume: true,
            show_delta: true,
            depth: Some(depth),
            heatmap,
            trades: None,
            annotations: Vec::new(),
            crosshair: None,
            title: title.to_string(),
        },
    )
}

/// Build the adapter status line: source/symbol, counts, BBO/spread, CVD, timings.
pub fn status_line(state: &ChartState, stats: &Stats) -> String {
    let book = match (state.best_bid(), state.best_ask()) {
        (Some(b), Some(a)) => format!("bid {b:.2} ask {a:.2} spr {:.2}", a - b),
        _ => "bid — ask —".to_string(),
    };
    format!(
        "source=matching-engine · {} · trades {} · depth {} · {} · CVD {:+.0} · upd {:.1}µs · ren {:.1}µs",
        stats.symbol,
        stats.trades,
        stats.depth_refreshes,
        book,
        state.cumulative_delta(),
        stats.update_us,
        stats.render_us,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_core::{
        generate, ChartState, DepthLevel, DepthSnapshot, GenConfig, MarketEvent, Tick,
    };

    fn state_with_data() -> ChartState {
        let history = generate(&GenConfig {
            seed: 1,
            count: 60,
            start_ts: 1_600_000_000,
            interval_secs: 60,
            start_price: 100.0,
            volatility: 0.01,
            drift: 0.0,
        });
        let mut state = ChartState::from_history(history, 60, 0.05, Some(64), 32);
        state.apply(&MarketEvent::DepthSnapshot(DepthSnapshot {
            ts: 1,
            bids: vec![DepthLevel {
                price: 99.95,
                size: 5.0,
            }],
            asks: vec![DepthLevel {
                price: 100.05,
                size: 4.0,
            }],
        }));
        state.apply(&MarketEvent::Heartbeat { ts: 1 });
        state.apply(&MarketEvent::Trade(Tick {
            ts: 1_600_003_600,
            price: 100.0,
            size: 2.0,
            side: Some(market_core::Side::Buy),
        }));
        state
    }

    fn cfg() -> RenderConfig {
        RenderConfig {
            width: 120,
            height: 30,
            window: 60,
            depth_levels: 6,
            show_heatmap: false,
        }
    }

    #[test]
    fn render_state_has_fixed_dimensions_and_no_ansi_in_plain() {
        let out = render_state(&state_with_data(), &cfg(), ColorMode::Plain, "t");
        assert_eq!(out.lines().count(), 30);
        for line in out.lines() {
            assert_eq!(line.chars().count(), 120);
        }
        assert!(!out.contains('\u{1b}'), "plain frame has escapes");
    }

    #[test]
    fn status_line_reports_source_symbol_counts_and_book() {
        let state = state_with_data();
        let stats = Stats {
            symbol: "BTCUSD".to_string(),
            trades: 3,
            depth_refreshes: 2,
            update_us: 1.5,
            render_us: 40.0,
        };
        let s = status_line(&state, &stats);
        assert!(s.contains("source=matching-engine"), "{s}");
        assert!(s.contains("BTCUSD"), "{s}");
        assert!(s.contains("trades 3"), "{s}");
        assert!(s.contains("depth 2"), "{s}");
        assert!(s.contains("bid"), "{s}");
        assert!(s.contains("ask"), "{s}");
        assert!(s.contains("CVD"), "{s}");
    }

    #[test]
    fn status_line_handles_an_empty_book() {
        let state =
            ChartState::from_history(market_core::CandleSeries::new(), 60, 0.05, Some(8), 8);
        let stats = Stats {
            symbol: "X".to_string(),
            trades: 0,
            depth_refreshes: 0,
            update_us: 0.0,
            render_us: 0.0,
        };
        let s = status_line(&state, &stats);
        assert!(s.contains("bid — ask —"), "{s}");
    }
}
