//! `finchart replay`: an interactive player for recorded `MarketEvent` files.
//!
//! The session clock is the recording's own timestamps: playback moves it at
//! wall time × speed and folds every event up to it into a [`ChartState`].
//! Seeking forward folds onward; seeking back refolds from the start (fast: a
//! fold is a few microseconds per event), so any position shows exactly the
//! state the market was in at that moment. All of this is terminal-free and
//! unit-tested; [`run`] is a thin input/draw loop.

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, MouseEvent};

use backend_terminal::{ColorMode, Layout};
use market_core::{parse_events, CandleSeries, ChartState, MarketEvent};
use render_core::Viewport;

use crate::frame::{self, Frame};
use crate::nav::{Navigator, ZOOM_IN, ZOOM_OUT};
use crate::term::{is_quit, Screen};

/// Candle interval when neither a flag nor the file's header gives one: the
/// matching-engine recordings are second-by-second.
pub const DEFAULT_INTERVAL: i64 = 1;
/// Tick size when neither a flag nor the file's header gives one.
pub const DEFAULT_TICK: f64 = 0.01;
/// Heatmap history kept (time buckets).
const HEATMAP_CAPACITY: usize = 160;
/// Recent trades kept by the chart state.
const TRADE_CAP: usize = 64;
/// Playback speeds `[` / `]` step through.
pub const SPEEDS: [f64; 9] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
/// Seconds moved by ← / →, and by ↓ / ↑.
pub const SEEK_SHORT: f64 = 5.0;
pub const SEEK_LONG: f64 = 30.0;
/// Columns taken by the price gutter and depth ladder, for fitting the window.
const CHROME_COLS: usize = 26;

/// How a recording is bucketed and drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub interval_secs: i64,
    pub tick_size: f64,
    /// Candles shown while following; `None` fits one candle per bar to the width.
    pub window: Option<usize>,
    pub heatmap: bool,
}

impl Settings {
    /// Flags win, then the file's `# finchart interval=… tick=…` header, then
    /// the defaults.
    pub fn resolve(
        text: &str,
        interval: Option<i64>,
        tick: Option<f64>,
        window: Option<usize>,
        heatmap: bool,
    ) -> Self {
        let (h_interval, h_tick) = header(text);
        Self {
            interval_secs: interval.or(h_interval).unwrap_or(DEFAULT_INTERVAL),
            tick_size: tick.or(h_tick).unwrap_or(DEFAULT_TICK),
            window,
            heatmap,
        }
    }
}

/// `interval=` and `tick=` from the leading `#` lines, if present.
fn header(text: &str) -> (Option<i64>, Option<f64>) {
    let (mut interval, mut tick) = (None, None);
    for line in text.lines().take_while(|l| l.starts_with('#')) {
        for word in line.split_whitespace() {
            if let Some(v) = word.strip_prefix("interval=") {
                interval = v.parse().ok().filter(|&n: &i64| n > 0);
            } else if let Some(v) = word.strip_prefix("tick=") {
                tick = v.parse().ok().filter(|&t: &f64| t > 0.0);
            }
        }
    }
    (interval, tick)
}

/// What a key asks the player to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    TogglePlay,
    /// Move the clock by this many seconds.
    Seek(f64),
    Faster,
    Slower,
    PanLeft,
    PanRight,
    ZoomIn,
    ZoomOut,
    Follow,
    Restart,
    End,
    ToggleHeatmap,
    ToggleHelp,
    /// Show the crosshair on the newest bar, or hide it.
    ToggleCrosshair,
    /// Move the crosshair one bar back / forward.
    StepBack,
    StepForward,
    /// Hide the crosshair, or quit if it is not showing.
    Escape,
    Quit,
    None,
}

/// Map a key to an action (mpv-style transport, browse-style navigation).
pub fn action_for(key: KeyEvent) -> Action {
    if key.code == KeyCode::Esc {
        return Action::Escape;
    }
    if is_quit(key) {
        return Action::Quit;
    }
    match key.code {
        KeyCode::Char(' ') => Action::TogglePlay,
        KeyCode::Right => Action::Seek(SEEK_SHORT),
        KeyCode::Left => Action::Seek(-SEEK_SHORT),
        KeyCode::Up => Action::Seek(SEEK_LONG),
        KeyCode::Down => Action::Seek(-SEEK_LONG),
        KeyCode::Char(']') => Action::Faster,
        KeyCode::Char('[') => Action::Slower,
        KeyCode::Char('h') => Action::PanLeft,
        KeyCode::Char('l') => Action::PanRight,
        KeyCode::Char('k') | KeyCode::Char('+') | KeyCode::Char('=') => Action::ZoomIn,
        KeyCode::Char('j') | KeyCode::Char('-') => Action::ZoomOut,
        KeyCode::Char('f') => Action::Follow,
        KeyCode::Char('0') | KeyCode::Home => Action::Restart,
        KeyCode::End => Action::End,
        KeyCode::Char('m') => Action::ToggleHeatmap,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('c') => Action::ToggleCrosshair,
        KeyCode::Char(',') => Action::StepBack,
        KeyCode::Char('.') => Action::StepForward,
        _ => Action::None,
    }
}

