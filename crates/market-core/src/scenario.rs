//! Deterministic market-structure scenarios for the liquidity heatmap.
//!
//! Each scenario produces a fixed sequence of [`ScenarioStep`]s — a book snapshot
//! plus executed trades per time bucket — scripting clear structure (walls, pulls,
//! sweeps, reloads) so the heatmap is interpretable. Structure is deterministic; a
//! seeded RNG only adds small size jitter to the baseline ladder.

use crate::depth::{DepthLevel, DepthSnapshot};
use crate::live::{Side, Tick};
use crate::rng::SplitMix64;

/// A market-structure scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioKind {
    /// A large persistent ask wall above the mid.
    StaticAskWall,
    /// A large persistent bid wall below the mid.
    StaticBidWall,
    /// An ask wall that is removed halfway through.
    AskWallPulled,
    /// A bid wall that is removed halfway through.
    BidWallPulled,
    /// Mid ramps up through a fixed ask ladder, consuming it (buy trades).
    SweepAsk,
    /// A bid level repeatedly drained and refilled (iceberg-like; sell trades).
    BidReload,
}

/// Parameters for [`scenario`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioConfig {
    pub kind: ScenarioKind,
    pub seed: u64,
    pub tick_size: f64,
    /// Ladder levels per side.
    pub levels: usize,
    /// Baseline resting size per level.
    pub base_size: f64,
    /// Base mid price.
    pub mid: f64,
    /// Number of time-bucket steps.
    pub steps: usize,
    /// Size of the wall / reload level.
    pub wall_size: f64,
    /// Offset (ticks from mid) where the wall / reload level sits (1..=levels).
    pub wall_offset: usize,
    pub start_ts: i64,
}

/// One time bucket of a scenario: the book at that instant plus executed trades.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioStep {
    pub ts: i64,
    pub mid: f64,
    pub snapshot: DepthSnapshot,
    pub trades: Vec<Tick>,
}

/// Generate the full deterministic step sequence for `cfg`.
pub fn scenario(cfg: &ScenarioConfig) -> Vec<ScenarioStep> {
    let mut rng = SplitMix64::new(cfg.seed);
    let pull = cfg.steps / 2;
    let mut steps = Vec::with_capacity(cfg.steps);

    for t in 0..cfg.steps {
        let ts = cfg.start_ts + t as i64;
        let mid = match cfg.kind {
            // Sweep ramps the mid up one tick per step.
            ScenarioKind::SweepAsk => cfg.mid + t as f64 * cfg.tick_size,
            _ => cfg.mid,
        };

        let mut bids = ladder_side(&mut rng, cfg, mid, -1);
        let mut asks = ladder_side(&mut rng, cfg, mid, 1);
        let mut trades = Vec::new();

        match cfg.kind {
            ScenarioKind::StaticAskWall => set_wall(&mut asks, cfg.wall_offset, cfg.wall_size),
            ScenarioKind::StaticBidWall => set_wall(&mut bids, cfg.wall_offset, cfg.wall_size),
            ScenarioKind::AskWallPulled => {
                if t < pull {
                    set_wall(&mut asks, cfg.wall_offset, cfg.wall_size);
                }
            }
            ScenarioKind::BidWallPulled => {
                if t < pull {
                    set_wall(&mut bids, cfg.wall_offset, cfg.wall_size);
                }
            }
            ScenarioKind::SweepAsk => {
                // Asks are anchored at the base mid and consumed as the mid rises.
                asks = (1..=cfg.levels)
                    .map(|i| cfg.mid + i as f64 * cfg.tick_size)
                    .filter(|&p| p > mid)
                    .map(|price| DepthLevel {
                        price,
                        size: jitter(&mut rng, cfg),
                    })
                    .collect();
                trades.push(Tick {
                    ts,
                    price: mid,
                    size: cfg.base_size,
                    side: Some(Side::Buy),
                });
            }
            ScenarioKind::BidReload => {
                // Iceberg: the reload level is large on even steps, thin on odd.
                let size = if t % 2 == 0 {
                    cfg.wall_size
                } else {
                    cfg.base_size * 0.3
                };
                set_wall(&mut bids, cfg.wall_offset, size);
                trades.push(Tick {
                    ts,
                    price: mid - cfg.wall_offset as f64 * cfg.tick_size,
                    size: cfg.base_size,
                    side: Some(Side::Sell),
                });
            }
        }

        steps.push(ScenarioStep {
            ts,
            mid,
            snapshot: DepthSnapshot { ts, bids, asks },
            trades,
        });
    }

    steps
}

/// A positive jittered resting size in `[0.7, 1.3] * base_size`.
fn jitter(rng: &mut SplitMix64, cfg: &ScenarioConfig) -> f64 {
    cfg.base_size * (0.7 + 0.6 * rng.next_f64())
}

/// Build one side's ladder: `dir = -1` for bids (below mid), `+1` for asks.
fn ladder_side(rng: &mut SplitMix64, cfg: &ScenarioConfig, mid: f64, dir: i64) -> Vec<DepthLevel> {
    (1..=cfg.levels)
        .map(|i| DepthLevel {
            price: mid + dir as f64 * i as f64 * cfg.tick_size,
            size: jitter(rng, cfg),
        })
        .collect()
}

