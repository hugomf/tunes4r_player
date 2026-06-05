//! TUI player — ratatui-based terminal UI for tunes4r.
//!
//! Controls:
//!   1/2/3       Select section
//!   p           Play selected section (prompts for URL)
//!   Space       Pause / Resume
//!   s           Stop
//!   k           Enter scrub mode
//!   ←/→ or A/D  Scrub ±1 s
//!   ↑/↓ or W/X  Scrub ±10 s
//!   Enter       Commit seek
//!   r           Edit URL for selected section
//!   Esc         Cancel scrub / Quit
//!   q           Quit

use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::{Frame, Terminal};

use tunes4r::audio::stream::source::Capability;
use tunes4r::models::{DownloadBuffer, PlaybackPosition, PlaybackState};
use tunes4r::PlaybackEngine;

// ── Constants ─────────────────────────────────────────────────────────

const POLL_INTERVAL_MS: u64 = 50;
const TICK_RATE_MS: u64 = 50;
const SEEK_SMALL_MS: u64 = 1_000;
const SEEK_LARGE_MS: u64 = 10_000;
const LIVE_MAX_DURATION_MS: u64 = 30 * 60 * 1_000;

// ── Section ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    File,
    YouTube,
    Live,
}

impl Section {
    #[allow(dead_code)]
    fn label(self) -> &'static str {
        match self {
            Section::File => "File",
            Section::YouTube => "YouTube",
            Section::Live => "Live",
        }
    }

    fn prompt_label(self) -> &'static str {
        match self {
            Section::File => "File path",
            Section::YouTube => "YouTube URL or ID",
            Section::Live => "Live stream URL",
        }
    }

    #[allow(dead_code)]
    fn title(self) -> &'static str {
        match self {
            Section::File => "🎵 Audio File  [1]",
            Section::YouTube => "📺 YouTube Stream  [2]",
            Section::Live => "🔴 Live Stream  [3]",
        }
    }
}

// ── SectionInfo ───────────────────────────────────────────────────────

/// Persistent state for one player section (file / YouTube / live).
#[derive(Clone, Default)]
struct SectionInfo {
    url: String,
    /// Whether this section is the currently active audio source.
    is_active: bool,
    state: PlaybackState,
    position: PlaybackPosition,
    buffer: DownloadBuffer,
    can_seek: bool,
}

impl SectionInfo {
    fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), ..Default::default() }
    }

    /// Apply a fresh engine snapshot to this section (only if it is active).
    fn apply_snapshot(&mut self, snap: &EngineSnapshot) {
        if !self.is_active {
            return;
        }
        self.state = snap.state.clone();
        self.position = snap.position;
        self.buffer = snap.buffer.clone();
        self.can_seek = snap.can_seek;

        // Auto-deactivate when the engine stops.
        if matches!(self.state, PlaybackState::Stopped) {
            self.is_active = false;
        }
    }
}

// ── EngineSnapshot ────────────────────────────────────────────────────

/// A point-in-time copy of engine state, polled off the hot path.
#[derive(Clone)]
struct EngineSnapshot {
    state: PlaybackState,
    position: PlaybackPosition,
    buffer: DownloadBuffer,
    can_seek: bool,
    load_error: String,
}

impl EngineSnapshot {
    fn capture(engine: &PlaybackEngine) -> Self {
        Self {
            state: engine.get_state(),
            position: engine.get_position(),
            buffer: engine.get_download_buffer(),
            can_seek: engine.source_supports(Capability::Seek),
            load_error: engine.load_error(),
        }
    }

    fn status_text(&self) -> String {
        match &self.state {
            PlaybackState::Playing => "▶  Playing".into(),
            PlaybackState::Paused => "⏸  Paused".into(),
            PlaybackState::Stopped => "⏹  Stopped".into(),
            PlaybackState::Connecting => "⏳  Connecting…".into(),
            PlaybackState::Buffering { .. } => "⬇  Buffering…".into(),
            PlaybackState::Decoding => "🔧  Decoding…".into(),
            PlaybackState::Error(e) => format!("❌  {}", e),
        }
    }
}

// ── ScrubState ────────────────────────────────────────────────────────

/// Encapsulates scrub-mode cursor logic so the rest of UiState stays lean.
#[derive(Default)]
struct ScrubState {
    position_ms: u64,
    active: bool,
}

impl ScrubState {
    fn enter(&mut self, current_ms: u64) {
        self.position_ms = current_ms;
        self.active = true;
    }

    fn cancel(&mut self) {
        self.active = false;
        self.position_ms = 0;
    }

    fn nudge(&mut self, delta_ms: i64, total_ms: u64) {
        if !self.active {
            return;
        }
        let new_pos = (self.position_ms as i64 + delta_ms).max(0) as u64;
        self.position_ms = new_pos.min(total_ms);
    }

    /// Consume the scrub position, returning it and resetting state.
    fn commit(&mut self) -> Option<u64> {
        if self.active {
            self.active = false;
            Some(std::mem::take(&mut self.position_ms))
        } else {
            None
        }
    }
}

