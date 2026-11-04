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
mod player;
mod term;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use backend_terminal::ColorMode;
use cli::{Command, ReplayArgs, SnapshotKind};
use player::Player;

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
    let mut player = Player::open(
        &args.file,
        args.interval,
        args.tick,
        args.window,
        args.heatmap,
    )?;
    player.set_speed(args.speed);

    let tty = io::stdout().is_terminal();
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
    player.seek(args.start);
    player.play();
    player::run(player)
}

fn print_written(paths: &[PathBuf]) {
    println!("wrote {} files:", paths.len());
    for p in paths {
        println!("  {}", p.display());
    }
}
