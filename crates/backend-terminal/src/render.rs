//! Candle rasterization and full-chart composition (plot grid + price gutter +
//! time axis). Plain (uncolored) output; ANSI color is layered on in a later step.

use crate::grid::{CellColor, Grid};
use crate::timefmt::{format_utc, format_utc_secs};
use market_core::{BookSide, LiquidityHeatmap};
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
const ANSI_NOTE: &str = "\u{1b}[33m";

/// Decimal places used for price-axis labels.
const PRICE_DECIMALS: usize = 2;

/// Width of the right-side depth panel strip.
const DEPTH_PANEL_COLS: usize = 18;

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
        CellColor::Note => ANSI_NOTE,
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

/// A render-ready Level 2 depth view: top bid/ask levels (best-first) and the
/// spread. Plain `(price, size)` tuples keep this decoupled from any order-book
/// type.
#[derive(Debug, Clone, Default)]
pub struct DepthPanel {
    /// Best-first (descending price).
    pub bids: Vec<(f64, f64)>,
    /// Best-first (ascending price).
    pub asks: Vec<(f64, f64)>,
    pub spread: Option<f64>,
}

/// One heatmap column: its recording time and `(price, liquidity, is_bid)` cells.
pub type HeatmapColumn = (i64, Vec<(f64, f64, bool)>);

/// A render-ready, side-aware liquidity heatmap: `columns` oldest→newest, each its
/// recording time plus a list of `(price, liquidity, is_bid)` cells. Each column is
/// drawn under the bar whose time span contains it. `max_liquidity` is kept for
/// reference; the renderer computes its own scale. Decoupled (plain tuples) from
/// the heatmap store; [`HeatmapView::from_heatmap`] converts one.
#[derive(Debug, Clone, Default)]
pub struct HeatmapView {
    pub columns: Vec<HeatmapColumn>,
    pub max_liquidity: f64,
}

impl HeatmapView {
    /// Export the columns of `heatmap` within the price band `[min, max]`.
    pub fn from_heatmap(heatmap: &LiquidityHeatmap, min: f64, max: f64) -> Self {
        let columns = heatmap
            .columns_in_range_sided(min, max)
            .into_iter()
            .map(|(ts, cells)| {
                let cells = cells
                    .into_iter()
                    .map(|(p, l, side)| (p, l, side == BookSide::Bid))
                    .collect();
                (ts, cells)
            })
            .collect();
        Self {
            columns,
            max_liquidity: heatmap.max_liquidity(),
        }
    }
}

/// One executed-trade marker, drawn at its price in the bar whose time span
/// contains `ts`; `buy` selects the glyph/color.
#[derive(Debug, Clone, Copy)]
pub struct TradeMarker {
    pub price: f64,
    pub ts: i64,
    pub buy: bool,
}

/// A render-ready set of executed-trade markers overlaid on the price plot.
#[derive(Debug, Clone, Default)]
pub struct TradeMarkers {
    pub markers: Vec<TradeMarker>,
}

/// A labelled moment in time, drawn into the price plot as a vertical rule at the
/// bar containing `ts` with `label` beside it. An annotation outside the visible
/// candles is skipped: before them it has scrolled away, after them it has not
/// happened yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// Epoch seconds, on the same clock as the candles.
    pub ts: i64,
    pub label: String,
}

