#![allow(
    clippy::too_many_arguments,
    dead_code
)]

// winamp_tui — Terminal Winamp classic clone with real tunes4r backend
//
// Usage:
//   cargo run --example winamp_tui                          (open file browser)
//   cargo run --example winamp_tui -- /path/to/song.mp3      (play directly)
//
// Controls:
//   Space  play/pause     S  stop          ←/→  seek (2 s)
//   Enter  confirm seek   Esc  cancel seek  R  toggle remaining
//   +/-    volume         [/]  balance      \  centre balance
//   O      open file browser               L  toggle log pane
//   E      toggle EQ       P  toggle PL     Q  quit

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
    Terminal,
};
use tunes4r::audio::engine::types::{set_band_count, GLOBAL_SPECTRUM};

// ─────────────────────────────────────────────────────────────────────────────
// Colour palette
// ─────────────────────────────────────────────────────────────────────────────
const C_LCD_BG: Color = Color::Rgb(20, 35, 20);
const C_LCD_ON: Color = Color::Rgb(57, 255, 20);
const C_LCD_OFF: Color = Color::Rgb(26, 46, 26);
const C_BODY_DARK: Color = Color::Rgb(19, 18, 28);
const C_TITLE_FG: Color = Color::Rgb(200, 200, 216);
const C_STATE_RED: Color = Color::Rgb(204, 51, 0);
const C_SPEC_GRN: Color = Color::Rgb(0, 204, 0);
const C_SPEC_DKB: Color = Color::Rgb(180, 120, 0);
const C_SPEC_AMB: Color = Color::Rgb(255, 170, 0);
const C_BADGE_BG: Color = Color::Rgb(10, 26, 10);
const C_GRAY: Color = Color::Rgb(107, 107, 122);
const C_DARK_GRAY: Color = Color::Rgb(40, 40, 55);
const C_LOG_BG: Color = Color::Rgb(10, 10, 18);
const C_LOG_FG: Color = Color::Rgb(140, 200, 140);
const C_LOG_WARN: Color = Color::Rgb(255, 200, 0);
const C_LOG_ERR: Color = Color::Rgb(220, 60, 60);
const C_FILE_SEL: Color = Color::Rgb(57, 255, 20);
const C_FILE_DIR: Color = Color::Rgb(100, 180, 255);
const C_BTN_BG: Color = Color::Rgb(42, 42, 66);
const C_BTN_BEVEL_HI: Color = Color::Rgb(180, 180, 205);
const C_BTN_BEVEL_LO: Color = Color::Rgb(12, 12, 24);
const C_BTN_PRESSED: Color = Color::Rgb(26, 26, 44);

// ─────────────────────────────────────────────────────────────────────────────
// Seven-segment glyphs (4 cols × 5 rows each, Unicode half-blocks)
// ─────────────────────────────────────────────────────────────────────────────
fn seven_seg_rows(digit: u8) -> [&'static str; 5] {
    match digit % 10 {
        0 => [" ▄▄ ", "█  █", "    ", "█  █", " ▀▀ "],
        1 => ["    ", "   █", "    ", "   █", "    "],
        2 => [" ▄▄ ", "   █", " ▄▄ ", "█   ", " ▀▀▀"],
        3 => [" ▄▄ ", "   █", " ▄▄ ", "   █", " ▀▀ "],
        4 => ["    ", "█  █", " ▄▄▄", "   █", "    "],
        5 => [" ▄▄▄", "█   ", " ▄▄ ", "   █", " ▀▀ "],
        6 => [" ▄▄ ", "█   ", " ▄▄ ", "█  █", " ▀▀ "],
        7 => [" ▄▄▄", "   █", "    ", "   █", "    "],
        8 => [" ▄▄ ", "█  █", " ▄▄ ", "█  █", " ▀▀ "],
        9 => [" ▄▄ ", "█  █", " ▄▄▄", "   █", " ▀▀ "],
        _ => ["    ", "    ", "    ", "    ", "    "],
    }
}

fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}", s / 60, s % 60)
}

// ─────────────────────────────────────────────────────────────────────────────
// Log ring buffer with levels
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Clone)]
enum LogLevel {
    Info,
    Warn,
    #[allow(dead_code)]
    Error,
}

#[derive(Clone)]
struct LogEntry {
    level: LogLevel,
    msg: String,
}

impl LogEntry {
    fn info(msg: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            msg: msg.into(),
        }
    }
    fn warn(msg: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Warn,
            msg: msg.into(),
        }
    }
    fn error(msg: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Error,
            msg: msg.into(),
        }
    }
    fn color(&self) -> Color {
        match self.level {
            LogLevel::Info => C_LOG_FG,
            LogLevel::Warn => C_LOG_WARN,
            LogLevel::Error => C_LOG_ERR,
        }
    }
    fn prefix(&self) -> &'static str {
        match self.level {
            LogLevel::Info => "[INFO] ",
            LogLevel::Warn => "[WARN] ",
            LogLevel::Error => "[ERR ] ",
        }
    }
}

struct LogBuffer {
    entries: VecDeque<LogEntry>,
    cap: usize,
}

