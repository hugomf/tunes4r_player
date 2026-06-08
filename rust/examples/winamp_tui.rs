//! Winamp-style ratatui TUI for tunes4r.
//!
//! Drop-in replacement for the original tui.rs.  All engine types are
//! unchanged.  The visual layout mirrors Winamp 2.x:
//!
//!  ╔══════════════════════════════════════════════╗
//!  ║  ▌TUNES4R▐          ● PLAYING               ║  <- masthead
//!  ╠══════════════════════════════════════════════╣
//!  ║ ░░▓▓████████▓▓░ │ ▶  PLAYING                ║  <- VU | track info
//!  ║ ░░▓▓████████▒▒░ │ 3:02  −0:31               ║
//!  ╠══════════════════════════════════════════════╣
//!  ║ ████████████████████◆░░░░░░░░░░░░░░░░░░░░░░ ║  <- seek bar
//!  ║  3:02  −0:31  3:33        buf 78%  SEEK      ║
//!  ╠══════════════════════════════════════════════╣
//!  ║  [⏮]  [▶]  [⏸]  [⏹]  [⏭]                   ║  <- transport
//!  ╠══════════════════════════════════════════════╣
//!  ║▌ [1] FILE  ▶  dQw4w9WgXcQ          3:02/3:33║  <- channel rows
//!  ║  [2] YT    ─  dQw4w9WgXcQ                   ║
//!  ║  [3] LIVE  ─  wdr-1live…stream.mp3           ║
//!  ╠══════════════════════════════════════════════╣
//!  ║  [p]play [SPC]pause [s]stop [←→]seek [q]quit║  <- keybind strip
//!  ╚══════════════════════════════════════════════╝
//!
//! Winamp colour palette (all via ratatui Color):
//!   BG          = Rgb(14,14,14)
//!   PANEL       = Rgb(10,10,10)
//!   GREEN_HI    = Rgb(0,255,0)
//!   GREEN_MID   = Rgb(0,160,0)
//!   GREEN_DIM   = Rgb(0,55,0)
//!   AMBER       = Rgb(255,179,0)   scrub / paused
//!   RED_LED     = Rgb(255,32,32)   error
//!   BLUE_LED    = Rgb(0,160,255)   connecting
//!   TEAL_BUF    = Rgb(0,95,95)     buffered-ahead
//!   TEXT_HI     = Rgb(200,255,200)
//!   TEXT_DIM    = Rgb(70,110,70)
//!   BORDER      = Rgb(40,75,40)
//!   BORDER_SEL  = Rgb(0,200,0)     selected channel

use std::collections::VecDeque;
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

// ── Palette ───────────────────────────────────────────────────────────

const BG:         Color = Color::Rgb(14, 14, 14);
const PANEL:      Color = Color::Rgb(10, 10, 10);
const GREEN_HI:   Color = Color::Rgb(0, 255, 0);
const GREEN_MID:  Color = Color::Rgb(0, 160, 0);
const GREEN_DIM:  Color = Color::Rgb(0, 55, 0);
const AMBER:      Color = Color::Rgb(255, 179, 0);
const RED_LED:    Color = Color::Rgb(255, 32, 32);
const BLUE_LED:   Color = Color::Rgb(0, 160, 255);
const TEAL_BUF:   Color = Color::Rgb(0, 95, 95);
const TEXT_HI:    Color = Color::Rgb(200, 255, 200);
const TEXT_DIM:   Color = Color::Rgb(70, 110, 70);
const BORDER:     Color = Color::Rgb(40, 75, 40);
const BORDER_SEL: Color = Color::Rgb(0, 200, 0);
const PURPLE_LED: Color = Color::Rgb(180, 120, 255);

// ── Constants ─────────────────────────────────────────────────────────

const POLL_MS:         u64 = 50;
const TICK_MS:         u64 = 50;
const SEEK_SMALL_MS:   u64 = 1_000;
const SEEK_LARGE_MS:   u64 = 10_000;
const LIVE_MAX_MS:     u64 = 30 * 60 * 1_000;
const VU_SEGMENTS:     usize = 16;

// ── Console / Log ───────────────────────────────────────────────────────

const MAX_LOG_ENTRIES: usize = 200;
const CONSOLE_HEIGHT:  u16   = 12;

// ── Section ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section { File, YouTube, Live }

impl Section {
    fn key(self)    -> &'static str { match self { Section::File => "1", Section::YouTube => "2", Section::Live => "3" } }
    fn badge(self)  -> &'static str { match self { Section::File => "FILE", Section::YouTube => "YT  ", Section::Live => "LIVE" } }
    fn prompt(self) -> &'static str { match self { Section::File => "File path", Section::YouTube => "YouTube URL / ID", Section::Live => "Live stream URL" } }
    fn accent(self) -> Color {
        match self {
            Section::File    => Color::Rgb(60, 140, 255),
            Section::YouTube => Color::Rgb(255, 60, 60),
            Section::Live    => Color::Rgb(60, 220, 120),
        }
    }
}

