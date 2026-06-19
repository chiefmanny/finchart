//! Candle rasterization and full-chart composition (plot grid + price gutter +
//! time axis). Plain (uncolored) output; ANSI color is layered on in a later step.

use crate::grid::{CellColor, Grid};
use crate::timefmt::format_utc;
use render_core::{aggregate, Bar, CulledView, PriceScale};

/// Open/close tick glyphs for the OHLC bar style.
const OPEN_TICK: char = '╴'; // left tick at the open price
const CLOSE_TICK: char = '╶'; // right tick at the close price

/// Wick / body glyphs. Direction is encoded in the body glyph so the chart stays
/// readable without color; color reinforces it on top of these.
pub(crate) const WICK: char = '│';
pub(crate) const BULL: char = '█'; // close >= open
pub(crate) const BEAR: char = '▒'; // close < open
pub(crate) const DOJI: char = '─'; // zero-height body

/// Whether to emit ANSI color escapes for candle direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// No escape codes; direction is conveyed by the body glyph alone.
    #[default]
    Plain,
    /// Green (up) / red (down) ANSI foreground colors.
    Ansi,
}

const ANSI_GREEN: &str = "\u{1b}[32m";
const ANSI_RED: &str = "\u{1b}[31m";
const ANSI_RESET: &str = "\u{1b}[0m";

/// Decimal places used for price-axis labels.
const PRICE_DECIMALS: usize = 2;

/// Append plot row `r` to `out`. In `Ansi` mode, runs of same-colored cells are
/// wrapped in escape codes (reset on color change and at end of row); in `Plain`
/// mode the raw glyphs are written.
fn push_plot_row(out: &mut String, grid: &Grid, r: usize, color: ColorMode) {
    if color == ColorMode::Plain {
        for c in 0..grid.cols() {
            out.push(grid.get(c, r));
        }
        return;
    }

    let mut current = CellColor::Default;
    for c in 0..grid.cols() {
        let cell_color = grid.get_color(c, r);
        if cell_color != current {
            if current != CellColor::Default {
                out.push_str(ANSI_RESET);
            }
            out.push_str(ansi_code(cell_color));
            current = cell_color;
        }
        out.push(grid.get(c, r));
    }
    if current != CellColor::Default {
        out.push_str(ANSI_RESET);
    }
}

fn ansi_code(color: CellColor) -> &'static str {
    match color {
        CellColor::Green => ANSI_GREEN,
        CellColor::Red => ANSI_RED,
        CellColor::Default => "",
    }
}

fn blank(width: usize, height: usize) -> String {
    let mut out = String::with_capacity(height * (width + 1));
    for r in 0..height {
        out.extend(std::iter::repeat_n(' ', width));
        if r + 1 < height {
            out.push('\n');
        }
    }
    out
}

// --- render_chart (V2): laid-out, LOD-aware chart -----------------------------

/// Candle rendering style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    /// Classic OHLC bar: vertical high-low line with left (open) / right (close)
    /// ticks. The cleaner default at terminal density.
    #[default]
    OhlcBars,
    /// Block candle: wick + filled body (`█`/`▒`), doji `─`.
    Candles,
}

impl Style {
    /// Columns one bar occupies, including its drawing footprint.
    fn cell_width(self) -> usize {
        match self {
            Style::OhlcBars => 3,
            Style::Candles => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Style::OhlcBars => "OHLC",
            Style::Candles => "Candles",
        }
    }
}

/// Options for [`render_chart`].
#[derive(Debug, Clone)]
pub struct ChartOptions {
    pub width: usize,
    pub height: usize,
    pub color: ColorMode,
    pub style: Style,
    /// Show a volume panel below the price plot.
    pub show_volume: bool,
    /// Status-line prefix label (e.g. `finchart · SYNTH`); may be empty.
    pub title: String,
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            width: 120,
            height: 30,
            color: ColorMode::Plain,
            style: Style::OhlcBars,
            show_volume: false,
            title: String::new(),
        }
    }
}

