//! Viewport navigation shared by the streaming views (`live`, `replay`): follow
//! the newest candles, or hold a viewport the user panned or zoomed to, plus the
//! crosshair and mouse handling from [`Pointer`].

use backend_terminal::Layout;
use crossterm::event::MouseEvent;
use render_core::Viewport;

use crate::pointer::{Nav, Pointer};
pub use crate::pointer::{ZOOM_IN, ZOOM_OUT};

/// Follow-or-hold viewport state and the crosshair. `len` is the series length
/// and `window` the follow window, both passed in since the series keeps growing.
#[derive(Debug, Default)]
pub struct Navigator {
    /// `None` follows the newest candles; `Some` is a viewport the user moved to.
    manual: Option<Viewport>,
    pub pointer: Pointer,
}

impl Navigator {
    #[cfg(test)]
    pub fn is_following(&self) -> bool {
        self.manual.is_none()
    }

    /// What is on screen: the newest `window` candles, or the held viewport.
    pub fn viewport(&self, len: usize, window: usize) -> Viewport {
        match self.manual {
            Some(vp) => vp.clamp(len),
            None => Viewport::tail(len, window),
        }
    }

    /// Return to following the newest candles.
    pub fn follow(&mut self) {
        self.manual = None;
    }

    /// Keyboard pan step: a tenth of what is on screen, at least one candle.
    pub fn pan_step(&self, len: usize, window: usize) -> isize {
        (self.viewport(len, window).candle_count() / 10).max(1) as isize
    }

    /// Pan by `candles` from what is on screen (leaves follow mode).
    pub fn pan(&mut self, candles: isize, len: usize, window: usize) {
        self.manual = Some(self.viewport(len, window).pan(candles, len));
    }

    /// Zoom by `factor` around what is on screen (leaves follow mode).
    pub fn zoom(&mut self, factor: f64, len: usize, window: usize) {
        self.manual = Some(self.viewport(len, window).zoom(factor, len));
    }

    /// Hover moves the crosshair, the wheel zooms, a left drag pans.
    pub fn mouse(&mut self, ev: MouseEvent, layout: Option<&Layout>, len: usize, window: usize) {
        let visible = self.viewport(len, window).candle_count();
        match self.pointer.mouse(ev, layout, visible) {
            Nav::Zoom(factor) => self.zoom(factor, len, window),
            Nav::Pan(candles) => self.pan(candles, len, window),
            Nav::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend_terminal::{layout, ChartOptions};
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use market_core::{generate, CandleSeries, GenConfig};
    use render_core::cull;

    fn series(n: usize) -> CandleSeries {
        generate(&GenConfig {
            seed: 3,
            count: n,
            start_ts: 0,
            interval_secs: 60,
            start_price: 100.0,
            volatility: 0.02,
            drift: 0.0,
        })
    }

    fn mouse(kind: MouseEventKind, col: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row: 8,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn follows_the_newest_candles_by_default() {
        let nav = Navigator::default();
        assert!(nav.is_following());
        assert_eq!(nav.viewport(500, 60), Viewport::tail(500, 60));
        // Following tracks a growing series.
        assert_eq!(nav.viewport(501, 60).end_index, 501);
    }

    #[test]
    fn panning_and_zooming_hold_a_viewport_until_follow() {
        let mut nav = Navigator::default();
        nav.pan(-10, 500, 60);
        assert!(!nav.is_following());
        assert_eq!(nav.viewport(500, 60), Viewport::tail(500, 60).pan(-10, 500));
        // A held viewport stays put while the series grows.
        assert_eq!(nav.viewport(600, 60).end_index, 490);

        nav.follow();
        assert_eq!(nav.viewport(600, 60), Viewport::tail(600, 60));

        nav.zoom(ZOOM_IN, 600, 60);
        assert!(nav.viewport(600, 60).candle_count() < 60);
    }

    #[test]
    fn pan_step_is_a_tenth_of_the_window() {
        let nav = Navigator::default();
        assert_eq!(nav.pan_step(500, 60), 6);
        assert_eq!(nav.pan_step(5, 60), 1); // at least one candle
    }

    #[test]
    fn mouse_wheel_and_drag_move_the_viewport() {
        let s = series(500);
        let mut nav = Navigator::default();
        let view = cull(&s, &nav.viewport(s.len(), 60));
        let l = layout(&view, &ChartOptions::default());

        nav.mouse(mouse(MouseEventKind::ScrollUp, 60), l.as_ref(), s.len(), 60);
        assert!(
            nav.viewport(s.len(), 60).candle_count() < 60,
            "wheel zooms in"
        );

        nav.follow();
        nav.mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 40),
            l.as_ref(),
            s.len(),
            60,
        );
        nav.mouse(
            mouse(MouseEventKind::Drag(MouseButton::Left), 100),
            l.as_ref(),
            s.len(),
            60,
        );
        assert!(
            nav.viewport(s.len(), 60).end_index < s.len(),
            "dragging right pans back"
        );

        nav.mouse(mouse(MouseEventKind::Moved, 50), l.as_ref(), s.len(), 60);
        assert!(nav.pointer.crosshair.is_some(), "hover shows the crosshair");
    }
}
