//! Crosshair and mouse handling shared by `browse` and `replay`.
//!
//! Pure state: the views pass in events and the current [`Layout`] and get back
//! what to do with the viewport, so all of this is testable without a terminal.

use backend_terminal::{Crosshair, Layout};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

/// Viewport zoom factors for the wheel (and the views' zoom keys).
pub const ZOOM_IN: f64 = 0.8;
pub const ZOOM_OUT: f64 = 1.25;

/// What a mouse event asks the view to do with its viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Nav {
    None,
    /// Zoom by this factor (`< 1` in).
    Zoom(f64),
    /// Pan by this many candles (negative = back in time).
    Pan(isize),
}

/// The crosshair plus an in-progress drag.
#[derive(Debug, Default)]
pub struct Pointer {
    pub crosshair: Option<Crosshair>,
    /// Column the current drag is anchored at; fractional so slow drags add up.
    drag_from: Option<f64>,
}

impl Pointer {
    /// `c`: show the crosshair on the newest bar, or hide it.
    pub fn toggle(&mut self, layout: Option<&Layout>) {
        self.crosshair = match self.crosshair {
            Some(_) => None,
            None => newest(layout),
        };
    }

    /// `,` / `.`: move the crosshair one bar back (`dir < 0`) or forward, keeping
    /// its row; shows it on the newest bar if hidden.
    pub fn step(&mut self, dir: i32, layout: Option<&Layout>) {
        let Some(l) = layout.filter(|l| !l.bar_centers.is_empty()) else {
            return;
        };
        let Some(c) = self.crosshair else {
            self.crosshair = newest(layout);
            return;
        };
        let here = nearest_bar(l, c.col);
        let next = (here as i64 + i64::from(dir.signum())).clamp(0, l.bar_centers.len() as i64 - 1);
        self.crosshair = Some(Crosshair {
            col: l.bar_centers[next as usize],
            row: c.row,
        });
    }

    /// `Esc`: hide the crosshair; `false` if it was not showing.
    pub fn hide(&mut self) -> bool {
        self.crosshair.take().is_some()
    }

    /// Handle a mouse event: hover moves the crosshair, the wheel zooms, a left
    /// drag pans. `visible` is the number of candles on screen.
    pub fn mouse(&mut self, ev: MouseEvent, layout: Option<&Layout>, visible: usize) -> Nav {
        let (col, row) = (ev.column as usize, ev.row as usize);
        match ev.kind {
            MouseEventKind::ScrollUp => return Nav::Zoom(ZOOM_IN),
            MouseEventKind::ScrollDown => return Nav::Zoom(ZOOM_OUT),
            MouseEventKind::Down(MouseButton::Left) => self.drag_from = Some(col as f64),
            MouseEventKind::Up(MouseButton::Left) => self.drag_from = None,
            MouseEventKind::Drag(MouseButton::Left) => {
                if let (Some(from), Some(l)) = (self.drag_from, layout) {
                    self.crosshair = Some(Crosshair {
                        col,
                        row: Some(row),
                    });
                    // Candles per column, so the content moves with the mouse.
                    let per_col = visible as f64 / l.plot_cols.max(1) as f64;
                    let candles = ((from - col as f64) * per_col).trunc();
                    if candles != 0.0 {
                        self.drag_from = Some(from - candles / per_col);
                        return Nav::Pan(candles as isize);
                    }
                }
                return Nav::None;
            }
            _ => {}
        }
        if matches!(ev.kind, MouseEventKind::Moved | MouseEventKind::Down(_)) {
            self.crosshair = Some(Crosshair {
                col,
                row: Some(row),
            });
        }
        Nav::None
    }
}

/// A crosshair on the newest bar, with no price row.
fn newest(layout: Option<&Layout>) -> Option<Crosshair> {
    let col = *layout?.bar_centers.last()?;
    Some(Crosshair { col, row: None })
}