/// Render a laid-out candlestick chart: a status/title line, a left price gutter,
/// the OHLC/candle plot (display-resolution LOD via [`aggregate`]), and a bottom
/// time axis. Always returns exactly `height` lines, each `width` visible chars.
pub fn render_chart(view: &CulledView, opts: &ChartOptions) -> String {
    let (w, h) = (opts.width, opts.height);
    if w == 0 || h == 0 {
        return String::new();
    }
    // Need at least: title row + one plot row + time-axis row.
    if h < 3 {
        return blank(w, h);
    }

    let title_row = 0;
    let axis_row = h - 1;
    let plot_top = 1;

    // Rows between the title and time axis, split into price + optional volume.
    let usable = h - 2;
    // ~1/5 of the area, at least 3 rows, but always leave the price plot ≥1 row.
    let volume_rows = if opts.show_volume && usable >= 4 {
        (usable / 5).max(3).min(usable - 1)
    } else {
        0
    };
    let plot_rows = usable - volume_rows;
    let vol_top = plot_top + plot_rows;

    let (price_min, price_max) = view.price_extent().unwrap_or((0.0, 1.0));
    let lbl_max = price_label(price_max);
    let lbl_mid = price_label((price_min + price_max) / 2.0);
    let lbl_min = price_label(price_min);
    let gutter = gutter_for(price_min, price_max);

    let plot_cols = w.saturating_sub(gutter);
    if plot_cols == 0 {
        return blank(w, h);
    }

    let cell = opts.style.cell_width();
    let target_bars = (plot_cols / cell).max(1);
    let bars = aggregate(view, target_bars);

    let ps = PriceScale::new(price_min, price_max, plot_rows);
    let mut grid = Grid::new(w, h);

    // Plot: one bar per aggregated bucket, left-aligned at a fixed cell stride.
    for (i, bar) in bars.iter().enumerate() {
        let base = gutter + i * cell;
        match opts.style {
            Style::OhlcBars => draw_ohlc_bar(&mut grid, bar, base, plot_top, &ps),
            Style::Candles => draw_candle_bar(&mut grid, bar, base, plot_top, &ps),
        }
    }

    // Price gutter labels: max at top, min at bottom, mid in the middle.
    put_label(&mut grid, gutter, plot_top, &lbl_max);
    put_label(&mut grid, gutter, plot_top + plot_rows - 1, &lbl_min);
    if plot_rows >= 3 {
        put_label(&mut grid, gutter, plot_top + plot_rows / 2, &lbl_mid);
    }

    // Optional volume panel under the price plot.
    if volume_rows > 0 {
        draw_volume_panel(&mut grid, &bars, vol_top, volume_rows, gutter, cell);
    }

    // Status/title line and time axis.
    let status = compose_status(opts, view.len(), bars.len(), price_min, price_max);
    put_text(&mut grid, 0, title_row, &status);
    draw_time_axis(&mut grid, &bars, axis_row, gutter, w);

    grid_to_string(&grid, opts.color)
}

/// The number of bars [`render_chart`] will draw for `view` under `opts` (after
/// display-resolution LOD). Shares the gutter/cell math with the renderer so
/// callers can report LOD without guessing.
pub fn rendered_bar_count(view: &CulledView, opts: &ChartOptions) -> usize {
    if opts.width == 0 || opts.height < 3 {
        return 0;
    }
    let (price_min, price_max) = view.price_extent().unwrap_or((0.0, 1.0));
    let plot_cols = opts.width.saturating_sub(gutter_for(price_min, price_max));
    if plot_cols == 0 {
        return 0;
    }
    let target = (plot_cols / opts.style.cell_width()).max(1);
    view.len().min(target)
}

/// A single price-axis label.
fn price_label(v: f64) -> String {
    format!("{v:.PRICE_DECIMALS$}")
}

/// Width of the price gutter for the given extent: the widest of the max / mid /
/// min labels, plus one separator column.
fn gutter_for(price_min: f64, price_max: f64) -> usize {
    let mid = (price_min + price_max) / 2.0;
    price_label(price_max)
        .len()
        .max(price_label(mid).len())
        .max(price_label(price_min).len())
        + 1
}

fn dir_color(open: f64, close: f64) -> CellColor {
    if close > open {
        CellColor::Green
    } else if close < open {
        CellColor::Red
    } else {
        CellColor::Default
    }
}

