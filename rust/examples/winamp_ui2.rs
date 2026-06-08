//! Full-size (540px) Winamp Classic clone – Authentic Winamp 2.x visuals.
//! Fixes: title bar grip, vertical background gradient, mono/stereo glow,
//! seek track groove, and authentic seek thumb bevel.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, Painter, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, Ui, Vec2, WindowLevel,
};

use tunes4r::audio::engine::types::{set_band_count, GLOBAL_SPECTRUM};
use tunes4r::audio::stream::source::Capability;
use tunes4r::models::{DownloadBuffer, PlaybackPosition, PlaybackState};
use tunes4r::PlaybackEngine;

// =============================================================================
// Color Palette
// =============================================================================

const TITLE_TEXT: Color32 = Color32::from_rgb(200, 200, 216);

const BODY_DARK: Color32 = Color32::from_rgb(19, 18, 28);
const BODY_MID: Color32 = Color32::from_rgb(54, 54, 84);

const LCD_BG: Color32 = Color32::from_rgb(20, 35, 20);
const LCD_DOT: Color32 = Color32::from_rgb(28, 48, 28);
const LCD_SEG_ON: Color32 = Color32::from_rgb(57, 255, 20);
const LCD_SEG_OFF: Color32 = Color32::from_rgb(26, 46, 26);
const STATE_RED: Color32 = Color32::from_rgb(204, 51, 0);

const SPEC_GREEN: Color32 = Color32::from_rgb(0, 204, 0);
const SPEC_YELGRN: Color32 = Color32::from_rgb(136, 204, 0);
const SPEC_AMBER: Color32 = Color32::from_rgb(255, 170, 0);
const SPEC_ORANGE: Color32 = Color32::from_rgb(221, 102, 0);
const SPEC_RED: Color32 = Color32::from_rgb(204, 51, 0);
const PEAK_WHITE: Color32 = Color32::WHITE;
const RULE_A: Color32 = Color32::from_rgb(0, 170, 170);
const RULE_B: Color32 = Color32::from_rgb(0, 136, 136);

// Seek bar green gradient colors
const BEVEL_DARK: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e);
const BEVEL_LIGHT: Color32 = Color32::from_rgb(0x6d, 0x6d, 0x7e);
const BEVEL_WIDTH: f32 = 2.0;
const SEEK_THUMB_W: f32 = 60.0;
const SEEK_THUMB_H: f32 = 20.0;
const SEEK_BAR_H: f32 = 15.0;
const BODY_BORDER_MARGIN: f32 = 8.0;
const BODY_BORDER_PADDING: f32 = 5.0;
const SHUFFLE_BTN_W: f32 = 75.0;
const SHUFFLE_BTN_H: f32 = 28.0;
const REPEAT_BTN_W: f32 = 48.0;
const REPEAT_BTN_H: f32 = 28.0;

const TRACK_BG: Color32 = Color32::from_rgb(26, 42, 26);
const TRACK_BORDER: Color32 = Color32::from_rgb(42, 74, 42);

const INFO_BADGE_BG: Color32 = Color32::from_rgb(10, 26, 10);

// =============================================================================
// Constants
// =============================================================================

const BORDER_PAD: i8 = 10;
const WIN_W: f32 = 450.0;
const WIN_H: f32 = 200.0;
const TITLE_BAR_H: f32 = 22.0;
const BODY_PAD_TOP: f32 = 4.0;
const BODY_PAD_LR: f32 = 8.0;
const BODY_PAD_BOT: f32 = 6.0;
const LCD_W: f32 = 180.0;
const LCD_H: f32 = 90.0;
const CONTROLS_BAR_H: f32 = 22.0;
const BOTTOM_STRIP_H: f32 = 28.0;
const GAP: f32 = 4.0;
const PLAYER_BTN_W: f32 = 32.0;
const PLAYER_BTN_H: f32 = 32.0;

const N_SPECTRUM_BARS: usize = 32;
const SPECTRUM_TOP_OFFSET: f32 = 52.0;
const METADATA_GAP: f32 = 8.0;
const PEAK_BOUNCE: f32 = 2.0;
const PEAK_GRAVITY: f32 = 0.04;
const PEAK_MAX: f32 = 1.0;


// =============================================================================
// 7-Segment Digit
// =============================================================================

struct SevenSegDigit {
    x: f32,
    y: f32,
    segments: [bool; 7],
    seg_w: f32,
    seg_h: f32,
    vert_w: f32,
    vert_h: f32,
    gap: f32,
}

impl SevenSegDigit {
    fn new(digit: u8, x: f32, y: f32) -> Self {
        const PATTERNS: [[bool; 7]; 10] = [
            [true, true, true, false, true, true, true],
            [false, false, true, false, false, true, false],
            [true, false, true, true, true, false, true],
            [true, false, true, true, false, true, true],
            [false, true, true, true, false, true, false],
            [true, true, false, true, false, true, true],
            [true, true, false, true, true, true, true],
            [true, false, true, false, false, true, false],
            [true, true, true, true, true, true, true],
            [true, true, true, true, false, true, true],
        ];
        let segments = PATTERNS[digit as usize % 10];
        let seg_w = 14.0;
        let seg_h = 3.0;
        let vert_w = 2.0;
        let vert_h = 13.0;
        let gap = 1.0;
        Self { x, y, segments, seg_w, seg_h, vert_w, vert_h, gap }
    }

    fn draw(&self, painter: &Painter) {
        let h_seg = |x: f32, y: f32| -> Vec<Pos2> {
            vec![
                Pos2::new(x, y),
                Pos2::new(x + self.seg_w, y),
                Pos2::new(x + self.seg_w, y + self.seg_h),
                Pos2::new(x, y + self.seg_h),
            ]
        };
        let v_seg = |x: f32, y: f32| -> Vec<Pos2> {
            vec![
                Pos2::new(x, y),
                Pos2::new(x + self.vert_w, y),
                Pos2::new(x + self.vert_w, y + self.vert_h),
                Pos2::new(x, y + self.vert_h),
            ]
        };

        let top_h_y = self.y;
        let top_v_y = top_h_y + self.seg_h + self.gap;
        let mid_h_y = top_v_y + self.vert_h + self.gap;
        let bottom_v_y = mid_h_y + self.seg_h + self.gap;
        let bottom_h_y = bottom_v_y + self.vert_h + self.gap;

        let left_x = self.x;
        let right_x = self.x + self.seg_w + 2.0;

        let polys = [
            h_seg(self.x + 2.0, top_h_y),
            v_seg(left_x, top_v_y),
            v_seg(right_x, top_v_y),
            h_seg(self.x + 2.0, mid_h_y),
            v_seg(left_x, bottom_v_y),
            v_seg(right_x, bottom_v_y),
            h_seg(self.x + 2.0, bottom_h_y),
        ];

        for (i, &active) in self.segments.iter().enumerate() {
            let color = if active { LCD_SEG_ON } else { LCD_SEG_OFF };
            painter.add(Shape::convex_polygon(polys[i].clone(), color, Stroke::NONE));
        }
    }
}


