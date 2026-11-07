//! Command-line parsing for `finchart`: a small hand-rolled subcommand
//! dispatcher (no dependencies). `parse` is pure, so every invocation is
//! unit-tested without running anything.

use std::path::PathBuf;

use market_core::ScenarioKind;

use crate::live::scenarios::parse_scenario;

/// Frame rate for `finchart live` when `--fps` is not given.
pub const DEFAULT_FPS: u64 = 12;

/// A fully parsed invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Print the overview, or one command's usage.
    Help(Option<&'static str>),
    Version,
    Live {
        fps: u64,
    },
    Browse,
    Replay(ReplayArgs),
    Bench,
    Snapshot {
        kind: SnapshotKind,
        dir: PathBuf,
    },
    Scenario {
        kind: ScenarioKind,
        dir: PathBuf,
    },
    Record {
        file: PathBuf,
    },
}

/// Which deterministic snapshot set `finchart snapshot` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    /// The static chart (plain / candles / ANSI / HTML + metrics).
    Chart,
    /// The live replay before / mid / after a candle roll-over.
    Live,
}

/// Every subcommand with its one-line summary, in `finchart help` order.
#[rustfmt::skip]
const COMMANDS: &[(&str, &str)] = &[
    ("live",     "Live synthetic market: candles forming tick by tick, order flow, depth"),
    ("browse",   "Pan and zoom 5,000 candles; aggregation keeps every frame screen-sized"),
    ("replay",   "Play a recorded session: pause, seek, speed, pan and zoom"),
    ("bench",    "Demo chart plus the 10k / 100k / 1M generate-cull-render benchmark"),
    ("snapshot", "Write deterministic frames + metrics for diffing (chart | live)"),
    ("scenario", "Render a scripted order-book scenario (walls, pulls, sweeps)"),
    ("record",   "Write the seeded synthetic event stream to a file"),
];

/// Options for `finchart replay`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayArgs {
    pub file: PathBuf,
    /// Candle interval in seconds (else the file's header, else 1).
    pub interval: Option<i64>,
    /// Price tick size (else the file's header, else 0.01).
    pub tick: Option<f64>,
    /// Candles shown while following (else fit one per bar to the width).
    pub window: Option<usize>,
    pub speed: f64,
    /// Seconds into the session to start at.
    /// Seconds into the session to start at (else the start, or the live edge
    /// with `--follow`).
    pub start: Option<f64>,
    /// Print the single frame at this time instead of playing.
    pub at: Option<f64>,
    pub heatmap: bool,
    /// Open paused at `start` instead of playing.
    pub paused: bool,
    /// Keep reading the file as it grows, staying on the newest event.
    pub follow: bool,
    /// Size of a printed frame.
    pub width: usize,
    pub height: usize,
}

const SCENARIOS: &str = "ask-wall, bid-wall, ask-wall-pulled, bid-wall-pulled, sweep, bid-reload";

/// The overview printed by `finchart`, `finchart help` and `finchart --help`.
pub fn overview() -> String {
    let mut out = String::from(
        "finchart — a terminal-first financial charting engine\n\n\
         USAGE:\n    finchart <COMMAND> [ARGS]\n\nCOMMANDS:\n",
    );
    for (name, summary) in COMMANDS {
        out.push_str(&format!("    {name:<10}{summary}\n"));
    }
    out.push_str("\nRun `finchart help <COMMAND>` for a command's options.\n");
    out
}