// ── SectionInfo ───────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct SectionInfo {
    url:       String,
    is_active: bool,
    state:     PlaybackState,
    position:  PlaybackPosition,
    buffer:    DownloadBuffer,
    can_seek:  bool,
}

impl SectionInfo {
    fn new(url: impl Into<String>) -> Self { Self { url: url.into(), ..Default::default() } }

    fn apply(&mut self, snap: &EngineSnapshot) {
        if !self.is_active { return; }
        self.state    = snap.state.clone();
        self.position = snap.position;
        self.buffer   = snap.buffer.clone();
        self.can_seek = snap.can_seek;
        if matches!(self.state, PlaybackState::Stopped) { self.is_active = false; }
    }
}

// ── EngineSnapshot ────────────────────────────────────────────────────

#[derive(Clone)]
struct EngineSnapshot {
    state:      PlaybackState,
    position:   PlaybackPosition,
    buffer:     DownloadBuffer,
    can_seek:   bool,
    load_error: String,
}

impl EngineSnapshot {
    fn capture(e: &PlaybackEngine) -> Self {
        Self {
            state:      e.get_state(),
            position:   e.get_position(),
            buffer:     e.get_download_buffer(),
            can_seek:   e.source_supports(Capability::Seek),
            load_error: e.load_error(),
        }
    }
}

// ── ScrubState ────────────────────────────────────────────────────────

#[derive(Default)]
struct ScrubState { position_ms: u64, active: bool }

impl ScrubState {
    fn enter(&mut self, ms: u64) { self.position_ms = ms; self.active = true; }
    fn cancel(&mut self)         { self.active = false; self.position_ms = 0; }
    fn nudge(&mut self, delta: i64, total: u64) {
        if !self.active { return; }
        self.position_ms = ((self.position_ms as i64 + delta).max(0) as u64).min(total);
    }
    fn commit(&mut self) -> Option<u64> {
        if self.active { self.active = false; Some(std::mem::take(&mut self.position_ms)) }
        else { None }
    }
}

// ── Logger & LogBuffer ───────────────────────────────────────────────

#[derive(Clone)]
struct LogEntry {
    level:   log::Level,
    message: String,
}

struct LogBuffer {
    entries: VecDeque<LogEntry>,
}

impl LogBuffer {
    fn new() -> Self { Self { entries: VecDeque::with_capacity(MAX_LOG_ENTRIES) } }

    fn push(&mut self, entry: LogEntry) {
        if self.entries.len() >= MAX_LOG_ENTRIES { self.entries.pop_front(); }
        self.entries.push_back(entry);
    }

    fn entries(&self) -> impl Iterator<Item = &LogEntry> { self.entries.iter() }
    fn len(&self) -> usize { self.entries.len() }
}

struct TuiLogger {
    buffer: Arc<Mutex<LogBuffer>>,
}

impl log::Log for TuiLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) { return; }
        let entry = LogEntry { level: record.level(), message: format!("{}", record.args()) };
        if let Ok(mut buf) = self.buffer.lock() { buf.push(entry); }
    }

    fn flush(&self) {}
}

// ── VuState ───────────────────────────────────────────────────────────

/// Simple peak-decay VU simulation.  Replace with real PCM amplitude taps
/// from the decode thread if you expose them via `Arc<Mutex<[f32;2]>>`.
#[derive(Default)]
struct VuState { levels: [f32; 2], tick: u64 }

impl VuState {
    fn update(&mut self, playing: bool) {
        self.tick = self.tick.wrapping_add(1);
        for (ch, lv) in self.levels.iter_mut().enumerate() {
            if playing {
                // Cheap LCG noise per channel
                let seed = self.tick.wrapping_mul(6364136223846793005 + ch as u64);
                let target = ((seed >> 33) as f32 / u32::MAX as f32) * 0.88 + 0.06;
                *lv = lv.mul_add(0.55, target * 0.45);
            } else {
                *lv = (*lv * 0.80).max(0.0);
            }
        }
    }

    /// Render a single VU channel as `VU_SEGMENTS` block characters.
    fn render_channel(&self, ch: usize) -> Vec<Span<'static>> {
        let lit = (self.levels[ch] * VU_SEGMENTS as f32).round() as usize;
        (0..VU_SEGMENTS).map(|seg| {
            let color = if seg < lit {
                if seg < 10 { GREEN_HI }
                else if seg < 13 { AMBER }
                else { RED_LED }
            } else {
                GREEN_DIM
            };
            Span::styled("█", Style::default().fg(color))
        }).collect()
    }
}

// ── UiState ───────────────────────────────────────────────────────────

struct UiState {
    selected:       Section,
    scrub:          ScrubState,
    vu:             VuState,
    error:          String,
    file:           SectionInfo,
    yt:             SectionInfo,
    live:           SectionInfo,
    show_console:   bool,
    console_scroll: usize,
    log_buffer:     Arc<Mutex<LogBuffer>>,
}