// =============================================================================
// Spectrum Analyzer
// =============================================================================

struct SpectrumState {
    smoothed: [f32; N_SPECTRUM_BARS],
    peaks: [f32; N_SPECTRUM_BARS],
    peak_vel: [f32; N_SPECTRUM_BARS],
}

impl SpectrumState {
    fn new() -> Self {
        Self {
            smoothed: [0.0; N_SPECTRUM_BARS],
            peaks: [0.0; N_SPECTRUM_BARS],
            peak_vel: [0.0; N_SPECTRUM_BARS],
        }
    }

    fn update(&mut self, is_playing: bool) {
        if !is_playing {
            for a in &mut self.smoothed {
                *a = (*a * 0.82).max(0.0);
            }
            for p in &mut self.peaks {
                *p = 0.0;
            }
            return;
        }

        // Read real spectrum data from the engine
        let raw = GLOBAL_SPECTRUM.read().unwrap();
        let n = raw.len().min(N_SPECTRUM_BARS);

        for i in 0..N_SPECTRUM_BARS {
            let t = if i < n { raw[i] } else { 0.0 };
            let c = self.smoothed[i];
            self.smoothed[i] = if t > c {
                (c + 0.22 * (t - c)).min(1.0)
            } else {
                (c - 0.10 * (c - t)).max(0.0)
            };
            // Peak bounce physics
            let amp = self.smoothed[i];
            if amp >= self.peaks[i] {
                self.peak_vel[i] = (amp - self.peaks[i]) * PEAK_BOUNCE;
                self.peaks[i] = amp;
            } else {
                self.peak_vel[i] -= PEAK_GRAVITY;
                self.peaks[i] = (self.peaks[i] + self.peak_vel[i]).clamp(0.0, PEAK_MAX);
            }
        }
    }
}

// =============================================================================
// Scrolling Title
// =============================================================================

struct ScrollingTitle {
    offset: f32,
    last_update: Instant,
}

impl ScrollingTitle {
    fn new() -> Self {
        Self { offset: 0.0, last_update: Instant::now() }
    }
    fn update(&mut self, now: Instant) {
        let delta = now.duration_since(self.last_update).as_secs_f32();
        self.last_update = now;
        self.offset += delta * 50.0;
        if self.offset >= 200.0 {
            self.offset -= 200.0;
        }
    }
    fn draw(&self, painter: &Painter, rect: Rect, text: &str, color: Color32) {
        let font_id = egui::FontId::new(16.0, FontFamily::Name("04b03".into()));
        let full_text = format!("  {}  ", text);
        let galley = painter.layout_no_wrap(full_text.clone(), font_id.clone(), color);
        let full_width = galley.size().x;
        let painter = painter.with_clip_rect(rect);
        let x_start = rect.left() - self.offset;
        painter.galley(Pos2::new(x_start, rect.center().y - galley.size().y / 2.0), galley.clone(), color);
        if x_start + full_width < rect.right() {
            painter.galley(Pos2::new(x_start + full_width, rect.center().y - galley.size().y / 2.0), galley, color);
        }
    }
}

// =============================================================================
// Engine Integration
// =============================================================================

#[derive(Clone)]
#[allow(dead_code)]
struct EngineSnapshot {
    state: PlaybackState,
    position: PlaybackPosition,
    buffer: DownloadBuffer,
    can_seek: bool,
    load_error: String,
    meta_title: String,
    meta_artist: String,
}

impl EngineSnapshot {
    fn capture(engine: &PlaybackEngine) -> Self {
        let info = engine.source_info();
        Self {
            state: engine.get_state(),
            position: engine.get_position(),
            buffer: engine.get_download_buffer(),
            can_seek: engine.source_supports(Capability::Seek),
            load_error: engine.load_error(),
            meta_title: info.as_ref().and_then(|i| i.title.clone()).unwrap_or_default(),
            meta_artist: info.as_ref().and_then(|i| i.artist.clone()).unwrap_or_default(),
        }
    }
}

#[derive(Default)]
struct ScrubState {
    position_ms: u64,
    active: bool,
}

impl ScrubState {
    fn enter(&mut self, ms: u64) { self.position_ms = ms; self.active = true; }
    fn cancel(&mut self) { self.active = false; self.position_ms = 0; }
    fn commit(&mut self) -> Option<u64> {
        if self.active { self.active = false; Some(std::mem::take(&mut self.position_ms)) } else { None }
    }
}

// =============================================================================
// App State
// =============================================================================

struct WinampTestApp {
    engine: Arc<Mutex<PlaybackEngine>>,
    snap: EngineSnapshot,
    scrub: ScrubState,
    spectrum: SpectrumState,
    scrolling: ScrollingTitle,
    volume: f32,
    balance: f32,
    shuffle: bool,
    repeat: bool,
    eq_on: bool,
    pl_on: bool,
    url: String,
    error: String,
    loaders_installed: bool,
}

impl WinampTestApp {
    fn new(engine: Arc<Mutex<PlaybackEngine>>) -> Self {
        let snap = EngineSnapshot::capture(&engine.lock().unwrap());
        Self {
            engine, snap, scrub: ScrubState::default(), spectrum: SpectrumState::new(),
            scrolling: ScrollingTitle::new(), volume: 0.8, balance: 0.5,
            shuffle: false, repeat: false, eq_on: false, pl_on: false, url: String::new(), error: String::new(),
            loaders_installed: false,
        }
    }
    fn poll_engine(&mut self) {
        let snap = { let e = self.engine.lock().unwrap(); EngineSnapshot::capture(&e) };
        if !snap.load_error.is_empty() { self.error = snap.load_error.clone(); }
        self.snap = snap;
    }
    fn total_ms(&self) -> u64 { self.snap.position.total_ms }
    fn current_ms(&self) -> u64 { self.snap.position.current_ms }
    fn is_playing(&self) -> bool { matches!(self.snap.state, PlaybackState::Playing) }
    fn vol_color(&self) -> Color32 {
        let v = self.volume;
        if v < 0.5 {
            let t = v / 0.5;
            Color32::from_rgb(
                (255.0 * t).round().min(255.0) as u8,
                (180.0 + 150.0 * t).round().min(255.0) as u8,
                0,
            )
        } else {
            let t = (v - 0.5) / 0.5;
            Color32::from_rgb(
                255,
                (255.0 * (1.0 - t)).round() as u8,
                0,
            )
        }
    }
    fn bal_color(&self) -> Color32 {
        let b = self.balance;
        if b < 0.5 {
            let t = b / 0.5;
            Color32::from_rgb(
                (255.0 * (1.0 - t)).round() as u8,
                255,
                0,
            )
        } else {
            let t = (b - 0.5) / 0.5;
            Color32::from_rgb(
                (510.0 * t).round().min(255.0) as u8,
                (255.0 * (1.0 - t)).round() as u8,
                0,
            )
        }
    }
}

