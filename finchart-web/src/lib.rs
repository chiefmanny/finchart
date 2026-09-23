//! `finchart-web`: a thin WebAssembly binding that runs the finchart engine in a
//! browser. It folds neutral `MarketEvent` text lines through a `ChartState` and
//! renders the existing terminal chart to an ANSI `String` — the same pure
//! pipeline the terminal binaries use, now compiled to WASM. JS owns the transport
//! (fetch a recorded scenario file, drive a player loop) so this stays pure/sync.
//!
//! All WASM-specific deps (`wasm-bindgen`) live in this crate only; the core crates
//! stay zero-dependency. The render assembly mirrors
//! `examples-matching-engine/src/render.rs` (kept local to avoid depending on that
//! crate's crossterm/gRPC deps). See `docs/web-showcase-v1-plan.md`.

use wasm_bindgen::prelude::*;

use backend_terminal::{
    render_chart, Annotation, ChartOptions, ColorMode, DepthPanel, HeatmapView, Style,
};
use market_core::{parse_event, BookSide, CandleSeries, ChartState, MarketEvent};
use render_core::{cull, Viewport};

/// Level 2 levels shown per side in the depth panel.
const DEPTH_LEVELS: usize = 6;
/// Fallback follow-window (candles) when the caller passes 0.
const DEFAULT_WINDOW: usize = 120;

/// An interactive chart driven by streamed `MarketEvent` text lines.
///
/// Follows the newest candle by default; `pan`/`zoom` switch to a manual viewport,
/// and `follow` returns to tailing the live edge.
#[wasm_bindgen]
pub struct WasmChart {
    state: ChartState,
    viewport: Viewport,
    following: bool,
    window: usize,
    /// Draw one-column block candles instead of three-column OHLC bars. At a
    /// fixed width this fits about three times as many bars on screen.
    dense_bars: bool,
    /// Labelled moments drawn into the plot. Kept across `reset`, because they
    /// describe the recording rather than the state rebuilt from it.
    annotations: Vec<Annotation>,
    symbol: String,
    trades: usize,
    // Retained so `reset` can rebuild an identical fresh state.
    interval_secs: i64,
    tick_size: f64,
    heatmap_capacity: Option<usize>,
    trade_cap: usize,
}

#[wasm_bindgen]
impl WasmChart {
    /// Create an empty chart. `heatmap_capacity == 0` disables the heatmap;
    /// `window == 0` uses a sensible default follow-window.
    #[wasm_bindgen(constructor)]
    pub fn new(
        interval_secs: i64,
        tick_size: f64,
        heatmap_capacity: usize,
        trade_cap: usize,
        window: usize,
        symbol: String,
    ) -> WasmChart {
        let heatmap_capacity = (heatmap_capacity != 0).then_some(heatmap_capacity);
        let state = ChartState::from_history(
            CandleSeries::new(),
            interval_secs,
            tick_size,
            heatmap_capacity,
            trade_cap,
        );
        WasmChart {
            state,
            viewport: Viewport::full(0),
            following: true,
            window: if window == 0 { DEFAULT_WINDOW } else { window },
            dense_bars: false,
            annotations: Vec::new(),
            symbol,
            trades: 0,
            interval_secs,
            tick_size,
            heatmap_capacity,
            trade_cap,
        }
    }

    /// Fold one serialized event line (e.g. `T 60 100.0 2 B`) into the chart.
    /// Unparseable lines are ignored.
    pub fn apply_line(&mut self, line: &str) {
        if let Some(ev) = parse_event(line) {
            if matches!(ev, MarketEvent::Trade(_)) {
                self.trades += 1;
            }
            self.state.apply(&ev);
        }
    }

    /// Fold a multi-line block of serialized events.
    pub fn apply_events(&mut self, text: &str) {
        for line in text.lines() {
            self.apply_line(line);
        }
    }

    /// Clear all state (used when switching scenarios). Returns to follow mode.
    pub fn reset(&mut self) {
        self.state = ChartState::from_history(
            CandleSeries::new(),
            self.interval_secs,
            self.tick_size,
            self.heatmap_capacity,
            self.trade_cap,
        );
        self.viewport = Viewport::full(0);
        self.following = true;
        self.trades = 0;
    }

    /// Render one ANSI frame of exactly `rows` lines, each `cols` visible chars.
    pub fn render(&self, cols: usize, rows: usize) -> String {
        self.render_with(cols, rows, ColorMode::Ansi)
    }

    /// The same frame as `render`, converted to an HTML fragment.
    ///
    /// The browser assigns this into a `<pre>`, so colors arrive as `.up` / `.down`
    /// classes and the host page's own theme drives them. Every non-escape character
    /// is HTML-escaped by the converter.
    pub fn render_html(&self, cols: usize, rows: usize) -> String {
        backend_terminal::html::ansi_to_html(&self.render(cols, rows))
    }