impl UiState {
    fn new(log_buffer: Arc<Mutex<LogBuffer>>) -> Self {
        Self {
            selected: Section::File,
            scrub:    ScrubState::default(),
            vu:       VuState::default(),
            error:    String::new(),
            file:     SectionInfo::new("../example/assets/music.mp3"),
            yt:       SectionInfo::new("dQw4w9WgXcQ"),
            live:     SectionInfo::new(
                "https://wdr-1live-live.icecastssl.wdr.de/wdr/1live/live/mp3/128/stream.mp3",
            ),
            show_console:   false,
            console_scroll: 0,
            log_buffer,
        }
    }

    fn info(&self, s: Section) -> &SectionInfo {
        match s { Section::File => &self.file, Section::YouTube => &self.yt, Section::Live => &self.live }
    }
    fn info_mut(&mut self, s: Section) -> &mut SectionInfo {
        match s { Section::File => &mut self.file, Section::YouTube => &mut self.yt, Section::Live => &mut self.live }
    }
    fn active_info(&self) -> &SectionInfo { self.info(self.selected) }
    fn active_total_ms(&self) -> u64 { self.active_info().position.total_ms }

    fn deactivate_all(&mut self) { self.file.is_active = false; self.yt.is_active = false; self.live.is_active = false; }
    fn mark_active(&mut self, s: Section) { self.deactivate_all(); self.info_mut(s).is_active = true; }

    fn apply_snapshot(&mut self, snap: &EngineSnapshot) {
        for s in [Section::File, Section::YouTube, Section::Live] { self.info_mut(s).apply(snap); }
        if !snap.load_error.is_empty() { self.error = snap.load_error.clone(); }
        let playing = matches!(snap.state, PlaybackState::Playing);
        self.vu.update(playing);
    }

    fn ensure_scrub(&mut self) {
        if !self.scrub.active {
            let ms = self.active_info().position.current_ms;
            self.scrub.enter(ms);
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
    let _ = log::set_boxed_logger(Box::new(TuiLogger { buffer: Arc::clone(&log_buffer) }));
    log::set_max_level(log::LevelFilter::Info);

    let engine = Arc::new(Mutex::new(PlaybackEngine::new_without_device()?));
    let ui     = Arc::new(Mutex::new(UiState::new(Arc::clone(&log_buffer))));

    start_poll_thread(Arc::clone(&ui), Arc::clone(&engine));

    let terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    let result   = run_event_loop(terminal, &ui, &engine);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    if let Err(ref e) = result { eprintln!("Fatal: {e}"); }
    Ok(())
}

fn start_poll_thread(ui: Arc<Mutex<UiState>>, engine: Arc<Mutex<PlaybackEngine>>) {
    std::thread::spawn(move || loop {
        let snap = { let e = engine.lock().unwrap(); EngineSnapshot::capture(&e) };
        ui.lock().unwrap().apply_snapshot(&snap);
        std::thread::sleep(Duration::from_millis(POLL_MS));
    });
}

// ── Event loop ────────────────────────────────────────────────────────

fn run_event_loop<B: ratatui::backend::Backend>(
    mut terminal: Terminal<B>,
    ui:     &Arc<Mutex<UiState>>,
    engine: &Arc<Mutex<PlaybackEngine>>,
) -> io::Result<()> {
    let tick = Duration::from_millis(TICK_MS);
    let mut last_tick = Instant::now();

    loop {
        let timeout = tick.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match handle_key(key.code, ui, engine)? {
                    KeyOutcome::Quit    => break,
                    KeyOutcome::Handled | KeyOutcome::Ignored => {}
                }
            }
        }
        if last_tick.elapsed() >= tick { last_tick = Instant::now(); }
        terminal.draw(|f| render(f, ui))?;
    }

    engine.lock().unwrap().stop();
    Ok(())
}

#[must_use]
enum KeyOutcome { Quit, Handled, Ignored }

