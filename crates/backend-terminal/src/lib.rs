//! `backend-terminal`: rasterizes a culled candle view into a text grid (with
//! optional ANSI color). Pure compute — it returns a `String`; callers print it.

pub mod html;

mod grid;
mod render;
mod timefmt;

pub use render::{
    layout, render_chart, rendered_bar_count, Annotation, ChartOptions, ColorMode, Crosshair,
    DepthPanel, HeatmapColumn, HeatmapView, Layout, Style, TradeMarker, TradeMarkers,
};
