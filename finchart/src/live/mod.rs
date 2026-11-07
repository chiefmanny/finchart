//! `finchart live`: a live synthetic market with incremental candle updates.
//!
//! An interactive real-time viewer that follows the forming candle, redrawing at
//! a fixed frame rate. The submodules also back `finchart snapshot live`,
//! `finchart scenario` and `finchart record`.
//!
//! Synthetic tick timestamps drive all candle logic deterministically; wall-clock
//! time only paces the visual frame rate. No async.

pub(crate) mod feed;
pub(crate) mod replay;
pub(crate) mod scenarios;
pub(crate) mod snapshot;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use backend_terminal::ColorMode;
use replay::{Replay, ReplayConfig};

use crate::term::{is_quit, Screen};

/// Ticks ingested per rendered frame (a candle of 30 ticks forms over ~15 frames).
const TICKS_PER_FRAME: usize = 2;

/// What a key asks the live view to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Show the crosshair on the newest bar, or hide it.
    ToggleCrosshair,
    /// Move the crosshair one bar back / forward.
    StepBack,
    StepForward,
    /// Hide the crosshair, or quit if it is not showing.
    Escape,
    PanLeft,
    PanRight,
    ZoomIn,
    ZoomOut,
    /// Return to following the forming candle.
    Follow,
    Quit,
    None,
}

/// Map a key to an action (the same navigation keys as `browse` and `replay`).
pub fn action_for(key: KeyEvent) -> Action {
    if key.code == KeyCode::Esc {
        return Action::Escape;
    }
    if is_quit(key) {
        return Action::Quit;
    }
    match key.code {
        KeyCode::Char('c') => Action::ToggleCrosshair,
        KeyCode::Char(',') => Action::StepBack,
        KeyCode::Char('.') => Action::StepForward,
        KeyCode::Left | KeyCode::Char('h') => Action::PanLeft,
        KeyCode::Right | KeyCode::Char('l') => Action::PanRight,
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('+') | KeyCode::Char('=') => {
            Action::ZoomIn
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('-') => Action::ZoomOut,
        KeyCode::Char('f') => Action::Follow,
        _ => Action::None,
    }
}

/// Run the interactive live view at `fps` frames per second until a quit key.
///
/// The market advances on a fixed deadline whether or not input arrives, so a
/// moving mouse or a held key never stalls the stream.
pub fn run(fps: u64) -> io::Result<()> {
    let mut replay = Replay::new(&ReplayConfig::default());
    let frame_dur = Duration::from_millis(1000 / fps.max(1));
    let mut screen = Screen::enter()?;

    let mut next_frame = Instant::now() + frame_dur;
    let mut last_update_us = 0.0;
    let mut last_render_us = 0.0;
    loop {
        let (cols, rows) = screen.size();
        replay.resize(cols, rows);
        // Status carries the forming candle state plus the previous frame's timings.
        let title = format!(
            "{} · upd {last_update_us:.2}µs · ren {last_render_us:.1}µs · c crosshair · q quit",
            replay.status()
        );

        let t = Instant::now();
        let chart = replay.render(cols, rows, ColorMode::Ansi, &title);
        last_render_us = t.elapsed().as_secs_f64() * 1e6;
        screen.paint(&chart, (cols, rows))?;

        // Wait for input until the next frame deadline.
        let timeout = next_frame.saturating_duration_since(Instant::now());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if !replay.act(action_for(key)) {
                        break;
                    }
                }
                Event::Mouse(ev) => replay.mouse(ev),
                _ => {}
            }
        }
        // Advance on the deadline regardless of input.
        let now = Instant::now();
        if now >= next_frame {
            let t = Instant::now();
            replay.advance(TICKS_PER_FRAME);
            last_update_us = t.elapsed().as_secs_f64() * 1e6;
            next_frame += frame_dur;
            if next_frame < now {
                next_frame = now + frame_dur; // fell behind: don't burst to catch up
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn keys_map_to_actions() {
        assert_eq!(action_for(key(KeyCode::Char('c'))), Action::ToggleCrosshair);
        assert_eq!(action_for(key(KeyCode::Char(','))), Action::StepBack);
        assert_eq!(action_for(key(KeyCode::Char('.'))), Action::StepForward);
        assert_eq!(action_for(key(KeyCode::Esc)), Action::Escape);
        assert_eq!(action_for(key(KeyCode::Char('h'))), Action::PanLeft);
        assert_eq!(action_for(key(KeyCode::Left)), Action::PanLeft);
        assert_eq!(action_for(key(KeyCode::Char('l'))), Action::PanRight);
        assert_eq!(action_for(key(KeyCode::Char('k'))), Action::ZoomIn);
        assert_eq!(action_for(key(KeyCode::Char('j'))), Action::ZoomOut);
        assert_eq!(action_for(key(KeyCode::Char('f'))), Action::Follow);
        assert_eq!(action_for(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(
            action_for(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        assert_eq!(action_for(key(KeyCode::Char('x'))), Action::None);
    }
}