    /// Scroll the manual viewport by `delta` candles (leaves follow mode).
    pub fn pan(&mut self, delta: i32) {
        let len = self.state.series().len();
        self.viewport = self.effective_viewport(len).pan(delta as isize, len);
        self.following = false;
    }

    /// Zoom the manual viewport around its center (leaves follow mode).
    /// `factor < 1.0` zooms in; `> 1.0` zooms out.
    pub fn zoom(&mut self, factor: f64) {
        let len = self.state.series().len();
        self.viewport = self.effective_viewport(len).zoom(factor, len);
        self.following = false;
    }

    /// Choose the bar style. Dense draws one-column block candles; the default
    /// draws three-column OHLC bars, which are clearer but fit a third as many.
    pub fn set_dense_bars(&mut self, dense: bool) {
        self.dense_bars = dense;
    }

    /// Whether dense one-column bars are in use.
    pub fn dense_bars(&self) -> bool {
        self.dense_bars
    }

    /// Mark a moment (epoch seconds, on the candles' clock) with a short label.
    /// It is drawn once the candle containing it is on screen.
    pub fn add_annotation(&mut self, ts: i64, label: String) {
        self.annotations.push(Annotation { ts, label });
    }

    /// Remove every annotation.
    pub fn clear_annotations(&mut self) {
        self.annotations.clear();
    }

    /// Resume tailing the newest candle.
    pub fn follow(&mut self) {
        self.following = true;
    }

    /// Number of candles currently in the series.
    pub fn series_len(&self) -> usize {
        self.state.series().len()
    }

    /// Number of candles currently visible in the viewport.
    pub fn visible_candles(&self) -> usize {
        self.effective_viewport(self.state.series().len())
            .candle_count()
    }

    /// Number of trade events folded so far.
    pub fn trade_count(&self) -> usize {
        self.trades
    }
}

impl WasmChart {
    /// The viewport to render with: the live tail when following, else the manual
    /// viewport clamped to the current series length.
    fn effective_viewport(&self, len: usize) -> Viewport {
        if self.following {
            Viewport::tail(len, self.window)
        } else {
            self.viewport.clamp(len)
        }
    }