fn handle_key(
    code:   KeyCode,
    ui:     &Arc<Mutex<UiState>>,
    engine: &Arc<Mutex<PlaybackEngine>>,
) -> io::Result<KeyOutcome> {
    match code {
        KeyCode::Char('1') => { ui.lock().unwrap().selected = Section::File; }
        KeyCode::Char('2') => { ui.lock().unwrap().selected = Section::YouTube; }
        KeyCode::Char('3') => { ui.lock().unwrap().selected = Section::Live; }

        KeyCode::Char('p') => { handle_play(ui, engine)?; }

        KeyCode::Char(' ') => {
            let mut e = engine.lock().unwrap();
            match e.get_state() {
                PlaybackState::Playing => e.pause(),
                PlaybackState::Paused  => e.resume(),
                _ => {}
            }
        }

        KeyCode::Char('s') => {
            engine.lock().unwrap().stop();
            ui.lock().unwrap().deactivate_all();
        }

        KeyCode::Char('k') => {
            let mut u = ui.lock().unwrap();
            if u.show_console {
                u.console_scroll = u.console_scroll.saturating_sub(1);
            } else {
                let (ms, total) = (u.active_info().position.current_ms, u.active_total_ms());
                if total > 0 { u.scrub.enter(ms); }
            }
        }

        KeyCode::Left  | KeyCode::Char('a') => { let mut u = ui.lock().unwrap(); let t = u.active_total_ms(); if t > 0 { u.ensure_scrub(); u.scrub.nudge(-(SEEK_SMALL_MS as i64), t); } }
        KeyCode::Right | KeyCode::Char('d') => { let mut u = ui.lock().unwrap(); let t = u.active_total_ms(); if t > 0 { u.ensure_scrub(); u.scrub.nudge(SEEK_SMALL_MS as i64, t); } }
        KeyCode::Up    | KeyCode::Char('w') => { let mut u = ui.lock().unwrap(); let t = u.active_total_ms(); if t > 0 { u.ensure_scrub(); u.scrub.nudge(-(SEEK_LARGE_MS as i64), t); } }
        KeyCode::Down  | KeyCode::Char('x') => { let mut u = ui.lock().unwrap(); let t = u.active_total_ms(); if t > 0 { u.ensure_scrub(); u.scrub.nudge(SEEK_LARGE_MS as i64, t); } }

        KeyCode::Enter => {
            if let Some(ms) = ui.lock().unwrap().scrub.commit() {
                let _ = engine.lock().unwrap().seek(ms);
            }
        }

        KeyCode::Esc => {
            let mut u = ui.lock().unwrap();
            if u.scrub.active { u.scrub.cancel(); } else { return Ok(KeyOutcome::Quit); }
        }
        KeyCode::Char('q') => return Ok(KeyOutcome::Quit),
        KeyCode::Char('r') => { handle_edit_url(ui)?; }

        KeyCode::Char('l') => {
            let mut u = ui.lock().unwrap();
            u.show_console = !u.show_console;
            u.console_scroll = 0;
        }

        KeyCode::Char('j') => {
            let mut u = ui.lock().unwrap();
            if u.show_console {
                let max = {
                    let buf = u.log_buffer.lock().unwrap();
                    buf.len().saturating_sub((CONSOLE_HEIGHT as usize).saturating_sub(2))
                };
                u.console_scroll = u.console_scroll.saturating_add(1).min(max);
            }
        }
        _ => return Ok(KeyOutcome::Ignored),
    }
    Ok(KeyOutcome::Handled)
}

fn handle_play(ui: &Arc<Mutex<UiState>>, engine: &Arc<Mutex<PlaybackEngine>>) -> io::Result<()> {
    disable_raw_mode()?;
    let url_opt = prompt_url(ui);
    enable_raw_mode()?;

    let url = match url_opt { Some(u) if !u.is_empty() => u, _ => return Ok(()) };
    let section = ui.lock().unwrap().selected;
    let is_live = matches!(section, Section::Live);

    ui.lock().unwrap().info_mut(section).url = url.clone();

    let result = {
        let mut e = engine.lock().unwrap();
        if is_live { e.play_live(&url, LIVE_MAX_MS) } else { e.play(&url, None) }
    };

    let mut u = ui.lock().unwrap();
    match result {
        Ok(_)  => { u.error.clear(); u.mark_active(section); }
        Err(e) => { u.error = format!("Play failed: {e}"); }
    }
    Ok(())
}

fn handle_edit_url(ui: &Arc<Mutex<UiState>>) -> io::Result<()> {
    disable_raw_mode()?;
    let url_opt = prompt_url(ui);
    enable_raw_mode()?;
    if let Some(url) = url_opt {
        let s = ui.lock().unwrap().selected;
        ui.lock().unwrap().info_mut(s).url = url;
    }
    Ok(())
}

fn prompt_url(ui: &Arc<Mutex<UiState>>) -> Option<String> {
    let (current, label) = {
        let u = ui.lock().unwrap();
        let s = u.selected;
        (u.info(s).url.clone(), s.prompt())
    };
    print!("{} [{}]: ", label, current);
    io::stdout().flush().ok()?;
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input).ok()?;
    let trimmed = input.trim().to_string();
    Some(if trimmed.is_empty() { current } else { trimmed })
}

// ── Render ────────────────────────────────────────────────────────────

fn render(f: &mut Frame, ui: &Arc<Mutex<UiState>>) {
    let u = ui.lock().unwrap();

    f.render_widget(
        Block::default().style(Style::default().bg(BG)),
        f.area(),
    );

    if u.show_console {
        let main_h = f.area().height.saturating_sub(CONSOLE_HEIGHT);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(main_h), Constraint::Length(CONSOLE_HEIGHT)])
            .split(f.area());
        render_main(f, chunks[0], &u);
        render_console(f, chunks[1], &u);
    } else {
        render_main(f, f.area(), &u);
    }
}

fn render_main(f: &mut Frame, area: Rect, u: &UiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // masthead
            Constraint::Length(4),  // VU + info
            Constraint::Length(3),  // seek
            Constraint::Length(3),  // transport
            Constraint::Length(3),  // ch FILE
            Constraint::Length(3),  // ch YT
            Constraint::Length(3),  // ch LIVE
            Constraint::Length(1),  // keybinds
        ])
        .split(area);

    render_masthead(f, chunks[0], u);
    render_vu_info(f, chunks[1], u);
    render_seek(f, chunks[2], u);
    render_transport(f, chunks[3], u);
    render_channel(f, chunks[4], Section::File,    &u.file, u);
    render_channel(f, chunks[5], Section::YouTube, &u.yt,   u);
    render_channel(f, chunks[6], Section::Live,    &u.live, u);
    render_keybinds(f, chunks[7]);
}

