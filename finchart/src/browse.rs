//! `finchart browse`: an interactive viewer over a deterministic synthetic series.
//!
//! Pan (`h`/`l` or `←`/`→`), zoom (`k`/`j` or `+`/`-`), reset (`r`), quit (`q`).
//! All navigation and rendering logic lives in tested library code
//! (`render_core::Viewport`, `backend_terminal::render_chart`); this module is a
//! thin input/draw loop.

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use backend_terminal::{render_chart, ChartOptions, ColorMode, Style};
use market_core::{generate, CandleSeries, GenConfig};
use render_core::{cull, Viewport};

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
    Quit,
    None,
}

/// Map a key event to an action.
fn action_for(key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') if ctrl => Action::Quit,
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

/// Interactive viewer state: the series plus the current viewport.
struct App {
    series: CandleSeries,
    viewport: Viewport,
}

impl App {
    fn new(series: CandleSeries) -> Self {
        let viewport = default_viewport(series.len());
        Self { series, viewport }
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
            Action::PanLeft => self.viewport = self.viewport.pan(-(self.step() as isize), len),
            Action::PanRight => self.viewport = self.viewport.pan(self.step() as isize, len),
            Action::ZoomIn => self.viewport = self.viewport.zoom(0.8, len),
            Action::ZoomOut => self.viewport = self.viewport.zoom(1.25, len),
            Action::Reset => self.viewport = default_viewport(len),
            Action::None => {}
        }
        true
    }

    /// Render the current view to fit a `(cols, rows)` terminal.
    fn render(&self, cols: usize, rows: usize) -> String {
        let view = cull(&self.series, &self.viewport);
        render_chart(
            &view,
            &ChartOptions {
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
                title: "finchart · h/l pan · k/j zoom · r reset · q quit".to_string(),
            },
        )
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
        screen.paint(&app.render(size.0, size.1), size)?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Press && !app.apply(action_for(key)) {
            break;
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
        assert_eq!(action_for(key(KeyCode::Esc)), Action::Quit);
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
}