/// A crosshair at a screen position: the bar under `col` is highlighted and read
/// out in the status line; `row`, when it falls in the price plot, adds a price
/// rule and the price (and heatmap liquidity) at that row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crosshair {
    pub col: usize,
    pub row: Option<usize>,
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
    /// Show an order-flow delta panel (buy−sell, pos/neg around a baseline).
    pub show_delta: bool,
    /// Optional Level 2 depth shown in a right-side panel.
    pub depth: Option<DepthPanel>,
    /// Optional liquidity heatmap drawn as a background layer in the price plot.
    pub heatmap: Option<HeatmapView>,
    /// Optional executed-trade markers overlaid above the heatmap, below candles.
    pub trades: Option<TradeMarkers>,
    /// Labelled moments drawn into the price plot; empty draws nothing.
    pub annotations: Vec<Annotation>,
    /// A crosshair; while it sits over a bar the status line reads that bar out
    /// instead of showing `title`.
    pub crosshair: Option<Crosshair>,
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
            show_delta: false,
            depth: None,
            heatmap: None,
            trades: None,
            annotations: Vec::new(),
            crosshair: None,
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
    let Some(g) = Geometry::new(view, opts) else {
        return blank(w, h);
    };
    let Geometry {
        depth_cols,
        chart_width,
        usable,
        plot_top,
        plot_rows,
        volume_rows,
        delta_rows,
        price_min,
        price_max,
        gutter,
        plot_cols,
        cell,
        ref bars,
    } = g;
    let title_row = 0;
    let axis_row = h - 1;
    let vol_top = plot_top + plot_rows;
    let delta_top = vol_top + volume_rows;

    let lbl_max = price_label(price_max);
    let lbl_mid = price_label((price_min + price_max) / 2.0);
    let lbl_min = price_label(price_min);

    let ps = g.scale();
    let mut grid = Grid::new(w, h);

    // Overlays are placed in time: each lands in the bar whose span contains it.
    let span = BarSpans::new(view, bars, gutter, cell, plot_cols);

    // Background layer: liquidity heatmap drawn before the bars so candles sit on top.
    let heat = opts
        .heatmap
        .as_ref()
        .and_then(|heatmap| draw_heatmap(&mut grid, heatmap, &span, plot_top, plot_rows, &ps));
    // Trade markers sit in a bar's right-hand column. A trade's price lies inside
    // its own bar, so with multi-column bars the markers go on top (that column only
    // holds the close tick); a one-column bar would be covered, so they go under it.
    let markers_on_top = cell > 1;
    if let (Some(trades), false) = (&opts.trades, markers_on_top) {
        draw_trade_markers(&mut grid, trades, &span, plot_top, plot_rows, &ps);
    }

    // Annotation rules go under the bars, so a bar keeps its own cells and the rule
    // shows above and below it; the labels go on top once the bars are drawn.
    let marks = annotation_columns(&span, &opts.annotations);
    for &(col, _) in &marks {
        for r in plot_top..plot_top + plot_rows {
            grid.set_styled(col, r, '┊', CellColor::Note);
        }
    }

    // The crosshair snaps to the bar under its column; its rules sit under the bars.
    let cross = opts.crosshair.and_then(|c| {
        let bar = g.bar_at_col(c.col)?;
        let row = c
            .row
            .filter(|r| (plot_top..plot_top + plot_rows).contains(r));
        Some((bar, row))
    });
    if let Some((bar, row)) = cross {
        let col = g.bar_center(bar);
        for r in plot_top..plot_top + plot_rows {
            grid.set_styled(col, r, '┆', CellColor::Note);
        }
        if let Some(r) = row {
            for c in gutter..gutter + plot_cols {
                grid.set_styled(c, r, '┄', CellColor::Note);
            }
        }
    }

    // Plot: one bar per aggregated bucket, left-aligned at a fixed cell stride.
    for (i, bar) in bars.iter().enumerate() {
        let base = gutter + i * cell;
        match opts.style {
            Style::OhlcBars => draw_ohlc_bar(&mut grid, bar, base, plot_top, &ps),
            Style::Candles => draw_candle_bar(&mut grid, bar, base, plot_top, &ps),
        }
    }

    if let (Some(trades), true) = (&opts.trades, markers_on_top) {
        draw_trade_markers(&mut grid, trades, &span, plot_top, plot_rows, &ps);
    }

    draw_annotation_labels(&mut grid, &marks, gutter, chart_width, plot_top, plot_rows);

    // Price gutter labels: max at top, min at bottom, mid in the middle.
    put_label(&mut grid, gutter, plot_top, &lbl_max);
    put_label(&mut grid, gutter, plot_top + plot_rows - 1, &lbl_min);
    if plot_rows >= 3 {
        put_label(&mut grid, gutter, plot_top + plot_rows / 2, &lbl_mid);
    }
    // The crosshair's price wins over the fixed labels on its row.
    if let Some((_, Some(r))) = cross {
        put_label(
            &mut grid,
            gutter,
            r,
            &price_label(ps.row_to_price(r - plot_top)),
        );
    }

    // Optional volume panel under the price plot.
    if volume_rows > 0 {
        draw_volume_panel(&mut grid, bars, vol_top, volume_rows, gutter, cell);
    }

    // Optional order-flow delta panel.
    if delta_rows > 0 {
        draw_delta_panel(
            &mut grid,
            bars,
            delta_top,
            delta_rows,
            gutter,
            cell,
            chart_width,
        );
    }

    // Optional right-side depth panel.
    if depth_cols > 0 {
        if let Some(depth) = &opts.depth {
            draw_depth_panel(&mut grid, depth, chart_width, depth_cols, plot_top, usable);
        }
    }

    // Status line (or the crosshair's readout) and time axis.
    let status = match cross {
        Some((bar, row)) => {
            let at = row.map(|r| {
                let liq = heat
                    .as_ref()
                    .map(|cells| cells[bar * plot_rows + (r - plot_top)]);
                (ps.row_to_price(r - plot_top), liq)
            });
            readout(&bars[bar], at)
        }
        None => compose_status(opts, view.len(), bars.len(), price_min, price_max),
    };
    put_text(&mut grid, 0, title_row, &status);
    draw_time_axis(&mut grid, bars, axis_row, gutter, chart_width);

    grid_to_string(&grid, opts.color)
}

/// The number of bars [`render_chart`] will draw for `view` under `opts` (after
/// display-resolution LOD). Shares the layout math with the renderer so callers
/// can report LOD without guessing.
pub fn rendered_bar_count(view: &CulledView, opts: &ChartOptions) -> usize {
    layout(view, opts).map_or(0, |l| l.bar_centers.len())
}

/// Where [`render_chart`] puts the price plot and each bar, in screen
/// coordinates, so an interactive caller can map a mouse or cursor position onto
/// the chart (or step a crosshair bar by bar). `None` when the frame is too small
/// to hold a plot.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub plot_left: usize,
    pub plot_cols: usize,
    pub plot_top: usize,
    pub plot_rows: usize,
    /// Columns per bar.
    pub cell: usize,
    /// The center column (the high-low line) of each drawn bar, left to right.
    pub bar_centers: Vec<usize>,
    scale: PriceScale,
}

impl Layout {
    /// The price at screen `row`, if it is inside the price plot.
    pub fn price_at(&self, row: usize) -> Option<f64> {
        (self.plot_top..self.plot_top + self.plot_rows)
            .contains(&row)
            .then(|| self.scale.row_to_price(row - self.plot_top))
    }

    /// The screen row `price` is drawn on (clamped to the price plot).
    pub fn row_of(&self, price: f64) -> usize {
        self.plot_top + self.scale.price_to_row(price) as usize
    }
}

/// The layout [`render_chart`] would use for `view` under `opts`.
pub fn layout(view: &CulledView, opts: &ChartOptions) -> Option<Layout> {
    let g = Geometry::new(view, opts)?;
    Some(Layout {
        plot_left: g.gutter,
        plot_cols: g.plot_cols,
        plot_top: g.plot_top,
        plot_rows: g.plot_rows,
        cell: g.cell,
        bar_centers: (0..g.bars.len()).map(|i| g.bar_center(i)).collect(),
        scale: g.scale(),
    })
}