impl LogBuffer {
    fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(cap),
            cap,
        }
    }
    fn push(&mut self, e: LogEntry) {
        if self.entries.len() >= self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(e);
    }
    fn log(&mut self, msg: impl Into<String>) {
        self.push(LogEntry::info(msg));
    }
    fn warn(&mut self, msg: impl Into<String>) {
        self.push(LogEntry::warn(msg));
    }
    fn error(&mut self, msg: impl Into<String>) {
        self.push(LogEntry::error(msg));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Spectrum state (uses real GLOBAL_SPECTRUM when playing)
// ─────────────────────────────────────────────────────────────────────────────
const N_BARS: usize = 18;

struct SpectrumState {
    smoothed: [f32; N_BARS],
    peaks: [f32; N_BARS],
    peak_vel: [f32; N_BARS],
}

impl SpectrumState {
    fn new() -> Self {
        Self {
            smoothed: [0.0; N_BARS],
            peaks: [0.0; N_BARS],
            peak_vel: [0.0; N_BARS],
        }
    }

    fn update(&mut self, is_playing: bool, delta: f32) {
        if !is_playing {
            for a in &mut self.smoothed {
                *a = (*a * 0.82).max(0.0);
            }
            for p in &mut self.peaks {
                *p = 0.0;
            }
            return;
        }
        // Read real spectrum from the engine
        let raw: Vec<f32> = {
            let guard = GLOBAL_SPECTRUM.read().unwrap();
            guard.iter().take(N_BARS).copied().collect()
        };
        for i in 0..N_BARS {
            let t = raw.get(i).copied().unwrap_or(0.0);
            let c = self.smoothed[i];
            self.smoothed[i] = if t > c {
                (c + 0.22 * (t - c)).min(1.0)
            } else {
                (c - 0.10 * (c - t)).max(0.0)
            };
            let amp = self.smoothed[i];
            if amp >= self.peaks[i] {
                self.peak_vel[i] = (amp - self.peaks[i]) * 2.0;
                self.peaks[i] = amp;
            } else {
                self.peak_vel[i] -= 0.04 * delta / 0.033;
                self.peaks[i] = (self.peaks[i] + self.peak_vel[i]).clamp(0.0, 1.0);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scrolling title (accumulator-based)
// ─────────────────────────────────────────────────────────────────────────────
struct ScrollingTitle {
    accum: f32,
    offset: usize,
    padded: String,
    chars_per_sec: f32,
}

impl ScrollingTitle {
    fn new() -> Self {
        Self {
            accum: 0.0,
            offset: 0,
            padded: String::new(),
            chars_per_sec: 8.0,
        }
    }

    fn set_text(&mut self, text: &str) {
        let new_padded = format!("  {}  ·  ", text);
        if new_padded != self.padded {
            self.padded = new_padded;
            self.offset = 0;
            self.accum = 0.0;
        }
    }

    fn tick(&mut self, delta: f32) {
        if self.padded.is_empty() {
            return;
        }
        self.accum += delta * self.chars_per_sec;
        let steps = self.accum.floor() as usize;
        if steps > 0 {
            self.accum -= steps as f32;
            let len = self.padded.chars().count().max(1);
            self.offset = (self.offset + steps) % len;
        }
    }

    fn visible(&self, width: usize) -> String {
        if self.padded.is_empty() {
            return " ".repeat(width);
        }
        let chars: Vec<char> = self.padded.chars().collect();
        let len = chars.len();
        (0..width).map(|i| chars[(self.offset + i) % len]).collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scrub state
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Default)]
struct ScrubState {
    position_ms: u64,
    active: bool,
}

impl ScrubState {
    fn enter(&mut self, ms: u64) {
        self.position_ms = ms;
        self.active = true;
    }
    fn cancel(&mut self) {
        self.active = false;
        self.position_ms = 0;
    }
    fn commit(&mut self) -> Option<u64> {
        if self.active {
            self.active = false;
            let ms = self.position_ms;
            self.position_ms = 0;
            Some(ms)
        } else {
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Engine snapshot (mirrors winamptest_ui.rs)
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Clone)]
struct EngineSnapshot {
    state: tunes4r::models::PlaybackState,
    position: tunes4r::models::PlaybackPosition,
    load_error: String,
    meta_title: String,
    meta_artist: String,
}

impl EngineSnapshot {
    fn capture(engine: &tunes4r::PlaybackEngine) -> Self {
        let info = engine.source_info();
        Self {
            state: engine.get_state(),
            position: engine.get_position(),
            load_error: engine.load_error(),
            meta_title: info
                .as_ref()
                .and_then(|i| i.title.clone())
                .unwrap_or_default(),
            meta_artist: info
                .as_ref()
                .and_then(|i| i.artist.clone())
                .unwrap_or_default(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TUI File Browser
// ─────────────────────────────────────────────────────────────────────────────
const AUDIO_EXTS: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac", "opus", "wma"];

struct FileBrowser {
    cwd: PathBuf,
    entries: Vec<(String, bool)>,
    selected: usize,
}

impl FileBrowser {
    fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut fb = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
        };
        fb.refresh();
        fb
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.entries.push(("..".to_string(), true));
        if let Ok(rd) = std::fs::read_dir(&self.cwd) {
            let mut dirs: Vec<String> = Vec::new();
            let mut files: Vec<String> = Vec::new();
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    dirs.push(name);
                } else {
                    let ext = Path::new(&name)
                        .extension()
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    if AUDIO_EXTS.contains(&ext.as_str()) {
                        files.push(name);
                    }
                }
            }
            dirs.sort();
            files.sort();
            for d in dirs {
                self.entries.push((d, true));
            }
            for f in files {
                self.entries.push((f, false));
            }
        }
        self.selected = 0;
    }

    fn selected_path(&self) -> Option<PathBuf> {
        let (name, _) = self.entries.get(self.selected)?;
        Some(self.cwd.join(name))
    }

    fn enter(&mut self) -> Option<PathBuf> {
        let (name, is_dir) = self.entries.get(self.selected)?.clone();
        if is_dir {
            let next = if name == ".." {
                self.cwd
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| self.cwd.clone())
            } else {
                self.cwd.join(&name)
            };
            self.cwd = next;
            self.refresh();
            None
        } else {
            Some(self.cwd.join(&name))
        }
    }

    fn up(&mut self) {
        self.selected = if self.selected == 0 {
            self.entries.len().saturating_sub(1)
        } else {
            self.selected - 1
        };
    }

    fn down(&mut self) {
        self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UI mode
// ─────────────────────────────────────────────────────────────────────────────
enum UiMode {
    Player,
    FileBrowser,
}

// ─────────────────────────────────────────────────────────────────────────────
// App state
// ─────────────────────────────────────────────────────────────────────────────
struct WinampApp {
    engine: Arc<Mutex<tunes4r::PlaybackEngine>>,
    snap: EngineSnapshot,
    spectrum: SpectrumState,
    scrolling: ScrollingTitle,
    scrub: ScrubState,
    shuffle: bool,
    repeat: bool,
    eq_on: bool,
    pl_on: bool,
    show_remaining: bool,
    show_log: bool,
    log: LogBuffer,
    last_tick: Instant,
    url: String,
    error: Option<String>,
    volume: f32,
    balance: f32,
    mode: UiMode,
    browser: FileBrowser,
}

impl WinampApp {
    fn new(engine: Arc<Mutex<tunes4r::PlaybackEngine>>, url: Option<String>) -> Self {
        set_band_count(N_BARS);

        let mut log = LogBuffer::new(200);
        log.log("Winamp TUI started — press O to open a file");
        log.log("Controls: Space=play/pause  S=stop  ←/→=seek  +/-=volume  [/]=balance");

        let (play_url, snap) = if let Some(ref u) = url {
            let u = u.clone();
            {
                let mut e = engine.lock().unwrap();
                let _ = e.play(&u, None);
            }
            let snap = EngineSnapshot::capture(&engine.lock().unwrap());
            log.log(format!("Playing: {}", u));
            (u, snap)
        } else {
            (
                String::new(),
                EngineSnapshot::capture(&engine.lock().unwrap()),
            )
        };

        Self {
            engine,
            snap,
            spectrum: SpectrumState::new(),
            scrolling: ScrollingTitle::new(),
            scrub: ScrubState::default(),
            shuffle: false,
            repeat: false,
            eq_on: false,
            pl_on: false,
            show_remaining: false,
            show_log: false,
            log,
            last_tick: Instant::now(),
            url: play_url,
            error: None,
            volume: 0.8,
            balance: 0.5,
            mode: UiMode::Player,
            browser: FileBrowser::new(),
        }
    }

    fn poll_engine(&mut self) {
        let snap = {
            let e = self.engine.lock().unwrap();
            EngineSnapshot::capture(&e)
        };
        if !snap.load_error.is_empty() {
            self.error = Some(snap.load_error.clone());
        }
        self.snap = snap;
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let delta = now.duration_since(self.last_tick).as_secs_f32();
        self.last_tick = now;

        self.poll_engine();
        let is_playing = self.is_playing();
        self.spectrum.update(is_playing, delta);

        let text = self.title_text();
        self.scrolling.set_text(&text);
        self.scrolling.tick(delta);
    }

    fn is_playing(&self) -> bool {
        matches!(self.snap.state, tunes4r::models::PlaybackState::Playing)
            && !(self.snap.position.total_ms > 0
                && self.snap.position.current_ms >= self.snap.position.total_ms.saturating_sub(500))
    }

    fn current_ms(&self) -> u64 {
        if self.scrub.active {
            self.scrub.position_ms
        } else {
            self.snap.position.current_ms
        }
    }

    fn total_ms(&self) -> u64 {
        self.snap.position.total_ms
    }

    fn playback_state(&self) -> tunes4r::models::PlaybackState {
        self.snap.state.clone()
    }

    fn title_text(&self) -> String {
        if let Some(err) = &self.error {
            return err.clone();
        }
        if !self.snap.meta_artist.is_empty() || !self.snap.meta_title.is_empty() {
            format!("{} — {}", self.snap.meta_artist, self.snap.meta_title)
        } else if !self.url.is_empty() {
            self.url.clone()
        } else {
            "Winamp Classic — press O to open".to_string()
        }
    }

    fn load_file(&mut self, path: PathBuf) {
        let p = path.to_string_lossy().to_string();
        self.log.log(format!("Loading: {}", p));
        self.url = p.clone();
        self.error = None;
        {
            let mut e = self.engine.lock().unwrap();
            let _ = e.play(&p, None);
        }
        self.log.log(format!("Playing: {}", p));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rendering helpers
// ─────────────────────────────────────────────────────────────────────────────

fn fill_bg(buf: &mut Buffer, area: Rect, color: Color) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_bg(color).set_char(' ');
        }
    }
}

fn put_str(buf: &mut Buffer, x: u16, y: u16, right: u16, s: &str, fg: Color, bg: Color) {
    for (i, ch) in s.chars().enumerate() {
        let cx = x + i as u16;
        if cx >= right {
            break;
        }
        buf[(cx, y)].set_char(ch).set_fg(fg).set_bg(bg);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: Title bar
// ─────────────────────────────────────────────────────────────────────────────
struct TitleBar;

impl Widget for TitleBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_BODY_DARK);
        if area.width < 6 {
            return;
        }

        let gold = Color::Rgb(0xe7, 0xcf, 0x86);


        // Corners
        buf[(area.left(), area.y)]
            .set_char('┌')
            .set_fg(C_GRAY)
            .set_bg(C_BODY_DARK);
        buf[(area.right() - 1, area.y)]
            .set_char('┐')
            .set_fg(C_GRAY)
            .set_bg(C_BODY_DARK);

        // Top border ─ between corners
        for x in (area.left() + 1)..(area.right() - 1) {
            buf[(x, area.y)]
                .set_char('─')
                .set_fg(C_GRAY)
                .set_bg(C_BODY_DARK);
        }

        // Menu icon
        if area.left() + 2 < area.right() {
            buf[(area.left() + 1, area.y)]
                .set_char(' ')
                .set_bg(C_BODY_DARK);
            buf[(area.left() + 2, area.y)]
                .set_char('▒')
                .set_fg(gold)
                .set_bg(C_BODY_DARK);
            buf[(area.left() + 3, area.y)]
                .set_char(' ')
                .set_bg(C_BODY_DARK);
        }

        // Chrome buttons " _ □ ×"
        let chrome = " _ □ ×";
        let chrome_start = area.right().saturating_sub(chrome.len() as u16 + 1);
        for (i, ch) in chrome.chars().enumerate() {
            let cx = chrome_start + i as u16;
            if cx < area.right() - 1 {
                buf[(cx, area.y)]
                    .set_char(ch)
                    .set_fg(C_GRAY)
                    .set_bg(C_BODY_DARK);
            }
        }

        // ════ WINAMP ════ centered between menu and chrome
        let title = "════ WINAMP ════";
        let left_bound = area.left() + 4;
        let right_bound = chrome_start;
        let avail = right_bound.saturating_sub(left_bound);
        if avail >= title.len() as u16 {
            let tx = left_bound + (avail.saturating_sub(title.len() as u16)) / 2;
            for (i, ch) in title.chars().enumerate() {
                let cx = tx + i as u16;
                if cx >= right_bound {
                    break;
                }
                let fg = if ch == '═' { gold } else { C_TITLE_FG };
                buf[(cx, area.y)]
                    .set_char(ch)
                    .set_fg(fg)
                    .set_bg(C_BODY_DARK);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: LCD panel  (timer + spectrum)
// ─────────────────────────────────────────────────────────────────────────────
struct LcdPanel<'a> {
    state: &'a tunes4r::models::PlaybackState,
    current_ms: u64,
    total_ms: u64,
    show_remaining: bool,
    spectrum: &'a SpectrumState,
}

impl<'a> Widget for LcdPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_LCD_BG);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(C_LCD_OFF).bg(C_LCD_BG));
        block.render(area, buf);

        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // Row 0: state icon + CUR/REM label
        let (state_ch, state_color) = match self.state {
            tunes4r::models::PlaybackState::Playing => ('▶', C_LCD_ON),
            tunes4r::models::PlaybackState::Paused => ('⏸', Color::Rgb(255, 200, 0)),
            tunes4r::models::PlaybackState::Stopped => ('■', C_STATE_RED),
            tunes4r::models::PlaybackState::Connecting => ('◌', C_SPEC_AMB),
            tunes4r::models::PlaybackState::Buffering { .. } => ('◌', C_SPEC_AMB),
            tunes4r::models::PlaybackState::Decoding => ('◌', C_SPEC_AMB),
            tunes4r::models::PlaybackState::Error(_) => ('‼', C_STATE_RED),
        };
        buf[(inner.x, inner.y)]
            .set_char(state_ch)
            .set_fg(state_color)
            .set_bg(C_LCD_BG);
        let label = if self.show_remaining { " REM" } else { " CUR" };
        put_str(
            buf,
            inner.x + 1,
            inner.y,
            inner.right(),
            label,
            C_LCD_OFF,
            C_LCD_BG,
        );

        // Rows 1–5: 7-segment time digits
        let (time_ms, with_minus) = if self.total_ms > 0 && self.show_remaining {
            (self.total_ms.saturating_sub(self.current_ms), true)
        } else {
            (self.current_ms, false)
        };
        let time_str = fmt_ms(time_ms);

        // Right-justify the timer
        const TIMER_W: u16 = 22; // 4 digits * 5 + colon * 2
        let mut dx = inner.right().saturating_sub(TIMER_W);
        if dx < inner.x {
            dx = inner.x;
        }

        if with_minus && inner.y + 3 < inner.bottom() {
            let mx = dx.saturating_sub(2);
            if mx >= inner.x {
                buf[(mx, inner.y + 3)]
                    .set_char('─')
                    .set_fg(C_LCD_ON)
                    .set_bg(C_LCD_BG);
            }
        }
        for ch in time_str.chars() {
            if ch == ':' {
                for dot_y_off in [1u16, 3u16] {
                    let ry = inner.y + 1 + dot_y_off;
                    if ry < inner.bottom() && dx < inner.right() {
                        buf[(dx, ry)]
                            .set_char('•')
                            .set_fg(C_LCD_ON)
                            .set_bg(C_LCD_BG);
                    }
                }
                dx += 2;
            } else if let Some(d) = ch.to_digit(10) {
                let rows = seven_seg_rows(d as u8);
                for (ri, row_str) in rows.iter().enumerate() {
                    let ry = inner.y + 1 + ri as u16;
                    if ry >= inner.bottom() {
                        break;
                    }
                    for (ci, glyph) in row_str.chars().enumerate() {
                        let cx = dx + ci as u16;
                        if cx >= inner.right() {
                            break;
                        }
                        let fg = if glyph == ' ' { C_LCD_OFF } else { C_LCD_ON };
                        let ch2 = if glyph == ' ' { '·' } else { glyph };
                        buf[(cx, ry)].set_char(ch2).set_fg(fg).set_bg(C_LCD_BG);
                    }
                }
                dx += 5;
            }
        }

        // Spectrum bars (rows 7+)
        let spec_y_start = inner.y + 7;
        if spec_y_start >= inner.bottom() {
            return;
        }
        let spec_h = inner.bottom() - spec_y_start;
        let spec_x = inner.x;
        let spec_w = inner.width;

        const RULE_A: Color = Color::Rgb(0, 170, 170);
        const RULE_B: Color = Color::Rgb(0, 136, 136);

        // Grid dots on left edge (Winamp-style ruler)
        for off in (0..spec_h.saturating_sub(1)).step_by(2) {
            let ry = spec_y_start + off;
            let c = if (off / 2) % 2 == 0 { RULE_A } else { RULE_B };
            buf[(spec_x, ry)].set_char('·').set_fg(c).set_bg(C_LCD_BG);
        }
        // Grid dots on bottom edge
        let bottom_y = inner.bottom().saturating_sub(1);
        if bottom_y >= spec_y_start {
            for off in (0..spec_w).step_by(2) {
                let cx = spec_x + off;
                let c = if (off / 2) % 2 == 0 { RULE_A } else { RULE_B };
                buf[(cx, bottom_y)].set_char('·').set_fg(c).set_bg(C_LCD_BG);
            }
        }

        // Bars area (padded: 2 cols from left, 1 row from bottom)
        let bars_x = spec_x + 2;
        let bars_w = spec_w.saturating_sub(3);
        let bars_h = spec_h.saturating_sub(1);
        if bars_w == 0 || bars_h == 0 {
            return;
        }

        let bar_count = N_BARS.min(bars_w as usize);

        let total_cells = bars_h as f32;

        for i in 0..bar_count {
            let amp = self.spectrum.smoothed[i];
            let peak = self.spectrum.peaks[i];

            let filled = (amp * total_cells).round() as u16;
            let bx = bars_x + i as u16;
            for row in 0..bars_h {
                let ry = spec_y_start + (bars_h - 1 - row);
                if ry >= inner.bottom() || bx >= inner.right() {
                    continue;
                }
                if row >= filled {
                    break;
                }
                let n = row as f32 / bars_h as f32;
                let color = if n < 0.25 {
                    C_SPEC_GRN
                } else if n < 0.55 {
                    C_SPEC_DKB
                } else {
                    C_SPEC_AMB
                };
                buf[(bx, ry)].set_char('█').set_fg(color).set_bg(C_LCD_BG);
            }

            if peak > 0.02 {
                let peak_row = (peak * total_cells).round() as u16;
                if peak_row > 0 && peak_row <= bars_h {
                    let ry = spec_y_start + (bars_h - peak_row);
                    if ry < inner.bottom() && bx < inner.right() {
                        buf[(bx, ry)]
                            .set_char('━')
                            .set_fg(Color::White)
                            .set_bg(C_LCD_BG);
                    }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: Metadata panel
// ─────────────────────────────────────────────────────────────────────────────
struct MetadataPanel<'a> {
    title_visible: String,
    volume: f32,
    balance: f32,
    eq_on: bool,
    pl_on: bool,
    is_playing: bool,
    error: Option<&'a str>,
}

impl<'a> Widget for MetadataPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_BODY_DARK);
        if area.height == 0 {
            return;
        }

        // Row 0–2: scrolling title in beveled text box (50 chars wide)
        let title_color = if self.error.is_some() {
            C_STATE_RED
        } else {
            C_LCD_ON
        };
        let box_w = area.width.min(50);
        let inner_w = box_w.saturating_sub(2) as usize;
        if inner_w == 0 || area.height < 3 {
            return;
        }
        let bevel_dark = Color::Rgb(6, 18, 6);
        let bevel_light = C_GRAY;

        buf[(area.x, area.y)]
            .set_char('┌')
            .set_fg(bevel_dark)
            .set_bg(C_BODY_DARK);
        for x in (area.x + 1)..(area.x + box_w - 1) {
            if x < area.right() {
                buf[(x, area.y)]
                    .set_char('─')
                    .set_fg(bevel_dark)
                    .set_bg(C_BODY_DARK);
            }
        }
        if area.x + box_w - 1 < area.right() {
            buf[(area.x + box_w - 1, area.y)]
                .set_char('┐')
                .set_fg(bevel_dark)
                .set_bg(C_BODY_DARK);
        }

        let content_rect = Rect {
            x: area.x,
            y: area.y + 1,
            width: box_w,
            height: 1,
        };
        fill_bg(buf, content_rect, C_BADGE_BG);
        let title: String = self.title_visible.chars().take(inner_w).collect();
        let padded = format!("{:<width$}", title, width = inner_w);
        buf[(area.x, area.y + 1)]
            .set_char('│')
            .set_fg(bevel_dark)
            .set_bg(C_BADGE_BG);
        put_str(
            buf,
            area.x + 1,
            area.y + 1,
            area.x + box_w - 1,
            &padded,
            title_color,
            C_BADGE_BG,
        );
        if area.x + box_w - 1 < area.right() {
            buf[(area.x + box_w - 1, area.y + 1)]
                .set_char('│')
                .set_fg(bevel_light)
                .set_bg(C_BADGE_BG);
        }

        buf[(area.x, area.y + 2)]
            .set_char('└')
            .set_fg(bevel_light)
            .set_bg(C_BODY_DARK);
        for x in (area.x + 1)..(area.x + box_w - 1) {
            if x < area.right() {
                buf[(x, area.y + 2)]
                    .set_char('─')
                    .set_fg(bevel_light)
                    .set_bg(C_BODY_DARK);
            }
        }
        if area.x + box_w - 1 < area.right() {
            buf[(area.x + box_w - 1, area.y + 2)]
                .set_char('┘')
                .set_fg(bevel_light)
                .set_bg(C_BODY_DARK);
        }

        if area.height < 4 {
            return;
        }

        // Row 3: bitrate / samplerate / MONO · STEREO
        let badge = " [256kbps] [44kHz]  ";
        fill_bg(
            buf,
            Rect {
                x: area.x,
                y: area.y + 3,
                width: area.width,
                height: 1,
            },
            C_BADGE_BG,
        );
        put_str(
            buf,
            area.x,
            area.y + 3,
            area.right(),
            badge,
            C_LCD_ON,
            C_BADGE_BG,
        );
        let mono_color = C_LCD_OFF;
        let stereo_color = if self.is_playing { C_LCD_ON } else { C_LCD_OFF };
        let label_x = area.x + badge.len() as u16;
        put_str(
            buf,
            label_x,
            area.y + 3,
            area.right(),
            "MONO ",
            mono_color,
            C_BADGE_BG,
        );
        put_str(
            buf,
            label_x + 5,
            area.y + 3,
            area.right(),
            "STEREO",
            stereo_color,
            C_BADGE_BG,
        );

        if area.height < 5 {
            return;
        }

        // Row 4: Volume slider
        let vol_w = area.width.min(28);
        render_labeled_slider(
            buf,
            area.x,
            area.y + 4,
            vol_w,
            self.volume,
            "VOL",
            Color::Rgb(57, 255, 20),
            C_BODY_DARK,
        );
        if area.height < 6 {
            return;
        }

        // Row 5: Balance slider + EQ/PL buttons
        let bal_w = area.width.saturating_sub(14).min(24);
        render_labeled_slider(
            buf,
            area.x,
            area.y + 5,
            bal_w,
            self.balance,
            "BAL",
            Color::Rgb(0, 204, 255),
            C_BODY_DARK,
        );

        let eq = if self.eq_on { "[EQ✓]" } else { "[EQ ]" };
        let pl = if self.pl_on { "[PL✓]" } else { "[PL ]" };
        let btns = format!("{} {}", eq, pl);
        let bx = area.right().saturating_sub(btns.chars().count() as u16 + 1);
        put_str(
            buf,
            bx,
            area.y + 5,
            area.right(),
            &btns,
            C_LCD_ON,
            C_BADGE_BG,
        );
    }
}

fn render_labeled_slider(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    value: f32,
    label: &str,
    color: Color,
    bg: Color,
) {
    let label_w = label.len() as u16 + 1;
    if width <= label_w + 2 {
        return;
    }
    let track_w = width - label_w - 2;
    let lbl = format!("{} ", label);
    put_str(buf, x, y, x + width, &lbl, Color::White, bg);
    buf[(x + label_w, y)]
        .set_char('[')
        .set_fg(C_GRAY)
        .set_bg(bg);
    let thumb = (value * (track_w as f32 - 1.0)).round() as u16;
    for i in 0..track_w {
        let (ch, fg) = if i == thumb {
            ('●', color)
        } else {
            ('─', C_DARK_GRAY)
        };
        buf[(x + label_w + 1 + i, y)]
            .set_char(ch)
            .set_fg(fg)
            .set_bg(bg);
    }
    let rb = x + label_w + 1 + track_w;
    buf[(rb, y)].set_char(']').set_fg(C_GRAY).set_bg(bg);
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: Seek bar
// ─────────────────────────────────────────────────────────────────────────────
struct SeekBar {
    ratio: f32,
    scrubbing: bool,
    current_str: String,
    total_str: String,
}

impl Widget for SeekBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_BODY_DARK);
        if area.width < 6 {
            return;
        }

        let time_label = format!(" {}/{}", self.current_str, self.total_str);
        let tl_w = time_label.len() as u16;
        let track_w = area.width.saturating_sub(tl_w);

        let thumb_x = area.x + (self.ratio * track_w.saturating_sub(2) as f32).round() as u16;
        for x in area.x..(area.x + track_w) {
            let is_thumb = x == thumb_x || x == thumb_x + 1;
            let (ch, fg, bg) = if is_thumb {
                (
                    '█',
                    Color::White,
                    if self.scrubbing { C_SPEC_AMB } else { C_GRAY },
                )
            } else if x < thumb_x {
                ('─', C_LCD_ON, C_BODY_DARK)
            } else {
                ('─', C_DARK_GRAY, C_BODY_DARK)
            };
            buf[(x, area.y)].set_char(ch).set_fg(fg).set_bg(bg);
        }
        put_str(
            buf,
            area.x + track_w,
            area.y,
            area.right(),
            &time_label,
            C_GRAY,
            C_BODY_DARK,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: Controls bar
// ─────────────────────────────────────────────────────────────────────────────
struct ControlsBar<'a> {
    state: &'a tunes4r::models::PlaybackState,
    shuffle: bool,
    repeat: bool,
    volume: f32,
    balance: f32,
}

impl<'a> Widget for ControlsBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_BODY_DARK);
        let shuf = if self.shuffle { "[SHUF✓]" } else { "[SHUF ]" };
        let rep = if self.repeat { "[REP✓]" } else { "[REP ]" };
        let vol_pct = (self.volume * 100.0).round() as u32;
        let bal_pct = ((self.balance - 0.5) * 200.0).round() as i32;
        let bar = format!(
            " [|◄][▶][⏸][■][►|] [⏏] {} {} Vol:{:3}% Bal:{:+3}%",
            shuf, rep, vol_pct, bal_pct
        );
        for (i, ch) in bar.chars().enumerate() {
            let cx = area.x + i as u16;
            if cx >= area.right() {
                break;
            }
            let state = self.state;
            let color = if ch == '[' || ch == ']' {
                C_GRAY
            } else {
                match state {
                    tunes4r::models::PlaybackState::Playing if ch == '▶' => C_LCD_ON,
                    tunes4r::models::PlaybackState::Paused if ch == '⏸' => {
                        Color::Rgb(255, 200, 0)
                    }
                    tunes4r::models::PlaybackState::Stopped if ch == '■' => C_STATE_RED,
                    _ => Color::White,
                }
            };
            buf[(cx, area.y)]
                .set_char(ch)
                .set_fg(color)
                .set_bg(C_BODY_DARK);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: Help bar
// ─────────────────────────────────────────────────────────────────────────────
struct HelpBar {
    show_log: bool,
}

impl Widget for HelpBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_DARK_GRAY);
        let log_key = if self.show_log { "L:hide-log" } else { "L:log" };
        let hint = format!(
            " Spc:▶/⏸  S:■  ←/→:seek  +/-:vol  [/]:bal  \\:ctr  R:rem  O:open  {}  Q:quit",
            log_key
        );
        let s: String = hint.chars().take(area.width as usize).collect();
        put_str(buf, area.x, area.y, area.right(), &s, C_GRAY, C_DARK_GRAY);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: Transport buttons (below LCD panel)
// ─────────────────────────────────────────────────────────────────────────────
struct TransportButtons<'a> {
    state: &'a tunes4r::models::PlaybackState,
}

impl<'a> Widget for TransportButtons<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_BODY_DARK);
        if area.height < 3 || area.width < 10 {
            return;
        }

        let icons: &[(&str, u8)] = &[
            ("|◀", 0),
            ("▶", 1),
            ("❚❚", 2),
            ("■", 3),
            ("▶|", 4),
            ("⏏", 5),
        ];

        let btn_w: u16 = 5;
        let gap: u16 = 1;
        let n = icons.len() as u16;
        let total_w = n * btn_w + (n - 1) * gap;
        let sx = area.x + area.width.saturating_sub(total_w) / 2;
        let by = area.y + (area.height.saturating_sub(3)) / 2;

        for (i, (label, _idx)) in icons.iter().enumerate() {
            let bx = sx + i as u16 * (btn_w + gap);
            if bx + btn_w > area.right() {
                break;
            }

            let active = match i {
                1 => matches!(self.state, tunes4r::models::PlaybackState::Playing),
                2 => matches!(self.state, tunes4r::models::PlaybackState::Paused),
                3 => matches!(self.state, tunes4r::models::PlaybackState::Stopped),
                _ => false,
            };

            let (hi, lo, bg, icon_fg) = if active {
                (
                    C_BTN_BEVEL_LO,
                    C_BTN_BEVEL_HI,
                    C_BTN_PRESSED,
                    match i {
                        1 => C_LCD_ON,
                        2 => C_LOG_WARN,
                        3 => C_STATE_RED,
                        _ => Color::White,
                    },
                )
            } else {
                (C_BTN_BEVEL_HI, C_BTN_BEVEL_LO, C_BTN_BG, Color::White)
            };

            // Top row: raised top-left / sunken bottom-right
            buf[(bx, by)].set_char('┌').set_fg(hi).set_bg(C_BODY_DARK);
            for x in (bx + 1)..(bx + btn_w - 1) {
                buf[(x, by)].set_char('─').set_fg(hi).set_bg(C_BODY_DARK);
            }
            buf[(bx + btn_w - 1, by)]
                .set_char('┐')
                .set_fg(lo)
                .set_bg(C_BODY_DARK);

            // Middle row: sides + icon
            buf[(bx, by + 1)].set_char('│').set_fg(hi).set_bg(bg);
            let inner_w = (btn_w - 2) as usize;
            let padded = format!("{:^width$}", label, width = inner_w);
            for (ci, ch) in padded.chars().enumerate() {
                let cx = bx + 1 + ci as u16;
                buf[(cx, by + 1)].set_char(ch).set_fg(icon_fg).set_bg(bg);
            }
            buf[(bx + btn_w - 1, by + 1)]
                .set_char('│')
                .set_fg(lo)
                .set_bg(bg);

            // Bottom row
            buf[(bx, by + 2)]
                .set_char('└')
                .set_fg(lo)
                .set_bg(C_BODY_DARK);
            for x in (bx + 1)..(bx + btn_w - 1) {
                buf[(x, by + 2)]
                    .set_char('─')
                    .set_fg(lo)
                    .set_bg(C_BODY_DARK);
            }
            buf[(bx + btn_w - 1, by + 2)]
                .set_char('┘')
                .set_fg(lo)
                .set_bg(C_BODY_DARK);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: Log pane
// ─────────────────────────────────────────────────────────────────────────────
struct LogPane<'a> {
    log: &'a LogBuffer,
}

impl<'a> Widget for LogPane<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_LOG_BG);
        let block = Block::default()
            .title(" LOG ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(C_GRAY).bg(C_LOG_BG))
            .title_style(Style::default().fg(C_LCD_ON).bg(C_LOG_BG));
        block.render(area, buf);

        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let h = inner.height as usize;
        let entries: Vec<_> = self.log.entries.iter().rev().take(h).collect();
        for (i, entry) in entries.iter().rev().enumerate() {
            let ry = inner.y + (h.saturating_sub(entries.len()) + i) as u16;
            if ry >= inner.bottom() {
                break;
            }
            let line = format!("{}{}", entry.prefix(), entry.msg);
            let s: String = line.chars().take(inner.width as usize).collect();
            put_str(buf, inner.x, ry, inner.right(), &s, entry.color(), C_LOG_BG);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Widget: File browser overlay
// ─────────────────────────────────────────────────────────────────────────────
struct FileBrowserWidget<'a> {
    browser: &'a FileBrowser,
}

impl<'a> Widget for FileBrowserWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill_bg(buf, area, C_LOG_BG);
        let title = format!(" Open File — {} ", self.browser.cwd.display());
        let block = Block::default()
            .title(title.as_str())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(C_LCD_ON).bg(C_LOG_BG))
            .title_style(Style::default().fg(C_LCD_ON).add_modifier(Modifier::BOLD));
        block.render(area, buf);

        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let selected = self.browser.selected;
        let h = inner.height as usize;
        let scroll_top = if selected >= h { selected - h + 1 } else { 0 };

        let hint = "↑/↓ navigate  Enter:open/cd  Esc:cancel";
        let hint_y = inner.bottom().saturating_sub(1);
        put_str(buf, inner.x, hint_y, inner.right(), hint, C_GRAY, C_LOG_BG);

        let list_h = (inner.height as usize).saturating_sub(1);
        for (vi, entry_i) in (scroll_top..).take(list_h).enumerate() {
            let ry = inner.y + vi as u16;
            if ry >= hint_y {
                break;
            }
            if let Some((name, is_dir)) = self.browser.entries.get(entry_i) {
                let is_sel = entry_i == selected;
                let display = if *is_dir {
                    format!("{}/", name)
                } else {
                    name.clone()
                };
                let (fg, bg) = if is_sel {
                    (C_BODY_DARK, C_FILE_SEL)
                } else if *is_dir {
                    (C_FILE_DIR, C_LOG_BG)
                } else {
                    (Color::White, C_LOG_BG)
                };
                let padded = format!("{:<width$}", display, width = inner.width as usize);
                put_str(buf, inner.x, ry, inner.right(), &padded, fg, bg);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Main draw
// ─────────────────────────────────────────────────────────────────────────────
fn draw(app: &WinampApp, frame: &mut ratatui::Frame) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    fill_bg(buf, area, C_BODY_DARK);

    // Player / log split
    let (player_area, log_area) = if app.show_log {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(50), Constraint::Length(42)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    const BODY_H: u16 = 22;

    let player_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(BODY_H),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(player_area);

    // Title bar
    TitleBar.render(player_rows[0], buf);

    // Body: LCD (left 36 cols) + metadata (rest)
    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(36), Constraint::Min(20)])
        .split(player_rows[1]);

    let state = app.playback_state();

    // Split LCD column: display (top) + transport buttons (bottom 3 rows)
    let lcd_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(3)])
        .split(body_cols[0]);

    LcdPanel {
        state: &state,
        current_ms: app.current_ms(),
        total_ms: app.total_ms(),
        show_remaining: app.show_remaining,
        spectrum: &app.spectrum,
    }
    .render(lcd_split[0], buf);

    TransportButtons { state: &state }.render(lcd_split[1], buf);

    let box_inner_w = (body_cols[1].width as usize).saturating_sub(2).min(48);
    let title_visible = app.scrolling.visible(box_inner_w);
    MetadataPanel {
        title_visible,
        volume: app.volume,
        balance: app.balance,
        eq_on: app.eq_on,
        pl_on: app.pl_on,
        is_playing: app.is_playing(),
        error: app.error.as_deref(),
    }
    .render(body_cols[1], buf);

    // Seek bar
    let total = app.total_ms();
    let disp_ms = if app.scrub.active {
        app.scrub.position_ms
    } else {
        app.current_ms()
    };
    let ratio = if total > 0 {
        (disp_ms as f32 / total as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    SeekBar {
        ratio,
        scrubbing: app.scrub.active,
        current_str: fmt_ms(disp_ms),
        total_str: fmt_ms(total),
    }
    .render(player_rows[2], buf);

    // Controls
    ControlsBar {
        state: &state,
        shuffle: app.shuffle,
        repeat: app.repeat,
        volume: app.volume,
        balance: app.balance,
    }
    .render(player_rows[3], buf);

    // Help
    HelpBar {
        show_log: app.show_log,
    }
    .render(player_rows[4], buf);

    // Log pane
    if let Some(la) = log_area {
        LogPane { log: &app.log }.render(la, buf);
    }

    // File browser overlay (modal)
    if matches!(app.mode, UiMode::FileBrowser) {
        let w = area.width.min(70);
        let h = area.height.min(22);
        let modal = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };
        FileBrowserWidget {
            browser: &app.browser,
        }
        .render(modal, buf);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Input handling
// ─────────────────────────────────────────────────────────────────────────────
fn handle_key(app: &mut WinampApp, code: KeyCode) -> bool {
    match &app.mode {
        UiMode::FileBrowser => match code {
            KeyCode::Esc => {
                app.log.log("File browser cancelled");
                app.mode = UiMode::Player;
            }
            KeyCode::Up => app.browser.up(),
            KeyCode::Down => app.browser.down(),
            KeyCode::Enter => {
                if let Some(path) = app.browser.enter() {
                    app.load_file(path);
                    app.mode = UiMode::Player;
                }
            }
            _ => {}
        },

        UiMode::Player => match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return true,

            KeyCode::Char('o') | KeyCode::Char('O') => {
                app.browser.refresh();
                app.log
                    .log(format!("File browser: {}", app.browser.cwd.display()));
                app.mode = UiMode::FileBrowser;
            }

            KeyCode::Char(' ') => {
                let mut e = app.engine.lock().unwrap();
                match e.get_state() {
                    tunes4r::models::PlaybackState::Playing => {
                        e.pause();
                        app.log.log("Paused");
                    }
                    tunes4r::models::PlaybackState::Paused => {
                        e.resume();
                        app.log.log("Resumed");
                    }
                    _ => {
                        if !app.url.is_empty() {
                            let url = app.url.clone();
                            let _ = e.play(&url, None);
                            app.log.log("Playing");
                        } else {
                            app.log.warn("No file loaded — press O to open");
                        }
                    }
                }
            }

            KeyCode::Char('s') | KeyCode::Char('S') => {
                app.engine.lock().unwrap().stop();
                app.log.log("Stopped");
            }

            KeyCode::Left => {
                let cur = app.current_ms();
                let total = app.total_ms();
                if total > 0 {
                    if !app.scrub.active {
                        app.scrub.enter(cur);
                    }
                    app.scrub.position_ms = app.scrub.position_ms.saturating_sub(2000);
                    app.log
                        .log(format!("Scrub → {}", fmt_ms(app.scrub.position_ms)));
                }
            }
            KeyCode::Right => {
                let cur = app.current_ms();
                let total = app.total_ms();
                if total > 0 {
                    if !app.scrub.active {
                        app.scrub.enter(cur);
                    }
                    app.scrub.position_ms = (app.scrub.position_ms + 2000).min(total);
                    app.log
                        .log(format!("Scrub → {}", fmt_ms(app.scrub.position_ms)));
                }
            }
            KeyCode::Enter => {
                if let Some(ms) = app.scrub.commit() {
                    let _ = app.engine.lock().unwrap().seek(ms);
                    app.log.log(format!("Seeked to {}", fmt_ms(ms)));
                }
            }
            KeyCode::Esc => {
                app.scrub.cancel();
                app.log.log("Scrub cancelled");
            }

            KeyCode::Char('r') | KeyCode::Char('R') => {
                app.show_remaining = !app.show_remaining;
                app.log.log(if app.show_remaining {
                    "Timer: remaining"
                } else {
                    "Timer: elapsed"
                });
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                app.eq_on = !app.eq_on;
                app.log.log(if app.eq_on { "EQ on" } else { "EQ off" });
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                app.pl_on = !app.pl_on;
                app.log.log(if app.pl_on { "PL on" } else { "PL off" });
            }

            // Volume
            KeyCode::Char('+') | KeyCode::Char('=') => {
                app.volume = (app.volume + 0.05).min(1.0);
                app.engine.lock().unwrap().set_volume(app.volume);
                app.log.log(format!("Volume: {:.0}%", app.volume * 100.0));
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                app.volume = (app.volume - 0.05).max(0.0);
                app.engine.lock().unwrap().set_volume(app.volume);
                app.log.log(format!("Volume: {:.0}%", app.volume * 100.0));
            }

            // Balance
            KeyCode::Char('[') => {
                app.balance = (app.balance - 0.05).max(0.0);
                app.engine.lock().unwrap().set_balance(app.balance);
                app.log
                    .log(format!("Balance: {:+.0}%", (app.balance - 0.5) * 200.0));
            }
            KeyCode::Char(']') => {
                app.balance = (app.balance + 0.05).min(1.0);
                app.engine.lock().unwrap().set_balance(app.balance);
                app.log
                    .log(format!("Balance: {:+.0}%", (app.balance - 0.5) * 200.0));
            }
            KeyCode::Char('\\') => {
                app.balance = 0.5;
                app.engine.lock().unwrap().set_balance(0.5);
                app.log.log("Balance: centred");
            }

            // Log toggle
            KeyCode::Char('l') | KeyCode::Char('L') => {
                app.show_log = !app.show_log;
            }

            _ => {}
        },
    }
    false
}

fn handle_mouse(app: &mut WinampApp, ev: MouseEvent, term_area: Rect) {
    if matches!(app.mode, UiMode::FileBrowser) {
        return;
    }

    if ev.kind != MouseEventKind::Down(MouseButton::Left) {
        return;
    }

    let col = ev.column;
    let row = ev.row;

    // Transport buttons area: bottom 4 rows of the LCD column (36 cols)
    let transport_top = term_area.y + 1 + 12;
    let transport_bot = transport_top + 3;
    let lcd_right = term_area.x + 36;

    if row >= transport_top && row <= transport_bot && col < lcd_right {
        let rel_x = col.saturating_sub(term_area.x);
        // 6 buttons × 5 wide + 5 gaps = 35; each group at stride 6, button width 5
        let stride: u16 = 6;
        let btn_idx = (rel_x / stride) as usize;
        let in_btn = rel_x % stride < 5;
        if !in_btn || btn_idx >= 6 {
            return;
        }
        match btn_idx {
            0 => {
                let cur = app.current_ms();
                let ms = cur.saturating_sub(5000);
                let _ = app.engine.lock().unwrap().seek(ms);
                app.log.log("⏪ Prev");
            }
            1 => {
                if app.url.is_empty() {
                    app.log.warn("No file loaded — press O to open");
                } else {
                    let url = app.url.clone();
                    let _ = app.engine.lock().unwrap().play(&url, None);
                    app.log.log("▶ Play");
                }
            }
            2 => {
                let mut e = app.engine.lock().unwrap();
                match e.get_state() {
                    tunes4r::models::PlaybackState::Playing => {
                        e.pause();
                        app.log.log("⏸ Paused");
                    }
                    tunes4r::models::PlaybackState::Paused => {
                        e.resume();
                        app.log.log("▶ Resumed");
                    }
                    _ => {}
                }
            }
            3 => {
                app.engine.lock().unwrap().stop();
                app.log.log("■ Stopped");
            }
            4 => {
                let cur = app.current_ms();
                let total = app.total_ms();
                if total > 0 {
                    let ms = (cur + 5000).min(total);
                    let _ = app.engine.lock().unwrap().seek(ms);
                    app.log.log("⏩ Next");
                }
            }
            5 => {
                app.browser.refresh();
                app.log
                    .log(format!("File browser: {}", app.browser.cwd.display()));
                app.mode = UiMode::FileBrowser;
            }
            _ => {}
        }
        return;
    }

    // Seek bar row
    let seek_y = term_area.y + 17;
    if row == seek_y {
        let total = app.total_ms();
        if total > 0 {
            let track_w = term_area.width.saturating_sub(14) as f32;
            let t = ((col.saturating_sub(term_area.x)) as f32 / track_w).clamp(0.0, 1.0);
            app.scrub.enter((t * total as f32) as u64);
            let ms = app.scrub.position_ms;
            let _ = app.engine.lock().unwrap().seek(ms);
            app.log.log(format!("Seeked to {}", fmt_ms(ms)));
            app.scrub.cancel();
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────
fn main() -> io::Result<()> {
    let url = std::env::args().nth(1).or_else(|| {
        let p = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sample.mp3"));
        if p.exists() {
            Some(p.to_string_lossy().to_string())
        } else {
            None
        }
    });

    let engine = Arc::new(Mutex::new(tunes4r::create_playback_engine()));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = WinampApp::new(engine, url);

    let tick_rate = Duration::from_millis(33);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| draw(&app, f))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(k) => {
                    if handle_key(&mut app, k.code) {
                        break;
                    }
                }
                Event::Mouse(ev) => {
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    handle_mouse(&mut app, ev, area);
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick();
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_ms_basic() {
        assert_eq!(fmt_ms(0), "00:00");
        assert_eq!(fmt_ms(59_999), "00:59");
        assert_eq!(fmt_ms(60_000), "01:00");
        assert_eq!(fmt_ms(3_661_000), "61:01");
    }

    #[test]
    fn scrub_lifecycle() {
        let mut s = ScrubState::default();
        assert_eq!(s.commit(), None);
        s.enter(5_000);
        assert!(s.active);
        assert_eq!(s.commit(), Some(5_000));
        assert!(!s.active);
        assert_eq!(s.commit(), None);
    }

    #[test]
    fn scrub_cancel() {
        let mut s = ScrubState::default();
        s.enter(5_000);
        s.cancel();
        assert!(!s.active);
        assert_eq!(s.position_ms, 0);
    }

    #[test]
    fn spectrum_decays_when_stopped() {
        let mut s = SpectrumState::new();
        s.smoothed[0] = 1.0;
        s.peaks[0] = 1.0;
        s.update(false, 0.033);
        assert!(s.smoothed[0] < 1.0);
        assert!(s.smoothed[0] > 0.0);
        assert_eq!(s.peaks[0], 0.0);
    }

    #[test]
    fn scrolling_title_advances() {
        let mut st = ScrollingTitle::new();
        st.set_text("Hello World");
        let before = st.offset;
        st.tick(1.0);
        assert_eq!(st.offset, (before + 8) % st.padded.chars().count());
    }

    #[test]
    fn scrolling_title_visible_width() {
        let mut st = ScrollingTitle::new();
        st.set_text("Test");
        assert_eq!(st.visible(10).chars().count(), 10);
        assert_eq!(st.visible(1).chars().count(), 1);
    }

    #[test]
    fn log_buffer_caps() {
        let mut lb = LogBuffer::new(3);
        lb.log("a");
        lb.log("b");
        lb.log("c");
        lb.log("d");
        assert_eq!(lb.entries.len(), 3);
        assert_eq!(lb.entries[2].msg, "d");
    }
}
