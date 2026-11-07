//! Follow a recording as it is written (`finchart replay --follow`), like
//! `tail -f`: read what is there, then pick up each line appended after it.
//!
//! Only complete lines are used: a writer may be mid-line when we read, so the
//! unterminated rest is held until its newline arrives.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use market_core::{parse_event, MarketEvent};

/// What a [`Tail::poll`] found.
#[derive(Debug, PartialEq)]
pub enum Polled {
    /// Events from lines completed since the last poll (possibly none).
    Events(Vec<MarketEvent>),
    /// The file got shorter (truncated or replaced): reload it from the start.
    Reset,
}

/// A recording being followed.
pub struct Tail {
    path: PathBuf,
    /// Bytes consumed so far, including the held partial line.
    offset: u64,
    /// The unterminated last line, waiting for its newline.
    partial: String,
}

impl Tail {
    /// Open `path` and return its complete lines as text (for parsing, header
    /// included); anything after the last newline is held for later.
    pub fn open(path: &Path) -> io::Result<(Self, String)> {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        let cut = text.rfind('\n').map_or(0, |i| i + 1);
        let tail = Self {
            path: path.to_path_buf(),
            offset: bytes.len() as u64,
            partial: text[cut..].to_string(),
        };
        Ok((tail, text[..cut].to_string()))
    }

    /// Read whatever was appended since the last poll.
    pub fn poll(&mut self) -> io::Result<Polled> {
        let len = std::fs::metadata(&self.path)?.len();
        if len < self.offset {
            return Ok(Polled::Reset);
        }
        if len == self.offset {
            return Ok(Polled::Events(Vec::new()));
        }
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut fresh = Vec::new();
        file.take(len - self.offset).read_to_end(&mut fresh)?;
        self.offset += fresh.len() as u64;
        self.partial.push_str(&String::from_utf8_lossy(&fresh));

        let cut = self.partial.rfind('\n').map_or(0, |i| i + 1);
        let complete: String = self.partial.drain(..cut).collect();
        Ok(Polled::Events(
            complete.lines().filter_map(parse_event).collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_core::MarketEvent;
    use std::fs::OpenOptions;
    use std::io::Write;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("finchart_tail_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("session.txt")
    }

    fn append(path: &Path, text: &str) {
        let mut f = OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn heartbeats(evs: &[MarketEvent]) -> Vec<i64> {
        evs.iter().map(MarketEvent::ts).collect()
    }

    #[test]
    fn opening_reads_complete_lines_and_holds_back_a_partial_one() {
        let path = scratch("open");
        std::fs::write(&path, "# finchart interval=1 tick=0.01\nH 1\nH 2\nH 3").unwrap();
        let (tail, text) = Tail::open(&path).unwrap();
        assert_eq!(text, "# finchart interval=1 tick=0.01\nH 1\nH 2\n");
        drop(tail);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn polling_returns_only_newly_completed_lines() {
        let path = scratch("poll");
        std::fs::write(&path, "H 1\nH 2").unwrap();
        let (mut tail, _) = Tail::open(&path).unwrap();
        assert_eq!(
            tail.poll().unwrap(),
            Polled::Events(vec![]),
            "nothing new yet"
        );

        append(&path, "0\nH 3"); // completes "H 20", starts "H 3"
        let Polled::Events(evs) = tail.poll().unwrap() else {
            panic!("expected events");
        };
        assert_eq!(heartbeats(&evs), vec![20]);

        append(&path, "\n# comment\nH 4\n");
        let Polled::Events(evs) = tail.poll().unwrap() else {
            panic!("expected events");
        };
        assert_eq!(heartbeats(&evs), vec![3, 4], "comments are skipped");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_truncated_or_replaced_file_asks_for_a_reload() {
        let path = scratch("reset");
        std::fs::write(&path, "H 1\nH 2\n").unwrap();
        let (mut tail, _) = Tail::open(&path).unwrap();
        std::fs::write(&path, "H 9\n").unwrap(); // a new, shorter recording
        assert_eq!(tail.poll().unwrap(), Polled::Reset);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
