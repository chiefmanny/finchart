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

use crossterm::event::{self, Event};

use backend_terminal::ColorMode;
use replay::{Replay, ReplayConfig};

use crate::term::{is_quit, Screen};

/// Ticks ingested per rendered frame (a candle of 30 ticks forms over ~15 frames).
const TICKS_PER_FRAME: usize = 2;

/// Run the interactive live view at `fps` frames per second until a quit key.
pub fn run(fps: u64) -> io::Result<()> {
    let mut replay = Replay::new(&ReplayConfig::default());
    let frame_dur = Duration::from_millis(1000 / fps.max(1));
    let mut screen = Screen::enter()?;

    let mut next_frame = Instant::now() + frame_dur;
    let mut last_update_us = 0.0;
    let mut last_render_us = 0.0;
    loop {
        let (cols, rows) = screen.size();
        // Status carries the forming candle state plus the previous frame's timings.
        let title = format!(
            "{} · upd {last_update_us:.2}µs · ren {last_render_us:.1}µs · q quit",
            replay.status()
        );

        let t = Instant::now();
        let chart = replay.render(cols, rows, ColorMode::Ansi, &title);
        last_render_us = t.elapsed().as_secs_f64() * 1e6;
        screen.paint(&chart, (cols, rows))?;

        // Wait until the next frame deadline, staying responsive to keys.
        let timeout = next_frame.saturating_duration_since(Instant::now());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if is_quit(key) {
                    break;
                }
            }
        } else {
            // Deadline reached with no input: advance the simulation one frame.
            let t = Instant::now();
            replay.advance(TICKS_PER_FRAME);
            last_update_us = t.elapsed().as_secs_f64() * 1e6;
            next_frame += frame_dur;
        }
    }

    Ok(())
}