/// One frame's geometry: the panel split, the price gutter, the plot area and the
/// aggregated bars. Computed once and shared by [`render_chart`] and [`layout`].
struct Geometry {
    depth_cols: usize,
    chart_width: usize,
    /// Rows between the status line and the time axis.
    usable: usize,
    plot_top: usize,
    plot_rows: usize,
    volume_rows: usize,
    delta_rows: usize,
    price_min: f64,
    price_max: f64,
    gutter: usize,
    plot_cols: usize,
    cell: usize,
    bars: Vec<Bar>,
}

impl Geometry {
    /// `None` when the frame cannot hold a plot (it renders blank).
    fn new(view: &CulledView, opts: &ChartOptions) -> Option<Self> {
        let (w, h) = (opts.width, opts.height);
        // Need at least: title row + one plot row + time-axis row.
        if w == 0 || h < 3 {
            return None;
        }
        // Reserve a fixed right strip for the optional depth panel; the chart (gutter
        // + plot + bottom panels + time axis) uses the remaining width.
        let depth_cols = if opts.depth.is_some() && w >= 60 {
            DEPTH_PANEL_COLS
        } else {
            0
        };
        let chart_width = w - depth_cols;

        // Rows between the title and time axis, split into price + optional panels.
        // Each panel takes ~1/5–1/6 of the area (≥3 rows) but the price plot keeps ≥1.
        let usable = h - 2;
        let volume_rows = if opts.show_volume && usable >= 4 {
            (usable / 5).max(3).min(usable - 1)
        } else {
            0
        };
        let delta_rows = if opts.show_delta {
            let remaining = usable - volume_rows;
            if remaining >= 4 {
                (usable / 6).max(3).min(remaining - 1)
            } else {
                0
            }
        } else {
            0
        };
        let plot_rows = usable - volume_rows - delta_rows;

        let (price_min, price_max) = view.price_extent().unwrap_or((0.0, 1.0));
        let gutter = gutter_for(price_min, price_max);
        let plot_cols = chart_width.saturating_sub(gutter);
        if plot_cols == 0 {
            return None;
        }
        let cell = opts.style.cell_width();
        let bars = aggregate(view, (plot_cols / cell).max(1));
        Some(Self {
            depth_cols,
            chart_width,
            usable,
            plot_top: 1,
            plot_rows,
            volume_rows,
            delta_rows,
            price_min,
            price_max,
            gutter,
            plot_cols,
            cell,
            bars,
        })
    }

    fn scale(&self) -> PriceScale {
        PriceScale::new(self.price_min, self.price_max, self.plot_rows)
    }

    /// The bar whose cell holds screen column `col`.
    fn bar_at_col(&self, col: usize) -> Option<usize> {
        let x = col.checked_sub(self.gutter)?;
        (x < self.plot_cols && x / self.cell < self.bars.len()).then_some(x / self.cell)
    }

    /// The center column (the high-low line) of bar `i`.
    fn bar_center(&self, i: usize) -> usize {
        self.gutter + i * self.cell + self.cell / 2
    }
}

/// The crosshair's status line: the bar's time, OHLC, volume and delta, then
/// (when the cursor is on the price plot) the price there and, with a heatmap, the
/// average resting bid / ask liquidity in that cell.
fn readout(bar: &Bar, at: Option<(f64, Option<[f64; 2]>)>) -> String {
    let delta = bar.delta();
    let sign = if delta < 0.0 { "-" } else { "+" };
    let mut s = format!(
        "⌖ {} · O {} H {} L {} C {} · V {} · Δ {sign}{}",
        format_utc_secs(bar.start_ts),
        price_label(bar.open),
        price_label(bar.high),
        price_label(bar.low),
        price_label(bar.close),
        fmt_qty(bar.volume),
        fmt_qty(delta.abs()),
    );
    if let Some((price, liq)) = at {
        s.push_str(&format!(" · @ {}", price_label(price)));
        if let Some([bid, ask]) = liq {
            s.push_str(&format!(
                " · liq bid {} / ask {}",
                fmt_qty(bid),
                fmt_qty(ask)
            ));
        }
    }
    s
}

/// A quantity for the readout: whole units below 1,000, else thousands to one
/// decimal (`10.7k`).
fn fmt_qty(v: f64) -> String {
    if v >= 1_000.0 {
        format!("{:.1}k", v / 1_000.0)
    } else {
        format!("{v:.0}")
    }
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

/// Draw an order-flow delta panel: per bar, `delta = buy - sell` as a bar growing
/// up (green) for positive or down (red) for negative from a center baseline.
/// Heights scale to the largest |delta| in view. A `─` baseline marks zero, with
/// the max-|delta| value labeled at the top and `0` at the baseline.
fn draw_delta_panel(
    grid: &mut Grid,
    bars: &[Bar],
    delta_top: usize,
    delta_rows: usize,
    gutter: usize,
    cell: usize,
    width: usize,
) {
    let baseline = delta_top + delta_rows / 2;
    // Zero line across the plot area.
    for col in gutter..width {
        grid.set_styled(col, baseline, DOJI, CellColor::Default);
    }

    let max_abs = bars.iter().map(|b| b.delta().abs()).fold(0.0_f64, f64::max);
    let up_rows = baseline - delta_top;
    let down_rows = (delta_top + delta_rows - 1) - baseline;
    let center = cell / 2;

    if max_abs > 0.0 {
        for (i, bar) in bars.iter().enumerate() {
            let col = gutter + i * cell + center;
            let d = bar.delta();
            let frac = d.abs() / max_abs;
            if d > 0.0 && up_rows > 0 {
                let height = ((frac * up_rows as f64).round() as usize).clamp(1, up_rows);
                for r in 0..height {
                    grid.set_styled(col, baseline - 1 - r, BULL, CellColor::Green);
                }
            } else if d < 0.0 && down_rows > 0 {
                let height = ((frac * down_rows as f64).round() as usize).clamp(1, down_rows);
                for r in 0..height {
                    grid.set_styled(col, baseline + 1 + r, BULL, CellColor::Red);
                }
            }
        }
    }

    put_label(grid, gutter, delta_top, &fmt_vol(max_abs));
    put_label(grid, gutter, baseline, "0");
}

/// Coarse liquidity intensity ramp (low → high). Capped at `▓` (no full block) so
/// the background never overpowers the candles.
const HEAT_RAMP: [char; 4] = ['·', '░', '▒', '▓'];

/// The glyph for a liquidity fraction in `[0, 1]`, or `None` for empty.
fn heat_glyph(frac: f64) -> Option<char> {
    if frac <= 0.0 {
        return None;
    }
    let idx = ((frac * HEAT_RAMP.len() as f64).ceil() as usize).clamp(1, HEAT_RAMP.len());
    Some(HEAT_RAMP[idx - 1])
}

/// Where each bar sits in time and on screen. Bar `i` spans
/// `[bars[i].start_ts, bars[i + 1].start_ts)`; the last bar ends one candle
/// interval after the last visible candle (inferred from the candle spacing, so a
/// one-candle view is open-ended). Overlays outside every span are not drawn:
/// before the view they have scrolled away, after it they have not happened yet.
struct BarSpans<'a> {
    starts: Vec<i64>,
    end: Option<i64>,
    bars: &'a [Bar],
    gutter: usize,
    cell: usize,
    plot_cols: usize,
}

