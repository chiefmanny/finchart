//! `examples-live`: live synthetic market replay with incremental candle updates.
//!
//! Without args: an interactive real-time viewer that follows the forming candle,
//! redrawing at a fixed frame rate (`--fps N`, default 12). Quit with `q`/`Esc`.
//! With `--snapshot <dir>`: writes deterministic before/mid/after frames + metrics.
//!
//! Synthetic tick timestamps drive all candle logic deterministically; wall-clock
//! time only paces the visual frame rate. No async.

mod replay;
mod snapshot;

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, queue};

use backend_terminal::ColorMode;
use replay::{Replay, ReplayConfig};

/// Ticks ingested per rendered frame (a candle of 30 ticks forms over ~15 frames).
const TICKS_PER_FRAME: usize = 2;
const DEFAULT_FPS: u64 = 12;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if let Some(pos) = args.iter().position(|a| a == "--snapshot") {
        let dir = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("artifacts/live");
        let paths = snapshot::write_all(std::path::Path::new(dir))?;
        println!("wrote {} live snapshot artifacts:", paths.len());
        for p in paths {
            println!("  {}", p.display());
        }
        return Ok(());
    }

    run_interactive(parse_fps(&args))
}

/// Parse `--fps N`; falls back to the default on missing/invalid input.
fn parse_fps(args: &[String]) -> u64 {
    args.iter()
        .position(|a| a == "--fps")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_FPS)
}

/// Whether a key event should quit the viewer.
fn is_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn run_interactive(fps: u64) -> io::Result<()> {
    let mut replay = Replay::new(&ReplayConfig::default());
    let frame_dur = Duration::from_millis(1000 / fps.max(1));

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let _guard = TerminalGuard;

    let mut next_frame = Instant::now() + frame_dur;
    let mut last_update_us = 0.0;
    let mut last_render_us = 0.0;
    loop {
        let (cols, rows) = terminal::size().unwrap_or((120, 30));
        // Status carries the forming candle state plus the previous frame's timings.
        let title = format!(
            "{} · upd {last_update_us:.2}µs · ren {last_render_us:.1}µs · q quit",
            replay.status()
        );

        let t = Instant::now();
        let chart = replay.render(cols as usize, rows as usize, ColorMode::Ansi, &title);
        last_render_us = t.elapsed().as_secs_f64() * 1e6;

        queue!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
        write!(stdout, "{}", chart.replace('\n', "\r\n"))?;
        stdout.flush()?;

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

/// Restores the terminal on drop, even on panic.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn quit_keys_are_recognized() {
        assert!(is_quit(key(KeyCode::Char('q'))));
        assert!(is_quit(key(KeyCode::Esc)));
        assert!(is_quit(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_quit(key(KeyCode::Char('x'))));
        assert!(!is_quit(key(KeyCode::Char('c')))); // plain 'c' is not quit
    }

    #[test]
    fn fps_parsing_falls_back_to_default() {
        let d = DEFAULT_FPS;
        assert_eq!(parse_fps(&["bin".into()]), d);
        assert_eq!(parse_fps(&["bin".into(), "--fps".into(), "30".into()]), 30);
        assert_eq!(parse_fps(&["bin".into(), "--fps".into(), "0".into()]), d);
        assert_eq!(parse_fps(&["bin".into(), "--fps".into(), "x".into()]), d);
    }
}
