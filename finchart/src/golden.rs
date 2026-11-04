//! Golden-file tests: every deterministic frame `finchart` produces is compared
//! byte for byte against a committed fixture in `finchart/tests/golden/`.
//!
//! The snapshot groups call the same writers as `finchart snapshot` and
//! `finchart scenario`, so a fixture is exactly what a user gets (minus
//! `metrics.json`, which carries timings). The replay group renders recorded
//! matching-engine sessions at fixed moments, covering the time-linear axis and
//! the heatmap alignment on real data.
//!
//! After an intentional visual change, re-bless the fixtures and review the diff:
//!
//! ```text
//! FINCHART_BLESS=1 cargo test -p finchart golden
//! git diff --stat finchart/tests/golden
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use backend_terminal::ColorMode;

use crate::bench::snapshot as chart_snapshot;
use crate::cli::parse_time;
use crate::live::{scenarios, snapshot as live_snapshot};
use crate::player::Player;

/// The fixtures directory for one group.
fn fixtures(group: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(group)
}

fn blessing() -> bool {
    std::env::var_os("FINCHART_BLESS").is_some()
}

/// Compare `(name, actual)` pairs against `fixtures(group)/name`, or rewrite the
/// fixtures when blessing. Fails once, listing every mismatch.
fn check(group: &str, files: Vec<(String, String)>) {
    let dir = fixtures(group);
    if blessing() {
        fs::create_dir_all(&dir).unwrap();
        for (name, actual) in &files {
            fs::write(dir.join(name), actual).unwrap();
        }
        return;
    }
    let problems: Vec<String> = files
        .iter()
        .filter_map(|(name, actual)| {
            let path = dir.join(name);
            match fs::read_to_string(&path) {
                Err(_) => Some(format!("{group}/{name}: missing fixture")),
                Ok(expected) if expected == *actual => None,
                Ok(expected) => Some(format!(
                    "{group}/{name}: {}",
                    first_difference(&expected, actual)
                )),
            }
        })
        .collect();
    assert!(
        problems.is_empty(),
        "golden mismatch:\n  {}\n\nIf the change is intended, re-bless and review the diff:\n  \
         FINCHART_BLESS=1 cargo test -p finchart golden\n  git diff finchart/tests/golden",
        problems.join("\n  ")
    );
}

/// Where two texts first differ, with both versions of that line.
fn first_difference(expected: &str, actual: &str) -> String {
    let (e, a): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    match (0..e.len().max(a.len())).find(|&i| e.get(i) != a.get(i)) {
        Some(i) => format!(
            "line {} differs\n    expected: {:?}\n    actual:   {:?}",
            i + 1,
            e.get(i).copied().unwrap_or("<end of file>"),
            a.get(i).copied().unwrap_or("<end of file>")
        ),
        None => "trailing newline differs".to_string(),
    }
}

/// Run a snapshot writer into a scratch dir and collect what it wrote, except
/// the timing-bearing `metrics.json`.
fn written(
    tag: &str,
    write: impl FnOnce(&Path) -> std::io::Result<Vec<PathBuf>>,
) -> Vec<(String, String)> {
    let dir = std::env::temp_dir().join(format!("finchart_golden_{tag}_{}", std::process::id()));
    let paths = write(&dir).unwrap();
    let files = paths
        .iter()
        .filter(|p| !p.ends_with("metrics.json"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(p).unwrap())
        })
        .collect();
    fs::remove_dir_all(&dir).ok();
    files
}

/// A recording's frame at `time` (M:SS, or `end`), plain, at the default size.
fn replay_frame(recording: &str, time: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/scenarios")
        .join(recording);
    let mut player = Player::open(&path, None, None, None, true).unwrap();
    let at = match time {
        "end" => player.duration(),
        t => parse_time(t).unwrap(),
    };
    player.seek(at);
    player.render(120, 30, ColorMode::Plain)
}

#[test]
fn golden_snapshot_chart() {
    check("chart", written("chart", chart_snapshot::write_all));
}

#[test]
fn golden_snapshot_live() {
    check("live", written("live", live_snapshot::write_all));
}

#[test]
fn golden_scenarios() {
    for name in [
        "ask-wall",
        "bid-wall",
        "ask-wall-pulled",
        "bid-wall-pulled",
        "sweep",
        "bid-reload",
    ] {
        let kind = scenarios::parse_scenario(name).unwrap();
        let files = written(name, |dir| scenarios::write_scenario(kind, dir));
        check(&format!("scenario/{name}"), files);
    }
}

#[test]
fn golden_replay_frames() {
    let frames = [
        ("large_buy_sweep.txt", "1:10", "sweep.before.txt"),
        ("large_buy_sweep.txt", "1:28", "sweep.after.txt"),
        ("liquidity_crisis.txt", "2:20", "crisis.thinning.txt"),
        ("liquidity_crisis.txt", "end", "crisis.end.txt"),
        ("normal_market.txt", "end", "normal.end.txt"),
        // Synthetic recording: interval and tick come from its `#` header.
        ("baseline.txt", "end", "baseline.end.txt"),
    ];
    let mut files: Vec<(String, String)> = frames
        .iter()
        .map(|(recording, time, name)| (name.to_string(), replay_frame(recording, time)))
        .collect();
    files.extend(crosshair_frames());
    check("replay", files);
}

/// The crosshair on the sweep bar from the keyboard (`c` then `,`), and from a
/// mouse hover at a fixed screen cell (which adds the price and liquidity there).
fn crosshair_frames() -> Vec<(String, String)> {
    use crate::player::Action;
    use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

    let open = || {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/scenarios/large_buy_sweep.txt");
        let mut p = Player::open(&path, None, None, None, true).unwrap();
        p.resize(120, 30);
        p.seek(72.0);
        p
    };
    let mut keys = open();
    keys.act(Action::ToggleCrosshair);
    keys.act(Action::StepBack);

    let mut hover = open();
    hover.mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 60,
        row: 6,
        modifiers: KeyModifiers::NONE,
    });
    vec![
        (
            "sweep.crosshair.keys.txt".to_string(),
            keys.render(120, 30, ColorMode::Plain),
        ),
        (
            "sweep.crosshair.hover.txt".to_string(),
            hover.render(120, 30, ColorMode::Plain),
        ),
    ]
}
