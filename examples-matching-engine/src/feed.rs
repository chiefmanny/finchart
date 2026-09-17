//! Async gRPC producers → a drainable channel of `MarketEvent`s.
//!
//! Three background tokio tasks translate the matching-engine streams into neutral
//! events: the `Subscribe` fill stream → `Trade`s, a `GetLevel2` poll loop →
//! `DepthSnapshot`s, and a heartbeat ticker → `Heartbeat`s. The sync render loop
//! drains [`ChannelFeed`] with `try_recv`, so it never touches gRPC or tokio — the
//! transport is swappable behind this seam. The gRPC tasks require a live server
//! and are exercised by integration/manual runs; the channel seam is unit-tested.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use market_core::MarketEvent;
use tokio::runtime::Runtime;
use tokio::time::interval;
use tonic::transport::Channel;

use crate::pb::level2_request::Side;
use crate::pb::matching_engine_client::MatchingEngineClient;
use crate::pb::{Level2Request, Symbol};
use crate::translate::{fill_to_trade, level2_to_snapshot};

/// Connection + cadence settings for the matching-engine feed.
#[derive(Debug, Clone)]
pub struct FeedConfig {
    pub endpoint: String,
    pub symbol: String,
    pub depth: u32,
    pub depth_interval_ms: u64,
    pub heartbeat_ms: u64,
}

/// Adapter receive time as epoch seconds. The documented proto has no source
/// timestamp, so all events are stamped on arrival (see the plan's §7 limitation).
fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The consume side: non-blocking drain of whatever the producers have queued.
pub struct ChannelFeed {
    rx: Receiver<MarketEvent>,
}

impl ChannelFeed {
    /// Move all currently-available events into `out`; returns how many.
    pub fn drain(&self, out: &mut Vec<MarketEvent>) -> usize {
        let mut n = 0;
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
            n += 1;
        }
        n
    }
}

/// A live feed: the background async runtime (kept alive) + the drainable channel.
pub struct FeedHandle {
    feed: ChannelFeed,
    _runtime: Runtime,
}

impl FeedHandle {
    pub fn drain(&self, out: &mut Vec<MarketEvent>) -> usize {
        self.feed.drain(out)
    }
}

/// Connect to the matching engine and spawn the trade/depth/heartbeat producers.
/// Fails fast if the endpoint is unreachable.
pub fn connect(cfg: FeedConfig) -> Result<FeedHandle, Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // Initial connect is blocking so an unreachable endpoint is reported now.
    let client = runtime.block_on(MatchingEngineClient::connect(cfg.endpoint.clone()))?;
    let (tx, rx) = mpsc::channel();

    runtime.spawn(trade_task(client.clone(), cfg.symbol.clone(), tx.clone()));
    runtime.spawn(depth_task(
        client.clone(),
        cfg.symbol.clone(),
        cfg.depth,
        cfg.depth_interval_ms,
        tx.clone(),
    ));
    runtime.spawn(heartbeat_task(cfg.heartbeat_ms, tx));

    Ok(FeedHandle {
        feed: ChannelFeed { rx },
        _runtime: runtime,
    })
}

/// Subscribe stream → `Trade` events.
async fn trade_task(
    mut client: MatchingEngineClient<Channel>,
    symbol: String,
    tx: Sender<MarketEvent>,
) {
    let Ok(resp) = client.subscribe(Symbol { symbol }).await else {
        return;
    };
    let mut stream = resp.into_inner();
    while let Ok(Some(fill)) = stream.message().await {
        if tx.send(fill_to_trade(&fill, now_ts())).is_err() {
            break; // consumer gone
        }
    }
}

/// Poll `GetLevel2` on a fixed interval → `DepthSnapshot` events (one per poll).
async fn depth_task(
    mut client: MatchingEngineClient<Channel>,
    symbol: String,
    depth: u32,
    interval_ms: u64,
    tx: Sender<MarketEvent>,
) {
    let mut ticker = interval(Duration::from_millis(interval_ms.max(1)));
    loop {
        ticker.tick().await;
        let req = Level2Request {
            symbol: symbol.clone(),
            side: Side::Both as i32,
            depth: depth as i32,
        };
        let Ok(resp) = client.get_level2(req).await else {
            continue; // transient error; try again next tick
        };
        // The stream yields one snapshot then closes.
        if let Ok(Some(l2)) = resp.into_inner().message().await {
            if tx.send(level2_to_snapshot(&l2, now_ts())).is_err() {
                break;
            }
        }
    }
}

/// Emit a `Heartbeat` on a fixed cadence to drive heatmap/depth-history recording.
async fn heartbeat_task(interval_ms: u64, tx: Sender<MarketEvent>) {
    let mut ticker = interval(Duration::from_millis(interval_ms.max(1)));
    loop {
        ticker.tick().await;
        if tx.send(MarketEvent::Heartbeat { ts: now_ts() }).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn channel_feed_drains_available_events() {
        let (tx, rx) = mpsc::channel();
        let feed = ChannelFeed { rx };
        tx.send(MarketEvent::Heartbeat { ts: 1 }).unwrap();
        tx.send(MarketEvent::Heartbeat { ts: 2 }).unwrap();

        let mut out = Vec::new();
        assert_eq!(feed.drain(&mut out), 2);
        assert_eq!(out.len(), 2);
        // Nothing left afterward.
        assert_eq!(feed.drain(&mut Vec::new()), 0);
    }

    #[test]
    fn channel_feed_is_empty_when_senders_drop() {
        let (tx, rx) = mpsc::channel::<MarketEvent>();
        drop(tx);
        let feed = ChannelFeed { rx };
        assert_eq!(feed.drain(&mut Vec::new()), 0);
    }
}