/// Index of the bar whose center is closest to `col`.
fn nearest_bar(layout: &Layout, col: usize) -> usize {
    (0..layout.bar_centers.len())
        .min_by_key(|&i| layout.bar_centers[i].abs_diff(col))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend_terminal::{layout, ChartOptions, Layout};
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use market_core::{generate, GenConfig};
    use render_core::{cull, Viewport};

    /// A 120x30 chart of 60 candles (no aggregation: 60 bars fit).
    fn chart_layout() -> Layout {
        let s = generate(&GenConfig {
            seed: 1,
            count: 60,
            start_ts: 0,
            interval_secs: 60,
            start_price: 100.0,
            volatility: 0.02,
            drift: 0.0,
        });
        let view = cull(&s, &Viewport::full(s.len()));
        layout(&view, &ChartOptions::default()).unwrap()
    }

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn toggling_shows_the_crosshair_on_the_newest_bar_then_hides_it() {
        let l = chart_layout();
        let mut p = Pointer::default();
        p.toggle(Some(&l));
        let last = *l.bar_centers.last().unwrap();
        assert_eq!(
            p.crosshair,
            Some(Crosshair {
                col: last,
                row: None
            })
        );
        p.toggle(Some(&l));
        assert_eq!(p.crosshair, None);
    }

    #[test]
    fn stepping_moves_one_bar_and_stops_at_the_ends() {
        let l = chart_layout();
        let mut p = Pointer::default();
        p.step(-1, Some(&l)); // shows it on the newest bar first
        let n = l.bar_centers.len();
        assert_eq!(p.crosshair.unwrap().col, l.bar_centers[n - 1]);
        p.step(-1, Some(&l));
        assert_eq!(p.crosshair.unwrap().col, l.bar_centers[n - 2]);
        p.step(1, Some(&l));
        p.step(1, Some(&l));
        assert_eq!(
            p.crosshair.unwrap().col,
            l.bar_centers[n - 1],
            "clamped at the newest"
        );
        for _ in 0..200 {
            p.step(-1, Some(&l));
        }
        assert_eq!(
            p.crosshair.unwrap().col,
            l.bar_centers[0],
            "clamped at the oldest"
        );
    }

    #[test]
    fn stepping_keeps_the_crosshairs_row() {
        let l = chart_layout();
        let mut p = Pointer::default();
        p.mouse(
            mouse(MouseEventKind::Moved, l.bar_centers[5] as u16, 7),
            Some(&l),
            60,
        );
        p.step(1, Some(&l));
        assert_eq!(
            p.crosshair,
            Some(Crosshair {
                col: l.bar_centers[6],
                row: Some(7)
            })
        );
    }

    #[test]
    fn hiding_reports_whether_there_was_anything_to_hide() {
        let mut p = Pointer::default();
        assert!(!p.hide());
        p.toggle(Some(&chart_layout()));
        assert!(p.hide());
        assert_eq!(p.crosshair, None);
    }

    #[test]
    fn hovering_moves_the_crosshair_to_the_mouse() {
        let l = chart_layout();
        let mut p = Pointer::default();
        let nav = p.mouse(mouse(MouseEventKind::Moved, 40, 9), Some(&l), 60);
        assert_eq!(nav, Nav::None);
        assert_eq!(
            p.crosshair,
            Some(Crosshair {
                col: 40,
                row: Some(9)
            })
        );
    }

    #[test]
    fn the_wheel_zooms() {
        let l = chart_layout();
        let mut p = Pointer::default();
        assert_eq!(
            p.mouse(mouse(MouseEventKind::ScrollUp, 40, 9), Some(&l), 60),
            Nav::Zoom(ZOOM_IN)
        );
        assert_eq!(
            p.mouse(mouse(MouseEventKind::ScrollDown, 40, 9), Some(&l), 60),
            Nav::Zoom(ZOOM_OUT)
        );
    }

    #[test]
    fn dragging_pans_the_content_with_the_mouse() {
        let l = chart_layout();
        let mut p = Pointer::default();
        let left = MouseButton::Left;
        p.mouse(mouse(MouseEventKind::Down(left), 60, 9), Some(&l), 60);
        // Dragging right by a third of the plot pulls a third of the window into
        // view from the past: pan back 20 of the 60 visible candles.
        let third = (l.plot_cols / 3) as u16;
        let nav = p.mouse(
            mouse(MouseEventKind::Drag(left), 60 + third, 9),
            Some(&l),
            60,
        );
        assert_eq!(
            nav,
            Nav::Pan(-((60 * third as usize / l.plot_cols) as isize))
        );
        p.mouse(mouse(MouseEventKind::Up(left), 60 + third, 9), Some(&l), 60);
        // After release, moving never pans.
        let nav = p.mouse(mouse(MouseEventKind::Drag(left), 10, 9), Some(&l), 60);
        assert_eq!(nav, Nav::None);
    }

    #[test]
    fn slow_drags_accumulate_until_a_whole_candle() {
        let l = chart_layout();
        let mut p = Pointer::default();
        let left = MouseButton::Left;
        p.mouse(mouse(MouseEventKind::Down(left), 60, 9), Some(&l), 60);
        // 60 candles over ~100 columns: under one candle per column, so single
        // columns must add up rather than round away.
        let mut panned = 0;
        for col in (40..60).rev() {
            if let Nav::Pan(n) = p.mouse(mouse(MouseEventKind::Drag(left), col, 9), Some(&l), 60) {
                panned += n;
            }
        }
        let expected = (20.0 * 60.0 / l.plot_cols as f64) as isize;
        assert_eq!(
            panned, expected,
            "20 columns left = {expected} candles forward"
        );
    }
}
