//! Event recording (`finchart record`): run the synthetic [`MarketEvent`] source
//! and write the stream to a text file that `finchart replay` (or the browser
//! player) folds back through a [`ChartState`] — the same neutral path every
//! source uses.

use std::fs;
use std::io;
use std::path::Path;

use market_core::{events, serialize_events, DepthConfig, EventConfig, LiveConfig, MarketEvent};

const TICK_SIZE: f64 = 0.05;
const INTERVAL: i64 = 60;
const RECORD_TRADES: usize = 400;

/// A fixed, deterministic synthetic event source config.
fn feed_config() -> EventConfig {
    EventConfig {
        live: LiveConfig {
            seed: 42,
            start_ts: 1_600_000_000,
            start_price: 100.0,
            interval_secs: INTERVAL,
            ticks_per_candle: 30,
            volatility: 0.0015,
            drift: 0.0002,
            volume_per_tick: 40.0,
        },
        depth: DepthConfig {
            seed: 99,
            tick_size: TICK_SIZE,
            levels: 12,
            base_size: 50.0,
            size_jitter: 0.5,
            recenter_band: 0.5,
            updates_per_step: 6,
        },
        depth_every: 5,
    }
}

/// Collect events spanning `trades` trades (the seeding snapshot plus the
/// interleaved depth/heartbeat events come along for free).
fn collect_events(trades: usize) -> Vec<MarketEvent> {
    let mut out = Vec::new();
    let mut seen = 0;
    for ev in events(&feed_config()) {
        let is_trade = matches!(ev, MarketEvent::Trade(_));
        out.push(ev);
        if is_trade {
            seen += 1;
            if seen >= trades {
                break;
            }
        }
    }
    out
}

/// The `#` header `record` writes first, so `finchart replay` can pick up the
/// candle interval and tick size the stream was generated with.
pub(crate) fn header() -> String {
    format!("# finchart interval={INTERVAL} tick={TICK_SIZE}\n")
}

/// Record the synthetic event stream to `path` (one event per line); returns the
/// number of events written.
pub(crate) fn record(path: &Path) -> io::Result<usize> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let evs = collect_events(RECORD_TRADES);
    fs::write(path, header() + &serialize_events(&evs))?;
    Ok(evs.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_core::{parse_events, replay_events, CandleSeries, ChartState};

    use crate::player::Player;

    /// Fold events into a fresh state with the recording's own settings.
    fn fold(evs: &[MarketEvent]) -> ChartState {
        let mut state =
            ChartState::from_history(CandleSeries::new(), INTERVAL, TICK_SIZE, Some(8), 8);
        replay_events(&mut state, evs);
        state
    }

    #[test]
    fn recorded_then_replayed_matches_direct_fold() {
        let evs = collect_events(300);
        // Round-trip through the text format and re-fold.
        let parsed = parse_events(&serialize_events(&evs));
        assert_eq!(fold(&evs), fold(&parsed));
    }

    #[test]
    fn a_recording_replays_with_the_settings_in_its_header() {
        let dir = std::env::temp_dir().join(format!("finchart_header_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("events.txt");
        record(&file).unwrap();

        // No flags: interval and tick must come from the header.
        let mut player = Player::open(&file, None, None, None, true).unwrap();
        player.seek(player.duration());
        let direct = fold(&collect_events(RECORD_TRADES));
        assert_eq!(player.state().series(), direct.series());
        assert_eq!(player.state().book(), direct.book());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn record_writes_a_header_and_every_event() {
        let dir = std::env::temp_dir().join(format!("finchart_feed_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("events.txt");

        let n = record(&file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with(&header()), "missing header");
        let events = parse_events(&text);
        assert_eq!(events.len(), n);
        assert_eq!(events, collect_events(RECORD_TRADES));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_events_starts_with_seed_and_covers_requested_trades() {
        let evs = collect_events(50);
        assert!(matches!(evs[0], market_core::MarketEvent::DepthSnapshot(_)));
        let trades = evs
            .iter()
            .filter(|e| matches!(e, market_core::MarketEvent::Trade(_)))
            .count();
        assert_eq!(trades, 50);
    }
}