// =============================================================================
// eframe::App
// =============================================================================

impl eframe::App for WinampTestApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if !self.loaders_installed {
            egui_extras::install_image_loaders(ui.ctx());
            self.loaders_installed = true;
        }
        self.poll_engine();
        let now = Instant::now();
        let is_playing = self.is_playing();
        self.spectrum.update(is_playing);
        self.scrolling.update(now);
        ui.ctx().request_repaint_after(Duration::from_millis(33));

        egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| {
            // Full-window horizontal gradient background
            let bg_rect = ui.available_rect_before_wrap();
            paint_h_gradient(ui.painter(), bg_rect, BODY_DARK, BODY_MID, 64);

            let avail = ui.available_width();
            let x_off = ((avail - WIN_W) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(x_off);
                ui.vertical(|ui| {
                    ui.set_min_width(WIN_W);
                    ui.set_max_width(WIN_W);
                    self.render_title_bar(ui);
                    egui::Frame::new()
                        .inner_margin(egui::Margin { left: BORDER_PAD, right: BORDER_PAD, top: BORDER_PAD, bottom: BORDER_PAD })
                        .show(ui, |ui| {
                            self.render_main_body(ui);
                            self.render_bottom_strip(ui);
                        });
                });
            });

            // Logo vertically aligned with player buttons
            let logo_size = Vec2::new(34.0, 34.0);
            let controls_bar_y = bg_rect.top() + TITLE_BAR_H + BORDER_PAD as f32 + BODY_PAD_TOP + LCD_H + GAP + SEEK_BAR_H + GAP;
            let logo_y = controls_bar_y + (CONTROLS_BAR_H - logo_size.y) / 2.0 + 10.0;
            let logo_pos = Pos2::new(bg_rect.right() - 40.0, logo_y);
            let logo_rect = Rect::from_min_size(logo_pos, logo_size);
            ui.put(logo_rect, egui::Image::new(egui::include_image!("../assets/logo-rustamp.png")));

        });

        ui.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                let mut e = self.engine.lock().unwrap();
                match e.get_state() {
                    PlaybackState::Playing => e.pause(),
                    PlaybackState::Paused => e.resume(),
                    _ => {}
                }
            }
            if i.key_pressed(egui::Key::S) { self.engine.lock().unwrap().stop(); }
            let total = self.total_ms();
            if total > 0 {
                let cur = self.current_ms();
                if i.key_pressed(egui::Key::ArrowLeft) {
                    if !self.scrub.active { self.scrub.enter(cur); }
                    self.scrub.position_ms = self.scrub.position_ms.saturating_sub(1000);
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    if !self.scrub.active { self.scrub.enter(cur); }
                    self.scrub.position_ms = (self.scrub.position_ms + 1000).min(total);
                }
            }
            if i.key_pressed(egui::Key::Enter) {
                if let Some(ms) = self.scrub.commit() {
                    let _ = self.engine.lock().unwrap().seek(ms);
                }
            }
            if i.key_pressed(egui::Key::Escape) { self.scrub.cancel(); }
        });
    }
}

// =============================================================================
// Rendering
// =============================================================================

impl WinampTestApp {
    fn render_title_bar(&mut self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(WIN_W, TITLE_BAR_H), Sense::hover());

        // Window drag (registered before buttons so buttons take priority)
        let drag_resp = ui.interact(rect, egui::Id::new("title_drag"), Sense::drag());

        // Menu button (top-left)
        let menu_center = Pos2::new(rect.left() + 11.0, rect.top() + 11.0);
        let menu_r = Rect::from_center_size(menu_center, Vec2::new(20.0, 18.0));
        let menu_resp = ui.put(menu_r, egui::Image::new(egui::include_image!("../assets/menu.png")).sense(Sense::click()));
        if menu_resp.is_pointer_button_down_on() {
            egui::Image::new(egui::include_image!("../assets/menu_pressed.png")).paint_at(ui, menu_r);
        }

        // Window control buttons with images
        let right = rect.right();
        let btn_size = Vec2::new(16.0, 12.0);
        let btn_y = rect.top() + (rect.height() - btn_size.y) / 2.0;

        // Minimize
        let min_r = Rect::from_min_size(Pos2::new(right - 3.0 * btn_size.x - 12.0, btn_y), btn_size);
        let min_resp = ui.put(min_r, egui::Image::new(egui::include_image!("../assets/min.png")).sense(Sense::click()));
        if min_resp.is_pointer_button_down_on() {
            egui::Image::new(egui::include_image!("../assets/min_pressed.png")).paint_at(ui, min_r);
        }

        // Maximize
        let max_r = Rect::from_min_size(Pos2::new(right - 2.0 * btn_size.x - 8.0, btn_y), btn_size);
        let max_resp = ui.put(max_r, egui::Image::new(egui::include_image!("../assets/max.png")).sense(Sense::click()));
        if max_resp.is_pointer_button_down_on() {
            egui::Image::new(egui::include_image!("../assets/max_pressed.png")).paint_at(ui, max_r);
        }

        // Close
        let close_r = Rect::from_min_size(Pos2::new(right - btn_size.x - 4.0, btn_y), btn_size);
        let close_resp = ui.put(close_r, egui::Image::new(egui::include_image!("../assets/close.png")).sense(Sense::click()));

        let painter = ui.painter();

        // Metallic gold decorative lines
        let gold_top_pad = 5.0;
        let menu_right = menu_r.right();
        let winamp_half_w = 22.0;
        let winamp_left = rect.center().x - winamp_half_w;
        let winamp_right = rect.center().x + winamp_half_w;
        let min_left = min_r.left();
        let pad = 8.0;

        let left_section = Rect::from_min_max(
            Pos2::new(menu_right + pad, rect.top() + gold_top_pad),
            Pos2::new(winamp_left - pad, rect.bottom()),
        );
        let right_section = Rect::from_min_max(
            Pos2::new(winamp_right + pad, rect.top() + gold_top_pad),
            Pos2::new(min_left - pad, rect.bottom()),
        );
        let metallic = MetallicGold::new();
        metallic.draw(&painter, left_section, right_section);

