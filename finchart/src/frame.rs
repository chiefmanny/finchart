//! The standard finchart frame for a [`ChartState`]: the price plot (with an
//! optional liquidity heatmap behind it), volume and order-flow delta panels, and
//! a Level 2 ladder on the right.

use backend_terminal::{
    layout as chart_layout, render_chart, ChartOptions, ColorMode, Crosshair, DepthPanel,
    HeatmapView, Layout, Style,
};
use market_core::{BookSide, ChartState};
use render_core::{cull, CulledView, Viewport};

/// Levels shown per side in the depth ladder.
pub const DEPTH_LEVELS: usize = 6;

/// Size, color and layers of one frame.
pub struct Frame<'a> {
    pub width: usize,
    pub height: usize,
    pub color: ColorMode,
    pub heatmap: bool,
    pub crosshair: Option<Crosshair>,
    pub title: &'a str,
}

/// Render `state` over `viewport` (clamped to the series).
pub fn render(state: &ChartState, viewport: &Viewport, frame: &Frame) -> String {
    let series = state.series();
    let view = cull(series, &viewport.clamp(series.len()));
    render_chart(&view, &options(state, &view, frame))
}

/// Where [`render`] puts the plot and each bar for the same inputs.
pub fn layout(state: &ChartState, viewport: &Viewport, frame: &Frame) -> Option<Layout> {
    let series = state.series();
    let view = cull(series, &viewport.clamp(series.len()));
    chart_layout(&view, &options(state, &view, frame))
}

fn options(state: &ChartState, view: &CulledView, frame: &Frame) -> ChartOptions {
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

    ChartOptions {
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
        crosshair: frame.crosshair,
        title: frame.title.to_string(),
    }
}