impl<'a> BarSpans<'a> {
    fn new(
        view: &CulledView,
        bars: &'a [Bar],
        gutter: usize,
        cell: usize,
        plot_cols: usize,
    ) -> Self {
        let n = view.ts.len();
        let end = (n >= 2).then(|| view.ts[n - 1] + (view.ts[n - 1] - view.ts[n - 2]));
        Self {
            starts: bars.iter().map(|b| b.start_ts).collect(),
            end,
            bars,
            gutter,
            cell,
            plot_cols,
        }
    }

    /// Index of the bar whose span contains `ts`.
    fn bar_at(&self, ts: i64) -> Option<usize> {
        let first = *self.starts.first()?;
        if ts < first || self.end.is_some_and(|end| ts >= end) {
            return None;
        }
        Some(self.starts.partition_point(|&s| s <= ts) - 1)
    }

    /// Screen column of offset `k` within bar `i`'s cell, if it fits the plot.
    fn col(&self, i: usize, k: usize) -> Option<usize> {
        let x = i * self.cell + k;
        (x < self.plot_cols).then_some(self.gutter + x)
    }
}

/// Draw the side-aware liquidity heatmap as a background layer. Each column goes
/// to the bar whose time span contains it, and fills that bar's whole cell; when
/// several columns share a bar, their liquidity is averaged per price row.
/// Intensity is scaled by the **max rendered row total** (so cells span the ramp
/// rather than saturate), and the dominant side colors it (green = bid, red = ask).
/// Returns the averaged `[bid, ask]` liquidity per `(bar, row)` for the crosshair.
fn draw_heatmap(
    grid: &mut Grid,
    heatmap: &HeatmapView,
    span: &BarSpans,
    plot_top: usize,
    plot_rows: usize,
    ps: &PriceScale,
) -> Option<Vec<[f64; 2]>> {
    let nbars = span.bars.len();
    if plot_rows == 0 || nbars == 0 {
        return None;
    }

    // Pass 1: accumulate per (bar, row) bid/ask liquidity and columns per bar.
    let mut cells = vec![[0.0f64; 2]; nbars * plot_rows]; // [bid, ask]
    let mut columns = vec![0usize; nbars];
    for (ts, levels) in &heatmap.columns {
        let Some(i) = span.bar_at(*ts) else {
            continue;
        };
        columns[i] += 1;
        for &(price, liq, is_bid) in levels {
            let r = ps.price_to_row(price);
            if r >= 0 && (r as usize) < plot_rows {
                cells[i * plot_rows + r as usize][usize::from(!is_bid)] += liq;
            }
        }
    }

    // Average over each bar's columns, tracking the largest rendered total.
    let mut max_total = 0.0_f64;
    for (i, &n) in columns.iter().enumerate() {
        if n == 0 {
            continue;
        }
        for cell in &mut cells[i * plot_rows..(i + 1) * plot_rows] {
            cell[0] /= n as f64;
            cell[1] /= n as f64;
            max_total = max_total.max(cell[0] + cell[1]);
        }
    }
    if max_total <= 0.0 {
        return Some(cells);
    }

    // Pass 2: render glyph by intensity, color by dominant side, across the cell.
    for i in 0..nbars {
        for r in 0..plot_rows {
            let [bid, ask] = cells[i * plot_rows + r];
            let Some(glyph) = heat_glyph((bid + ask) / max_total) else {
                continue;
            };
            let color = if bid >= ask {
                CellColor::Green
            } else {
                CellColor::Red
            };
            for k in 0..span.cell {
                if let Some(col) = span.col(i, k) {
                    grid.set_styled(col, plot_top + r, glyph, color);
                }
            }
        }
    }
    Some(cells)
}

/// Draw executed-trade markers: `▲` buy (green) / `▼` sell (red), at the trade's
/// price row in the right-hand column of the bar containing it (the OHLC close-tick
/// column, clear of the bar's high-low line).
fn draw_trade_markers(
    grid: &mut Grid,
    trades: &TradeMarkers,
    span: &BarSpans,
    plot_top: usize,
    plot_rows: usize,
    ps: &PriceScale,
) {
    for m in &trades.markers {
        let Some(col) = span.bar_at(m.ts).and_then(|i| span.col(i, span.cell - 1)) else {
            continue;
        };
        let r = ps.price_to_row(m.price);
        if r >= 0 && (r as usize) < plot_rows {
            let (glyph, color) = if m.buy {
                ('▲', CellColor::Green)
            } else {
                ('▼', CellColor::Red)
            };
            grid.set_styled(col, plot_top + r as usize, glyph, color);
        }
    }
}

