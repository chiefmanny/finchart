//! Terminal plumbing shared by the interactive subcommands: raw mode and the
//! alternate screen (restored on drop, even on panic), in-place repainting, and
//! the common quit keys.

use std::io::{self, Stdout, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, queue};

/// The terminal while an interactive view owns it.
pub struct Screen {
    stdout: Stdout,
    painted_size: Option<(u16, u16)>,
}

impl Screen {
    /// Enter raw mode and the alternate screen, hiding the cursor.
    pub fn enter() -> io::Result<Self> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        if let Err(e) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(e);
        }
        Ok(Self {
            stdout,
            painted_size: None,
        })
    }

    /// The current size as `(columns, rows)`.
    pub fn size(&self) -> (usize, usize) {
        let (cols, rows) = terminal::size().unwrap_or((120, 30));
        (cols as usize, rows as usize)
    }

    /// Paint a full-screen frame rendered for `size`. Frames fill the whole
    /// screen, so they overwrite in place; a full clear (which can flash a blank
    /// frame mid-write) happens only when the size changes.
    pub fn paint(&mut self, frame: &str, size: (usize, usize)) -> io::Result<()> {
        let size = (size.0 as u16, size.1 as u16);
        if self.painted_size != Some(size) {
            queue!(self.stdout, Clear(ClearType::All))?;
            self.painted_size = Some(size);
        }
        queue!(self.stdout, MoveTo(0, 0))?;
        // Raw mode needs explicit carriage returns between rows.
        write!(self.stdout, "{}", frame.replace('\n', "\r\n"))?;
        self.stdout.flush()
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.stdout, LeaveAlternateScreen, Show);
    }
}

/// Whether a key quits an interactive view: `q`, `Esc` or `Ctrl-C`.
pub fn is_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
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
}
