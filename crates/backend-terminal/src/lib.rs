//! `backend-terminal`: rasterizes a culled candle view into a text grid (with
//! optional ANSI color). Pure compute — it returns a `String`; callers print it.

pub mod html;

mod grid;
mod render;
mod timefmt;

pub use render::{render_chart, rendered_bar_count, ChartOptions, ColorMode, Style};