        // Title text – use monospace for pixelated look
        let text = "WINAMP";
        let font_id = egui::FontId::new(12.0, FontFamily::Monospace);
        painter.text(rect.center() + egui::vec2(-0.5, 0.0), Align2::CENTER_CENTER, text, font_id.clone(), TITLE_TEXT);
        painter.text(rect.center() + egui::vec2(0.5, 0.0), Align2::CENTER_CENTER, text, font_id, TITLE_TEXT);

        // Handle interactions
        if drag_resp.drag_started() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag); }
        if min_resp.clicked() { /* TODO: minimize to compact player */ }
        if max_resp.clicked() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)); }
        if close_resp.clicked() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close); }
        if menu_resp.clicked() { /* TODO: open menu */ }
    }

    fn render_main_body(&mut self, ui: &mut Ui) {
        let body_rect = ui.available_rect_before_wrap();
        ui.add_space(BODY_PAD_TOP);
        ui.horizontal(|ui| { ui.add_space(BODY_PAD_LR); ui.horizontal(|ui| { self.render_lcd_panel(ui); ui.add_space(METADATA_GAP); self.render_metadata_panel(ui); }); ui.add_space(BODY_PAD_LR); });
        ui.add_space(GAP);
        ui.horizontal(|ui| { ui.add_space(BODY_PAD_LR); self.render_seek_bar(ui); ui.add_space(BODY_PAD_LR); });
        ui.add_space(GAP);
        ui.horizontal(|ui| { ui.add_space(BODY_PAD_LR); self.render_controls_bar(ui); ui.add_space(BODY_PAD_LR); });
        ui.add_space(BODY_PAD_BOT);
        let painter = ui.painter();
        let border_rect = body_rect.shrink(BODY_BORDER_MARGIN + BODY_BORDER_PADDING);
        painter.rect_stroke(border_rect, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(106, 106, 154)), StrokeKind::Inside);
    }

    fn render_lcd_panel(&mut self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(LCD_W, LCD_H), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, LCD_BG);
        let dark_edge = Color32::from_rgb(6, 18, 6);
        painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(BEVEL_WIDTH, dark_edge));
        painter.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(BEVEL_WIDTH, dark_edge));
        painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
        painter.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
        
        // Dot matrix
        let cell = 4.0;
        let dot_r = 1.0;
        let mut y = rect.top() + cell / 2.0;
        while y < rect.bottom() {
            let mut x = rect.left() + cell / 2.0;
            while x < rect.right() {
                painter.circle_filled(Pos2::new(x, y), dot_r, LCD_DOT);
                x += cell;
            }
            y += cell;
        }
        
        // Play/Pause/Stop icon inside LCD (left side)
        let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 18.0, rect.top() + 6.0), Vec2::new(16.0, 22.0));
        match self.snap.state {
            PlaybackState::Playing => {
                painter.add(Shape::convex_polygon(vec![
                    Pos2::new(icon_rect.left(), icon_rect.top()),
                    Pos2::new(icon_rect.left(), icon_rect.bottom()),
                    Pos2::new(icon_rect.right(), icon_rect.center().y),
                ], LCD_SEG_ON, Stroke::NONE));
            }
            PlaybackState::Paused => {
                let bar_w = 4.0;
                let gap = 2.0;
                painter.rect_filled(Rect::from_min_size(Pos2::new(icon_rect.left(), icon_rect.top()), Vec2::new(bar_w, icon_rect.height())), CornerRadius::ZERO, LCD_SEG_ON);
                painter.rect_filled(Rect::from_min_size(Pos2::new(icon_rect.left() + bar_w + gap, icon_rect.top()), Vec2::new(bar_w, icon_rect.height())), CornerRadius::ZERO, LCD_SEG_ON);
            }
            _ => {
                painter.rect_filled(icon_rect, CornerRadius::same(1), LCD_SEG_ON);
            }
        }
        
        // "CUR" / "TOT" label
        let cur_label = if self.is_playing() { "CUR" } else { "TOT" };
        painter.text(Pos2::new(rect.left() + 28.0, rect.top() + 6.0), Align2::LEFT_TOP, cur_label, egui::FontId::new(8.0, FontFamily::Monospace), LCD_SEG_OFF);
        
        // Timer
        let timer_origin = Pos2::new(rect.left() + 46.0, rect.top() + 5.0);
        let total = self.total_ms();
        let current = self.current_ms();
        let (time_str, with_minus) = if self.is_playing() { (fmt_ms(current), false) } else if total > 0 { (fmt_ms(total.saturating_sub(current)), true) } else { ("00:00".to_string(), false) };
        self.draw_timer(painter, timer_origin, &time_str, with_minus);
        
        // Spectrum
        let spec_rect = Rect::from_min_max(Pos2::new(rect.left() + 18.0, rect.top() + SPECTRUM_TOP_OFFSET), Pos2::new(rect.right() - 5.0, rect.bottom() - 5.0));
        self.draw_spectrum(painter, spec_rect, rect);
    }

    fn draw_timer(&self, painter: &Painter, origin: Pos2, time_str: &str, with_minus: bool) {
        let mut x = origin.x + 15.0;
        if with_minus {
            let mr = Rect::from_min_size(Pos2::new(origin.x, origin.y + 18.0), Vec2::new(12.0, 5.0));
            painter.rect_filled(mr, CornerRadius::same(2), LCD_SEG_ON);
        }
        for ch in time_str.chars() {
            if ch == ':' {
                let cx = x + 4.0;
                painter.circle_filled(Pos2::new(cx, origin.y + 14.0), 2.5, LCD_SEG_ON);
                painter.circle_filled(Pos2::new(cx, origin.y + 24.0), 2.5, LCD_SEG_ON);
                x += 10.0;
            } else if let Some(d) = ch.to_digit(10) {
                SevenSegDigit::new(d as u8, x, origin.y).draw(painter);
                x += 22.0 + 5.0;
            }
        }
    }

    fn draw_spectrum(&self, painter: &Painter, rect: Rect, lcd_rect: Rect) {
        let pitch = 7.0;
        let dot_r = 1.5;
        let mut y = rect.top() + 1.5;
        let mut idx = 0;
        while y < rect.bottom() - 1.5 {
            let color = if idx % 2 == 0 { RULE_A } else { RULE_B };
            painter.circle_filled(Pos2::new(rect.left() + 1.5, y), dot_r, color);
            y += pitch; idx += 1;
        }
        let mut x = rect.left() + 1.5;
        idx = 0;
        while x < rect.right() - 1.5 {
            let color = if idx % 2 == 0 { RULE_A } else { RULE_B };
            painter.circle_filled(Pos2::new(x, rect.bottom() - 1.5), dot_r, color);
            x += pitch; idx += 1;
        }
        if !self.is_playing() { return; }
        let bars_rect = rect.shrink2(Vec2::new(5.0, 5.0));
        let gap = 1.0;
        let n = N_SPECTRUM_BARS as f32;
        let bar_w = ((bars_rect.width() - (n - 1.0) * gap) / n).max(2.0).min(5.0);
        let total_w = n * bar_w + (n - 1.0) * gap;
        let start_x = bars_rect.left() + (bars_rect.width() - total_w) * 0.5;
        let zones = [(0.00,0.20,SPEC_GREEN),(0.20,0.45,SPEC_YELGRN),(0.45,0.65,SPEC_AMBER),(0.65,0.82,SPEC_ORANGE),(0.82,1.00,SPEC_RED)];
        let lcd_painter = painter.with_clip_rect(lcd_rect);
        for i in 0..N_SPECTRUM_BARS {
            let amp = self.spectrum.smoothed[i];
            let peak = self.spectrum.peaks[i];
            let bx = start_x + i as f32 * (bar_w + gap);
            for (z_lo, z_hi, color) in &zones {
                if amp > *z_lo {
                    let z_top = bars_rect.bottom() - (amp.min(*z_hi) * bars_rect.height());
                    let z_bot = bars_rect.bottom() - *z_lo * bars_rect.height();
                    let z_rect = Rect::from_min_max(Pos2::new(bx, z_top), Pos2::new(bx + bar_w, z_bot));
                    lcd_painter.rect_filled(z_rect, CornerRadius::ZERO, *color);
                }
            }
            if peak > 0.02 {
                let py = bars_rect.bottom() - peak * bars_rect.height() - 2.0;
                lcd_painter.line_segment([Pos2::new(bx, py), Pos2::new(bx + bar_w, py)], Stroke::new(2.0, PEAK_WHITE));
            }
        }
    }

    fn render_metadata_panel(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            let title_rect = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover()).0;
            let painter = ui.painter();
            painter.rect_filled(title_rect, CornerRadius::ZERO, INFO_BADGE_BG);
            let dark_edge = Color32::from_rgb(6, 18, 6);
            painter.line_segment([title_rect.left_top(), title_rect.right_top()], Stroke::new(BEVEL_WIDTH, dark_edge));
            painter.line_segment([title_rect.left_top(), title_rect.left_bottom()], Stroke::new(BEVEL_WIDTH, dark_edge));
            painter.line_segment([title_rect.left_bottom(), title_rect.right_bottom()], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
            painter.line_segment([title_rect.right_top(), title_rect.right_bottom()], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
            let display = if !self.snap.meta_artist.is_empty() || !self.snap.meta_title.is_empty() {
                format!("{} - {}", self.snap.meta_artist, self.snap.meta_title)
            } else {
                self.url.clone()
            };
            let title_text = if !self.error.is_empty() { self.error.clone() } else if self.url.is_empty() { "Winamp Classic 2.x".to_string() } else { display };
            let title_color = if !self.error.is_empty() { STATE_RED } else { LCD_SEG_ON };
            self.scrolling.draw(painter, title_rect, &title_text, title_color);
            ui.add_space(4.0);
            
            // Bitrate / Sample rate / Mono / Stereo row – drawn directly for glow
            let row_rect = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover()).0;
            let rp = ui.painter();
            let mut x = row_rect.left();
            let y = row_rect.center().y;
            let box_h = 16.0;

            // 256 kbps
            let val_rect = Rect::from_min_size(Pos2::new(x, row_rect.center().y - box_h / 2.0), Vec2::new(30.0, box_h));
            rp.rect_filled(val_rect, CornerRadius::ZERO, INFO_BADGE_BG);
            rp.line_segment([val_rect.left_top(), val_rect.right_top()], Stroke::new(BEVEL_WIDTH, Color32::from_rgb(6, 18, 6)));
            rp.line_segment([val_rect.left_top(), val_rect.left_bottom()], Stroke::new(BEVEL_WIDTH, Color32::from_rgb(6, 18, 6)));
            rp.line_segment([val_rect.left_bottom(), val_rect.right_bottom()], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
            rp.line_segment([val_rect.right_top(), val_rect.right_bottom()], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
            rp.text(val_rect.center(), Align2::CENTER_CENTER, "256", egui::FontId::new(12.0, FontFamily::Name("04b03".into())), LCD_SEG_ON);
            x = val_rect.right() + 4.0;
            rp.text(Pos2::new(x, y), Align2::LEFT_CENTER, "kbps", egui::FontId::new(14.0, FontFamily::Name("04b03".into())), Color32::WHITE);
            x += 36.0;

            // 44 kHz
            let val_rect = Rect::from_min_size(Pos2::new(x, row_rect.center().y - box_h / 2.0), Vec2::new(24.0, box_h));
            rp.rect_filled(val_rect, CornerRadius::ZERO, INFO_BADGE_BG);
            rp.line_segment([val_rect.left_top(), val_rect.right_top()], Stroke::new(BEVEL_WIDTH, Color32::from_rgb(6, 18, 6)));
            rp.line_segment([val_rect.left_top(), val_rect.left_bottom()], Stroke::new(BEVEL_WIDTH, Color32::from_rgb(6, 18, 6)));
            rp.line_segment([val_rect.left_bottom(), val_rect.right_bottom()], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
            rp.line_segment([val_rect.right_top(), val_rect.right_bottom()], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
            rp.text(val_rect.center(), Align2::CENTER_CENTER, "44", egui::FontId::new(12.0, FontFamily::Name("04b03".into())), LCD_SEG_ON);
            x = val_rect.right() + 4.0;
            rp.text(Pos2::new(x, y), Align2::LEFT_CENTER, "kHz", egui::FontId::new(12.0, FontFamily::Name("04b03".into())), Color32::WHITE);

            // mono / stereo aligned to right using images
            let _ = rp;
            let is_stereo = self.is_playing();
            let img_w = 42.0;
            let img_h = 24.0;
            let gap = 0.0;
            let y_off = row_rect.center().y - img_h / 2.0;
            let stereo_x = row_rect.right() - img_w;
            let mono_x = stereo_x - img_w - gap;
            ui.put(
                Rect::from_min_size(Pos2::new(mono_x, y_off), Vec2::new(img_w, img_h)),
                egui::Image::new(egui::include_image!("../assets/mono_off.png")),
            );
            let stereo_img = if is_stereo {
                egui::include_image!("../assets/stereo_on.png")
            } else {
                egui::include_image!("../assets/stereo_off.png")
            };
            ui.put(
                Rect::from_min_size(Pos2::new(stereo_x, y_off), Vec2::new(img_w, img_h)),
                egui::Image::new(stereo_img),
            );
            
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 0.0;
                let vc = self.vol_color();
                let (_, vresp) = ui.allocate_exact_size(Vec2::new(88.0, 14.0), Sense::click_and_drag());
                if let Some(new_v) = draw_slider(ui, &vresp, self.volume, vc) { self.volume = new_v; self.engine.lock().unwrap().set_volume(new_v); }
                ui.add_space(4.0);
                let bc = self.bal_color();
                let (_, bresp) = ui.allocate_exact_size(Vec2::new(44.0, 14.0), Sense::click_and_drag());
                if let Some(new_b) = draw_slider(ui, &bresp, self.balance, bc) { self.balance = new_b; self.engine.lock().unwrap().set_balance(new_b); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // PL button with pl_off.png / pl_on.png
                    let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(38.0, 22.0), Sense::click());
                    let pressed = btn_resp.is_pointer_button_down_on();
                    let pl_src = if self.pl_on {
                        if pressed { egui::include_image!("../assets/pl_on_pressed.png") } else { egui::include_image!("../assets/pl_on.png") }
                    } else {
                        if pressed { egui::include_image!("../assets/pl_off_pressed.png") } else { egui::include_image!("../assets/pl_off.png") }
                    };
                    egui::Image::new(pl_src).paint_at(ui, btn_rect);
                    if btn_resp.clicked() { self.pl_on = !self.pl_on; }
                    
                    // EQ button with eq_off.png / eq_on.png
                    let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(38.0, 22.0), Sense::click());
                    let pressed = btn_resp.is_pointer_button_down_on();
                    if self.eq_on {
                        let eq_src = if pressed { egui::include_image!("../assets/eq_on_pressed.png") } else { egui::include_image!("../assets/eq_on.png") };
                        egui::Image::new(eq_src).paint_at(ui, btn_rect);
                    } else {
                        let eq_src = if pressed { egui::include_image!("../assets/eq_off_pressed.png") } else { egui::include_image!("../assets/eq_off.png") };
                        egui::Image::new(eq_src).paint_at(ui, btn_rect);
                    }
                    if btn_resp.clicked() { self.eq_on = !self.eq_on; }
                });
            });
        });
    }

    fn render_seek_bar(&mut self, ui: &mut Ui) {
        let total = self.total_ms();
        let is_scrubbing = self.scrub.active;
        let display_ms = if is_scrubbing { self.scrub.position_ms } else { self.current_ms() };
        let ratio = if total > 0 { (display_ms as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width() + 65.0, SEEK_BAR_H), Sense::hover());
        let painter = ui.painter();
        
        // Recessed track groove – thin, centered
        let track_h = 17.0;
        let track_pad_l = 0.0;
        let track_pad_r = 80.0;
        let track_y = rect.top() + (rect.height() - track_h) / 2.0;
        let track_rect = Rect::from_min_size(
            Pos2::new(rect.left() + track_pad_l, track_y),
            Vec2::new(rect.width() - track_pad_l - track_pad_r, track_h),
        );
        
        // Dark background
        painter.rect_filled(track_rect, CornerRadius::same(1), Color32::from_rgb(20, 20, 30));
        
        // Recessed border: dark top/left, light bottom/right
        painter.line_segment([Pos2::new(track_rect.left(), track_rect.top()), Pos2::new(track_rect.right(), track_rect.top())], Stroke::new(BEVEL_WIDTH, BEVEL_DARK));
        painter.line_segment([Pos2::new(track_rect.left(), track_rect.top()), Pos2::new(track_rect.left(), track_rect.bottom())], Stroke::new(BEVEL_WIDTH, BEVEL_DARK));
        painter.line_segment([Pos2::new(track_rect.left(), track_rect.bottom()), Pos2::new(track_rect.right(), track_rect.bottom())], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
        painter.line_segment([Pos2::new(track_rect.right(), track_rect.top()), Pos2::new(track_rect.right(), track_rect.bottom())], Stroke::new(BEVEL_WIDTH, BEVEL_LIGHT));
        
        // Authentic Winamp seek thumb – centered on track
        let thumb_w = SEEK_THUMB_W;
        let thumb_h = SEEK_THUMB_H;
        let thumb_x = (track_rect.left() + track_rect.width() * ratio).min(track_rect.right() - thumb_w);
        let thumb_y = rect.top() + (rect.height() - thumb_h) / 2.0;
        let thumb = Rect::from_min_size(Pos2::new(thumb_x, thumb_y), Vec2::new(thumb_w, thumb_h));
        egui::Image::new(egui::include_image!("../assets/slider-thumb.png")).paint_at(ui, thumb);
        
        let resp = ui.interact(rect, egui::Id::new("seek"), Sense::click_and_drag());
        if resp.dragged() { if let Some(pos) = resp.interact_pointer_pos() { let t = ((pos.x - track_rect.left()) / track_rect.width()).clamp(0.0, 1.0); self.scrub.enter((t * total as f32) as u64); } }
        if resp.drag_stopped() && self.scrub.active { let ms = self.scrub.position_ms; let _ = self.engine.lock().unwrap().seek(ms); self.scrub.cancel(); }
        if resp.clicked() { if let Some(pos) = resp.interact_pointer_pos() { let t = ((pos.x - track_rect.left()) / track_rect.width()).clamp(0.0, 1.0); let ms = (t * total as f32) as u64; let _ = self.engine.lock().unwrap().seek(ms); } }
    }

    fn render_controls_bar(&mut self, ui: &mut Ui) {
        let _rect = ui.available_rect_before_wrap();
        
        ui.horizontal(|ui| {
            ui.style_mut().spacing.item_spacing.x = 0.0;
            ui.set_height(CONTROLS_BAR_H);
            
            let btn_h = PLAYER_BTN_H;
            let btn_w = PLAYER_BTN_W;
            
            // Prev button with prev.png / prev_pressed.png
            let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(btn_w, btn_h), Sense::click());
            let pressed = btn_resp.is_pointer_button_down_on();
            let prev_src = if pressed { egui::include_image!("../assets/prev_pressed.png") } else { egui::include_image!("../assets/prev.png") };
            egui::Image::new(prev_src).paint_at(ui, btn_rect);
            if btn_resp.clicked() {
                let ms = self.current_ms().saturating_sub(5000);
                let _ = self.engine.lock().unwrap().seek(ms);
            }
            
            // Play button with play.png / play_pressed.png
            let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(btn_w, btn_h), Sense::click());
            let pressed = btn_resp.is_pointer_button_down_on();
            let play_src = if pressed { egui::include_image!("../assets/play_pressed.png") } else { egui::include_image!("../assets/play.png") };
            egui::Image::new(play_src).paint_at(ui, btn_rect);
            if btn_resp.clicked() {
                if self.url.is_empty() { self.error = "NO MUSIC LOADED!".to_string(); }
                else { self.error.clear(); let mut e = self.engine.lock().unwrap(); let _ = e.play(&self.url, None); }
            }
            
            // Pause button with pause.png / pause_pressed.png
            let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(btn_w, btn_h), Sense::click());
            let pressed = btn_resp.is_pointer_button_down_on();
            let pause_src = if pressed { egui::include_image!("../assets/pause_pressed.png") } else { egui::include_image!("../assets/pause.png") };
            egui::Image::new(pause_src).paint_at(ui, btn_rect);
            if btn_resp.clicked() {
                let mut e = self.engine.lock().unwrap();
                match e.get_state() { PlaybackState::Playing => e.pause(), PlaybackState::Paused => e.resume(), _ => {} }
            }
            
            // Stop button with stop.png / stop_pressed.png
            let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(btn_w, btn_h), Sense::click());
            let pressed = btn_resp.is_pointer_button_down_on();
            let stop_src = if pressed { egui::include_image!("../assets/stop_pressed.png") } else { egui::include_image!("../assets/stop.png") };
            egui::Image::new(stop_src).paint_at(ui, btn_rect);
            if btn_resp.clicked() {
                self.engine.lock().unwrap().stop();
            }
            
            // Next button with next.png / next_pressed.png
            let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(btn_w, btn_h), Sense::click());
            let pressed = btn_resp.is_pointer_button_down_on();
            let next_src = if pressed { egui::include_image!("../assets/next_pressed.png") } else { egui::include_image!("../assets/next.png") };
            egui::Image::new(next_src).paint_at(ui, btn_rect);
            if btn_resp.clicked() {
                let ms = self.current_ms() + 5000;
                let _ = self.engine.lock().unwrap().seek(ms);
            }
            ui.add_space(30.0);
            
            // Eject button with eject.png / eject_pressed.png
            let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(btn_w, btn_h), Sense::click());
            let pressed = btn_resp.is_pointer_button_down_on();
            let eject_src = if pressed { egui::include_image!("../assets/eject_pressed.png") } else { egui::include_image!("../assets/eject.png") };
            egui::Image::new(eject_src).paint_at(ui, btn_rect);
            if btn_resp.clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("Audio", &["mp3","wav","flac","ogg","m4a","aac","opus"]).pick_file() {
                    let p = path.to_string_lossy().to_string();
                    self.url = p.clone();
                    self.error.clear();
                    let mut e = self.engine.lock().unwrap();
                    let _ = e.play(&p, None);
                }
            }
            ui.add_space(30.0);
            
            // Shuffle button with shuffle_off.png / shuffle_on.png
            let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(SHUFFLE_BTN_W, SHUFFLE_BTN_H), Sense::click());
            let pressed = btn_resp.is_pointer_button_down_on();
            let shuffle_src = if self.shuffle {
                if pressed { egui::include_image!("../assets/shuffle_on_pressed.png") } else { egui::include_image!("../assets/shuffle_on.png") }
            } else {
                if pressed { egui::include_image!("../assets/shuffle_off_pressed.png") } else { egui::include_image!("../assets/shuffle_off.png") }
            };
            egui::Image::new(shuffle_src).paint_at(ui, btn_rect);
            if btn_resp.clicked() {
                self.shuffle = !self.shuffle;
            }
            
            // Repeat button with repeat_off.png / repeat_on.png
            let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(REPEAT_BTN_W, REPEAT_BTN_H), Sense::click());
            let pressed = btn_resp.is_pointer_button_down_on();
            let repeat_src = if self.repeat {
                if pressed { egui::include_image!("../assets/repeat_on_pressed.png") } else { egui::include_image!("../assets/repeat_on.png") }
            } else {
                if pressed { egui::include_image!("../assets/repeat_off_pressed.png") } else { egui::include_image!("../assets/repeat_off.png") }
            };
            egui::Image::new(repeat_src).paint_at(ui, btn_rect);
            if btn_resp.clicked() {
                self.repeat = !self.repeat;
            }
        });
    }

    fn render_bottom_strip(&mut self, ui: &mut Ui) {
        ui.allocate_exact_size(Vec2::new(WIN_W, BOTTOM_STRIP_H), Sense::hover());
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb((a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8, (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8, (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8)
}

/// Horizontal gradient: dark → mid → dark (left to right)
fn paint_h_gradient(painter: &Painter, rect: Rect, dark: Color32, mid: Color32, steps: usize) {
    let n = steps.max(2);
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let color = if t < 0.5 { lerp_color(dark, mid, t * 2.0) } else { lerp_color(mid, dark, (t - 0.5) * 2.0) };
        let x0 = rect.left() + rect.width() * (i as f32 / n as f32);
        let x1 = rect.left() + rect.width() * ((i + 1) as f32 / n as f32);
        let strip = Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1, rect.bottom()));
        painter.rect_filled(strip, CornerRadius::ZERO, color);
    }
}