/// One command's usage text.
pub fn usage(command: &str) -> &'static str {
    match command {
        "live" => {
            "finchart live [--fps N]\n\n\
             A seeded synthetic tick stream folds into the forming candle, the volume and\n\
             delta panels, and a Level 2 depth ladder in real time.\n\n\
             OPTIONS:\n    --fps N    frames per second (default 12)\n\n\
             KEYS:\n    h l / ← →    pan (holds the view while candles keep arriving)\n    \
             k j / + -    zoom in / out\n    f            follow the forming candle again\n    \
             c            crosshair: read out a bar\n    , .          move the crosshair one bar\n    \
             Esc          hide the crosshair, else quit\n    q            quit\n\n\
             MOUSE:\n    hover for the crosshair, wheel to zoom, drag to pan\n"
        }
        "browse" => {
            "finchart browse\n\n\
             Pan and zoom a deterministic 5,000-candle series. When candles outnumber the\n\
             screen columns they are aggregated into OHLC bars (see the status line).\n\n\
             KEYS:\n    h l / ← →    pan\n    k j / + -    zoom in / out\n    r            reset\n    \
             c            crosshair: read out a bar\n    , .          move the crosshair one bar\n    \
             Esc          hide the crosshair, else quit\n    q            quit\n\n\
             MOUSE:\n    hover for the crosshair, wheel to zoom, drag to pan\n"
        }
        "replay" => {
            "finchart replay <FILE> [OPTIONS]\n\n\
             Play a recorded MarketEvent file at its own pace. Every frame is the exact\n\
             chart state at that moment of the session. When stdout is not a terminal,\n\
             or with --at, print one frame instead (plain text when piped).\n\n\
             OPTIONS:\n\
             \x20   --speed X       playback speed, 0.25 to 64 (default 1)\n\
             \x20   --start TIME    start this far in, as SECS or M:SS (default 0)\n\
             \x20   --paused        open paused at the start time\n\
             \x20   --follow        keep reading the file as it is written (like tail -f) and\n\
             \x20                   stay on the newest event; seek back and it catches up\n\
             \x20   --at TIME       print the frame at TIME and exit\n\
             \x20   --size WxH      size of a printed frame (default 120x30)\n\
             \x20   --interval S    candle interval in seconds (default: file header, else 1)\n\
             \x20   --tick T        price tick size (default: file header, else 0.01)\n\
             \x20   --window N      candles shown while following (default: fit the width)\n\
             \x20   --no-heatmap    start with the liquidity heatmap off\n\n\
             KEYS:\n\
             \x20   space  play / pause      ← →    seek 5 s       ↓ ↑  seek 30 s\n\
             \x20   [ ]    slower / faster   0 End  restart / end\n\
             \x20   h l    pan               k j    zoom           f    follow\n\
             \x20   m      heatmap           ?      help           q    quit\n\
             \x20   c      crosshair         , .    step it a bar  Esc  hide it\n\n\
             MOUSE:\n\
             \x20   hover for the crosshair, wheel to zoom, drag to pan\n"
        }
        "bench" => {
            "finchart bench\n\n\
             Print a demo chart, then time generate / cull / render for 10k, 100k and 1M\n\
             candles. Timings vary per machine; the pattern should not.\n"
        }
        "snapshot" => {
            "finchart snapshot <chart|live> [DIR]\n\n\
             Write byte-deterministic frames (plain, ANSI, HTML) and metrics.json.\n\
             chart  static chart            (default DIR: artifacts/tui)\n\
             live   forming candle before / mid / after roll-over  (default DIR: artifacts/live)\n"
        }
        "scenario" => {
            "finchart scenario <NAME> [DIR]\n\n\
             Render before / mid / after frames of a scripted book structure with the\n\
             liquidity heatmap and trade markers (default DIR: artifacts/scenario).\n\
             The price line is a placeholder, not built from trades.\n\n\
             NAMES:\n    ask-wall, bid-wall, ask-wall-pulled, bid-wall-pulled, sweep, bid-reload\n"
        }
        "record" => {
            "finchart record <FILE>\n\n\
             Write the seeded synthetic MarketEvent stream to FILE, one event per line.\n"
        }
        _ => "",
    }
}