// ── UiState ───────────────────────────────────────────────────────────

struct UiState {
    selected_section: Section,
    scrub: ScrubState,
    status_line: String,
    error_line: String,
    file_info: SectionInfo,
    yt_info: SectionInfo,
    live_info: SectionInfo,
}

impl UiState {
    fn new() -> Self {
        Self {
            selected_section: Section::File,
            scrub: ScrubState::default(),
            status_line: "Ready".into(),
            error_line: String::new(),
            file_info: SectionInfo::new("../example/assets/music.mp3"),
            yt_info: SectionInfo::new("dQw4w9WgXcQ"),
            live_info: SectionInfo::new(
                "https://wdr-1live-live.icecastssl.wdr.de/wdr/1live/live/mp3/128/stream.mp3",
            ),
        }
    }

    fn section_info(&self, section: Section) -> &SectionInfo {
        match section {
            Section::File => &self.file_info,
            Section::YouTube => &self.yt_info,
            Section::Live => &self.live_info,
        }
    }

    fn section_info_mut(&mut self, section: Section) -> &mut SectionInfo {
        match section {
            Section::File => &mut self.file_info,
            Section::YouTube => &mut self.yt_info,
            Section::Live => &mut self.live_info,
        }
    }

    fn active_section_info(&self) -> &SectionInfo {
        self.section_info(self.selected_section)
    }

    fn active_total_ms(&self) -> u64 {
        self.active_section_info().position.total_ms
    }

    fn apply_snapshot(&mut self, snap: &EngineSnapshot) {
        self.file_info.apply_snapshot(snap);
        self.yt_info.apply_snapshot(snap);
        self.live_info.apply_snapshot(snap);

        if !snap.load_error.is_empty() {
            self.error_line = snap.load_error.clone();
        }
        self.status_line = snap.status_text();
    }

    fn deactivate_all(&mut self) {
        self.file_info.is_active = false;
        self.yt_info.is_active = false;
        self.live_info.is_active = false;
    }

    fn mark_active(&mut self, section: Section) {
        self.deactivate_all();
        self.section_info_mut(section).is_active = true;
    }
}

// ── Progress slider widget ────────────────────────────────────────────

/// Self-contained reusable progress-bar / seek-slider widget.
///
/// Renders three visual segments across `area`:
///   - **played** (green)  — from 0 to the playhead
///   - **buffered** (cyan) — from the playhead to the download cursor
///   - **empty** (dark)    — remainder
///
/// In scrub mode the playhead turns yellow and a `◆` cursor is drawn at
/// the scrub position rather than the real position.
#[allow(dead_code)]
struct ProgressSlider<'a> {
    position_ms: u64,
    total_ms: u64,
    buffer_ms: u64,
    scrub_ms: Option<u64>,
    /// Extra annotation spans appended after the time readout.
    annotation: Option<Vec<Span<'a>>>,
}

impl<'a> ProgressSlider<'a> {
    #[allow(dead_code)]
    fn new(position_ms: u64, total_ms: u64, buffer_ms: u64) -> Self {
        Self {
            position_ms,
            total_ms,
            buffer_ms,
            scrub_ms: None,
            annotation: None,
        }
    }

    #[allow(dead_code)]
    fn with_scrub(mut self, scrub_ms: u64) -> Self {
        self.scrub_ms = Some(scrub_ms);
        self
    }

    #[allow(dead_code)]
    fn with_annotation(mut self, annotation: Vec<Span<'a>>) -> Self {
        self.annotation = Some(annotation);
        self
    }