fn draw_slider(ui: &mut Ui, resp: &Response, value: f32, fill_color: Color32) -> Option<f32> {
    let rect = resp.rect;
    let painter = ui.painter();
    let track_rect = rect.shrink2(Vec2::new(0.0, 4.0));
    let radius = (track_rect.height() / 2.0).round() as u8;
    painter.rect_filled(track_rect, CornerRadius::same(radius), TRACK_BG);
    painter.rect_stroke(track_rect, CornerRadius::same(radius), Stroke::new(1.0, TRACK_BORDER), StrokeKind::Inside);
    painter.rect_filled(track_rect, CornerRadius::same(radius), fill_color);
    
    // Thumb position driven by value
    let thumb_x = track_rect.left() + track_rect.width() * value - 7.0;
    let thumb = Rect::from_min_size(Pos2::new(thumb_x, track_rect.top() - 3.0), Vec2::new(14.0, 14.0));
    let pressed = resp.is_pointer_button_down_on();
    draw_winamp_thumb(painter, thumb, pressed);
    
    // Grip lines
    let cx = thumb.center().x;
    let grip_top = thumb.top() + 3.0;
    let grip_bot = thumb.bottom() - 3.0;
    for dx in [-2.0, 0.0, 2.0] {
        let x = cx + dx;
        painter.line_segment([Pos2::new(x, grip_top), Pos2::new(x, grip_bot)], Stroke::new(1.0, Color32::from_rgb(100, 100, 120)));
        painter.line_segment([Pos2::new(x + 1.0, grip_top), Pos2::new(x + 1.0, grip_bot)], Stroke::new(1.0, Color32::from_rgb(255, 255, 255)));
    }
    
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let t = ((pos.x - track_rect.left()) / track_rect.width()).clamp(0.0, 1.0);
            return Some(t);
        }
    }
    None
}