/// Parse the arguments after the program name.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some((first, rest)) = args.split_first() else {
        return Ok(Command::Help(None));
    };
    let name = match first.as_str() {
        "-h" | "--help" => return Ok(Command::Help(None)),
        "-V" | "--version" => return Ok(Command::Version),
        "help" => {
            return match rest {
                [] => Ok(Command::Help(None)),
                [topic] => command_name(topic).map(|n| Command::Help(Some(n))),
                _ => Err("usage: finchart help [COMMAND]".to_string()),
            }
        }
        other => command_name(other)?,
    };
    if rest.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(Command::Help(Some(name)));
    }

    let mut args = Args::new(name, rest);
    let command = match name {
        "live" => {
            let fps = match args.flag("--fps")? {
                None => DEFAULT_FPS,
                Some(v) => v.parse::<u64>().ok().filter(|&n| n > 0).ok_or_else(|| {
                    format!("finchart live: --fps needs a positive integer, got '{v}'")
                })?,
            };
            Command::Live { fps }
        }
        "browse" => Command::Browse,
        "bench" => Command::Bench,
        "replay" => Command::Replay(replay_args(&mut args)?),
        "record" => Command::Record {
            file: args.required("FILE")?.into(),
        },
        "snapshot" => {
            let kind = match args.required("chart|live")?.as_str() {
                "chart" => SnapshotKind::Chart,
                "live" => SnapshotKind::Live,
                other => {
                    return Err(format!(
                        "finchart snapshot: expected 'chart' or 'live', got '{other}'"
                    ))
                }
            };
            let default = match kind {
                SnapshotKind::Chart => "artifacts/tui",
                SnapshotKind::Live => "artifacts/live",
            };
            Command::Snapshot {
                kind,
                dir: args
                    .optional()
                    .unwrap_or_else(|| default.to_string())
                    .into(),
            }
        }
        "scenario" => {
            let raw = args.required("NAME")?;
            let kind = parse_scenario(&raw).ok_or_else(|| {
                format!("finchart scenario: unknown scenario '{raw}' (one of: {SCENARIOS})")
            })?;
            Command::Scenario {
                kind,
                dir: args
                    .optional()
                    .unwrap_or_else(|| "artifacts/scenario".to_string())
                    .into(),
            }
        }
        _ => unreachable!("command_name only returns known commands"),
    };
    args.finish()?;
    Ok(command)
}

fn replay_args(args: &mut Args) -> Result<ReplayArgs, String> {
    fn bad(flag: &str, value: &str, want: &str) -> String {
        format!("finchart replay: {flag} needs {want}, got '{value}'")
    }
    /// Parse `--flag VALUE` with `parse`, which rejects out-of-range values.
    fn valued<T>(
        args: &mut Args,
        flag: &str,
        want: &str,
        parse: impl Fn(&str) -> Option<T>,
    ) -> Result<Option<T>, String> {
        match args.flag(flag)? {
            None => Ok(None),
            Some(v) => parse(&v).map(Some).ok_or_else(|| bad(flag, &v, want)),
        }
    }

    let interval = valued(args, "--interval", "a positive number of seconds", |v| {
        v.parse::<i64>().ok().filter(|&n| n > 0)
    })?;
    let tick = valued(args, "--tick", "a positive price step", |v| {
        v.parse::<f64>().ok().filter(|&t| t > 0.0)
    })?;
    let window = valued(args, "--window", "a positive candle count", |v| {
        v.parse::<usize>().ok().filter(|&n| n > 0)
    })?;
    let speed = valued(args, "--speed", "a speed from 0.25 to 64", |v| {
        v.parse::<f64>().ok().filter(|s| (0.25..=64.0).contains(s))
    })?;
    let start = valued(args, "--start", "SECS or M:SS", parse_time)?;
    let at = valued(args, "--at", "SECS or M:SS", parse_time)?;
    let size = valued(args, "--size", "WIDTHxHEIGHT, e.g. 120x30", |v| {
        let (w, h) = v.split_once('x')?;
        let (w, h): (usize, usize) = (w.parse().ok()?, h.parse().ok()?);
        (w >= 20 && h >= 5).then_some((w, h))
    })?;
    let heatmap = !args.switch("--no-heatmap");
    let paused = args.switch("--paused");
    let follow = args.switch("--follow");
    if follow && at.is_some() {
        return Err(
            "finchart replay: --follow plays a growing file; it cannot be combined with --at"
                .to_string(),
        );
    }
    let (width, height) = size.unwrap_or((120, 30));
    Ok(ReplayArgs {
        file: args.required("FILE")?.into(),
        interval,
        tick,
        window,
        speed: speed.unwrap_or(1.0),
        start,
        at,
        heatmap,
        paused,
        follow,
        width,
        height,
    })
}

/// Parse a time as seconds (`75`, `75.5`) or minutes and seconds (`1:15`).
pub fn parse_time(text: &str) -> Option<f64> {
    let secs = match text.split_once(':') {
        None => text.parse::<f64>().ok()?,
        Some((m, s)) => {
            let m: u32 = m.parse().ok()?;
            let s: f64 = s.parse().ok().filter(|s| (0.0..60.0).contains(s))?;
            f64::from(m) * 60.0 + s
        }
    };
    (secs >= 0.0 && secs.is_finite()).then_some(secs)
}

