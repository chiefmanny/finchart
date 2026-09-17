//! Record / replay of the translated `MarketEvent` stream, reusing `market-core`'s
//! text format. Record appends events as they are applied during a live run;
//! replay folds a recorded file back into a fresh `ChartState` — offline, with no
//! engine connection required.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use market_core::{
    parse_events, replay_events, serialize_event, CandleSeries, ChartState, MarketEvent,
};

/// Appends serialized events to a file, one per line.
pub struct Recorder {
    writer: BufWriter<File>,
}

impl Recorder {
    /// Create/truncate the record file (creating parent dirs).
    pub fn create(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        Ok(Self {
            writer: BufWriter::new(File::create(path)?),
        })
    }

    /// Append one event line.
    pub fn write(&mut self, ev: &MarketEvent) -> io::Result<()> {
        writeln!(self.writer, "{}", serialize_event(ev))
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Replay a recorded event file into a fresh `ChartState` (no engine). The
/// `ChartState` parameters must match those used when the events were applied for
/// the state to reproduce exactly.
pub fn replay_file(
    path: &Path,
    tick_size: f64,
    interval_secs: i64,
    heatmap_capacity: usize,
    trade_cap: usize,
) -> io::Result<ChartState> {
    let text = fs::read_to_string(path)?;
    let mut state = ChartState::from_history(
        CandleSeries::new(),
        interval_secs,
        tick_size,
        Some(heatmap_capacity),
        trade_cap,
    );
    replay_events(&mut state, &parse_events(&text));
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pb::{FillNotification, Level2Response, PriceLevelData};
    use crate::translate::{fill_to_trade, level2_to_snapshot};
    use market_core::{ChartState, MarketEvent};

    fn fixture_events() -> Vec<MarketEvent> {
        let resp = Level2Response {
            bids: vec![PriceLevelData {
                price: 99.95,
                size: 5,
                rank: 0,
            }],
            asks: vec![PriceLevelData {
                price: 100.05,
                size: 4,
                rank: 0,
            }],
        };
        let fill = |price: f64, buy: bool| FillNotification {
            symbol: "AAPL".to_string(),
            price,
            size: 2,
            buy,
            partial_fill: false,
            remaining_size: 0,
        };
        vec![
            level2_to_snapshot(&resp, 1),
            MarketEvent::Heartbeat { ts: 1 },
            fill_to_trade(&fill(100.0, true), 2, false),
            fill_to_trade(&fill(100.05, false), 3, false),
            MarketEvent::Heartbeat { ts: 3 },
        ]
    }

    fn fresh() -> ChartState {
        ChartState::from_history(market_core::CandleSeries::new(), 60, 0.05, Some(64), 32)
    }

    #[test]
    fn recorder_appends_one_line_per_event() {
        let dir = std::env::temp_dir().join(format!("finchart_me_lines_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("events.txt");

        let evs = fixture_events();
        let mut rec = Recorder::create(&file).unwrap();
        for e in &evs {
            rec.write(e).unwrap();
        }
        rec.flush().unwrap();

        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), evs.len());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn record_then_replay_matches_direct_apply() {
        let evs = fixture_events();

        // Direct application.
        let mut direct = fresh();
        for e in &evs {
            direct.apply(e);
        }

        // Record to disk, then replay a fresh state from the file (no engine).
        let dir = std::env::temp_dir().join(format!("finchart_me_rr_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("events.txt");
        let mut rec = Recorder::create(&file).unwrap();
        for e in &evs {
            rec.write(e).unwrap();
        }
        rec.flush().unwrap();

        let replayed = replay_file(&file, 0.05, 60, 64, 32).unwrap();
        assert_eq!(direct, replayed);
        std::fs::remove_dir_all(&dir).ok();
    }
}
