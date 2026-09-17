//! `examples-matching-engine`: an isolated adapter that connects to the external
//! matching-engine gRPC API, translates messages into `market_core::MarketEvent`,
//! applies them through `ChartState`, and renders the terminal chart.
//!
//! All gRPC/async dependencies live in this crate only; the core crates stay
//! zero-dependency. See `docs/matching-engine-adapter-v1-plan.md`.
//!
//! Modes:
//! - default: connect to the engine (`--endpoint`), stream + render live.
//! - `--record <file>`: also append the translated event stream to a file.
//! - `--replay <file>`: fold a recorded file through `ChartState` and print a
//!   frame — offline, no engine required.

/// Generated protobuf types + gRPC client for the matching engine.
pub mod pb {
    tonic::include_proto!("matchingengine");
}

mod feed;
mod record;
mod render;
mod translate;

#[cfg(test)]
mod integration;

use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, queue};

use backend_terminal::ColorMode;
use market_core::{CandleSeries, ChartState, MarketEvent};

use feed::FeedConfig;
use render::{RenderConfig, Stats};

/// Adapter configuration (CLI-parsed).
struct Config {
    endpoint: String,
    symbol: String,
    tick_size: f64,
    interval_secs: i64,
    depth: u32,
    depth_interval_ms: u64,
    heartbeat_ms: u64,
    window: usize,
    heatmap_capacity: usize,
    trade_cap: usize,
    depth_panel_levels: usize,
    show_heatmap: bool,
    record: Option<String>,
    replay: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:50051".to_string(),
            symbol: "BTCUSD".to_string(),
            tick_size: 0.05,
            interval_secs: 60,
            depth: 20,
            depth_interval_ms: 1000,
            heartbeat_ms: 1000,
            window: 60,
            heatmap_capacity: 160,
            trade_cap: 64,
            depth_panel_levels: 6,
            show_heatmap: false,
            record: None,
            replay: None,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return Ok(());
    }
    let cfg = Config::from_args(&args);

    if let Some(file) = cfg.replay.clone() {
        return run_replay(&cfg, &file).map_err(Into::into);
    }
    run_live(&cfg)
}

/// Offline: replay a recorded event file into a `ChartState` and print one frame.
fn run_replay(cfg: &Config, file: &str) -> io::Result<()> {
    let state = record::replay_file(
        Path::new(file),
        cfg.tick_size,
        cfg.interval_secs,
        cfg.heatmap_capacity,
        cfg.trade_cap,
    )?;
    let rcfg = render_config(cfg, 120, 30);
    let title = format!("source=replay · {file}");
    println!(
        "{}",
        render::render_state(&state, &rcfg, ColorMode::Plain, &title)
    );
    Ok(())
}

/// Live: connect to the engine, drain translated events, apply + render in a loop.
fn run_live(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let handle = match feed::connect(feed_config(cfg)) {
        Ok(h) => h,
        Err(e) => {
            eprintln!(
                "could not connect to matching engine at {}: {e}\n\
                 (use --replay <file> to view a recorded stream offline)",
                cfg.endpoint
            );
            std::process::exit(1);
        }
    };

    let mut recorder = match &cfg.record {
        Some(p) => Some(record::Recorder::create(Path::new(p))?),
        None => None,
    };
    let mut state = new_state(cfg);
    let (mut trades, mut depth_refreshes) = (0usize, 0usize);
    let (mut last_update_us, mut last_render_us) = (0.0, 0.0);

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let _guard = TerminalGuard;
    let frame_dur = Duration::from_millis(80);

    loop {
        // Ingest whatever the producers have queued.
        let mut events = Vec::new();
        handle.drain(&mut events);
        let t = Instant::now();
        for ev in &events {
            match ev {
                MarketEvent::Trade(_) => trades += 1,
                MarketEvent::DepthSnapshot(_) => depth_refreshes += 1,
                _ => {}
            }
            state.apply(ev);
            if let Some(rec) = &mut recorder {
                let _ = rec.write(ev);
            }
        }
        if !events.is_empty() {
            last_update_us = t.elapsed().as_secs_f64() * 1e6 / events.len() as f64;
        }

        let (cols, rows) = terminal::size().unwrap_or((120, 30));
        let stats = Stats {
            symbol: cfg.symbol.clone(),
            trades,
            depth_refreshes,
            update_us: last_update_us,
            render_us: last_render_us,
        };
        let title = render::status_line(&state, &stats);
        let rcfg = render_config(cfg, cols as usize, rows as usize);

        let t = Instant::now();
        let chart = render::render_state(&state, &rcfg, ColorMode::Ansi, &title);
        last_render_us = t.elapsed().as_secs_f64() * 1e6;

        queue!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
        write!(stdout, "{}", chart.replace('\n', "\r\n"))?;
        stdout.flush()?;

        // Pace the loop and stay responsive to quit keys.
        if event::poll(frame_dur)? {
            if let Event::Key(key) = event::read()? {
                if is_quit(key) {
                    break;
                }
            }
        }
    }

    if let Some(mut rec) = recorder {
        let _ = rec.flush();
    }
    Ok(())
}