/// A recording, a clock over it, and the chart state at that clock.
pub struct Player {
    name: String,
    events: Vec<MarketEvent>,
    start_ts: i64,
    duration: f64,
    settings: Settings,
    state: ChartState,
    /// Events folded into `state` so far.
    applied: usize,
    position: f64,
    playing: bool,
    speed: f64,
    /// Follow-or-hold viewport plus the crosshair.
    nav: Navigator,
    help: bool,
    /// The terminal size frames are drawn at, for mapping the crosshair and mouse.
    size: (usize, usize),
}

impl Player {
    /// A paused player at the start of `events` (which must be in time order, as
    /// recordings are).
    pub fn new(name: &str, events: Vec<MarketEvent>, settings: Settings) -> Self {
        let start_ts = events.first().map(MarketEvent::ts).unwrap_or(0);
        let end_ts = events.last().map(MarketEvent::ts).unwrap_or(start_ts);
        let mut player = Self {
            name: name.to_string(),
            events,
            start_ts,
            duration: (end_ts - start_ts).max(0) as f64,
            state: fresh_state(&settings),
            settings,
            applied: 0,
            position: 0.0,
            playing: false,
            speed: 1.0,
            nav: Navigator::default(),
            help: false,
            size: (120, 30),
        };
        player.fold_to_position();
        player
    }