fn draw_ohlc_bar(grid: &mut Grid, bar: &Bar, base_col: usize, plot_top: usize, ps: &PriceScale) {
    let color = dir_color(bar.open, bar.close);
    let high_row = plot_top + ps.price_to_row(bar.high).max(0) as usize;
    let low_row = plot_top + ps.price_to_row(bar.low).max(0) as usize;
    let line_col = base_col + 1;
    for r in high_row.min(low_row)..=high_row.max(low_row) {
        grid.set_styled(line_col, r, WICK, color);
    }
    let open_row = plot_top + ps.price_to_row(bar.open).max(0) as usize;
    let close_row = plot_top + ps.price_to_row(bar.close).max(0) as usize;
    grid.set_styled(base_col, open_row, OPEN_TICK, color);
    grid.set_styled(base_col + 2, close_row, CLOSE_TICK, color);
}

fn draw_candle_bar(grid: &mut Grid, bar: &Bar, base_col: usize, plot_top: usize, ps: &PriceScale) {
    let color = dir_color(bar.open, bar.close);
    let high_row = plot_top + ps.price_to_row(bar.high).max(0) as usize;
    let low_row = plot_top + ps.price_to_row(bar.low).max(0) as usize;
    for r in high_row.min(low_row)..=high_row.max(low_row) {
        grid.set_styled(base_col, r, WICK, color);
    }
    let open_row = plot_top + ps.price_to_row(bar.open).max(0) as usize;
    let close_row = plot_top + ps.price_to_row(bar.close).max(0) as usize;
    let body_top = open_row.min(close_row);
    let body_bottom = open_row.max(close_row);
    if body_top == body_bottom {
        grid.set_styled(base_col, body_top, DOJI, color);
    } else {
        let body = if bar.close >= bar.open { BULL } else { BEAR };
        for r in body_top..=body_bottom {
            grid.set_styled(base_col, r, body, color);
        }
    }
}

/// Draw a volume panel: one block bar per bucket, growing up from the panel
/// bottom with height proportional to `volume / max_volume`, colored by bar
/// direction. The max-volume value is labeled in the gutter at the top row.
fn draw_volume_panel(
    grid: &mut Grid,
    bars: &[Bar],
    vol_top: usize,
    vol_rows: usize,
    gutter: usize,
    cell: usize,
) {
    let max_vol = bars.iter().map(|b| b.volume).fold(0.0_f64, f64::max);
    if max_vol <= 0.0 {
        return;
    }
    let vol_bottom = vol_top + vol_rows - 1;
    let center = cell / 2; // align with the price bar's vertical line
    for (i, bar) in bars.iter().enumerate() {
        let col = gutter + i * cell + center;
        let mut height = (bar.volume / max_vol * vol_rows as f64).round() as usize;
        if bar.volume > 0.0 {
            height = height.max(1);
        }
        height = height.min(vol_rows);
        let color = dir_color(bar.open, bar.close);
        for r in 0..height {
            grid.set_styled(col, vol_bottom - r, BULL, color);
        }
    }
    put_label(grid, gutter, vol_top, &fmt_vol(max_vol));
}

/// Compact volume label, e.g. `8k`, `1.2k`, `500`.
fn fmt_vol(v: f64) -> String {
    if v >= 1_000.0 {
        format!("{:.0}k", v / 1_000.0)
    } else {
        format!("{v:.0}")
    }
}

/// Right-align `lbl` within the gutter (`gutter - 1` columns + separator).
fn put_label(grid: &mut Grid, gutter: usize, row: usize, lbl: &str) {
    let start = (gutter - 1).saturating_sub(lbl.chars().count());
    put_text(grid, start, row, lbl);
}

/// Write `text` (terminal-default color) into `grid` starting at `(col, row)`.
fn put_text(grid: &mut Grid, col: usize, row: usize, text: &str) {
    for (i, ch) in text.chars().enumerate() {
        grid.set_styled(col + i, row, ch, CellColor::Default);
    }
}