    #[allow(dead_code)]
    fn render(self, f: &mut Frame, area: Rect) {
        let is_scrubbing = self.scrub_ms.is_some();
        let display_pos = self.scrub_ms.unwrap_or(self.position_ms);
        let total = self.total_ms.max(1); // avoid division by zero

        // ── Track ──────────────────────────────────────────────────────
        let track_area = Rect { height: 1, y: area.y, ..area };
        let w = track_area.width as usize;

        if w > 0 {
            let pos_ratio = (display_pos as f64 / total as f64).min(1.0);
            let buf_ratio = (self.buffer_ms as f64 / total as f64).min(1.0);

            let played_cols = (pos_ratio * w as f64) as usize;
            let buf_cols = ((buf_ratio * w as f64) as usize).max(played_cols).min(w);
            let cursor_col = played_cols.min(w.saturating_sub(1));

            let track_color = if is_scrubbing { Color::Yellow } else { Color::Green };

            let spans: Vec<Span> = (0..w)
                .map(|col| {
                    let bg = if col < played_cols {
                        track_color
                    } else if col < buf_cols {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    };
                    if col == cursor_col {
                        Span::styled("◆", Style::default().fg(Color::White).bg(bg))
                    } else {
                        Span::styled(" ", Style::default().bg(bg))
                    }
                })
                .collect();

            f.render_widget(Paragraph::new(Line::from(spans)), track_area);
        }

        // ── Time label ─────────────────────────────────────────────────
        if area.height >= 3 {
            let label_area = Rect {
                y: area.y + 2,
                height: 1,
                ..area
            };

            let time_style = if is_scrubbing {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };

            let mut spans = vec![Span::styled(
                format!(
                    " {} / {}",
                    format_duration(display_pos),
                    format_duration(self.total_ms)
                ),
                time_style,
            )];

            if let Some(mut ann) = self.annotation {
                spans.push(Span::raw("  "));
                spans.append(&mut ann);
            }

            f.render_widget(Paragraph::new(Line::from(spans)), label_area);
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let engine = Arc::new(Mutex::new(PlaybackEngine::new_without_device()?));
    let ui = Arc::new(Mutex::new(UiState::new()));

    start_poll_thread(Arc::clone(&ui), Arc::clone(&engine));

    let terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    let result = run_event_loop(terminal, &ui, &engine);

    // Always restore terminal, even on error.
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    if let Err(ref e) = result {
        eprintln!("Fatal error: {}", e);
    }
    Ok(())
}

// ── Poll thread ───────────────────────────────────────────────────────

/// Spawns a background thread that captures engine state at ~20 Hz and
/// writes it into `UiState`.  Kept separate from the render loop so
/// rendering is never blocked by engine locking.
fn start_poll_thread(ui: Arc<Mutex<UiState>>, engine: Arc<Mutex<PlaybackEngine>>) {
    std::thread::spawn(move || loop {
        let snap = {
            let e = engine.lock().unwrap();
            EngineSnapshot::capture(&e)
        };
        ui.lock().unwrap().apply_snapshot(&snap);
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    });
}

// ── Event loop ────────────────────────────────────────────────────────

fn run_event_loop<B: ratatui::backend::Backend>(
    mut terminal: Terminal<B>,
    ui: &Arc<Mutex<UiState>>,
    engine: &Arc<Mutex<PlaybackEngine>>,
) -> io::Result<()> {
    let tick = Duration::from_millis(TICK_RATE_MS);
    let mut last_tick = Instant::now();

    loop {
        let timeout = tick.saturating_sub(last_tick.elapsed());

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match handle_key(key.code, ui, engine)? {
                    KeyOutcome::Quit => break,
                    KeyOutcome::Handled | KeyOutcome::Ignored => {}
                }
            }
        }

        if last_tick.elapsed() >= tick {
            last_tick = Instant::now();
        }

        terminal.draw(|f| render_frame(f, ui))?;
    }

    engine.lock().unwrap().stop();
    Ok(())
}

#[must_use]
enum KeyOutcome {
    Quit,
    Handled,
    Ignored,
}

fn handle_key(
    code: KeyCode,
    ui: &Arc<Mutex<UiState>>,
    engine: &Arc<Mutex<PlaybackEngine>>,
) -> io::Result<KeyOutcome> {
    match code {
        // ── Section selection ──────────────────────────────────────────
        KeyCode::Char('1') => {
            ui.lock().unwrap().selected_section = Section::File;
        }
        KeyCode::Char('2') => {
            ui.lock().unwrap().selected_section = Section::YouTube;
        }
        KeyCode::Char('3') => {
            ui.lock().unwrap().selected_section = Section::Live;
        }

        // ── Play ───────────────────────────────────────────────────────
        KeyCode::Char('p') => {
            handle_play(ui, engine)?;
        }

        // ── Pause / Resume ─────────────────────────────────────────────
        KeyCode::Char(' ') => {
            let mut e = engine.lock().unwrap();
            match e.get_state() {
                PlaybackState::Playing => e.pause(),
                PlaybackState::Paused => e.resume(),
                _ => {}
            }
        }

        // ── Stop ───────────────────────────────────────────────────────
        KeyCode::Char('s') => {
            engine.lock().unwrap().stop();
            ui.lock().unwrap().deactivate_all();
        }

        // ── Enter scrub mode ───────────────────────────────────────────
        KeyCode::Char('k') => {
            let mut u = ui.lock().unwrap();
            let current = u.active_section_info().position.current_ms;
            let total = u.active_total_ms();
            if total > 0 {
                u.scrub.enter(current);
            }
        }

        // ── Scrub navigation ───────────────────────────────────────────
        // Arrow keys auto-enter scrub mode on the first press so the user
        // doesn't have to hit [k] first.  They are a no-op when no seekable
        // track is loaded (total_ms == 0).
        KeyCode::Left | KeyCode::Char('a') => {
            let mut u = ui.lock().unwrap();
            let total = u.active_total_ms();
            if total > 0 {
                ensure_scrub_active(&mut u);
                u.scrub.nudge(-(SEEK_SMALL_MS as i64), total);
            }
        }
        KeyCode::Right | KeyCode::Char('d') => {
            let mut u = ui.lock().unwrap();
            let total = u.active_total_ms();
            if total > 0 {
                ensure_scrub_active(&mut u);
                u.scrub.nudge(SEEK_SMALL_MS as i64, total);
            }
        }
        KeyCode::Up | KeyCode::Char('w') => {
            let mut u = ui.lock().unwrap();
            let total = u.active_total_ms();
            if total > 0 {
                ensure_scrub_active(&mut u);
                u.scrub.nudge(-(SEEK_LARGE_MS as i64), total);
            }
        }
        KeyCode::Down | KeyCode::Char('x') => {
            let mut u = ui.lock().unwrap();
            let total = u.active_total_ms();
            if total > 0 {
                ensure_scrub_active(&mut u);
                u.scrub.nudge(SEEK_LARGE_MS as i64, total);
            }
        }

        // ── Commit seek ────────────────────────────────────────────────
        KeyCode::Enter => {
            let pos = ui.lock().unwrap().scrub.commit();
            if let Some(ms) = pos {
                let _ = engine.lock().unwrap().seek(ms);
            }
        }

        // ── Cancel scrub or Quit ───────────────────────────────────────
        KeyCode::Esc => {
            let mut u = ui.lock().unwrap();
            if u.scrub.active {
                u.scrub.cancel();
            } else {
                return Ok(KeyOutcome::Quit);
            }
        }

        KeyCode::Char('q') => return Ok(KeyOutcome::Quit),

        // ── Edit URL without playing ────────────────────────────────────
        KeyCode::Char('r') => {
            handle_edit_url(ui)?;
        }

        _ => return Ok(KeyOutcome::Ignored),
    }
    Ok(KeyOutcome::Handled)
}