// -----------------------------------------------------------------------------
// Beveled drawing primitives
// -----------------------------------------------------------------------------

fn draw_beveled_rect(painter: &Painter, rect: Rect, bg: Color32, pressed: bool) {
    painter.rect_filled(rect, CornerRadius::ZERO, bg);
    
    let light = if pressed { Color32::from_rgb(80, 80, 100) } else { Color32::WHITE };
    let dark = if pressed { Color32::WHITE } else { Color32::from_rgb(80, 80, 100) };
    let inner_dark = if pressed { Color32::from_rgb(180, 180, 200) } else { Color32::from_rgb(140, 140, 160) };
    
    // Outer bevel
    painter.line_segment([Pos2::new(rect.left(), rect.top()), Pos2::new(rect.right(), rect.top())], Stroke::new(1.0, light));
    painter.line_segment([Pos2::new(rect.left(), rect.top()), Pos2::new(rect.left(), rect.bottom())], Stroke::new(1.0, light));
    painter.line_segment([Pos2::new(rect.left(), rect.bottom()), Pos2::new(rect.right(), rect.bottom())], Stroke::new(1.0, dark));
    painter.line_segment([Pos2::new(rect.right(), rect.top()), Pos2::new(rect.right(), rect.bottom())], Stroke::new(1.0, dark));
    
    if !pressed {
        // Inner shadow for depth
        painter.line_segment([Pos2::new(rect.left() + 1.0, rect.bottom() - 1.0), Pos2::new(rect.right() - 1.0, rect.bottom() - 1.0)], Stroke::new(1.0, inner_dark));
        painter.line_segment([Pos2::new(rect.right() - 1.0, rect.top() + 1.0), Pos2::new(rect.right() - 1.0, rect.bottom() - 1.0)], Stroke::new(1.0, inner_dark));
    }
}

