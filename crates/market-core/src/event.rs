//! Neutral market events — the source-agnostic ingestion boundary.
//!
//! Synthetic generators, scenarios, file replay, and (future) gRPC/exchange
//! adapters all produce `MarketEvent`s, which [`crate::ChartState`] folds into a
//! candle series + order book + heatmap. A tiny line-based text format (no
//! dependencies) supports deterministic record/replay.

use crate::depth::{BookSide, DepthLevel, DepthSnapshot, DepthUpdate};
use crate::live::{Side, Tick};

/// A single neutral market event.
#[derive(Debug, Clone, PartialEq)]
pub enum MarketEvent {
    /// An executed trade.
    Trade(Tick),
    /// A full-book replacement.
    DepthSnapshot(DepthSnapshot),
    /// A single level update (`size == 0` deletes).
    DepthUpdate(DepthUpdate),
    /// A time tick: advances heatmap recording / recency. No book change.
    Heartbeat { ts: i64 },
}

/// Serialize one event to a single text line (no trailing newline). Uses `f64`'s
/// shortest round-trip `Display`, so `parse_event` recovers it exactly.
pub fn serialize_event(ev: &MarketEvent) -> String {
    match ev {
        MarketEvent::Trade(t) => {
            format!("T {} {} {} {}", t.ts, t.price, t.size, side_char(t.side))
        }
        MarketEvent::DepthUpdate(u) => {
            format!(
                "U {} {} {} {}",
                u.ts,
                book_side_char(u.side),
                u.price,
                u.size
            )
        }
        MarketEvent::Heartbeat { ts } => format!("H {ts}"),
        MarketEvent::DepthSnapshot(s) => {
            format!("S {} {}|{}", s.ts, levels_str(&s.bids), levels_str(&s.asks))
        }
    }
}

/// Serialize a stream of events to text, one per line (trailing newline).
pub fn serialize_events(events: &[MarketEvent]) -> String {
    let mut out = String::new();
    for ev in events {
        out.push_str(&serialize_event(ev));
        out.push('\n');
    }
    out
}

/// Parse an event stream, skipping blank lines and silently dropping any
/// unparseable line.
pub fn parse_events(text: &str) -> Vec<MarketEvent> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_event)
        .collect()
}

/// Parse one event line, or `None` if malformed.
pub fn parse_event(line: &str) -> Option<MarketEvent> {
    let kind = line.chars().next()?;
    match kind {
        'T' => {
            let mut it = line.split(' ').skip(1);
            let ts = it.next()?.parse().ok()?;
            let price = it.next()?.parse().ok()?;
            let size = it.next()?.parse().ok()?;
            let side = parse_side(it.next()?)?;
            Some(MarketEvent::Trade(Tick {
                ts,
                price,
                size,
                side,
            }))
        }
        'U' => {
            let mut it = line.split(' ').skip(1);
            let ts = it.next()?.parse().ok()?;
            let side = parse_book_side(it.next()?)?;
            let price = it.next()?.parse().ok()?;
            let size = it.next()?.parse().ok()?;
            Some(MarketEvent::DepthUpdate(DepthUpdate {
                ts,
                side,
                price,
                size,
            }))
        }
        'H' => {
            let ts = line.split(' ').nth(1)?.parse().ok()?;
            Some(MarketEvent::Heartbeat { ts })
        }
        'S' => {
            // "S <ts> <bidlevels>|<asklevels>"
            let mut it = line.splitn(3, ' ');
            it.next()?; // 'S'
            let ts = it.next()?.parse().ok()?;
            let (bid_str, ask_str) = it.next()?.split_once('|')?;
            Some(MarketEvent::DepthSnapshot(DepthSnapshot {
                ts,
                bids: parse_levels(bid_str)?,
                asks: parse_levels(ask_str)?,
            }))
        }
        _ => None,
    }
}

fn side_char(side: Option<Side>) -> char {
    match side {
        Some(Side::Buy) => 'B',
        Some(Side::Sell) => 'S',
        None => '-',
    }
}

fn parse_side(s: &str) -> Option<Option<Side>> {
    match s {
        "B" => Some(Some(Side::Buy)),
        "S" => Some(Some(Side::Sell)),
        "-" => Some(None),
        _ => None,
    }
}