/// Resolve a command name to its `'static` spelling.
fn command_name(name: &str) -> Result<&'static str, String> {
    COMMANDS
        .iter()
        .map(|(n, _)| *n)
        .find(|n| *n == name)
        .ok_or_else(|| format!("finchart: unknown command '{name}' (see `finchart help`)"))
}

/// The remaining arguments of one command: flags are taken by name, then
/// positionals in order; anything left over is an error.
struct Args<'a> {
    command: &'static str,
    rest: Vec<&'a str>,
}

impl<'a> Args<'a> {
    fn new(command: &'static str, rest: &'a [String]) -> Self {
        Self {
            command,
            rest: rest.iter().map(String::as_str).collect(),
        }
    }

    /// Take `--name VALUE`, if present.
    fn flag(&mut self, name: &str) -> Result<Option<String>, String> {
        let Some(i) = self.rest.iter().position(|a| *a == name) else {
            return Ok(None);
        };
        let value = self
            .rest
            .get(i + 1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| format!("finchart {}: {name} needs a value", self.command))?
            .to_string();
        self.rest.drain(i..=i + 1);
        Ok(Some(value))
    }

    /// Take a bare `--name` switch, reporting whether it was present.
    fn switch(&mut self, name: &str) -> bool {
        let Some(i) = self.rest.iter().position(|a| *a == name) else {
            return false;
        };
        self.rest.remove(i);
        true
    }

    /// Take the next positional argument, if any.
    fn optional(&mut self) -> Option<String> {
        let i = self.rest.iter().position(|a| !a.starts_with('-'))?;
        Some(self.rest.remove(i).to_string())
    }

    /// Take the next positional argument, or fail naming it.
    fn required(&mut self, what: &str) -> Result<String, String> {
        self.optional().ok_or_else(|| {
            format!(
                "finchart {}: missing <{what}>\n\n{}",
                self.command,
                usage(self.command)
            )
        })
    }