fn draw_time_axis(grid: &mut Grid, bars: &[Bar], row: usize, gutter: usize, width: usize) {
    let Some(first) = bars.first() else {
        return;
    };
    let left = format_utc(first.start_ts);
    put_text(grid, gutter, row, &left);

    let last = bars.last().expect("bars non-empty");
    let right = format_utc(last.start_ts);
    let rcol = width.saturating_sub(right.chars().count());
    put_text(grid, rcol, row, &right);

    // A centered middle tick, only if it clears both edges.
    if bars.len() >= 3 {
        let mid = format_utc(bars[bars.len() / 2].start_ts);
        let mcol = width / 2 - mid.chars().count() / 2;
        let clears_left = mcol > gutter + left.chars().count();
        let clears_right = mcol + mid.chars().count() < rcol;
        if clears_left && clears_right {
            put_text(grid, mcol, row, &mid);
        }
    }
}

fn compose_status(
    opts: &ChartOptions,
    visible: usize,
    bars: usize,
    price_min: f64,
    price_max: f64,
) -> String {
    let mut s = String::new();
    if !opts.title.is_empty() {
        s.push_str(&opts.title);
        s.push_str(" · ");
    }
    s.push_str(&format!(
        "{visible} candles → {bars} bars · {price_min:.2}-{price_max:.2} · {}",
        opts.style.label()
    ));
    s
}