fn render_console(f: &mut Frame, area: Rect, u: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER))
        .title(" Console [l] ")
        .title_alignment(ratatui::layout::Alignment::Left)
        .style(Style::default().bg(Color::Rgb(5, 5, 10)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 2 || inner.width < 4 { return; }

    let buf = u.log_buffer.lock().unwrap();
    let entries: Vec<&LogEntry> = buf.entries().collect();
    let total = entries.len();
    if total == 0 {
        f.render_widget(
            Paragraph::new(Span::styled(" (no log entries yet) ", Style::default().fg(TEXT_DIM)))
                .style(Style::default().bg(Color::Rgb(5, 5, 10))),
            inner,
        );
        return;
    }

    let visible = inner.height as usize;
    let start   = u.console_scroll.min(total.saturating_sub(visible));
    let end     = start + visible.min(total - start);

    for (i, entry) in entries[start..end].iter().enumerate() {
        let (level_color, level_tag) = match entry.level {
            log::Level::Error => (RED_LED, "ERR"),
            log::Level::Warn  => (AMBER,  "WRN"),
            log::Level::Info  => (GREEN_MID, "INF"),
            log::Level::Debug => (TEXT_DIM, "DBG"),
            log::Level::Trace => (Color::Rgb(40, 60, 40), "TRC"),
        };
        let line = Line::from(vec![
            Span::styled(
                format!(" {:>3} ", level_tag),
                Style::default().fg(level_color).bg(Color::Rgb(10, 10, 15)),
            ),
            Span::styled(
                truncate(&entry.message, inner.width.saturating_sub(6) as usize),
                Style::default().fg(TEXT_HI),
            ),
        ]);
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(Color::Rgb(5, 5, 10))),
            Rect { y: inner.y + i as u16, height: 1, x: inner.x, width: inner.width },
        );
    }

    // Scroll indicator
    if total > visible && u.console_scroll + visible < total {
        let pct = (u.console_scroll as f64 / total.saturating_sub(visible) as f64 * 100.0) as u8;
        let hint = format!(" ↑ {pct}% ");
        let hint_len = hint.chars().count() as u16;
        let hint_x = inner.x + inner.width.saturating_sub(hint_len);
        f.render_widget(
            Paragraph::new(Span::styled(&hint, Style::default().fg(TEXT_DIM)))
                .style(Style::default().bg(Color::Rgb(5, 5, 10))),
            Rect { y: inner.y, height: 1, x: hint_x, width: hint_len },
        );
    }
}

// ── Masthead ──────────────────────────────────────────────────────────

fn render_masthead(f: &mut Frame, area: Rect, u: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(Color::Rgb(8, 8, 8)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Brand
    let mut spans = vec![
        Span::styled(
            " ▌TUNES4R▐ ",
            Style::default().fg(GREEN_HI).bg(Color::Rgb(20, 36, 20)).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];

    // Transport LED from whichever section is active
    let active_state = [&u.file, &u.yt, &u.live]
        .iter()
        .find(|s| s.is_active)
        .map(|s| &s.state)
        .unwrap_or(&PlaybackState::Stopped);

    let (led, led_color, label) = state_led(active_state);
    spans.push(Span::styled(led, Style::default().fg(led_color)));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(label, Style::default().fg(led_color).add_modifier(Modifier::BOLD)));

    // Error or source URL on the right
    let right_text = if !u.error.is_empty() {
        format!(" ⚠ {} ", u.error)
    } else {
        let url = &u.active_info().url;
        format!(" src: {} ", truncate(url, inner.width.saturating_sub(32) as usize))
    };
    let right_color = if !u.error.is_empty() { RED_LED } else { TEXT_DIM };

    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(8, 8, 8))), inner);

    // Right-align the source hint
    let rl = right_text.chars().count() as u16;
    if rl < inner.width {
        let right_area = Rect { x: inner.x + inner.width - rl, y: inner.y, width: rl, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(right_text, Style::default().fg(right_color))), right_area);
    }
}

// ── VU meters + track info ────────────────────────────────────────────