/// Prompt for a URL and start playback.  Temporarily exits raw mode so
/// the user can type in the normal terminal.
fn handle_play(
    ui: &Arc<Mutex<UiState>>,
    engine: &Arc<Mutex<PlaybackEngine>>,
) -> io::Result<()> {
    disable_raw_mode()?;
    let url_opt = prompt_url(ui);
    enable_raw_mode()?;

    let url = match url_opt {
        Some(u) if !u.is_empty() => u,
        _ => return Ok(()),
    };

    let section = ui.lock().unwrap().selected_section;
    let is_live = matches!(section, Section::Live);

    // Persist the URL.
    ui.lock().unwrap().section_info_mut(section).url = url.clone();

    let result = {
        let mut e = engine.lock().unwrap();
        if is_live {
            e.play_live(&url, LIVE_MAX_DURATION_MS)
        } else {
            e.play(&url, None)
        }
    };

    let mut u = ui.lock().unwrap();
    match result {
        Ok(_) => {
            u.error_line.clear();
            u.status_line = "Playing".into();
            u.mark_active(section);
        }
        Err(e) => {
            u.error_line = format!("Play failed: {}", e);
        }
    }
    Ok(())
}

/// Prompt for a new URL and store it without starting playback.
fn handle_edit_url(ui: &Arc<Mutex<UiState>>) -> io::Result<()> {
    disable_raw_mode()?;
    let url_opt = prompt_url(ui);
    enable_raw_mode()?;

    if let Some(url) = url_opt {
        let section = ui.lock().unwrap().selected_section;
        ui.lock().unwrap().section_info_mut(section).url = url;
    }
    Ok(())
}

/// Prompt the user to enter a URL, pre-filling with the current value.
/// Returns `None` only on I/O failure; returns `Some(current)` on empty input.
fn prompt_url(ui: &Arc<Mutex<UiState>>) -> Option<String> {
    let (current, label) = {
        let u = ui.lock().unwrap();
        let section = u.selected_section;
        let current = u.section_info(section).url.clone();
        (current, section.prompt_label())
    };
    print!("{} [{}]: ", label, current);
    io::stdout().flush().ok()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input).ok()?;
    let trimmed = input.trim().to_string();
    Some(if trimmed.is_empty() { current } else { trimmed })
}

/// Enter scrub mode at the current playback position if not already scrubbing.
/// Call this before any `nudge` so the first arrow press both activates scrub
/// and moves the cursor in a single keypress.
fn ensure_scrub_active(u: &mut UiState) {
    if !u.scrub.active {
        let current = u.active_section_info().position.current_ms;
        u.scrub.enter(current);
    }
}

// ── Theme ─────────────────────────────────────────────────────────────
//
// Palette used throughout the render layer:
//
//   accent_hi   = Cyan         — selected border, playhead cursor, key labels
//   accent_lo   = DarkGray     — inactive borders, muted text, empty track
//   state_play  = Green        — playing LED / played portion of track
//   state_pause = Yellow       — paused LED / scrub cursor
//   state_conn  = Blue         — connecting / buffering LED
//   state_err   = Red          — error LED
//   buf_fill    = Rgb(0,95,95) — buffered-ahead portion of track (dark teal)
//   header_bg   = section-specific accent (see `section_accent`)
//   text_dim    = DarkGray
//   text_bright = White

const BLOCK_CHARS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
const BUF_COLOR: Color = Color::Rgb(0, 95, 95);

// ── Rendering ─────────────────────────────────────────────────────────