/// Override the size of the level at `offset` ticks from the mid (1-based).
fn set_wall(side: &mut [DepthLevel], offset: usize, size: f64) {
    if let Some(level) = side.get_mut(offset.saturating_sub(1)) {
        level.size = size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::Side;

    fn cfg(kind: ScenarioKind) -> ScenarioConfig {
        ScenarioConfig {
            kind,
            seed: 7,
            tick_size: 0.05,
            levels: 12,
            base_size: 10.0,
            mid: 100.0,
            steps: 40,
            wall_size: 200.0,
            wall_offset: 5,
            start_ts: 0,
        }
    }

    fn max_ask_size(s: &ScenarioStep) -> f64 {
        s.snapshot.asks.iter().map(|l| l.size).fold(0.0, f64::max)
    }
    fn max_bid_size(s: &ScenarioStep) -> f64 {
        s.snapshot.bids.iter().map(|l| l.size).fold(0.0, f64::max)
    }

    #[test]
    fn scenario_is_deterministic() {
        assert_eq!(
            scenario(&cfg(ScenarioKind::StaticAskWall)),
            scenario(&cfg(ScenarioKind::StaticAskWall))
        );
        assert_eq!(
            scenario(&cfg(ScenarioKind::SweepAsk)),
            scenario(&cfg(ScenarioKind::SweepAsk))
        );
    }

    #[test]
    fn produces_requested_step_count() {
        assert_eq!(scenario(&cfg(ScenarioKind::StaticBidWall)).len(), 40);
    }

    #[test]
    fn static_ask_wall_sits_at_expected_price_every_step() {
        let c = cfg(ScenarioKind::StaticAskWall);
        let wall_price = c.mid + c.wall_offset as f64 * c.tick_size;
        for s in scenario(&c) {
            let big = s
                .snapshot
                .asks
                .iter()
                .max_by(|a, b| a.size.total_cmp(&b.size))
                .unwrap();
            assert!(big.size >= c.wall_size * 0.9, "wall missing");
            assert!(
                (big.price - wall_price).abs() < 1e-9,
                "wall at {}",
                big.price
            );
        }
    }

    #[test]
    fn static_bid_wall_sits_at_expected_price() {
        let c = cfg(ScenarioKind::StaticBidWall);
        let wall_price = c.mid - c.wall_offset as f64 * c.tick_size;
        let steps = scenario(&c);
        let big = steps[0]
            .snapshot
            .bids
            .iter()
            .max_by(|a, b| a.size.total_cmp(&b.size))
            .unwrap();
        assert!(big.size >= c.wall_size * 0.9);
        assert!((big.price - wall_price).abs() < 1e-9);
    }

    #[test]
    fn ask_wall_is_pulled_midway() {
        let c = cfg(ScenarioKind::AskWallPulled);
        let steps = scenario(&c);
        assert!(
            max_ask_size(&steps[0]) >= c.wall_size * 0.9,
            "wall missing early"
        );
        assert!(
            max_ask_size(steps.last().unwrap()) < c.wall_size * 0.5,
            "wall not pulled"
        );
    }

    #[test]
    fn bid_wall_is_pulled_midway() {
        let c = cfg(ScenarioKind::BidWallPulled);
        let steps = scenario(&c);
        assert!(max_bid_size(&steps[0]) >= c.wall_size * 0.9);
        assert!(max_bid_size(steps.last().unwrap()) < c.wall_size * 0.5);
    }

    #[test]
    fn sweep_ramps_mid_and_emits_buy_trades() {
        let steps = scenario(&cfg(ScenarioKind::SweepAsk));
        for w in steps.windows(2) {
            assert!(w[1].mid > w[0].mid, "mid not ramping up");
        }
        assert!(steps.iter().all(|s| !s.trades.is_empty()), "no trades");
        assert!(
            steps
                .iter()
                .all(|s| s.trades.iter().all(|t| t.side == Some(Side::Buy))),
            "expected buy-initiated trades"
        );
    }

    #[test]
    fn bid_reload_oscillates_and_emits_sells() {
        let c = cfg(ScenarioKind::BidReload);
        let reload_price = c.mid - c.wall_offset as f64 * c.tick_size;
        let steps = scenario(&c);
        let sizes: Vec<f64> = steps
            .iter()
            .map(|s| {
                s.snapshot
                    .bids
                    .iter()
                    .find(|l| (l.price - reload_price).abs() < 1e-9)
                    .map(|l| l.size)
                    .unwrap_or(0.0)
            })
            .collect();
        let (mn, mx) = sizes.iter().fold((f64::INFINITY, 0.0_f64), |(mn, mx), &v| {
            (mn.min(v), mx.max(v))
        });
        assert!(mx - mn > 1.0, "reload level did not oscillate");
        assert!(
            steps
                .iter()
                .any(|s| s.trades.iter().any(|t| t.side == Some(Side::Sell))),
            "expected sell-initiated trades"
        );
    }

    #[test]
    fn all_scenarios_stay_non_crossing() {
        for kind in [
            ScenarioKind::StaticAskWall,
            ScenarioKind::StaticBidWall,
            ScenarioKind::AskWallPulled,
            ScenarioKind::BidWallPulled,
            ScenarioKind::SweepAsk,
            ScenarioKind::BidReload,
        ] {
            for s in scenario(&cfg(kind)) {
                if s.snapshot.bids.is_empty() || s.snapshot.asks.is_empty() {
                    continue;
                }
                let mb = s
                    .snapshot
                    .bids
                    .iter()
                    .map(|l| l.price)
                    .fold(f64::NEG_INFINITY, f64::max);
                let ma = s
                    .snapshot
                    .asks
                    .iter()
                    .map(|l| l.price)
                    .fold(f64::INFINITY, f64::min);
                assert!(mb < ma, "crossed in {kind:?}: {mb} >= {ma}");
            }
        }
    }
}