fn render_vu_info(f: &mut Frame, area: Rect, u: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 2 { return; }

    // Split horizontally: VU (fixed 20 cols) | info (rest)
    let vu_w = (VU_SEGMENTS as u16 + 2).min(inner.width / 2);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(vu_w), Constraint::Min(0)])
        .split(inner);

    // ── VU meters ─────────────────────────────────────────────────────
    let vu_area = cols[0];
    // Channel L
    {
        let mut spans = u.vu.render_channel(0);
        spans.insert(0, Span::styled("L", Style::default().fg(TEXT_DIM)));
        f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(PANEL)),
            Rect { y: vu_area.y, height: 1, ..vu_area });
    }
    // Channel R
    {
        let mut spans = u.vu.render_channel(1);
        spans.insert(0, Span::styled("R", Style::default().fg(TEXT_DIM)));
        f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(PANEL)),
            Rect { y: vu_area.y + 1, height: 1, ..vu_area });
    }

    // ── Track info ────────────────────────────────────────────────────
    let info_area = cols[1];
    let active = [&u.file, &u.yt, &u.live].into_iter().find(|s| s.is_active);

    if let Some(info) = active {
        // Row 0: state label
        let (_, led_color, state_label) = state_led(&info.state);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(state_label, Style::default().fg(led_color).add_modifier(Modifier::BOLD)),
            ])).style(Style::default().bg(PANEL)),
            Rect { y: info_area.y, height: 1, ..info_area },
        );

        // Row 1: elapsed / remaining
        if info.position.total_ms > 0 {
            let is_scrub  = u.scrub.active;
            let disp_ms   = if is_scrub { u.scrub.position_ms } else { info.position.current_ms };
            let disp_str  = fmt_ms(disp_ms);
            let total_str = fmt_ms(info.position.total_ms);
            let color     = if is_scrub { AMBER } else { GREEN_HI };

            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(disp_str, Style::default().fg(color).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!("  −{}", fmt_ms(info.position.total_ms.saturating_sub(disp_ms))),
                        Style::default().fg(TEXT_DIM),
                    ),
                    Span::styled(format!("  {total_str}"), Style::default().fg(TEXT_DIM)),
                ])).style(Style::default().bg(PANEL)),
                Rect { y: info_area.y + 1, height: 1, ..info_area },
            );
        }
    } else {
        f.render_widget(
            Paragraph::new(Span::styled("  NO SOURCE LOADED", Style::default().fg(TEXT_DIM)))
                .style(Style::default().bg(PANEL)),
            Rect { y: info_area.y, height: 1, ..info_area },
        );
    }
}

// ── Seek bar ──────────────────────────────────────────────────────────

fn render_seek(f: &mut Frame, area: Rect, u: &UiState) {
    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 1 || inner.width < 4 { return; }

    let info     = u.active_info();
    let total    = info.position.total_ms;
    let is_scrub = u.scrub.active;
    let disp_ms  = if is_scrub { u.scrub.position_ms } else { info.position.current_ms };

    // ── Track row ─────────────────────────────────────────────────────
    let track_area = Rect { y: inner.y, height: 1, ..inner };
    if total > 0 {
        let w      = inner.width as usize;
        let pos_f  = (disp_ms as f64 / total as f64).clamp(0.0, 1.0) * w as f64;
        let buf_f  = (info.buffer.write_offset_ms as f64 / total as f64).clamp(0.0, 1.0) * w as f64;
        let pos_col = pos_f as usize;
        let buf_col = buf_f as usize;
        let track_color = if is_scrub { AMBER } else { GREEN_HI };

        // Sub-char fill using eighths
        let frac_idx = ((pos_f - pos_col as f64) * 8.0) as usize;
        const EIGHTHS: [&str; 9] = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];

        let spans: Vec<Span> = (0..w).map(|col| {
            if col < pos_col {
                Span::styled("█", Style::default().fg(track_color))
            } else if col == pos_col {
                let ch = if frac_idx == 0 { "◆" } else { EIGHTHS[frac_idx] };
                Span::styled(ch, Style::default().fg(Color::White).bg(track_color))
            } else if col < buf_col {
                Span::styled("▒", Style::default().fg(TEAL_BUF))
            } else {
                Span::styled("·", Style::default().fg(GREEN_DIM))
            }
        }).collect();

        f.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(PANEL)),
            track_area,
        );
    } else {
        // Idle groove
        let groove: String = (0..inner.width as usize).map(|i| if i % 5 == 0 { '─' } else { '·' }).collect();
        f.render_widget(
            Paragraph::new(Span::styled(groove, Style::default().fg(GREEN_DIM)))
                .style(Style::default().bg(PANEL)),
            track_area,
        );
    }

    // ── Meta row ──────────────────────────────────────────────────────
    if inner.height < 2 { return; }
    let meta_area = Rect { y: inner.y + 1, height: 1, ..inner };
    let color = if is_scrub { AMBER } else { TEXT_HI };

    let mut spans = vec![
        Span::styled(
            fmt_ms(disp_ms),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  −{}  {}", fmt_ms(total.saturating_sub(disp_ms)), fmt_ms(total)),
            Style::default().fg(TEXT_DIM),
        ),
    ];

    if is_scrub {
        spans.push(Span::styled("   [←→]±1s [↑↓]±10s [⏎]commit [Esc]cancel",
            Style::default().fg(AMBER)));
    } else {
        // buf % and seek badge right-aligned
        let info = u.active_info();
        if total > 0 {
            let buf_pct = (info.buffer.write_offset_ms as f64 / total as f64 * 100.0).min(100.0) as u8;
            let seek_str = if info.can_seek { " SEEK " } else { " LIVE " };
            let seek_col = if info.can_seek { Color::Rgb(0, 180, 100) } else { Color::Rgb(200, 60, 60) };
            let right = format!("buf {:3}%  {}", buf_pct, seek_str.trim());
            let used: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
            let pad = inner.width.saturating_sub(used + right.chars().count() as u16);
            if pad > 0 { spans.push(Span::raw(" ".repeat(pad as usize))); }
            spans.push(Span::styled(format!("buf {:3}% ", buf_pct), Style::default().fg(TEXT_DIM)));
            spans.push(Span::styled(seek_str, Style::default().fg(seek_col).bg(Color::Rgb(10, 22, 10))));
        }
    }

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(PANEL)),
        meta_area,
    );
}

