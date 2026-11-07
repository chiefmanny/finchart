//! `finchart browse`: an interactive viewer over a deterministic synthetic series.
//!
//! Pan (`h`/`l` or `←`/`→`), zoom (`k`/`j` or `+`/`-`), reset (`r`), quit (`q`);
//! `c` toggles a crosshair that `,`/`.` step bar by bar; the mouse hovers (crosshair),
//! scrolls (zoom) and drags (pan).
//! All navigation and rendering logic lives in tested library code
//! (`render_core::Viewport`, `backend_terminal::render_chart`); this module is a
//! thin input/draw loop.

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};

use backend_terminal::{layout, render_chart, ChartOptions, ColorMode, Layout, Style};
use market_core::{generate, CandleSeries, GenConfig};
use render_core::{cull, Viewport};

use crate::pointer::{Nav, Pointer, ZOOM_IN, ZOOM_OUT};
use crate::term::Screen;

/// Candles in the deterministic demo series.
const SERIES_LEN: usize = 5_000;
/// Default visible window (also used by reset).
const DEFAULT_WINDOW: usize = 200;

/// A navigation action derived from a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    PanLeft,
    PanRight,
    ZoomIn,
    ZoomOut,
    Reset,
    /// Show the crosshair on the newest bar, or hide it.
    ToggleCrosshair,
    /// Move the crosshair one bar back / forward.
    StepBack,
    StepForward,
    /// Hide the crosshair, or quit if it is not showing.
    Escape,
    Quit,
    None,
}

/// Map a key event to an action.
fn action_for(key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if ctrl => Action::Quit,
        KeyCode::Esc => Action::Escape,
        KeyCode::Char('c') => Action::ToggleCrosshair,
        KeyCode::Char(',') => Action::StepBack,
        KeyCode::Char('.') => Action::StepForward,
        KeyCode::Left | KeyCode::Char('h') => Action::PanLeft,
        KeyCode::Right | KeyCode::Char('l') => Action::PanRight,
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('+') | KeyCode::Char('=') => {
            Action::ZoomIn
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('-') => Action::ZoomOut,
        KeyCode::Char('r') => Action::Reset,
        _ => Action::None,
    }
}

/// The default visible window: the most recent `DEFAULT_WINDOW` candles.
fn default_viewport(series_len: usize) -> Viewport {
    Viewport::new(series_len.saturating_sub(DEFAULT_WINDOW), series_len)
}

/// Interactive viewer state: the series, the current viewport and the crosshair.
struct App {
    series: CandleSeries,
    viewport: Viewport,
    pointer: Pointer,
    /// The terminal size frames are drawn at, for mapping the crosshair and mouse.
    size: (usize, usize),
}

impl App {
    fn new(series: CandleSeries) -> Self {
        let viewport = default_viewport(series.len());
        Self {
            series,
            viewport,
            pointer: Pointer::default(),
            size: (120, 30),
        }
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        self.size = (cols, rows);
    }

    /// Handle a mouse event: hover moves the crosshair, the wheel zooms, a left
    /// drag pans.
    fn mouse(&mut self, ev: MouseEvent) {
        let layout = self.layout();
        let len = self.len();
        match self
            .pointer
            .mouse(ev, layout.as_ref(), self.viewport.candle_count())
        {
            Nav::Zoom(factor) => self.viewport = self.viewport.zoom(factor, len),
            Nav::Pan(candles) => self.viewport = self.viewport.pan(candles, len),
            Nav::None => {}
        }
    }

    fn layout(&self) -> Option<Layout> {
        let view = cull(&self.series, &self.viewport);
        layout(&view, &self.options(self.size.0, self.size.1))
    }

    fn len(&self) -> usize {
        self.series.len()
    }

    /// Pan/zoom step: ~10% of the current window, at least one candle.
    fn step(&self) -> usize {
        (self.viewport.candle_count() / 10).max(1)
    }

    /// Apply an action, returning `false` if the app should quit.
    fn apply(&mut self, action: Action) -> bool {
        let len = self.len();
        match action {
            Action::Quit => return false,
            Action::Escape => return self.pointer.hide(),
            Action::ToggleCrosshair => self.pointer.toggle(self.layout().as_ref()),
            Action::StepBack => self.pointer.step(-1, self.layout().as_ref()),
            Action::StepForward => self.pointer.step(1, self.layout().as_ref()),
            Action::PanLeft => self.viewport = self.viewport.pan(-(self.step() as isize), len),
            Action::PanRight => self.viewport = self.viewport.pan(self.step() as isize, len),
            Action::ZoomIn => self.viewport = self.viewport.zoom(ZOOM_IN, len),
            Action::ZoomOut => self.viewport = self.viewport.zoom(ZOOM_OUT, len),
            Action::Reset => self.viewport = default_viewport(len),
            Action::None => {}
        }
        true
    }

    /// Render the current view to fit a `(cols, rows)` terminal.
    fn render(&self, cols: usize, rows: usize) -> String {
        let view = cull(&self.series, &self.viewport);
        render_chart(&view, &self.options(cols, rows))
    }

    fn options(&self, cols: usize, rows: usize) -> ChartOptions {
        ChartOptions {
            width: cols,
            height: rows,
            color: ColorMode::Ansi,
            style: Style::OhlcBars,
            show_volume: true,
            show_delta: false,
            depth: None,
            heatmap: None,
            trades: None,
            annotations: Vec::new(),
            crosshair: self.pointer.crosshair,
            title: "finchart · h/l pan · k/j zoom · c crosshair · r reset · q quit".to_string(),
        }
    }
}