fn render_frame(f: &mut Frame, ui: &Arc<Mutex<UiState>>) {
    let u = ui.lock().unwrap();
    let area = f.area();

    // Fill the entire background with a very dark base so the whole
    // terminal feels "owned" by the player rather than transparent.
    let bg = Block::default().style(Style::default().bg(Color::Rgb(10, 10, 14)));
    f.render_widget(bg, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // masthead / transport bar
            Constraint::Min(8),    // ch-1  file
            Constraint::Min(8),    // ch-2  youtube
            Constraint::Min(8),    // ch-3  live
            Constraint::Length(1), // keybind strip
        ])
        .split(area);

    render_masthead(f, chunks[0], &u);
    render_channel(f, chunks[1], Section::File,    &u.file_info, &u);
    render_channel(f, chunks[2], Section::YouTube, &u.yt_info,   &u);
    render_channel(f, chunks[3], Section::Live,    &u.live_info, &u);
    render_keybind_strip(f, chunks[4]);
}

// ── Masthead ──────────────────────────────────────────────────────────

fn render_masthead(f: &mut Frame, area: Rect, u: &UiState) {
    // Outer box — full-width, 3 rows tall
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(Color::Rgb(50, 50, 60)))
        .style(Style::default().bg(Color::Rgb(10, 10, 14)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // ── Left: app brand + transport state ─────────────────────────────
    let (led_char, led_color, state_label) = transport_led(u);

    let left_spans = vec![
        Span::styled(
            " ▌TUNES4R▐ ",
            Style::default()
                .fg(Color::Rgb(220, 220, 255))
                .bg(Color::Rgb(30, 30, 50))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(led_char, Style::default().fg(led_color)),
        Span::raw(" "),
        Span::styled(state_label, Style::default().fg(led_color)),
    ];

    // ── Right: error or active URL (truncated) ─────────────────────────
    let right_text = if !u.error_line.is_empty() {
        format!("⚠ {} ", u.error_line)
    } else {
        let url = &u.active_section_info().url;
        format!("src: {} ", truncate(url, inner.width.saturating_sub(28) as usize))
    };
    let right_color = if !u.error_line.is_empty() {
        Color::Red
    } else {
        Color::Rgb(100, 100, 120)
    };

    // Render left-aligned left part
    f.render_widget(
        Paragraph::new(Line::from(left_spans))
            .style(Style::default().bg(Color::Rgb(10, 10, 14))),
        inner,
    );

    // Render right-aligned right part (manual offset)
    let right_len = right_text.chars().count() as u16;
    if right_len < inner.width {
        let right_area = Rect {
            x: inner.x + inner.width - right_len,
            y: inner.y,
            width: right_len,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Span::styled(right_text, Style::default().fg(right_color))),
            right_area,
        );
    }
}

fn transport_led(u: &UiState) -> (&'static str, Color, &'static str) {
    // Show the state of whichever section is currently active,
    // falling back to the selected section's state.
    let state = [&u.file_info, &u.yt_info, &u.live_info]
        .iter()
        .find(|s| s.is_active)
        .map(|s| &s.state)
        .unwrap_or(&PlaybackState::Stopped);

    match state {
        PlaybackState::Playing    => ("●", Color::Green,              "PLAYING "),
        PlaybackState::Paused     => ("●", Color::Yellow,             "PAUSED  "),
        PlaybackState::Stopped    => ("○", Color::Rgb(60, 60, 70),    "IDLE    "),
        PlaybackState::Connecting => ("◌", Color::Blue,               "CONNECT…"),
        PlaybackState::Buffering {..} => ("◌", Color::Cyan,           "BUFFER… "),
        PlaybackState::Decoding   => ("◌", Color::Rgb(180, 120, 255), "DECODE… "),
        PlaybackState::Error(_)   => ("●", Color::Red,                "ERROR   "),
    }
}

// ── Channel strip ─────────────────────────────────────────────────────

/// Each audio source gets a "channel strip" — a bordered panel whose
/// header bar is coloured to mark the source type, containing:
///   row 0 : header bar  (channel label + key hint + state LED + URL)
///   row 1 : progress track (waveform block chars, 1 row)
///   row 2 : time readout + scrub hint OR buffer %
///   row 3 : metadata row (state label, seek availability, buffer %)
fn render_channel(
    f: &mut Frame,
    area: Rect,
    section: Section,
    info: &SectionInfo,
    ui: &UiState,
) {
    let is_selected = ui.selected_section == section;
    let accent = section_accent(section);

    // Outer border
    let (border_fg, border_type) = if is_selected {
        (accent, BorderType::Thick)
    } else {
        (Color::Rgb(40, 40, 50), BorderType::Plain)
    };

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(border_fg))
        .style(Style::default().bg(Color::Rgb(10, 10, 14)));

    let inner = outer_block.inner(area);
    f.render_widget(outer_block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // ── Header row ────────────────────────────────────────────────────
    render_channel_header(f, inner, section, info, ui, is_selected, accent);

    if inner.height < 2 {
        return;
    }

    // ── Body: idle placeholder or active content ───────────────────────
    let body = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };

    if !info.is_active {
        render_idle_body(f, body, info, section, is_selected);
    } else if info.position.total_ms == 0 {
        render_connecting_body(f, body, &info.state, accent);
    } else {
        render_active_body(f, body, info, ui, accent);
    }
}