fn new_state(cfg: &Config) -> ChartState {
    ChartState::from_history(
        CandleSeries::new(),
        cfg.interval_secs,
        cfg.tick_size,
        Some(cfg.heatmap_capacity),
        cfg.trade_cap,
    )
}

fn feed_config(cfg: &Config) -> FeedConfig {
    FeedConfig {
        endpoint: cfg.endpoint.clone(),
        symbol: cfg.symbol.clone(),
        depth: cfg.depth,
        depth_interval_ms: cfg.depth_interval_ms,
        heartbeat_ms: cfg.heartbeat_ms,
    }
}

fn render_config(cfg: &Config, width: usize, height: usize) -> RenderConfig {
    RenderConfig {
        width,
        height,
        window: cfg.window,
        depth_levels: cfg.depth_panel_levels,
        show_heatmap: cfg.show_heatmap,
    }
}

/// Restores the terminal on drop, even on panic.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

fn is_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn print_usage() {
    println!(
        "finchart matching-engine adapter\n\n\
         USAGE:\n  \
           examples-matching-engine [OPTIONS]\n\n\
         OPTIONS:\n  \
           --endpoint <url>          gRPC endpoint (default http://127.0.0.1:50051)\n  \
           --symbol <sym>            instrument symbol (default BTCUSD)\n  \
           --tick-size <f>           price tick size (default 0.05)\n  \
           --interval-secs <n>       candle interval seconds (default 60)\n  \
           --depth <n>               Level 2 depth requested (default 20)\n  \
           --depth-interval-ms <n>   depth poll interval (default 1000)\n  \
           --heartbeat-ms <n>        heartbeat cadence (default 1000)\n  \
           --window <n>              visible candle window (default 60)\n  \
           --heatmap                 draw the liquidity heatmap layer\n  \
           --record <file>           append the translated event stream to a file\n  \
           --replay <file>           replay a recorded file offline (no engine)\n  \
           -h, --help                show this help"
    );
}

impl Config {
    fn from_args(args: &[String]) -> Self {
        let mut cfg = Config::default();
        if let Some(v) = arg_str(args, "--endpoint") {
            cfg.endpoint = v;
        }
        if let Some(v) = arg_str(args, "--symbol") {
            cfg.symbol = v;
        }
        if let Some(v) = arg_str(args, "--tick-size").and_then(|s| s.parse().ok()) {
            cfg.tick_size = v;
        }
        if let Some(v) = arg_str(args, "--interval-secs").and_then(|s| s.parse().ok()) {
            cfg.interval_secs = v;
        }
        if let Some(v) = arg_str(args, "--depth").and_then(|s| s.parse().ok()) {
            cfg.depth = v;
        }
        if let Some(v) = arg_str(args, "--depth-interval-ms").and_then(|s| s.parse().ok()) {
            cfg.depth_interval_ms = v;
        }
        if let Some(v) = arg_str(args, "--heartbeat-ms").and_then(|s| s.parse().ok()) {
            cfg.heartbeat_ms = v;
        }
        if let Some(v) = arg_str(args, "--window").and_then(|s| s.parse().ok()) {
            cfg.window = v;
        }
        cfg.show_heatmap = has_flag(args, "--heatmap");
        cfg.record = arg_str(args, "--record");
        cfg.replay = arg_str(args, "--replay");
        cfg
    }
}

/// Value following `flag`, if present.
fn arg_str(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_parse_overrides_and_defaults() {
        let args: Vec<String> = [
            "bin",
            "--symbol",
            "ETHUSD",
            "--tick-size",
            "0.01",
            "--heatmap",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let cfg = Config::from_args(&args);
        assert_eq!(cfg.symbol, "ETHUSD");
        assert_eq!(cfg.tick_size, 0.01);
        assert!(cfg.show_heatmap);
        assert_eq!(cfg.endpoint, "http://127.0.0.1:50051"); // default
        assert_eq!(cfg.depth, 20); // default
    }

    #[test]
    fn replay_and_record_flags_capture_paths() {
        let args: Vec<String> = ["bin", "--replay", "e.txt"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cfg = Config::from_args(&args);
        assert_eq!(cfg.replay.as_deref(), Some("e.txt"));
        assert_eq!(cfg.record, None);
    }
}