// ── Transport bar ─────────────────────────────────────────────────────

fn render_transport(f: &mut Frame, area: Rect, u: &UiState) {
    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(Color::Rgb(10, 10, 10)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let active_state = [&u.file, &u.yt, &u.live]
        .iter().find(|s| s.is_active)
        .map(|s| &s.state)
        .unwrap_or(&PlaybackState::Stopped);

    // Button definitions: (symbol, always-lit-color)
    let buttons: &[(&str, Option<Color>)] = &[
        ("  ⏮  ", None),
        ("  ▶  ", Some(GREEN_HI)),
        ("  ⏸  ", Some(AMBER)),
        ("  ⏹  ", None),
        ("  ⏭  ", None),
    ];

    let mut spans = vec![Span::raw(" ")];
    for (i, (sym, accent)) in buttons.iter().enumerate() {
        let lit = match (i, active_state) {
            (1, PlaybackState::Playing)    => true,
            (2, PlaybackState::Paused)     => true,
            (3, PlaybackState::Stopped)    => true,
            _ => false,
        };
        let fg = if lit { accent.unwrap_or(GREEN_HI) } else { TEXT_DIM };
        let bg = if lit { Color::Rgb(12, 28, 12) } else { Color::Rgb(8, 8, 8) };
        spans.push(Span::styled(
            *sym,
            Style::default().fg(fg).bg(bg).add_modifier(if lit { Modifier::BOLD } else { Modifier::empty() }),
        ));
        spans.push(Span::raw(" "));
    }

    // State pill on the right
    let (pill_str, pill_fg, pill_bg) = state_pill(active_state);
    let pill_len = pill_str.chars().count() as u16 + 2;
    let used: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
    let pad = inner.width.saturating_sub(used + pill_len);
    if pad > 0 { spans.push(Span::raw(" ".repeat(pad as usize))); }
    spans.push(Span::styled(
        format!(" {} ", pill_str),
        Style::default().fg(pill_fg).bg(pill_bg).add_modifier(Modifier::BOLD),
    ));

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(10, 10, 10))),
        Rect { y: inner.y, height: 1, ..inner },
    );
}

// ── Channel strip ─────────────────────────────────────────────────────

fn render_channel(f: &mut Frame, area: Rect, section: Section, info: &SectionInfo, u: &UiState) {
    let is_selected = u.selected == section;
    let accent      = section.accent();

    let (border_fg, border_type) = if is_selected {
        (BORDER_SEL, BorderType::Thick)
    } else {
        (BORDER, BorderType::Plain)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(border_fg))
        .style(Style::default().bg(BG));

    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 { return; }

    // ── Header row ─────────────────────────────────────────────────────
    {
        let (led, led_col) = channel_led(info);
        let badge_col = if is_selected { Color::Black } else { accent };
        let badge_bg  = if is_selected { accent } else { Color::Rgb(16, 18, 16) };

        // Time string (right-side)
        let time_str = if info.is_active && info.position.total_ms > 0 {
            format!(" {}/{} ", fmt_ms(info.position.current_ms), fmt_ms(info.position.total_ms))
        } else {
            String::new()
        };

        let url_budget = inner.width
            .saturating_sub(16 + time_str.chars().count() as u16) as usize;
        let url_str = truncate(&info.url, url_budget);

        let mut spans = vec![
            Span::styled(
                format!("[{}]", section.key()),
                Style::default().fg(if is_selected { TEXT_HI } else { TEXT_DIM }),
            ),
            Span::styled(
                format!(" {} ", section.badge()),
                Style::default().fg(badge_col).bg(badge_bg),
            ),
            Span::raw(" "),
            Span::styled(led, Style::default().fg(led_col)),
            Span::raw(" "),
            Span::styled(url_str, Style::default().fg(TEXT_DIM)),
        ];

        if !time_str.is_empty() {
            let used: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
            let pad = inner.width.saturating_sub(used + time_str.chars().count() as u16);
            if pad > 0 { spans.push(Span::raw(" ".repeat(pad as usize))); }
            spans.push(Span::styled(time_str, Style::default().fg(GREEN_HI)));
        }

        f.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(10, 14, 10))),
            Rect { y: inner.y, height: 1, ..inner },
        );
    }

    if inner.height < 2 { return; }

    // ── Progress mini-bar ──────────────────────────────────────────────
    //   One slim coloured line at the bottom of the channel strip.
    if info.is_active && info.position.total_ms > 0 {
        let w = inner.width as usize;
        let pos_ratio = (info.position.current_ms as f64 / info.position.total_ms as f64).clamp(0.0, 1.0);
        let buf_ratio = (info.buffer.write_offset_ms as f64 / info.position.total_ms as f64).clamp(0.0, 1.0);
        let pos_col   = (pos_ratio * w as f64) as usize;
        let buf_col   = (buf_ratio * w as f64) as usize;

        let spans: Vec<Span> = (0..w).map(|col| {
            let color = if col < pos_col { accent }
                else if col < buf_col { TEAL_BUF }
                else { GREEN_DIM };
            Span::styled("▄", Style::default().fg(color))
        }).collect();

        f.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(BG)),
            Rect { y: inner.y + 1, height: 1, ..inner },
        );
    } else {
        // Idle groove
        let groove: String = (0..inner.width as usize).map(|i| if i % 4 == 0 { '·' } else { ' ' }).collect();
        f.render_widget(
            Paragraph::new(Span::styled(groove, Style::default().fg(Color::Rgb(25, 35, 25))))
                .style(Style::default().bg(BG)),
            Rect { y: inner.y + 1, height: 1, ..inner },
        );
    }
}