fn render_channel_header(
    f: &mut Frame,
    inner: Rect,
    section: Section,
    info: &SectionInfo,
    _ui: &UiState,
    is_selected: bool,
    accent: Color,
) {
    let header_area = Rect { height: 1, ..inner };

    // Coloured tab badge on the left
    let key_num = match section {
        Section::File => "1",
        Section::YouTube => "2",
        Section::Live => "3",
    };
    let badge_label = match section {
        Section::File => " FILE ",
        Section::YouTube => " YT   ",
        Section::Live => " LIVE ",
    };

    // State LED for this channel
    let (led, led_fg) = channel_led(info);

    // URL — truncated to fit
    let badge_len = badge_label.len() as u16 + 3; // key + space + badge
    let led_len: u16 = 3;
    let time_col_len: u16 = if info.is_active && info.position.total_ms > 0 { 14 } else { 0 };
    let available_for_url = inner.width
        .saturating_sub(badge_len + led_len + time_col_len + 2);
    let url_str = truncate(&info.url, available_for_url as usize);

    let mut spans = vec![
        // key hint
        Span::styled(
            format!("[{}]", key_num),
            Style::default().fg(if is_selected {
                Color::Rgb(220, 220, 255)
            } else {
                Color::Rgb(80, 80, 100)
            }),
        ),
        // coloured badge
        Span::styled(
            badge_label,
            Style::default()
                .fg(if is_selected { Color::Black } else { accent })
                .bg(if is_selected { accent } else { Color::Rgb(20, 20, 28) }),
        ),
        Span::raw(" "),
        // state LED
        Span::styled(led, Style::default().fg(led_fg)),
        Span::raw(" "),
        // URL (dimmed)
        Span::styled(
            url_str,
            Style::default().fg(Color::Rgb(90, 90, 110)),
        ),
    ];

    // If active and has duration, pin elapsed/total to the right
    if info.is_active && info.position.total_ms > 0 {
        let time_str = format!(
            " {}/{}",
            format_duration(info.position.current_ms),
            format_duration(info.position.total_ms)
        );
        // Right-align by padding
        let used: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
        let pad = inner.width.saturating_sub(used + time_str.chars().count() as u16);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad as usize)));
        }
        spans.push(Span::styled(
            time_str,
            Style::default().fg(Color::Rgb(160, 160, 200)),
        ));
    }

    f.render_widget(
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Rgb(15, 15, 22))),
        header_area,
    );
}

fn channel_led(info: &SectionInfo) -> (&'static str, Color) {
    if !info.is_active {
        return ("─", Color::Rgb(40, 40, 50));
    }
    match &info.state {
        PlaybackState::Playing        => ("▶", Color::Green),
        PlaybackState::Paused         => ("⏸", Color::Yellow),
        PlaybackState::Stopped        => ("■", Color::Rgb(60, 60, 70)),
        PlaybackState::Connecting     => ("~", Color::Blue),
        PlaybackState::Buffering {..} => ("~", Color::Cyan),
        PlaybackState::Decoding       => ("~", Color::Rgb(180, 120, 255)),
        PlaybackState::Error(_)       => ("!", Color::Red),
    }
}

fn render_idle_body(
    f: &mut Frame,
    area: Rect,
    _info: &SectionInfo,
    section: Section,
    is_selected: bool,
) {
    // Draw a subtle dashed "empty track" line + hint text
    let accent = section_accent(section);
    let track_area = Rect { y: area.y, height: 1, ..area };
    let hint_area  = Rect { y: area.y + 1, height: 1, ..area };

    // Dashed empty track
    let dash: String = (0..area.width as usize)
        .map(|i| if i % 4 == 0 { '·' } else { ' ' })
        .collect();
    f.render_widget(
        Paragraph::new(Span::styled(dash, Style::default().fg(Color::Rgb(35, 35, 45)))),
        track_area,
    );

    if area.height >= 2 {
        let hint = if is_selected {
            format!(" press [p] to load a source")
        } else {
            format!(" press [{}] then [p]", match section {
                Section::File => "1",
                Section::YouTube => "2",
                Section::Live => "3",
            })
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                hint,
                Style::default().fg(if is_selected {
                    accent
                } else {
                    Color::Rgb(50, 50, 60)
                }),
            )),
            hint_area,
        );
    }
}