fn book_side_char(side: BookSide) -> char {
    match side {
        BookSide::Bid => 'B',
        BookSide::Ask => 'A',
    }
}

fn parse_book_side(s: &str) -> Option<BookSide> {
    match s {
        "B" => Some(BookSide::Bid),
        "A" => Some(BookSide::Ask),
        _ => None,
    }
}

fn levels_str(levels: &[DepthLevel]) -> String {
    levels
        .iter()
        .map(|l| format!("{}:{}", l.price, l.size))
        .collect::<Vec<_>>()
        .join(";")
}

fn parse_levels(s: &str) -> Option<Vec<DepthLevel>> {
    if s.is_empty() {
        return Some(Vec::new());
    }
    s.split(';')
        .map(|pair| {
            let (p, sz) = pair.split_once(':')?;
            Some(DepthLevel {
                price: p.parse().ok()?,
                size: sz.parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depth::{BookSide, DepthLevel};
    use crate::live::Side;

    fn round_trips(ev: &MarketEvent) {
        let line = serialize_event(ev);
        assert!(
            !line.contains('\n'),
            "event line must be single-line: {line}"
        );
        assert_eq!(
            parse_event(&line).as_ref(),
            Some(ev),
            "round-trip failed: {line}"
        );
    }

    #[test]
    fn trade_round_trips_for_each_side() {
        for side in [Some(Side::Buy), Some(Side::Sell), None] {
            round_trips(&MarketEvent::Trade(Tick {
                ts: 5,
                price: 100.25,
                size: 3.5,
                side,
            }));
        }
    }

    #[test]
    fn depth_update_round_trips_for_each_side() {
        for side in [BookSide::Bid, BookSide::Ask] {
            round_trips(&MarketEvent::DepthUpdate(DepthUpdate {
                ts: 7,
                side,
                price: 99.95,
                size: 12.0,
            }));
        }
    }

    #[test]
    fn heartbeat_round_trips() {
        round_trips(&MarketEvent::Heartbeat { ts: 1_600_000_123 });
    }

    #[test]
    fn depth_snapshot_round_trips() {
        round_trips(&MarketEvent::DepthSnapshot(DepthSnapshot {
            ts: 9,
            bids: vec![
                DepthLevel {
                    price: 99.9,
                    size: 5.0,
                },
                DepthLevel {
                    price: 99.8,
                    size: 3.0,
                },
            ],
            asks: vec![DepthLevel {
                price: 100.1,
                size: 4.0,
            }],
        }));
    }

    #[test]
    fn depth_snapshot_with_empty_side_round_trips() {
        round_trips(&MarketEvent::DepthSnapshot(DepthSnapshot {
            ts: 2,
            bids: vec![],
            asks: vec![DepthLevel {
                price: 100.1,
                size: 4.0,
            }],
        }));
        round_trips(&MarketEvent::DepthSnapshot(DepthSnapshot {
            ts: 3,
            bids: vec![],
            asks: vec![],
        }));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(parse_event(""), None);
        assert_eq!(parse_event("Z 1 2 3"), None);
        assert_eq!(parse_event("T 1 notanumber 3 B"), None);
        assert_eq!(parse_event("H"), None);
    }

    #[test]
    fn event_stream_round_trips_and_skips_blank_lines() {
        let events = vec![
            MarketEvent::Trade(Tick {
                ts: 1,
                price: 100.25,
                size: 2.0,
                side: Some(Side::Buy),
            }),
            MarketEvent::DepthSnapshot(DepthSnapshot {
                ts: 1,
                bids: vec![DepthLevel {
                    price: 99.9,
                    size: 5.0,
                }],
                asks: vec![DepthLevel {
                    price: 100.1,
                    size: 4.0,
                }],
            }),
            MarketEvent::DepthUpdate(DepthUpdate {
                ts: 2,
                side: BookSide::Ask,
                price: 100.15,
                size: 0.0,
            }),
            MarketEvent::Heartbeat { ts: 2 },
        ];
        let text = serialize_events(&events);
        assert_eq!(parse_events(&text), events);
        // Blank lines are ignored.
        assert_eq!(parse_events(&format!("\n{text}\n\n")), events);
    }
}
