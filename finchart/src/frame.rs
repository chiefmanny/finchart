//! The standard finchart frame for a [`ChartState`]: the price plot (with an
//! optional liquidity heatmap behind it), volume and order-flow delta panels, and
//! a Level 2 ladder on the right.

use backend_terminal::{render_chart, ChartOptions, ColorMode, DepthPanel, HeatmapView, Style};
use market_core::{BookSide, ChartState};
use render_core::{cull, Viewport};

/// Levels shown per side in the depth ladder.
pub const DEPTH_LEVELS: usize = 6;

/// Size, color and layers of one frame.
pub struct Frame<'a> {
    pub width: usize,
    pub height: usize,
    pub color: ColorMode,
    pub heatmap: bool,
    pub title: &'a str,
}

/// Render `state` over `viewport` (clamped to the series).
pub fn render(state: &ChartState, viewport: &Viewport, frame: &Frame) -> String {
    let series = state.series();
    let view = cull(series, &viewport.clamp(series.len()));
    let (lo, hi) = view.price_extent().unwrap_or((0.0, 1.0));

    let heatmap = state
        .heatmap()
        .filter(|_| frame.heatmap)
        .map(|h| HeatmapView::from_heatmap(h, lo, hi));

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
            width: frame.width,
            height: frame.height,
            color: frame.color,
            style: Style::OhlcBars,
            show_volume: true,
            show_delta: true,
            depth: Some(depth),
            heatmap,
            trades: None,
            annotations: Vec::new(),
            title: frame.title.to_string(),
        },
    )
}
