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
    Replay {
        file: PathBuf,
    },
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
const COMMANDS: &[(&str, &str)] = &[
    (
        "live",
        "Live synthetic market: candles forming tick by tick, order flow, depth",
    ),
    (
        "browse",
        "Pan and zoom 5,000 candles; aggregation keeps every frame screen-sized",
    ),
    (
        "replay",
        "Print the final frame of a recorded MarketEvent file",
    ),
    (
        "bench",
        "Demo chart plus the 10k / 100k / 1M generate-cull-render benchmark",
    ),
    (
        "snapshot",
        "Write deterministic frames + metrics for diffing (chart | live)",
    ),
    (
        "scenario",
        "Render a scripted order-book scenario (walls, pulls, sweeps)",
    ),
    (
        "record",
        "Write the seeded synthetic event stream to a file",
    ),
];

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
             KEYS:\n    q / Esc    quit\n"
        }
        "browse" => {
            "finchart browse\n\n\
             Pan and zoom a deterministic 5,000-candle series. When candles outnumber the\n\
             screen columns they are aggregated into OHLC bars (see the status line).\n\n\
             KEYS:\n    h l / ← →    pan\n    k j / + -    zoom in / out\n    r            reset\n    q / Esc      quit\n"
        }
        "replay" => {
            "finchart replay <FILE>\n\n\
             Fold a recorded MarketEvent file into a chart and print its final frame.\n"
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
        "replay" => Command::Replay {
            file: args.required("FILE")?.into(),
        },
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