// ── Keybind strip ─────────────────────────────────────────────────────

fn render_keybinds(f: &mut Frame, area: Rect) {
    let keys: &[(&str, &str)] = &[
        ("p", "play"), ("SPC", "pause"), ("s", "stop"),
        ("←→", "seek"), ("↑↓", "±10s"), ("⏎", "commit"),
        ("l", "log"), ("j/k", "scroll"), ("r", "url"), ("q", "quit"),
    ];

    let mut spans = vec![Span::raw(" ")];
    for (i, (k, action)) in keys.iter().enumerate() {
        if i > 0 { spans.push(Span::styled("  ", Style::default().fg(GREEN_DIM))); }
        spans.push(Span::styled(
            format!("[{}]", k),
            Style::default().fg(GREEN_MID).bg(Color::Rgb(10, 20, 10)),
        ));
        spans.push(Span::styled(
            format!(" {}", action),
            Style::default().fg(TEXT_DIM),
        ));
    }

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(BG)),
        area,
    );
}

// ── Theme helpers ─────────────────────────────────────────────────────

fn state_led(state: &PlaybackState) -> (&'static str, Color, &'static str) {
    match state {
        PlaybackState::Playing        => ("●", GREEN_HI,                 "PLAYING "),
        PlaybackState::Paused         => ("●", AMBER,                    "PAUSED  "),
        PlaybackState::Stopped        => ("○", Color::Rgb(55, 55, 55),   "IDLE    "),
        PlaybackState::Connecting     => ("◌", BLUE_LED,                 "CONNECT…"),
        PlaybackState::Buffering {..} => ("◌", Color::Rgb(0, 200, 220),  "BUFFER… "),
        PlaybackState::Decoding       => ("◌", PURPLE_LED,               "DECODE… "),
        PlaybackState::Error(_)       => ("●", RED_LED,                  "ERROR   "),
    }
}

fn channel_led(info: &SectionInfo) -> (&'static str, Color) {
    if !info.is_active { return ("─", Color::Rgb(35, 55, 35)); }
    match &info.state {
        PlaybackState::Playing        => ("▶", GREEN_HI),
        PlaybackState::Paused         => ("⏸", AMBER),
        PlaybackState::Stopped        => ("■", Color::Rgb(55, 55, 55)),
        PlaybackState::Connecting     => ("~", BLUE_LED),
        PlaybackState::Buffering {..} => ("~", Color::Rgb(0, 200, 220)),
        PlaybackState::Decoding       => ("~", PURPLE_LED),
        PlaybackState::Error(_)       => ("!", RED_LED),
    }
}

fn state_pill(state: &PlaybackState) -> (&'static str, Color, Color) {
    match state {
        PlaybackState::Playing        => ("▶ PLAY ", GREEN_HI,                Color::Rgb(0, 30, 0)),
        PlaybackState::Paused         => ("⏸ PAUSE", AMBER,                  Color::Rgb(30, 20, 0)),
        PlaybackState::Stopped        => ("■ STOP ", Color::Rgb(80, 80, 80), Color::Rgb(8, 8, 8)),
        PlaybackState::Connecting     => ("◌ CONN ", BLUE_LED,                Color::Rgb(0, 15, 30)),
        PlaybackState::Buffering {..} => ("▒ BUF  ", Color::Rgb(0,200,220),  Color::Rgb(0, 20, 22)),
        PlaybackState::Decoding       => ("◌ PROC ", PURPLE_LED,              Color::Rgb(18, 8, 30)),
        PlaybackState::Error(_)       => ("! ERROR", RED_LED,                 Color::Rgb(30, 5, 5)),
    }
}

// ── Formatting helpers ────────────────────────────────────────────────

fn fmt_ms(ms: u64) -> String {
    let s = ms / 1_000;
    let h = s / 3_600;
    let m = (s % 3_600) / 60;
    let s = s % 60;
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 { return String::new(); }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max { s.to_string() }
    else { chars[..max.saturating_sub(1)].iter().collect::<String>() + "…" }
}