fn render_connecting_body(
    f: &mut Frame,
    area: Rect,
    state: &PlaybackState,
    accent: Color,
) {
    // Animated-feel spinner using the state text + a scrolling ellipsis bar
    let label = match state {
        PlaybackState::Connecting     => "connecting to source",
        PlaybackState::Buffering {..} => "buffering stream",
        PlaybackState::Decoding       => "decoding audio",
        _                             => "loading",
    };
    let track_area = Rect { y: area.y, height: 1, ..area };
    let label_area = Rect { y: area.y + 1, height: 1, ..area };

    // Sparse marquee bar
    let bar: String = (0..area.width as usize)
        .map(|i| if i % 6 < 2 { '▒' } else { '░' })
        .collect();
    f.render_widget(
        Paragraph::new(Span::styled(bar, Style::default().fg(accent).bg(Color::Rgb(15, 15, 22)))),
        track_area,
    );
    if area.height >= 2 {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(" ◌  {label}…"),
                Style::default().fg(accent),
            )),
            label_area,
        );
    }
}

fn render_active_body(
    f: &mut Frame,
    area: Rect,
    info: &SectionInfo,
    ui: &UiState,
    accent: Color,
) {
    let is_scrubbing = ui.scrub.active;
    let display_ms   = if is_scrubbing { ui.scrub.position_ms } else { info.position.current_ms };

    // ── Row 0: waveform progress track ────────────────────────────────
    let track_area = Rect { y: area.y, height: 1, ..area };
    render_waveform_track(f, track_area, display_ms, info.position.total_ms,
                          info.buffer.write_offset_ms, is_scrubbing, accent);

    if area.height < 2 {
        return;
    }

    // ── Row 1: time + scrub hint OR buffer info ────────────────────────
    let meta_area = Rect { y: area.y + 1, height: 1, ..area };
    render_meta_row(f, meta_area, display_ms, info, ui, is_scrubbing, accent);

    if area.height < 3 {
        return;
    }

    // ── Row 2: state pill + seek badge + buffer % ──────────────────────
    let status_area = Rect { y: area.y + 2, height: 1, ..area };
    render_status_row(f, status_area, info);
}