/// Emit the whole grid as a string (color-aware), rows joined by newlines.
fn grid_to_string(grid: &Grid, color: ColorMode) -> String {
    let mut out = String::new();
    for r in 0..grid.rows() {
        push_plot_row(&mut out, grid, r, color);
        if r + 1 < grid.rows() {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_core::CandleSeries;
    use render_core::{cull, Viewport};

    fn bar(open: f64, high: f64, low: f64, close: f64) -> Bar {
        Bar {
            start_ts: 0,
            open,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    // --- bar cell-placement primitives ---------------------------------------

    #[test]
    fn candle_bar_up_draws_wick_and_filled_body() {
        let ps = PriceScale::new(0.0, 100.0, 5);
        let mut g = Grid::new(1, 5);
        // high=100->row0, close=75->row1, open=25->row3, low=0->row4.
        draw_candle_bar(&mut g, &bar(25.0, 100.0, 0.0, 75.0), 0, 0, &ps);
        assert_eq!(g.get(0, 0), WICK);
        assert_eq!(g.get(0, 1), BULL);
        assert_eq!(g.get(0, 2), BULL);
        assert_eq!(g.get(0, 3), BULL);
        assert_eq!(g.get(0, 4), WICK);
    }

    #[test]
    fn candle_bar_down_uses_bear_body() {
        let ps = PriceScale::new(0.0, 100.0, 5);
        let mut g = Grid::new(1, 5);
        draw_candle_bar(&mut g, &bar(75.0, 100.0, 0.0, 25.0), 0, 0, &ps);
        assert_eq!(g.get(0, 2), BEAR);
    }

    #[test]
    fn candle_bar_zero_height_body_is_doji() {
        let ps = PriceScale::new(0.0, 100.0, 5);
        let mut g = Grid::new(1, 5);
        draw_candle_bar(&mut g, &bar(50.0, 100.0, 0.0, 50.0), 0, 0, &ps);
        assert_eq!(g.get(0, 0), WICK);
        assert_eq!(g.get(0, 2), DOJI);
        assert_eq!(g.get(0, 4), WICK);
    }

    #[test]
    fn ohlc_bar_places_line_and_open_close_ticks() {
        let ps = PriceScale::new(0.0, 100.0, 5);
        let mut g = Grid::new(3, 5);
        // open=25->row3, close=75->row1; line at center col 1, ticks flank it.
        draw_ohlc_bar(&mut g, &bar(25.0, 100.0, 0.0, 75.0), 0, 0, &ps);
        assert_eq!(g.get(1, 0), WICK); // high-low line (center col)
        assert_eq!(g.get(1, 4), WICK);
        assert_eq!(g.get(0, 3), OPEN_TICK); // left tick at open row
        assert_eq!(g.get(2, 1), CLOSE_TICK); // right tick at close row
    }

    // --- render_chart (V2) ---------------------------------------------------

    fn gen(count: usize) -> CandleSeries {
        market_core::generate(&market_core::GenConfig {
            seed: 7,
            count,
            start_ts: 1_600_000_000,
            interval_secs: 60,
            start_price: 100.0,
            volatility: 0.02,
            drift: 0.0003,
        })
    }

    fn opts(width: usize, height: usize, style: Style) -> ChartOptions {
        ChartOptions {
            width,
            height,
            color: ColorMode::Plain,
            style,
            show_volume: false,
            title: "finchart · SYNTH".to_string(),
        }
    }

    #[test]
    fn render_chart_has_exact_dimensions_ohlc() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let out = render_chart(&view, &opts(120, 30, Style::OhlcBars));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 30);
        for line in lines {
            assert_eq!(line.chars().count(), 120);
        }
    }

    #[test]
    fn render_chart_has_exact_dimensions_candles() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let out = render_chart(&view, &opts(80, 20, Style::Candles));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 20);
        for line in lines {
            assert_eq!(line.chars().count(), 80);
        }
    }

    #[test]
    fn ohlc_style_uses_bar_ticks_not_blocks() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let out = render_chart(&view, &opts(120, 30, Style::OhlcBars));
        assert!(out.contains('╴'), "expected open ticks");
        assert!(out.contains('╶'), "expected close ticks");
        assert!(
            !out.contains('█') && !out.contains('▒'),
            "OHLC must not use body blocks"
        );
    }

    #[test]
    fn candle_style_uses_blocks_not_bar_ticks() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let out = render_chart(&view, &opts(120, 30, Style::Candles));
        assert!(
            out.contains('█') || out.contains('▒'),
            "expected candle bodies"
        );
        assert!(
            !out.contains('╴') && !out.contains('╶'),
            "candles must not use OHLC ticks"
        );
    }

    #[test]
    fn the_two_styles_differ() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let ohlc = render_chart(&view, &opts(120, 30, Style::OhlcBars));
        let candles = render_chart(&view, &opts(120, 30, Style::Candles));
        assert_ne!(ohlc, candles);
    }

    #[test]
    fn title_line_shows_status_with_lod_bar_count() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(0, 500)); // 500 candles, narrow width -> LOD
        let out = render_chart(&view, &opts(80, 20, Style::OhlcBars));
        let first = out.lines().next().unwrap();
        assert!(first.contains("finchart"), "title prefix missing");
        assert!(first.contains("500 candles"), "candle count missing");
        assert!(first.contains("bars"), "bar count missing");
    }

    #[test]
    fn time_axis_shows_formatted_timestamps() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let out = render_chart(&view, &opts(120, 30, Style::OhlcBars));
        let last = out.lines().last().unwrap();
        assert!(last.contains(':'), "expected HH:MM in time axis");
        assert!(last.contains('-'), "expected MM-DD in time axis");
    }

    #[test]
    fn render_chart_empty_view_keeps_dimensions() {
        let s = CandleSeries::new();
        let view = cull(&s, &Viewport::new(0, 0));
        let out = render_chart(&view, &opts(60, 16, Style::OhlcBars));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 16);
        for line in lines {
            assert_eq!(line.chars().count(), 60);
        }
    }

    #[test]
    fn render_chart_ansi_colors_bars() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.color = ColorMode::Ansi;
        let out = render_chart(&view, &o);
        assert!(
            out.contains("\u{1b}[32m") || out.contains("\u{1b}[31m"),
            "expected colors"
        );
    }

    #[test]
    fn render_chart_with_volume_has_exact_dimensions() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.show_volume = true;
        let out = render_chart(&view, &o);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 30);
        for line in lines {
            assert_eq!(line.chars().count(), 120);
        }
    }

    #[test]
    fn volume_panel_adds_block_bars_for_ohlc() {
        // OHLC price bars use ticks/lines (no blocks), so any '█' must be a
        // volume bar — a clean discriminator for the panel being present.
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);

        o.show_volume = false;
        assert!(
            !render_chart(&view, &o).contains('█'),
            "no blocks without volume"
        );

        o.show_volume = true;
        assert!(
            render_chart(&view, &o).contains('█'),
            "volume bars expected"
        );
    }

    #[test]
    fn volume_bars_are_colored_in_ansi() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.show_volume = true;
        o.color = ColorMode::Ansi;
        let out = render_chart(&view, &o);
        assert!(
            out.contains("\u{1b}[32m") || out.contains("\u{1b}[31m"),
            "expected colored volume bars"
        );
    }
}