    /// Load a recording from disk.
    pub fn open(
        path: &Path,
        interval: Option<i64>,
        tick: Option<f64>,
        window: Option<usize>,
        heatmap: bool,
    ) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let settings = Settings::resolve(&text, interval, tick, window, heatmap);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Self::new(&name, parse_events(&text), settings))
    }

    #[cfg(test)]
    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    #[cfg(test)]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    #[cfg(test)]
    pub fn is_following(&self) -> bool {
        self.nav.is_following()
    }

    #[cfg(test)]
    pub fn speed(&self) -> f64 {
        self.speed
    }

    #[cfg(test)]
    pub fn state(&self) -> &ChartState {
        &self.state
    }

    /// Start at a non-default speed, clamped to the supported range.
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.clamp(SPEEDS[0], SPEEDS[SPEEDS.len() - 1]);
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    /// Move the clock to `secs` from the start (clamped), folding events to it.
    pub fn seek(&mut self, secs: f64) {
        let target = secs.clamp(0.0, self.duration);
        if target < self.position {
            self.state = fresh_state(&self.settings);
            self.applied = 0;
        }
        self.position = target;
        self.fold_to_position();
    }

    /// Advance the clock by `wall` × speed while playing; pause at the end.
    pub fn advance(&mut self, wall: Duration) {
        if !self.playing {
            return;
        }
        self.seek(self.position + wall.as_secs_f64() * self.speed);
        if self.position >= self.duration {
            self.playing = false;
        }
    }

    /// Apply an action; returns `false` when the player should quit.
    pub fn act(&mut self, action: Action) -> bool {
        match action {
            Action::Quit => return false,
            Action::Escape => return self.nav.pointer.hide(),
            Action::ToggleCrosshair => self.nav.pointer.toggle(self.layout().as_ref()),
            Action::StepBack => self.nav.pointer.step(-1, self.layout().as_ref()),
            Action::StepForward => self.nav.pointer.step(1, self.layout().as_ref()),
            Action::TogglePlay => {
                if !self.playing && self.position >= self.duration {
                    self.seek(0.0);
                }
                self.playing = !self.playing;
            }
            Action::Seek(delta) => self.seek(self.position + delta),
            Action::Faster => self.speed = step_speed(self.speed, 1),
            Action::Slower => self.speed = step_speed(self.speed, -1),
            Action::PanLeft => self.pan_by(-self.nav.pan_step(self.len(), self.window())),
            Action::PanRight => self.pan_by(self.nav.pan_step(self.len(), self.window())),
            Action::ZoomIn => self.zoom_by(ZOOM_IN),
            Action::ZoomOut => self.zoom_by(ZOOM_OUT),
            Action::Follow => self.nav.follow(),
            Action::Restart => self.seek(0.0),
            Action::End => self.seek(self.duration),
            Action::ToggleHeatmap => self.settings.heatmap = !self.settings.heatmap,
            Action::ToggleHelp => self.help = !self.help,
            Action::None => {}
        }
        true
    }

    /// Handle a mouse event: hover moves the crosshair, the wheel zooms, a left
    /// drag pans.
    pub fn mouse(&mut self, ev: MouseEvent) {
        let layout = self.layout();
        let (len, window) = (self.len(), self.window());
        self.nav.mouse(ev, layout.as_ref(), len, window);
    }

    /// Remember the terminal size frames are drawn at.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.size = (width, height);
    }

    /// The viewport for a frame `width` columns wide.
    pub fn viewport(&self, width: usize) -> Viewport {
        self.nav.viewport(self.len(), self.follow_window(width))
    }

    /// Render the chart (or the help screen) at the given size.
    pub fn render(&self, width: usize, height: usize, color: ColorMode) -> String {
        if self.help {
            return help_screen(width, height);
        }
        let title = self.status();
        frame::render(
            &self.state,
            &self.viewport(width),
            &self.frame(width, height, color, &title),
        )
    }

    fn frame<'a>(
        &self,
        width: usize,
        height: usize,
        color: ColorMode,
        title: &'a str,
    ) -> Frame<'a> {
        Frame {
            width,
            height,
            color,
            heatmap: self.settings.heatmap,
            crosshair: self.nav.pointer.crosshair,
            title,
        }
    }

    /// The layout of a frame at the remembered size.
    fn layout(&self) -> Option<Layout> {
        let (w, h) = self.size;
        frame::layout(
            &self.state,
            &self.viewport(w),
            &self.frame(w, h, ColorMode::Plain, ""),
        )
    }

    /// The transport + flow summary that leads the status line.
    fn status(&self) -> String {
        let icon = if self.playing {
            "▶"
        } else if self.position >= self.duration {
            "■"
        } else {
            "‖"
        };
        format!(
            "finchart REPLAY · {} · {icon} {}× {}/{} · CVD {:+.0} · ? keys",
            self.name,
            fmt_speed(self.speed),
            clock(self.position),
            clock(self.duration),
            self.state.cumulative_delta(),
        )
    }

    fn follow_window(&self, width: usize) -> usize {
        self.settings
            .window
            .unwrap_or_else(|| (width.saturating_sub(CHROME_COLS) / 3).max(10))
    }

    fn len(&self) -> usize {
        self.state.series().len()
    }

    /// The follow window at the remembered width.
    fn window(&self) -> usize {
        self.follow_window(self.size.0)
    }

    fn pan_by(&mut self, candles: isize) {
        let (len, window) = (self.len(), self.window());
        self.nav.pan(candles, len, window);
    }

    fn zoom_by(&mut self, factor: f64) {
        let (len, window) = (self.len(), self.window());
        self.nav.zoom(factor, len, window);
    }

    /// Fold every not-yet-applied event at or before the clock.
    fn fold_to_position(&mut self) {
        while let Some(ev) = self.events.get(self.applied) {
            if (ev.ts() - self.start_ts) as f64 > self.position {
                break;
            }
            self.state.apply(ev);
            self.applied += 1;
        }
    }
}

fn fresh_state(settings: &Settings) -> ChartState {
    ChartState::from_history(
        CandleSeries::new(),
        settings.interval_secs,
        settings.tick_size,
        Some(HEATMAP_CAPACITY),
        TRADE_CAP,
    )
}

/// The next speed up (`dir > 0`) or down from `speed` in [`SPEEDS`].
fn step_speed(speed: f64, dir: i32) -> f64 {
    if dir > 0 {
        SPEEDS
            .iter()
            .copied()
            .find(|&s| s > speed)
            .unwrap_or(SPEEDS[SPEEDS.len() - 1])
    } else {
        SPEEDS
            .iter()
            .rev()
            .copied()
            .find(|&s| s < speed)
            .unwrap_or(SPEEDS[0])
    }
}

fn fmt_speed(speed: f64) -> String {
    if speed.fract() == 0.0 {
        format!("{speed:.0}")
    } else {
        format!("{speed}")
    }
}