fn draw_winamp_thumb(painter: &Painter, rect: Rect, pressed: bool) {
    let bg = if pressed { Color32::from_rgb(170, 170, 190) } else { Color32::from_rgb(200, 200, 210) };
    draw_beveled_rect(painter, rect, bg, pressed);
    let inner = rect.shrink(2.0);
    painter.rect_filled(inner, CornerRadius::ZERO, Color32::from_rgb(220, 220, 230));
}



// -----------------------------------------------------------------------------
// Legacy helpers
// -----------------------------------------------------------------------------

fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    let m = s / 60;
    format!("{:02}:{:02}", m, s % 60)
}

// =============================================================================
// Metallic Gold decorative lines
// =============================================================================

struct MetallicGoldLine {
    color: Color32,
    width: f32,
}

struct MetallicGold {
    lines: Vec<MetallicGoldLine>,
}

impl MetallicGold {
    fn new() -> Self {
        Self {
            lines: vec![
                MetallicGoldLine { color: Color32::from_rgb(0x5b, 0x54, 0x42), width: 0.5 },
                MetallicGoldLine { color: Color32::from_rgb(0xe7, 0xcf, 0x86), width: 1.0 },
                MetallicGoldLine { color: Color32::from_rgb(0xee, 0xdd, 0xab), width: 0.5 },
                MetallicGoldLine { color: Color32::from_rgb(0xff, 0xff, 0xff), width: 2.5 },
                MetallicGoldLine { color: Color32::from_rgb(0xc6, 0xc5, 0xc4), width: 0.5 },
                MetallicGoldLine { color: Color32::from_rgb(0x45, 0x41, 0x3d), width: 1.5 },
                MetallicGoldLine { color: Color32::from_rgb(0x61, 0x5a, 0x4c), width: 0.5 },
                MetallicGoldLine { color: Color32::from_rgb(0xa1, 0x95, 0x6f), width: 1.5 },
                MetallicGoldLine { color: Color32::from_rgb(0xb6, 0xa6, 0x76), width: 0.5 },
                MetallicGoldLine { color: Color32::from_rgb(0xe7, 0xcf, 0x86), width: 1.5 },
                MetallicGoldLine { color: Color32::from_rgb(0x25, 0x26, 0x2c), width: 0.5 },
                MetallicGoldLine { color: Color32::from_rgb(0x5b, 0x54, 0x42), width: 1.5 },
            ],
        }
    }

