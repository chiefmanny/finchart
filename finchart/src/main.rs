//! `finchart`: the terminal front-end for the finchart engine.
//!
//! One binary, several subcommands (`finchart help` lists them). Each is a thin
//! shell over the library crates; parsing lives in [`cli`] and is unit-tested.

mod bench;
mod browse;
mod cli;
mod frame;
#[cfg(test)]
mod golden;
mod live;
mod nav;
mod player;
mod pointer;
mod tail;
mod term;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use backend_terminal::ColorMode;
use cli::{Command, ReplayArgs, SnapshotKind};
use player::Player;
use tail::Tail;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(&args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("finchart: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> io::Result<()> {
    match command {
        Command::Help(None) => print!("{}", cli::overview()),
        Command::Help(Some(name)) => print!("{}", cli::usage(name)),
        Command::Version => println!("finchart {}", env!("CARGO_PKG_VERSION")),
        Command::Live { fps } => live::run(fps)?,
        Command::Browse => browse::run()?,
        Command::Bench => bench::run(),
        Command::Replay(args) => replay(args)?,
        Command::Record { file } => {
            let n = live::feed::record(&file)?;
            println!("recorded {n} events to {}", file.display());
        }
        Command::Snapshot { kind, dir } => {
            let paths = match kind {
                SnapshotKind::Chart => bench::snapshot::write_all(&dir)?,
                SnapshotKind::Live => live::snapshot::write_all(&dir)?,
            };
            print_written(&paths);
        }
        Command::Scenario { kind, dir } => {
            print_written(&live::scenarios::write_scenario(kind, &dir)?);
        }
    }
    Ok(())
}

/// Play a recording, or print one frame of it when asked (`--at`) or when stdout
/// is not a terminal (plain text then, so piped output stays diffable).
fn replay(args: ReplayArgs) -> io::Result<()> {
    let tty = io::stdout().is_terminal();
    if args.follow {
        if !tty {
            return Err(io::Error::other("--follow needs a terminal"));
        }
        let (tail, player) = open_follow(&args)?;
        return player::run(player, Some((tail, &|| open_follow(&args))));
    }

    let mut player = Player::open(
        &args.file,
        args.interval,
        args.tick,
        args.window,
        args.heatmap,
    )?;
    player.set_speed(args.speed);
    if args.at.is_some() || !tty {
        player.seek(args.at.unwrap_or(player.duration()));
        let color = if tty {
            ColorMode::Ansi
        } else {
            ColorMode::Plain
        };
        println!("{}", player.render(args.width, args.height, color));
        return Ok(());
    }
    player.seek(args.start.unwrap_or(0.0));
    if !args.paused {
        player.play();
    }
    player::run(player, None)
}

/// Open a growing recording for `--follow`: a player over what is written so
/// far, at the live edge (or `--start`), plus the tail that feeds it more.
fn open_follow(args: &ReplayArgs) -> io::Result<(Tail, Player)> {
    let (tail, text) = Tail::open(&args.file)?;
    let mut player = Player::from_text(
        &args.file,
        &text,
        args.interval,
        args.tick,
        args.window,
        args.heatmap,
    );
    player.set_speed(args.speed);
    player.set_tailing(true);
    player.seek(args.start.unwrap_or(player.duration()));
    if !args.paused {
        player.play();
    }
    Ok((tail, player))
}

fn print_written(paths: &[PathBuf]) {
    println!("wrote {} files:", paths.len());
    for p in paths {
        println!("  {}", p.display());
    }
}