    /// Full render assembly (mirrors `examples-matching-engine::render_state`),
    /// parameterized on color so tests can request plain output.
    fn render_with(&self, cols: usize, rows: usize, color: ColorMode) -> String {
        let series = self.state.series();
        let view = cull(series, &self.effective_viewport(series.len()));
        let (lo, hi) = view.price_extent().unwrap_or((0.0, 1.0));

        let heatmap = self.state.heatmap().map(|h| {
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
            self.state
                .book()
                .top_n(side, DEPTH_LEVELS)
                .into_iter()
                .map(|l| (l.price, l.size))
                .collect()
        };
        let depth = DepthPanel {
            bids: to_tuples(BookSide::Bid),
            asks: to_tuples(BookSide::Ask),
            spread: self.state.spread(),
        };

        render_chart(
            &view,
            &ChartOptions {
                width: cols,
                height: rows,
                color,
                style: if self.dense_bars {
                    Style::Candles
                } else {
                    Style::OhlcBars
                },
                show_volume: true,
                show_delta: true,
                depth: Some(depth),
                heatmap,
                trades: None,
                annotations: self.annotations.clone(),
                title: self.status_line(),
            },
        )
    }

    /// Compact status line: symbol, trade count, BBO/spread, CVD.
    fn status_line(&self) -> String {
        let book = match (self.state.best_bid(), self.state.best_ask()) {
            (Some(b), Some(a)) => format!("bid {b:.2} ask {a:.2} spr {:.2}", a - b),
            _ => "bid — ask —".to_string(),
        };
        format!(
            "finchart · {} · trades {} · {} · CVD {:+.0}",
            self.symbol,
            self.trades,
            book,
            self.state.cumulative_delta(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart() -> WasmChart {
        WasmChart::new(60, 0.05, 64, 32, 120, "TEST".to_string())
    }

    #[test]
    fn empty_render_has_exact_dimensions_and_no_escapes_in_plain() {
        let out = chart().render_with(120, 30, ColorMode::Plain);
        assert_eq!(out.lines().count(), 30);
        for line in out.lines() {
            assert_eq!(line.chars().count(), 120);
        }
        assert!(!out.contains('\u{1b}'), "plain frame has escapes");
    }

    #[test]
    fn trades_build_candles_and_are_counted() {
        let mut c = chart();
        // One trade per interval bucket → one candle each (last forming).
        for i in 0..5 {
            c.apply_line(&format!("T {} 100.{} 2 B", i * 60, i));
        }
        assert_eq!(c.series_len(), 5);
        assert_eq!(c.trade_count(), 5);
        // Still renders to exact dimensions with data present.
        let out = c.render_with(120, 30, ColorMode::Plain);
        assert_eq!(out.lines().count(), 30);
    }

    #[test]
    fn zoom_in_reduces_visible_candles() {
        let mut c = chart();
        for i in 0..20 {
            c.apply_line(&format!("T {} 100.0 1 B", i * 60));
        }
        let before = c.visible_candles(); // following → tail(20, 120) == 20
        c.zoom(0.5);
        let after = c.visible_candles();
        assert!(
            after < before,
            "zoom did not shrink window: {after} !< {before}"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut c = chart();
        for i in 0..5 {
            c.apply_line(&format!("T {} 100.0 1 B", i * 60));
        }
        assert!(c.series_len() > 0);
        c.reset();
        assert_eq!(c.series_len(), 0);
        assert_eq!(c.trade_count(), 0);
    }

    #[test]
    fn render_html_has_no_escapes_and_spans_the_colors() {
        let mut c = chart();
        for i in 0..5 {
            c.apply_line(&format!("T {} 100.{} 1 B", i * 60, i));
        }
        let html = c.render_html(120, 30);
        assert!(!html.contains('\u{1b}'), "raw escape leaked into html");
        assert!(
            html.contains("<span class=\"up\">") || html.contains("<span class=\"down\">"),
            "expected up/down spans in {html:.200}"
        );
    }

    #[test]
    fn render_html_escapes_markup_characters() {
        // The frame is assigned with innerHTML in the browser, so `<` and `&`
        // appearing in axis labels or the title must arrive escaped.
        let html = chart().render_html(120, 30);
        assert!(
            !html.contains("<pre"),
            "unexpected raw markup in {html:.200}"
        );
        for chunk in html.split("<span class=").skip(1) {
            let body = chunk.split_once('>').map(|x| x.1).unwrap_or("");
            assert!(!body.contains('\u{1b}'));
        }
    }

    /// The bar count on screen is set by how many columns each bar occupies, not
    /// by how many candles exist: OHLC bars are three columns wide, block candles
    /// one. At a fixed width the dense style therefore fits roughly three times as
    /// many bars, which is the difference between a chart and a sketch of one.
    #[test]
    fn dense_style_fits_more_bars_in_the_same_width() {
        fn bars(out: &str) -> usize {
            // The title line records "N candles -> M bars".
            let head = out.lines().next().unwrap();
            let after = head.split("\u{2192} ").nth(1).unwrap();
            after.split(' ').next().unwrap().parse().unwrap()
        }

        let mut wide = chart();
        let mut dense = chart();
        for i in 0..120 {
            let line = format!("T {} 100.{} 1 B", i * 60, i % 10);
            wide.apply_line(&line);
            dense.apply_line(&line);
        }
        dense.set_dense_bars(true);

        let wide_bars = bars(&wide.render_with(120, 30, ColorMode::Plain));
        let dense_bars = bars(&dense.render_with(120, 30, ColorMode::Plain));
        assert!(
            dense_bars > wide_bars * 2,
            "dense style drew {dense_bars} bars against {wide_bars}"
        );
    }

    fn filled() -> WasmChart {
        let mut c = chart();
        for i in 0..20 {
            c.apply_line(&format!("T {} 100.{} 1 B", i * 60, i % 10));
        }
        c
    }

    #[test]
    fn an_added_annotation_is_drawn() {
        let mut c = filled();
        c.add_annotation(10 * 60, "SWEEP".to_string());
        assert!(c.render_with(120, 30, ColorMode::Plain).contains("SWEEP"));
        assert!(c.render_html(120, 30).contains("<span class=\"note\">"));
    }

    #[test]
    fn clearing_annotations_removes_them() {
        let mut c = filled();
        c.add_annotation(10 * 60, "SWEEP".to_string());
        c.clear_annotations();
        assert!(!c.render_with(120, 30, ColorMode::Plain).contains("SWEEP"));
    }

    /// Seeking resets the chart and refolds events up to the target, so the
    /// annotations belong to the recording, not to the state being rebuilt.
    #[test]
    fn annotations_survive_a_reset() {
        let mut c = filled();
        c.add_annotation(10 * 60, "SWEEP".to_string());
        c.reset();
        for i in 0..20 {
            c.apply_line(&format!("T {} 100.{} 1 B", i * 60, i % 10));
        }
        assert!(c.render_with(120, 30, ColorMode::Plain).contains("SWEEP"));
    }

    #[test]
    fn bar_style_defaults_to_the_wide_one() {
        assert!(!chart().dense_bars());
    }

    #[test]
    fn ansi_render_carries_color_escapes() {
        let mut c = chart();
        for i in 0..5 {
            c.apply_line(&format!("T {} 100.{} 1 B", i * 60, i));
        }
        let out = c.render(120, 30);
        assert!(out.contains('\u{1b}'), "ansi frame should carry escapes");
    }
}