    fn draw(&self, painter: &Painter, left_rect: Rect, right_rect: Rect) {
        let radius = CornerRadius::same(8);
        let mut y_offset = 0.0;
        for line in &self.lines {
            if y_offset + line.width > left_rect.height() {
                break;
            }

            let rect = Rect::from_min_max(
                Pos2::new(left_rect.left(), left_rect.top() + y_offset),
                Pos2::new(left_rect.right(), left_rect.top() + y_offset + line.width),
            );
            painter.rect_filled(rect, radius, line.color);

            let rect = Rect::from_min_max(
                Pos2::new(right_rect.left(), right_rect.top() + y_offset),
                Pos2::new(right_rect.right(), right_rect.top() + y_offset + line.width),
            );
            painter.rect_filled(rect, radius, line.color);

            y_offset += line.width;
        }
    }
}

// =============================================================================
// Main
// =============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    set_band_count(N_SPECTRUM_BARS);
    let engine = Arc::new(Mutex::new(PlaybackEngine::new()?));
    let app = WinampTestApp::new(engine);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([WIN_W, WIN_H]).with_min_inner_size([WIN_W, WIN_H]).with_max_inner_size([WIN_W, WIN_H]).with_decorations(false).with_transparent(false),
        ..Default::default()
    };
    eframe::run_native("Winamp 2.x Classic", options, Box::new(|cc| {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "04b03".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../assets/04B_03__.TTF"))),
        );
        fonts.families.insert(
            egui::FontFamily::Name("04b03".into()),
            vec!["04b03".to_owned()],
        );
        cc.egui_ctx.set_fonts(fonts);
        Ok(Box::new(app))
    }))?;
    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    const CORRECT: [[bool; 7]; 10] = [
        [true, true, true, false, true, true, true],
        [false, false, true, false, false, true, false],
        [true, false, true, true, true, false, true],
        [true, false, true, true, false, true, true],
        [false, true, true, true, false, true, false],
        [true, true, false, true, false, true, true],
        [true, true, false, true, true, true, true],
        [true, false, true, false, false, true, false],
        [true, true, true, true, true, true, true],
        [true, true, true, true, false, true, true],
    ];
    #[test]
    fn digit_patterns_correct() {
        for digit in 0..=9 {
            let d = SevenSegDigit::new(digit, 0.0, 0.0);
            assert_eq!(d.segments, CORRECT[digit as usize]);
        }
    }
}