/// Draw a compact Level 2 depth panel in the right strip `[left, left+cols)`:
/// top-N asks (worst-on-top, red) → spread row → top-N bids (best-on-top, green),
/// each as `price size` with a mini size bar. Levels are clamped to the rows
/// available in the `usable`-row band starting at `top`.
fn draw_depth_panel(
    grid: &mut Grid,
    depth: &DepthPanel,
    left: usize,
    cols: usize,
    top: usize,
    usable: usize,
) {
    // Rows: ASK header + spread + BID header are fixed; the rest splits per side.
    let per_side = usable.saturating_sub(3) / 2;
    if per_side == 0 {
        return;
    }
    let n = per_side.min(depth.asks.len().max(depth.bids.len()));

    let max_size = depth
        .bids
        .iter()
        .chain(&depth.asks)
        .map(|&(_, s)| s)
        .fold(0.0_f64, f64::max);

    let mut row = top;
    let put = |grid: &mut Grid, text: &str, color: CellColor, row: usize| {
        for (i, ch) in text.chars().take(cols).enumerate() {
            grid.set_styled(left + i, row, ch, color);
        }
    };

    put(grid, "ASK", CellColor::Default, row);
    row += 1;
    // Asks worst-first so the best ask sits just above the spread line.
    for &(price, size) in depth.asks.iter().take(n).rev() {
        put(
            grid,
            &fmt_depth_level(price, size, max_size, cols),
            CellColor::Red,
            row,
        );
        row += 1;
    }

    let spread = depth
        .spread
        .map(|s| format!("spr {s:.2}"))
        .unwrap_or_else(|| "spr  —".to_string());
    put(grid, &spread, CellColor::Default, row);
    row += 1;

    // Bids best-first, just below the spread line.
    for &(price, size) in depth.bids.iter().take(n) {
        put(
            grid,
            &fmt_depth_level(price, size, max_size, cols),
            CellColor::Green,
            row,
        );
        row += 1;
    }
    if row < top + usable {
        put(grid, "BID", CellColor::Default, row);
    }
}

/// Format one depth level as `price size bar` within `cols` columns.
fn fmt_depth_level(price: f64, size: f64, max_size: f64, cols: usize) -> String {
    let head = format!("{price:>8.2} {size:>4.0}");
    let bar_room = cols.saturating_sub(head.chars().count() + 1);
    let bars = if max_size > 0.0 {
        ((size / max_size) * bar_room as f64).round() as usize
    } else {
        0
    };
    format!("{head} {}", "█".repeat(bars.min(bar_room)))
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

/// Resolve each visible annotation to the plot column of the bar containing it:
/// the middle of that bar's cell, which for OHLC bars is its high-low line.
fn annotation_columns<'a>(span: &BarSpans, annotations: &'a [Annotation]) -> Vec<(usize, &'a str)> {
    annotations
        .iter()
        .filter_map(|a| {
            let col = span.bar_at(a.ts).and_then(|i| span.col(i, span.cell / 2))?;
            Some((col, a.label.as_str()))
        })
        .collect()
}