    /// Fail on anything not consumed.
    fn finish(self) -> Result<(), String> {
        match self.rest.first() {
            None => Ok(()),
            Some(extra) => Err(format!(
                "finchart {}: unexpected argument '{extra}' (see `finchart help {}`)",
                self.command, self.command
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_core::ScenarioKind;
    use std::path::PathBuf;

    fn parse_str(line: &str) -> Result<Command, String> {
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        parse(&args)
    }

    #[test]
    fn no_arguments_and_help_flags_show_the_overview() {
        assert_eq!(parse_str(""), Ok(Command::Help(None)));
        assert_eq!(parse_str("--help"), Ok(Command::Help(None)));
        assert_eq!(parse_str("-h"), Ok(Command::Help(None)));
        assert_eq!(parse_str("help"), Ok(Command::Help(None)));
        assert_eq!(parse_str("--version"), Ok(Command::Version));
    }

    #[test]
    fn per_command_help() {
        assert_eq!(parse_str("help live"), Ok(Command::Help(Some("live"))));
        assert_eq!(parse_str("live --help"), Ok(Command::Help(Some("live"))));
        assert_eq!(
            parse_str("scenario -h"),
            Ok(Command::Help(Some("scenario")))
        );
        assert!(parse_str("help bogus").is_err());
    }

    #[test]
    fn live_takes_an_optional_frame_rate() {
        assert_eq!(parse_str("live"), Ok(Command::Live { fps: DEFAULT_FPS }));
        assert_eq!(parse_str("live --fps 30"), Ok(Command::Live { fps: 30 }));
        assert!(parse_str("live --fps 0").is_err());
        assert!(parse_str("live --fps x").is_err());
        assert!(parse_str("live --fps").is_err());
    }

    #[test]
    fn simple_commands() {
        assert_eq!(parse_str("browse"), Ok(Command::Browse));
        assert_eq!(parse_str("bench"), Ok(Command::Bench));
    }

    #[test]
    fn snapshot_kinds_and_default_directories() {
        assert_eq!(
            parse_str("snapshot chart"),
            Ok(Command::Snapshot {
                kind: SnapshotKind::Chart,
                dir: PathBuf::from("artifacts/tui")
            })
        );
        assert_eq!(
            parse_str("snapshot live out/live"),
            Ok(Command::Snapshot {
                kind: SnapshotKind::Live,
                dir: PathBuf::from("out/live")
            })
        );
        assert!(parse_str("snapshot").is_err());
        assert!(parse_str("snapshot nope").is_err());
    }

    #[test]
    fn scenario_names_and_default_directory() {
        assert_eq!(
            parse_str("scenario sweep"),
            Ok(Command::Scenario {
                kind: ScenarioKind::SweepAsk,
                dir: PathBuf::from("artifacts/scenario")
            })
        );
        assert_eq!(
            parse_str("scenario ask-wall-pulled out/x"),
            Ok(Command::Scenario {
                kind: ScenarioKind::AskWallPulled,
                dir: PathBuf::from("out/x")
            })
        );
        let err = parse_str("scenario bogus").unwrap_err();
        assert!(err.contains("bid-reload"), "lists the valid names: {err}");
    }

    #[test]
    fn replay_defaults() {
        assert_eq!(
            parse_str("replay web/scenarios/large_buy_sweep.txt"),
            Ok(Command::Replay(ReplayArgs {
                file: PathBuf::from("web/scenarios/large_buy_sweep.txt"),
                interval: None,
                tick: None,
                window: None,
                speed: 1.0,
                start: None,
                at: None,
                heatmap: true,
                paused: false,
                follow: false,
                width: 120,
                height: 30,
            }))
        );
        assert!(parse_str("replay").is_err());
        let Ok(Command::Replay(a)) = parse_str("replay s.txt --start 1:28 --paused") else {
            panic!("did not parse");
        };
        assert!(a.paused);
    }

    #[test]
    fn replay_options() {
        let Ok(Command::Replay(a)) = parse_str(
            "replay s.txt --interval 60 --tick 0.05 --window 40 --speed 4 --start 0:56 --no-heatmap",
        ) else {
            panic!("did not parse");
        };
        assert_eq!(a.interval, Some(60));
        assert_eq!(a.tick, Some(0.05));
        assert_eq!(a.window, Some(40));
        assert_eq!(a.speed, 4.0);
        assert_eq!(a.start, Some(56.0));
        assert!(!a.heatmap);

        let Ok(Command::Replay(a)) = parse_str("replay s.txt --at 1:28.5 --size 100x24") else {
            panic!("did not parse");
        };
        assert_eq!(a.at, Some(88.5));
        assert_eq!((a.width, a.height), (100, 24));
    }

    #[test]
    fn replay_can_follow_a_growing_file() {
        let Ok(Command::Replay(a)) = parse_str("replay live.txt --follow") else {
            panic!("did not parse");
        };
        assert!(a.follow);
        assert!(parse_str("replay live.txt --follow --at 1:00")
            .unwrap_err()
            .contains("--follow"));
    }

    #[test]
    fn replay_rejects_bad_values() {
        for bad in [
            "replay s.txt --speed 0",
            "replay s.txt --speed fast",
            "replay s.txt --interval 0",
            "replay s.txt --tick -1",
            "replay s.txt --start soon",
            "replay s.txt --at 1:75",
            "replay s.txt --size 100",
        ] {
            assert!(parse_str(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn times_parse_as_seconds_or_minutes_and_seconds() {
        assert_eq!(parse_time("75"), Some(75.0));
        assert_eq!(parse_time("1:15"), Some(75.0));
        assert_eq!(parse_time("0:56.5"), Some(56.5));
        assert_eq!(parse_time("-3"), None);
        assert_eq!(parse_time("1:60"), None);
        assert_eq!(parse_time("x"), None);
    }

    #[test]
    fn record_needs_a_file() {
        assert_eq!(
            parse_str("record out/events.txt"),
            Ok(Command::Record {
                file: PathBuf::from("out/events.txt")
            })
        );
        assert!(parse_str("record").is_err());
    }

    #[test]
    fn unknown_commands_flags_and_extra_arguments_are_errors() {
        assert!(parse_str("frobnicate")
            .unwrap_err()
            .contains("finchart help"));
        assert!(parse_str("browse --nope").is_err());
        assert!(parse_str("bench extra").is_err());
        assert!(parse_str("record a.txt b.txt").is_err());
    }
}