/// `m:ss` for a number of seconds.
pub fn clock(secs: f64) -> String {
    let whole = secs.max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

const HELP: &[&str] = &[
    "finchart replay — keys",
    "",
    "  space      play / pause",
    "  ← →        seek 5 s",
    "  ↓ ↑        seek 30 s",
    "  [ ]        slower / faster",
    "  0  End     restart / jump to the end",
    "  h l        pan back / forward",
    "  k j        zoom in / out",
    "  f          follow the newest candle",
    "  m          heatmap on / off",
    "  c          crosshair: read out a bar",
    "  , .        move the crosshair one bar",
    "  mouse      hover: crosshair · wheel: zoom · drag: pan",
    "  ?          close this help",
    "  Esc        hide the crosshair, else quit",
    "  q          quit",
];

/// The help text padded to exactly `width` × `height`.
fn help_screen(width: usize, height: usize) -> String {
    (0..height)
        .map(|r| {
            let line: String = HELP
                .get(r)
                .copied()
                .unwrap_or("")
                .chars()
                .take(width)
                .collect();
            let pad = width - line.chars().count();
            line + &" ".repeat(pad)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Play `player` in the terminal until a quit key.
pub fn run(mut player: Player) -> io::Result<()> {
    let mut screen = Screen::enter()?;
    let frame_dur = Duration::from_millis(33);
    let mut last = Instant::now();
    loop {
        let size = screen.size();
        player.resize(size.0, size.1);
        screen.paint(&player.render(size.0, size.1, ColorMode::Ansi), size)?;

        if event::poll(frame_dur)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if !player.act(action_for(key)) {
                        break;
                    }
                }
                Event::Mouse(ev) => player.mouse(ev),
                _ => {}
            }
        }
        let now = Instant::now();
        player.advance(now - last);
        last = now;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use market_core::{
        replay_events, CandleSeries, DepthLevel, DepthSnapshot, MarketEvent, Side, Tick,
    };

    const T0: i64 = 1_790_000_000;

    fn settings() -> Settings {
        Settings {
            interval_secs: 1,
            tick_size: 0.01,
            window: Some(30),
            heatmap: true,
        }
    }

    /// Ten seconds: a book at T0, a trade every other second, a heartbeat each second.
    fn session() -> Vec<MarketEvent> {
        let mut evs = vec![MarketEvent::DepthSnapshot(DepthSnapshot {
            ts: T0,
            bids: vec![DepthLevel {
                price: 99.99,
                size: 50.0,
            }],
            asks: vec![DepthLevel {
                price: 100.01,
                size: 40.0,
            }],
        })];
        for s in 0..=10 {
            if s % 2 == 1 {
                evs.push(MarketEvent::Trade(Tick {
                    ts: T0 + s,
                    price: 100.0 + s as f64 * 0.01,
                    size: 5.0,
                    side: Some(if s % 4 == 1 { Side::Buy } else { Side::Sell }),
                }));
            }
            evs.push(MarketEvent::Heartbeat { ts: T0 + s });
        }
        evs
    }

    fn player() -> Player {
        Player::new("session.txt", session(), settings())
    }

    /// The state a fresh fold reaches after `secs` seconds of the session.
    fn folded_to(secs: i64) -> market_core::ChartState {
        let s = settings();
        let mut state = market_core::ChartState::from_history(
            CandleSeries::new(),
            s.interval_secs,
            s.tick_size,
            Some(HEATMAP_CAPACITY),
            TRADE_CAP,
        );
        let upto: Vec<MarketEvent> = session()
            .into_iter()
            .filter(|e| e.ts() - T0 <= secs)
            .collect();
        replay_events(&mut state, &upto);
        state
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_new_player_starts_paused_at_the_first_event() {
        let p = player();
        assert_eq!(p.position(), 0.0);
        assert_eq!(p.duration(), 10.0);
        assert!(!p.is_playing());
        assert_eq!(*p.state(), folded_to(0));
        // The book keys levels by integer tick, so compare prices with a tolerance.
        assert!(p
            .state()
            .best_bid()
            .is_some_and(|b| (b - 99.99).abs() < 1e-9));
    }

    #[test]
    fn seeking_forward_and_back_matches_a_fresh_fold() {
        let mut p = player();
        p.seek(7.0);
        assert_eq!(*p.state(), folded_to(7));
        p.seek(3.0);
        assert_eq!(*p.state(), folded_to(3));
        p.seek(3.5); // mid-second: nothing new yet
        assert_eq!(*p.state(), folded_to(3));
    }

    #[test]
    fn seeking_clamps_to_the_session() {
        let mut p = player();
        p.seek(-5.0);
        assert_eq!(p.position(), 0.0);
        p.seek(99.0);
        assert_eq!(p.position(), 10.0);
        assert_eq!(*p.state(), folded_to(10));
    }

    #[test]
    fn playback_advances_by_wall_time_times_speed() {
        let mut p = player();
        p.advance(Duration::from_secs(1)); // paused: nothing moves
        assert_eq!(p.position(), 0.0);

        p.act(Action::TogglePlay);
        p.advance(Duration::from_millis(1500));
        assert_eq!(p.position(), 1.5);

        p.act(Action::Faster); // 1x -> 2x
        p.advance(Duration::from_secs(2));
        assert_eq!(p.position(), 5.5);
        assert_eq!(*p.state(), folded_to(5));
    }

    #[test]
    fn playback_stops_at_the_end() {
        let mut p = player();
        p.act(Action::TogglePlay);
        p.advance(Duration::from_secs(60));
        assert_eq!(p.position(), 10.0);
        assert!(!p.is_playing());
        // Play at the end restarts from the beginning.
        p.act(Action::TogglePlay);
        assert!(p.is_playing());
        assert_eq!(p.position(), 0.0);
    }

    #[test]
    fn speed_steps_are_clamped() {
        let mut p = player();
        for _ in 0..20 {
            p.act(Action::Faster);
        }
        assert_eq!(p.speed(), *SPEEDS.last().unwrap());
        for _ in 0..20 {
            p.act(Action::Slower);
        }
        assert_eq!(p.speed(), SPEEDS[0]);
    }

    #[test]
    fn panning_leaves_follow_mode_and_f_returns_to_the_live_edge() {
        let mut p = player();
        p.seek(10.0);
        let len = p.state().series().len();
        assert!(p.is_following());
        assert_eq!(p.viewport(120).end_index, len);

        p.act(Action::ZoomIn);
        p.act(Action::PanLeft);
        assert!(!p.is_following());
        assert!(p.viewport(120).end_index < len);

        p.act(Action::Follow);
        assert!(p.is_following());
        assert_eq!(p.viewport(120).end_index, len);
    }

    #[test]
    fn restart_and_end_jump_to_the_session_bounds() {
        let mut p = player();
        p.act(Action::End);
        assert_eq!(p.position(), 10.0);
        p.act(Action::Restart);
        assert_eq!(p.position(), 0.0);
        p.act(Action::Seek(4.0));
        assert_eq!(p.position(), 4.0);
    }

    #[test]
    fn keys_map_to_actions() {
        assert_eq!(action_for(key(KeyCode::Char(' '))), Action::TogglePlay);
        assert_eq!(action_for(key(KeyCode::Right)), Action::Seek(SEEK_SHORT));
        assert_eq!(action_for(key(KeyCode::Left)), Action::Seek(-SEEK_SHORT));
        assert_eq!(action_for(key(KeyCode::Up)), Action::Seek(SEEK_LONG));
        assert_eq!(action_for(key(KeyCode::Down)), Action::Seek(-SEEK_LONG));
        assert_eq!(action_for(key(KeyCode::Char(']'))), Action::Faster);
        assert_eq!(action_for(key(KeyCode::Char('['))), Action::Slower);
        assert_eq!(action_for(key(KeyCode::Char('h'))), Action::PanLeft);
        assert_eq!(action_for(key(KeyCode::Char('l'))), Action::PanRight);
        assert_eq!(action_for(key(KeyCode::Char('k'))), Action::ZoomIn);
        assert_eq!(action_for(key(KeyCode::Char('j'))), Action::ZoomOut);
        assert_eq!(action_for(key(KeyCode::Char('f'))), Action::Follow);
        assert_eq!(action_for(key(KeyCode::Char('0'))), Action::Restart);
        assert_eq!(action_for(key(KeyCode::End)), Action::End);
        assert_eq!(action_for(key(KeyCode::Char('m'))), Action::ToggleHeatmap);
        assert_eq!(action_for(key(KeyCode::Char('?'))), Action::ToggleHelp);
        assert_eq!(action_for(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(action_for(key(KeyCode::Char('x'))), Action::None);
        assert!(!player().act(Action::Quit));
    }

    #[test]
    fn frames_keep_their_size_and_carry_the_transport_state() {
        let mut p = player();
        p.seek(5.0);
        let out = p.render(120, 30, ColorMode::Plain);
        assert_eq!(out.lines().count(), 30);
        for line in out.lines() {
            assert_eq!(line.chars().count(), 120);
        }
        let status = out.lines().next().unwrap();
        assert!(status.contains("session.txt"), "{status}");
        assert!(status.contains("0:05/0:10"), "{status}");
        assert!(status.contains("1×"), "{status}");
    }

    #[test]
    fn the_help_screen_lists_the_keys() {
        let mut p = player();
        p.act(Action::ToggleHelp);
        let out = p.render(80, 24, ColorMode::Plain);
        assert_eq!(out.lines().count(), 24);
        assert!(out.contains("play / pause"));
        p.act(Action::ToggleHelp);
        assert!(!p.render(80, 24, ColorMode::Plain).contains("play / pause"));
    }

    #[test]
    fn the_heatmap_toggles() {
        let mut p = player();
        p.seek(10.0);
        assert!(p.render(120, 30, ColorMode::Plain).contains("heatmap"));
        p.act(Action::ToggleHeatmap);
        assert!(!p.render(120, 30, ColorMode::Plain).contains("heatmap"));
    }

    #[test]
    fn settings_come_from_flags_then_header_then_defaults() {
        let header = "# finchart interval=60 tick=0.05\nH 1\n";
        let s = Settings::resolve(header, None, None, None, true);
        assert_eq!((s.interval_secs, s.tick_size), (60, 0.05));

        let s = Settings::resolve(header, Some(5), Some(0.25), Some(40), false);
        assert_eq!(
            (s.interval_secs, s.tick_size, s.window),
            (5, 0.25, Some(40))
        );
        assert!(!s.heatmap);

        let s = Settings::resolve("H 1\n", None, None, None, true);
        assert_eq!(
            (s.interval_secs, s.tick_size),
            (DEFAULT_INTERVAL, DEFAULT_TICK)
        );
    }

    #[test]
    fn times_format_as_minutes_and_seconds() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(75.9), "1:15");
        assert_eq!(clock(3600.0), "60:00");
    }

    #[test]
    fn an_empty_recording_is_a_zero_length_session() {
        let p = Player::new("empty.txt", vec![], settings());
        assert_eq!(p.duration(), 0.0);
        assert_eq!(p.render(80, 20, ColorMode::Plain).lines().count(), 20);
    }

    // --- crosshair & mouse ------------------------------------------------------

    fn mouse_at(kind: crossterm::event::MouseEventKind, col: u16) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind,
            column: col,
            row: 8,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn crosshair_keys_map_to_actions() {
        assert_eq!(action_for(key(KeyCode::Char('c'))), Action::ToggleCrosshair);
        assert_eq!(action_for(key(KeyCode::Char(','))), Action::StepBack);
        assert_eq!(action_for(key(KeyCode::Char('.'))), Action::StepForward);
        assert_eq!(action_for(key(KeyCode::Esc)), Action::Escape);
        assert_eq!(action_for(key(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn escape_hides_the_crosshair_before_it_quits() {
        let mut p = player();
        p.seek(10.0);
        p.act(Action::ToggleCrosshair);
        assert!(p.render(120, 30, ColorMode::Plain).starts_with("⌖"));
        assert!(p.act(Action::Escape), "first Esc only hides the crosshair");
        assert!(!p.render(120, 30, ColorMode::Plain).starts_with("⌖"));
        assert!(!p.act(Action::Escape), "second Esc quits");
    }

    #[test]
    fn stepping_reads_out_an_earlier_bar() {
        let mut p = player();
        p.seek(10.0);
        p.act(Action::ToggleCrosshair);
        let newest = p.render(120, 30, ColorMode::Plain);
        p.act(Action::StepBack);
        let earlier = p.render(120, 30, ColorMode::Plain);
        let status = |f: &str| f.lines().next().unwrap().to_string();
        assert_ne!(status(&newest), status(&earlier));
        assert!(status(&earlier).starts_with("⌖"));
    }

    #[test]
    fn the_wheel_zooms_and_a_drag_pans_out_of_follow_mode() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut p = player();
        p.seek(10.0);
        p.mouse(mouse_at(MouseEventKind::ScrollUp, 60));
        assert!(!p.is_following());
        let zoomed = p.viewport(120);
        p.act(Action::Follow);

        p.mouse(mouse_at(MouseEventKind::Down(MouseButton::Left), 100));
        p.mouse(mouse_at(MouseEventKind::Drag(MouseButton::Left), 30));
        assert!(!p.is_following(), "a drag pans");
        assert!(zoomed.candle_count() < p.state().series().len());
    }
}