/// Waveform-style progress track.
///
/// Uses sub-character block fills to give a smooth, high-resolution look
/// even in a single terminal row:
///
///   played  : solid blocks in accent colour
///   buffered: half-filled blocks in dark teal (BUF_COLOR)
///   empty   : near-invisible dots
///   cursor  : bright ◆ at the playhead
fn render_waveform_track(
    f: &mut Frame,
    area: Rect,
    display_ms: u64,
    total_ms:   u64,
    buffer_ms:  u64,
    is_scrubbing: bool,
    accent: Color,
) {
    let w = area.width as usize;
    if w == 0 || total_ms == 0 {
        return;
    }

    // Sub-character precision: each column represents (total_ms / w) ms,
    // and we use eighths-of-a-block characters for the fractional column.
    let ms_per_col = total_ms as f64 / w as f64;
    let pos_f       = display_ms as f64 / ms_per_col;
    let buf_f       = buffer_ms  as f64 / ms_per_col;
    let pos_col     = pos_f as usize;
    let buf_col     = buf_f as usize;

    let track_color = if is_scrubbing { Color::Yellow } else { accent };

    let spans: Vec<Span> = (0..w).map(|col| {
        if col < pos_col {
            // Fully played
            Span::styled("█", Style::default().fg(track_color))
        } else if col == pos_col {
            // Fractional playhead column + cursor diamond
            let frac = ((pos_f - pos_col as f64) * 8.0) as usize;
            let ch = if frac == 0 { '◆' } else { BLOCK_CHARS[frac] };
            Span::styled(
                ch.to_string(),
                Style::default()
                    .fg(Color::White)
                    .bg(track_color),
            )
        } else if col < buf_col {
            // Buffered-ahead zone
            Span::styled("▒", Style::default().fg(BUF_COLOR))
        } else {
            // Empty
            Span::styled("·", Style::default().fg(Color::Rgb(28, 28, 38)))
        }
    }).collect();

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Time readout + scrub annotations on the left, buffer % on the right.
fn render_meta_row(
    f: &mut Frame,
    area: Rect,
    display_ms: u64,
    info: &SectionInfo,
    ui: &UiState,
    is_scrubbing: bool,
    _accent: Color,
) {
    let time_color = if is_scrubbing { Color::Yellow } else { Color::Rgb(200, 200, 220) };

    let elapsed   = format_duration(display_ms);
    let remaining = format_duration(info.position.total_ms.saturating_sub(display_ms));

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(elapsed, Style::default().fg(time_color).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" −{remaining} "),
            Style::default().fg(Color::Rgb(90, 90, 110)),
        ),
    ];

    if is_scrubbing {
        spans.push(Span::styled(" seek: ", Style::default().fg(Color::Rgb(80, 80, 100))));
        spans.push(Span::styled("[←→] ±1s  [↑↓] ±10s", Style::default().fg(Color::Yellow)));
        spans.push(Span::styled("  [⏎] commit", Style::default().fg(Color::Green)));
        spans.push(Span::styled("  [Esc] cancel", Style::default().fg(Color::Red)));
    } else if !ui.scrub.active && info.can_seek {
        spans.push(Span::styled(
            "← → seek",
            Style::default().fg(Color::Rgb(55, 55, 70)),
        ));
    }

    // Buffer % right-aligned
    if info.position.total_ms > 0 {
        let buf_pct = (info.buffer.write_offset_ms as f64 / info.position.total_ms as f64 * 100.0)
            .min(100.0) as u8;
        let buf_str = format!("buf {buf_pct:3}% ");
        let used: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
        let pad = area.width.saturating_sub(used + buf_str.chars().count() as u16);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad as usize)));
        }
        spans.push(Span::styled(
            buf_str,
            Style::default().fg(if buf_pct >= 80 {
                BUF_COLOR
            } else {
                Color::Rgb(55, 55, 70)
            }),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// State pill (playing/paused/…) + seek capability badge.
fn render_status_row(f: &mut Frame, area: Rect, info: &SectionInfo) {
    let (state_str, state_fg, state_bg) = state_pill(&info.state);

    let seek_str = if info.can_seek { " SEEK " } else { " LIVE " };
    let seek_fg  = if info.can_seek { Color::Rgb(0, 150, 100) } else { Color::Rgb(180, 60, 60) };
    let seek_bg  = if info.can_seek { Color::Rgb(0, 30, 20)   } else { Color::Rgb(40, 10, 10)  };

    let spans = vec![
        Span::raw(" "),
        Span::styled(
            state_str,
            Style::default().fg(state_fg).bg(state_bg),
        ),
        Span::raw("  "),
        Span::styled(
            seek_str,
            Style::default().fg(seek_fg).bg(seek_bg),
        ),
    ];

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn state_pill(state: &PlaybackState) -> (&'static str, Color, Color) {
    match state {
        PlaybackState::Playing        => (" ▶ PLAY  ", Color::Rgb(0, 220, 120),  Color::Rgb(0, 35, 20)),
        PlaybackState::Paused         => (" ⏸ PAUSE ", Color::Rgb(230, 180, 0),  Color::Rgb(35, 25, 0)),
        PlaybackState::Stopped        => (" ■ STOP  ", Color::Rgb(90, 90, 110),  Color::Rgb(18, 18, 24)),
        PlaybackState::Connecting     => (" ◌ CONN  ", Color::Rgb(80, 140, 255), Color::Rgb(10, 20, 40)),
        PlaybackState::Buffering {..} => (" ▒ BUF   ", Color::Rgb(0, 200, 220),  Color::Rgb(0, 25, 30)),
        PlaybackState::Decoding       => (" ◌ PROC  ", Color::Rgb(180, 120, 255),Color::Rgb(20, 10, 35)),
        PlaybackState::Error(_)       => (" ! ERR   ", Color::Rgb(255, 80, 80),  Color::Rgb(40, 8, 8)),
    }
}

// ── Keybind strip ─────────────────────────────────────────────────────

fn render_keybind_strip(f: &mut Frame, area: Rect) {
    let entries: &[(&str, &str)] = &[
        ("p",     "play"),
        ("SPC",   "pause"),
        ("s",     "stop"),
        ("←→",   "seek"),
        ("↑↓",   "±10s"),
        ("⏎",    "commit"),
        ("r",     "url"),
        ("q",     "quit"),
    ];

    let mut spans = vec![Span::raw(" ")];
    for (i, (key, action)) in entries.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                "  ",
                Style::default().fg(Color::Rgb(35, 35, 45)),
            ));
        }
        spans.push(Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(Color::Rgb(10, 10, 14))
                .bg(Color::Rgb(70, 70, 90)),
        ));
        spans.push(Span::styled(
            format!(" {action}"),
            Style::default().fg(Color::Rgb(70, 70, 90)),
        ));
    }

    f.render_widget(
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Rgb(10, 10, 14))),
        area,
    );
}

// ── Colour / theme helpers ────────────────────────────────────────────

/// Per-section accent colour — gives each channel strip a distinct identity.
fn section_accent(section: Section) -> Color {
    match section {
        Section::File    => Color::Rgb(80,  160, 255), // cool blue  — local file
        Section::YouTube => Color::Rgb(255, 70,  70),  // YouTube red
        Section::Live    => Color::Rgb(80,  220, 140), // broadcast green
    }
}

// ── Formatting helpers ────────────────────────────────────────────────

fn format_duration(ms: u64) -> String {
    let total_secs = ms / 1_000;
    let hours   = total_secs / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[allow(dead_code)]
fn format_state(state: &PlaybackState) -> String {
    match state {
        PlaybackState::Playing        => "▶  Playing".into(),
        PlaybackState::Paused         => "⏸  Paused".into(),
        PlaybackState::Stopped        => "⏹  Stopped".into(),
        PlaybackState::Connecting     => "⏳  Connecting…".into(),
        PlaybackState::Buffering {..} => "⬇  Buffering…".into(),
        PlaybackState::Decoding       => "🔧  Decoding…".into(),
        PlaybackState::Error(e)       => format!("❌  {}", e),
    }
}

/// Truncate a string to at most `max_chars` characters, appending `…` if cut.
fn truncate(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let cut = max_chars.saturating_sub(1);
        chars[..cut].iter().collect::<String>() + "…"
    }
}