/// Place each label beside its rule, flipping to the left side near the right
/// edge, and dropping to a lower row when it would collide with an earlier label.
fn draw_annotation_labels(
    grid: &mut Grid,
    marks: &[(usize, &str)],
    gutter: usize,
    chart_width: usize,
    plot_top: usize,
    plot_rows: usize,
) {
    let rows = plot_rows.min(3);
    let mut taken: Vec<(usize, usize, usize)> = Vec::new(); // (row, start, end)
    for &(col, label) in marks {
        let len = label.chars().count();
        let start = if col + 1 + len <= chart_width {
            col + 1
        } else {
            col.saturating_sub(len).max(gutter)
        };
        let end = (start + len).min(chart_width);
        let row = (0..rows)
            .map(|k| plot_top + k)
            .find(|&r| {
                !taken
                    .iter()
                    .any(|&(tr, s, e)| tr == r && start < e && s < end)
            })
            .unwrap_or(plot_top + rows.saturating_sub(1));
        taken.push((row, start, end));
        for (i, ch) in label.chars().take(end - start).enumerate() {
            grid.set_styled(start + i, row, ch, CellColor::Note);
        }
    }
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
    if opts.heatmap.is_some() {
        s.push_str(" · heatmap bid/ask");
    }
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
            buy_volume: 0.0,
            sell_volume: 0.0,
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
            show_delta: false,
            depth: None,
            heatmap: None,
            trades: None,
            annotations: Vec::new(),
            crosshair: None,
            title: "finchart · SYNTH".to_string(),
        }
    }

    // --- annotations -----------------------------------------------------------

    /// Epoch seconds of candle `i` in `gen`'s series.
    fn ts_of(i: i64) -> i64 {
        1_600_000_000 + i * 60
    }

    fn annotated(annotations: Vec<Annotation>, color: ColorMode) -> String {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.color = color;
        o.annotations = annotations;
        render_chart(&view, &o)
    }

    fn note(i: i64, label: &str) -> Annotation {
        Annotation {
            ts: ts_of(i),
            label: label.to_string(),
        }
    }

    #[test]
    fn annotation_inside_the_view_draws_its_label_and_a_rule() {
        let out = annotated(vec![note(450, "SWEEP")], ColorMode::Plain);
        assert!(out.contains("SWEEP"), "label missing:\n{out}");
        assert!(out.contains('┊'), "rule missing:\n{out}");
    }

    #[test]
    fn annotation_outside_the_view_is_not_drawn() {
        // The view holds candles 380..500. Before it is scrolled away; after it has
        // not happened yet, so there is nothing to mark.
        let out = annotated(
            vec![note(100, "EARLIER"), note(650, "LATER")],
            ColorMode::Plain,
        );
        assert!(!out.contains("EARLIER"));
        assert!(!out.contains("LATER"));
        assert!(!out.contains('┊'));
    }

    #[test]
    fn annotation_near_the_right_edge_stays_inside_the_frame() {
        let out = annotated(vec![note(499, "A LONG LABEL")], ColorMode::Plain);
        assert!(out.contains("A LONG LABEL"));
        for line in out.lines() {
            assert_eq!(line.chars().count(), 120);
        }
    }

    #[test]
    fn annotations_are_colored_as_notes() {
        let out = annotated(vec![note(450, "SWEEP")], ColorMode::Ansi);
        assert!(out.contains("\u{1b}[33m"), "expected the note color");
    }

    #[test]
    fn no_annotations_leaves_the_frame_unchanged() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let plain = render_chart(&view, &opts(120, 30, Style::OhlcBars));
        assert_eq!(plain, annotated(Vec::new(), ColorMode::Plain));
        assert!(!plain.contains('┊'));
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

    #[test]
    fn delta_panel_keeps_dimensions() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.show_volume = true;
        o.show_delta = true;
        let out = render_chart(&view, &o);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 30);
        for line in lines {
            assert_eq!(line.chars().count(), 120);
        }
    }

    #[test]
    fn delta_panel_adds_bars_around_a_baseline() {
        // OHLC price bars have no blocks; with volume off, any '█' is a delta bar.
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.show_volume = false;

        o.show_delta = false;
        assert!(
            !render_chart(&view, &o).contains('█'),
            "no blocks without delta"
        );

        o.show_delta = true;
        let out = render_chart(&view, &o);
        assert!(out.contains('█'), "delta bars expected");
        assert!(out.contains('─'), "delta baseline expected");
    }

    #[test]
    fn delta_bars_are_colored_in_ansi() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.show_volume = false;
        o.show_delta = true;
        o.color = ColorMode::Ansi;
        let out = render_chart(&view, &o);
        assert!(
            out.contains("\u{1b}[32m") || out.contains("\u{1b}[31m"),
            "expected colored delta bars"
        );
    }

    fn depth_panel() -> DepthPanel {
        DepthPanel {
            bids: vec![(99.9, 5.0), (99.8, 3.0), (99.7, 2.0)], // best-first (desc)
            asks: vec![(100.1, 4.0), (100.2, 6.0), (100.3, 1.0)], // best-first (asc)
            spread: Some(0.2),
        }
    }

    #[test]
    fn depth_panel_keeps_dimensions() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.depth = Some(depth_panel());
        let out = render_chart(&view, &o);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 30);
        for line in lines {
            assert_eq!(line.chars().count(), 120);
        }
    }

    #[test]
    fn depth_panel_shows_labels_and_levels() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.depth = Some(depth_panel());
        let out = render_chart(&view, &o);
        assert!(out.contains("ASK"), "missing ASK header");
        assert!(out.contains("BID"), "missing BID header");
        assert!(out.contains("spr"), "missing spread row");
        assert!(out.contains("100.10"), "missing best ask price");
        assert!(out.contains("99.90"), "missing best bid price");
    }

    #[test]
    fn depth_panel_colors_sides_in_ansi() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.depth = Some(depth_panel());
        o.color = ColorMode::Ansi;
        let out = render_chart(&view, &o);
        assert!(out.contains("\u{1b}[32m"), "expected green bids");
        assert!(out.contains("\u{1b}[31m"), "expected red asks");
    }

    /// A synthetic side-aware heatmap spanning the view's price range: bids in the
    /// lower half, asks in the upper half, with a liquidity gradient.
    fn heatmap_for(view: &render_core::CulledView) -> HeatmapView {
        let (lo, hi) = view.price_extent().unwrap();
        let rows = 30usize;
        let split = (lo + hi) / 2.0;
        let columns = view
            .ts
            .iter()
            .map(|&ts| {
                let cells = (0..rows)
                    .map(|k| {
                        let price = lo + (hi - lo) * k as f64 / (rows as f64 - 1.0);
                        (price, k as f64 + 1.0, price < split)
                    })
                    .collect();
                (ts, cells)
            })
            .collect();
        HeatmapView {
            columns,
            max_liquidity: rows as f64,
        }
    }

    #[test]
    fn heatmap_uses_no_full_block_glyph() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.heatmap = Some(heatmap_for(&view));
        // Volume/delta are off and OHLC bars don't use blocks, so any `█` would
        // be a heatmap cell — the ramp must not produce one.
        assert!(
            !render_chart(&view, &o).contains('█'),
            "heatmap used a full block"
        );
    }

    #[test]
    fn heatmap_colors_bid_and_ask_sides() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.heatmap = Some(heatmap_for(&view));
        o.color = ColorMode::Ansi;
        let out = render_chart(&view, &o);
        assert!(out.contains("\u{1b}[32m"), "expected green bid cells");
        assert!(out.contains("\u{1b}[31m"), "expected red ask cells");
    }

    #[test]
    fn status_line_has_heatmap_legend() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.heatmap = Some(heatmap_for(&view));
        let first = render_chart(&view, &o).lines().next().unwrap().to_string();
        assert!(first.contains("heatmap"), "missing heatmap legend: {first}");
    }

    #[test]
    fn trade_markers_render_at_price_and_recency() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let (lo, hi) = view.price_extent().unwrap();
        let mut o = opts(120, 30, Style::OhlcBars);
        o.trades = Some(TradeMarkers {
            markers: vec![
                TradeMarker {
                    price: (lo + hi) / 2.0,
                    ts: view.ts[view.len() - 1],
                    buy: true,
                },
                TradeMarker {
                    price: (lo + hi) / 2.0,
                    ts: view.ts[view.len() - 6],
                    buy: false,
                },
            ],
        });
        let out = render_chart(&view, &o);
        assert!(out.contains('▲'), "expected buy marker");
        assert!(out.contains('▼'), "expected sell marker");
        // Dimensions unchanged with markers.
        assert_eq!(out.lines().count(), 30);
        for line in out.lines() {
            assert_eq!(line.chars().count(), 120);
        }
    }

    #[test]
    fn heatmap_keeps_dimensions() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.heatmap = Some(heatmap_for(&view));
        let out = render_chart(&view, &o);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 30);
        for line in lines {
            assert_eq!(line.chars().count(), 120);
        }
    }

    #[test]
    fn heatmap_draws_intensity_glyphs() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.heatmap = Some(heatmap_for(&view));
        let out = render_chart(&view, &o);
        assert!(
            out.contains('░') || out.contains('▒') || out.contains('▓'),
            "expected heatmap intensity glyphs"
        );
    }

    #[test]
    fn candles_render_over_heatmap() {
        let s = gen(500);
        let view = cull(&s, &Viewport::new(380, 500));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.heatmap = Some(heatmap_for(&view));
        let out = render_chart(&view, &o);
        // Both layers present: heatmap background glyph and candle wick on top.
        assert!(out.contains('▓'), "expected heatmap background");
        assert!(out.contains('│'), "expected candle bars over the heatmap");
    }

    // --- time alignment: overlays sit under the bar containing their time ------

    /// Four 60 s candles at ts 0, 60, 120, 180 sitting near 99, plus a tall last
    /// candle so prices up to 102 are on the scale. Rows around 100.5 stay clear of
    /// bars 0..=2, so overlay cells there are not overwritten.
    fn quad() -> CandleSeries {
        let mut s = CandleSeries::new();
        for (i, high) in [99.2, 99.2, 99.2, 102.0].into_iter().enumerate() {
            s.push(market_core::Candle {
                ts: i as i64 * 60,
                open: 99.0,
                high,
                low: 99.0,
                close: 99.0,
                volume: 1.0,
            });
        }
        s
    }

    fn quad_chart(heat: Vec<HeatmapColumn>, trades: Vec<TradeMarker>) -> String {
        let s = quad();
        let view = cull(&s, &Viewport::full(s.len()));
        let mut o = opts(60, 14, Style::OhlcBars);
        o.heatmap = Some(HeatmapView {
            columns: heat,
            max_liquidity: 10.0,
        });
        o.trades = Some(TradeMarkers { markers: trades });
        render_chart(&view, &o)
    }

    /// `(row, col, glyph)` of every heatmap glyph inside the price plot.
    fn heat_cells(out: &str) -> Vec<(usize, usize, char)> {
        let lines: Vec<&str> = out.lines().collect();
        let mut cells = Vec::new();
        for (r, line) in lines.iter().enumerate().take(lines.len() - 1).skip(1) {
            for (c, ch) in line.chars().enumerate() {
                if HEAT_RAMP.contains(&ch) {
                    cells.push((r, c, ch));
                }
            }
        }
        cells
    }

    fn cols_of(cells: &[(usize, usize, char)]) -> Vec<usize> {
        let mut cols: Vec<usize> = cells.iter().map(|&(_, c, _)| c).collect();
        cols.sort_unstable();
        cols.dedup();
        cols
    }

    fn heat_at(ts: i64, liq: f64) -> HeatmapColumn {
        (ts, vec![(100.5, liq, true)])
    }

    #[test]
    fn a_heatmap_column_fills_the_bar_that_contains_its_time() {
        let in_bar_1 = cols_of(&heat_cells(&quad_chart(vec![heat_at(70, 10.0)], vec![])));
        let in_bar_2 = cols_of(&heat_cells(&quad_chart(vec![heat_at(130, 10.0)], vec![])));
        let cell = Style::OhlcBars.cell_width();
        assert_eq!(in_bar_1.len(), cell, "should span the whole bar cell");
        let shifted: Vec<usize> = in_bar_1.iter().map(|c| c + cell).collect();
        assert_eq!(
            in_bar_2, shifted,
            "one bar later in time = one cell to the right"
        );
    }

    #[test]
    fn heatmap_columns_outside_the_visible_candles_are_dropped() {
        // Before the first candle, and after the last candle's interval (180 + 60).
        let out = quad_chart(vec![heat_at(-10, 10.0), heat_at(240, 10.0)], vec![]);
        assert!(heat_cells(&out).is_empty(), "unexpected heat:\n{out}");
        // Inside the last candle's interval, after its open, still counts.
        let out = quad_chart(vec![heat_at(200, 10.0)], vec![]);
        assert!(!heat_cells(&out).is_empty(), "missing heat:\n{out}");
    }

    #[test]
    fn heatmap_columns_sharing_a_bar_are_averaged() {
        // Bar 1 holds 10 then nothing (average 5); bar 2 holds 10.
        let out = quad_chart(
            vec![
                (60, vec![(100.5, 10.0, true)]),
                (90, vec![]),
                heat_at(120, 10.0),
            ],
            vec![],
        );
        let cells = heat_cells(&out);
        let cell = Style::OhlcBars.cell_width();
        let cols = cols_of(&cells);
        assert_eq!(cols.len(), 2 * cell);
        let glyph_at = |col: usize| cells.iter().find(|&&(_, c, _)| c == col).unwrap().2;
        assert_eq!(glyph_at(cols[0]), '░', "half intensity in bar 1");
        assert_eq!(glyph_at(cols[cell]), '▓', "full intensity in bar 2");
    }

    #[test]
    fn trade_markers_sit_in_the_bar_that_contains_their_time() {
        let marker = |ts, buy| TradeMarker {
            price: 100.5,
            ts,
            buy,
        };
        let out = quad_chart(vec![], vec![marker(70, true), marker(130, false)]);
        let col_of = |glyph: char| {
            out.lines()
                .find_map(|l| l.chars().position(|c| c == glyph))
                .unwrap_or_else(|| panic!("missing {glyph}:\n{out}"))
        };
        assert_eq!(col_of('▼') - col_of('▲'), Style::OhlcBars.cell_width());
        // A trade outside the visible candles is not drawn.
        let out = quad_chart(vec![], vec![marker(500, true)]);
        assert!(!out.contains('▲'));
    }

    #[test]
    fn a_trade_at_the_bars_close_still_shows_its_marker() {
        // Bar 1 closes at 99.0; a trade there must not vanish under the close tick.
        let out = quad_chart(
            vec![],
            vec![TradeMarker {
                price: 99.0,
                ts: 70,
                buy: true,
            }],
        );
        assert!(out.contains('▲'), "marker hidden by the bar:\n{out}");
        assert!(out.contains('│'), "the high-low line must still be drawn");
    }

    #[test]
    fn annotation_inside_the_last_candles_interval_is_drawn() {
        let s = quad();
        let view = cull(&s, &Viewport::full(s.len()));
        let mut o = opts(60, 14, Style::OhlcBars);
        o.annotations = vec![Annotation {
            ts: 200,
            label: "NOW".to_string(),
        }];
        assert!(render_chart(&view, &o).contains("NOW"));
    }

    // --- crosshair & layout -----------------------------------------------------

    fn quad_opts() -> ChartOptions {
        let mut o = opts(120, 14, Style::OhlcBars);
        o.show_volume = true;
        o
    }

    fn quad_view(s: &CandleSeries) -> render_core::CulledView<'_> {
        cull(s, &Viewport::full(s.len()))
    }

    fn col_has(out: &str, col: usize, glyph: char) -> bool {
        out.lines().any(|l| l.chars().nth(col) == Some(glyph))
    }

    #[test]
    fn layout_puts_each_bar_center_on_its_high_low_line() {
        let s = quad();
        let o = quad_opts();
        let out = render_chart(&quad_view(&s), &o);
        let layout = layout(&quad_view(&s), &o).unwrap();
        assert_eq!(layout.bar_centers.len(), 4);
        for &col in &layout.bar_centers {
            assert!(
                col_has(&out, col, '│'),
                "no bar line at column {col}:\n{out}"
            );
        }
        assert_eq!(rendered_bar_count(&quad_view(&s), &o), 4);
    }

    #[test]
    fn layout_and_bar_count_leave_room_for_the_depth_panel() {
        let s = gen(500);
        let view = cull(&s, &Viewport::full(s.len()));
        let mut o = opts(120, 30, Style::OhlcBars);
        o.depth = Some(depth_panel());
        let l = layout(&view, &o).unwrap();
        assert!(l.plot_left + l.plot_cols <= 120 - DEPTH_PANEL_COLS);
        assert_eq!(rendered_bar_count(&view, &o), l.bar_centers.len());
        assert_eq!(
            l.bar_centers.len(),
            l.plot_cols / Style::OhlcBars.cell_width()
        );
    }

    #[test]
    fn crosshair_draws_rules_and_reads_out_the_bar_under_it() {
        let s = quad();
        let mut o = quad_opts();
        let l = layout(&quad_view(&s), &o).unwrap();
        let row = l.row_of(100.5);
        o.crosshair = Some(Crosshair {
            col: l.bar_centers[1],
            row: Some(row),
        });
        let out = render_chart(&quad_view(&s), &o);
        let status = out.lines().next().unwrap();
        assert!(status.starts_with("⌖ 01-01 00:01:00"), "{status}");
        assert!(
            status.contains("O 99.00 H 99.20 L 99.00 C 99.00"),
            "{status}"
        );
        assert!(
            status.contains(&format!("@ {:.2}", l.price_at(row).unwrap())),
            "{status}"
        );
        // A vertical rule above the bar and a horizontal rule across the plot.
        assert!(
            col_has(&out, l.bar_centers[1], '┆'),
            "no vertical rule:\n{out}"
        );
        let line: Vec<char> = out.lines().nth(row).unwrap().chars().collect();
        assert!(line.contains(&'┄'), "no horizontal rule:\n{out}");
        assert_eq!(out.lines().count(), 14);
        for line in out.lines() {
            assert_eq!(line.chars().count(), 120);
        }
    }

    #[test]
    fn crosshair_reads_the_heatmap_liquidity_under_the_cursor() {
        let s = quad();
        let mut o = quad_opts();
        let l = layout(&quad_view(&s), &o).unwrap();
        o.heatmap = Some(HeatmapView {
            columns: vec![(70, vec![(100.5, 1500.0, true)])],
            max_liquidity: 1500.0,
        });
        o.crosshair = Some(Crosshair {
            col: l.bar_centers[1],
            row: Some(l.row_of(100.5)),
        });
        let out = render_chart(&quad_view(&s), &o);
        let status = out.lines().next().unwrap();
        assert!(status.contains("liq bid 1.5k / ask 0"), "{status}");
    }

    #[test]
    fn crosshair_outside_the_bars_changes_nothing() {
        let s = quad();
        let plain = render_chart(&quad_view(&s), &quad_opts());
        let l = layout(&quad_view(&s), &quad_opts()).unwrap();
        for col in [0, l.plot_left - 1, *l.bar_centers.last().unwrap() + 10] {
            let mut o = quad_opts();
            o.crosshair = Some(Crosshair { col, row: Some(3) });
            assert_eq!(render_chart(&quad_view(&s), &o), plain, "col {col}");
        }
    }

    #[test]
    fn crosshair_row_outside_the_price_plot_draws_only_the_time_rule() {
        let s = quad();
        let mut o = quad_opts();
        let l = layout(&quad_view(&s), &o).unwrap();
        o.crosshair = Some(Crosshair {
            col: l.bar_centers[2],
            row: Some(l.plot_top + l.plot_rows + 1), // inside the volume panel
        });
        let out = render_chart(&quad_view(&s), &o);
        assert!(!out.contains('┄'), "unexpected horizontal rule:\n{out}");
        assert!(col_has(&out, l.bar_centers[2], '┆'));
        assert!(!out.lines().next().unwrap().contains('@'));
    }
}