fn demo_series() -> CandleSeries {
    generate(&GenConfig {
        seed: 42,
        count: SERIES_LEN,
        start_ts: 1_600_000_000,
        interval_secs: 60,
        start_price: 100.0,
        volatility: 0.02,
        drift: 0.0002,
    })
}

/// Run the viewer until a quit key.
pub fn run() -> io::Result<()> {
    let mut app = App::new(demo_series());
    let mut screen = Screen::enter()?;
    loop {
        let size = screen.size();
        app.resize(size.0, size.1);
        screen.paint(&app.render(size.0, size.1), size)?;

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if !app.apply(action_for(key)) {
                    break;
                }
            }
            Event::Mouse(ev) => app.mouse(ev),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn gen(count: usize) -> CandleSeries {
        generate(&GenConfig {
            seed: 1,
            count,
            start_ts: 0,
            interval_secs: 60,
            start_price: 100.0,
            volatility: 0.02,
            drift: 0.0,
        })
    }

    #[test]
    fn keys_map_to_actions() {
        assert_eq!(action_for(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(action_for(key(KeyCode::Esc)), Action::Escape);
        assert_eq!(action_for(key(KeyCode::Char('c'))), Action::ToggleCrosshair);
        assert_eq!(action_for(key(KeyCode::Char(','))), Action::StepBack);
        assert_eq!(action_for(key(KeyCode::Char('.'))), Action::StepForward);
        assert_eq!(
            action_for(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        assert_eq!(action_for(key(KeyCode::Left)), Action::PanLeft);
        assert_eq!(action_for(key(KeyCode::Char('h'))), Action::PanLeft);
        assert_eq!(action_for(key(KeyCode::Right)), Action::PanRight);
        assert_eq!(action_for(key(KeyCode::Char('l'))), Action::PanRight);
        assert_eq!(action_for(key(KeyCode::Char('k'))), Action::ZoomIn);
        assert_eq!(action_for(key(KeyCode::Char('j'))), Action::ZoomOut);
        assert_eq!(action_for(key(KeyCode::Char('r'))), Action::Reset);
        assert_eq!(action_for(key(KeyCode::Char('x'))), Action::None);
    }

    #[test]
    fn quit_action_stops_the_loop() {
        let mut app = App::new(gen(100));
        assert!(!app.apply(Action::Quit));
        assert!(app.apply(Action::None));
    }

    #[test]
    fn pan_and_zoom_stay_in_bounds() {
        let mut app = App::new(gen(1_000));
        app.apply(Action::ZoomIn);
        for _ in 0..1_000 {
            app.apply(Action::PanLeft);
        }
        assert!(app.viewport.start_index <= app.viewport.end_index);
        assert!(app.viewport.end_index <= app.len());
        assert_eq!(app.viewport.start_index, 0); // pinned at the left edge

        for _ in 0..5_000 {
            app.apply(Action::PanRight);
        }
        assert!(app.viewport.end_index <= app.len());
        assert_eq!(app.viewport.end_index, app.len()); // pinned at the right edge
    }

    #[test]
    fn zoom_out_saturates_to_full_series() {
        let mut app = App::new(gen(1_000));
        for _ in 0..50 {
            app.apply(Action::ZoomOut);
        }
        assert_eq!(app.viewport, Viewport::new(0, 1_000));
    }

    #[test]
    fn reset_restores_default_window() {
        let mut app = App::new(gen(1_000));
        app.apply(Action::ZoomIn);
        app.apply(Action::PanLeft);
        app.apply(Action::Reset);
        assert_eq!(app.viewport, default_viewport(app.len()));
    }

    #[test]
    fn the_crosshair_reads_out_bars_and_escape_hides_it_before_quitting() {
        let mut app = App::new(gen(500));
        app.resize(120, 30);
        app.apply(Action::ToggleCrosshair);
        let first = app.render(120, 30);
        assert!(first.starts_with("⌖"), "{}", first.lines().next().unwrap());
        app.apply(Action::StepBack);
        assert_ne!(
            app.render(120, 30).lines().next(),
            first.lines().next(),
            "stepping reads out another bar"
        );
        assert!(app.apply(Action::Escape), "first Esc hides the crosshair");
        assert!(!app.render(120, 30).starts_with("⌖"));
        assert!(!app.apply(Action::Escape), "second Esc quits");
    }

    #[test]
    fn the_wheel_zooms_and_a_drag_pans() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let ev = |kind, col| MouseEvent {
            kind,
            column: col,
            row: 10,
            modifiers: KeyModifiers::NONE,
        };
        let mut app = App::new(gen(500));
        app.resize(120, 30);
        let before = app.viewport;
        app.mouse(ev(MouseEventKind::ScrollUp, 60));
        assert!(
            app.viewport.candle_count() < before.candle_count(),
            "zoomed in"
        );

        let zoomed = app.viewport;
        app.mouse(ev(MouseEventKind::Down(MouseButton::Left), 40));
        app.mouse(ev(MouseEventKind::Drag(MouseButton::Left), 100));
        assert!(
            app.viewport.start_index < zoomed.start_index,
            "dragging right pans back"
        );
    }
}
