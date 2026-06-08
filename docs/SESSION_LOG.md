# Session Log — 2026-06-07

## Session 2 — winamp_ui example fix

### Done
- Added `eframe` 0.27 and `egui` 0.27 to `[dev-dependencies]` in `rust/Cargo.toml`
- Added `[[example]]` entry for `winamp_ui` in `rust/Cargo.toml`
- Added `fn main()` entry point with `eframe::run_native` and framed window options
- Restructured edit dialog closure to avoid E0500 (borrow of `self` inside closure capturing `buf`)
- `drag_released()` → `drag_stopped()` (renamed in egui 0.27)
- Removed `.desired_width(50.0)` on `Slider` (not available in egui 0.27)
- Removed unused imports (`Align`, `FontData`, `FontDefinitions`, `Id`, `Layout`)
- Removed unused `nudge` closure and `PANEL_MID`/`TEXT_HI` constants

### Verification
- `cargo check -p tunes4r --example winamp_ui` — 0 errors, 0 warnings

## Session 3 — eframe/egui version bump to latest, volume + seek fixes

### Done
- Bumped `eframe 0.27 → 0.34` and `egui 0.27 → 0.34` in `rust/Cargo.toml`
- `App::update(ctx, frame)` → `App::ui(ui, frame)` (0.34 changed the outer parameter from `&Context` to `&mut Ui`)
- Replaced all `ctx.*` calls with `ui.*` (`Ui: Deref<Target = Context>`)
- `CentralPanel::show()` → `show_inside()` (deprecated in 0.34)
- `Frame::none()` → `Frame::NONE`, `Rounding` → `CornerRadius`
- `Button::rounding` → `corner_radius`
- Added 4th `StrokeKind` arg to `painter.rect_stroke()`
- `Margin::same(8.0)` → `Margin::same(8)` (now takes `i8`)
- `run_native` closure returns `Ok(Box::new(app))` (creator now returns `Result`)

### Impact
- Fixed the `icrate` runtime crash on macOS (the root cause was the old `icrate 0.0.4` being incompatible with newer macOS SDKs)

### Subsequent fix — volume overflow + seek bar
- Constrained volume slider to 40px via `ui.add_sized` to prevent overflow beyond 275px window
- Wired up `engine.set_volume()` (was a commented-out placeholder)
- Added `display_info()` / `playing_info()` helpers — seek bar now shows the **actively playing** section (matching info panel) instead of the **selected** section
- Keyboard scrub auto-enter now uses `display_info()` position

### Seek fix (mouse release) + file browser
- **Seek on mouse release fix**: `interact_pointer_pos()` returns `None` on `drag_stopped()` frame, so the seek was never committed. Split `dragged`/`drag_stopped`/`clicked` into three separate `if` blocks. `drag_stopped()` reads from `self.scrub.position_ms` (stored during drag); `clicked()` calculates from `interact_pointer_pos()`.
- **File browser**: Added `show_browser` toggle + `browser_path` field. Triangle button `▾` in transport bar + `B` key toggles it. Renders as docked panel below the player, same width. Shows filesystem navigation: `[..]` for parent, directories listed first with `▶` prefix, then files. Clicking a file plays it via `start_play()`.
- Clean compile, 0 warnings.

### Native file picker via rfd + transport bar tightening
- Replaced in-app browser with system-native file picker via `rfd = 0.15.4` (dev-dep).
- Removed: `show_browser`, `browser_path` fields, `render_browser()` method, `B` key shortcut, "browse" keybind entry, browser block in `App::ui()`.
- Added: `⏏` (eject symbol) button at the end of the transport bar (after ⏭, before VOL). Calls `rfd::FileDialog::new().add_filter("Audio", &["mp3","wav","flac","ogg","m4a","aac","opus"]).add_filter("All files", &["*"]).pick_file()` then `start_play(path, Section::File)`.
- Reduced transport bar gaps: top spacer 4→2, leading space 4→2, gaps around VOL 8→4.
- Cargo build clean, 0 warnings, 0 errors.

### LCD display + spectrum analyzer rewrite
- **Replaced** `render_info_panel` (VU meters + text) with a full Winamp-style LCD panel (140px tall) containing a sidebar + 7-segment timer + spectrum analyzer.
- **LCD palette added**: `LCD_BG #0d1a0d`, `LCD_DOT #152515`, `LCD_BORDER #2a4a2a`, `LCD_SEG_ON #39ff14`, `LCD_SEG_OFF #1a2e1a`, `STATE_RED #cc3300`, plus 5 spectrum zone colors, `PEAK_WHITE`, `RULE_A/B` for the dotted boundary lines.
- **Sidebar** (left 32px column): state square (9×9, rx=1, #cc3300, only when stopped) + play triangle (20×22, right-pointing #39ff14) at top, then 6 stacked monospace letters (CORTOC when playing, OAITDV when idle).
- **7-segment timer** (top-right, 50px tall): 4 digits drawn from a 7-segment lookup table using rounded 22×5 horizontal and 5×18 vertical bars. MM:SS elapsed when playing, -MM:SS remaining when idle. Colon is two 4-radius circles. Minus sign (12×5) appears only in idle mode.
- **Spectrum analyzer** (bottom-right, ~80px tall): `N_SPECTRUM_BARS = 20` bars, 9px wide, 2px gap. Each bar split into 5 color zones (low green → yellow-green → amber → orange → red) painted bottom-up to amplitude. White peak-hold markers above each bar.
- **Dotted boundary rules** (always visible): left + bottom edges of the spectrum zone, alternating `#00aaaa` / `#008888` dots, 3px radius, 7px pitch.
- **State change**: replaced `vu: [f32; 2]` with `spectrum: [f32; N_SPECTRUM_BARS]` and `spectrum_peaks: [f32; N_SPECTRUM_BARS]`. `poll_engine` now animates them: sine wave + hash noise, EMA smoothing (`amp = amp*0.35 + target*0.65`), peaks rise immediately, fall at 0.006/frame.
- **Window size**: `275x320 → 275x420` (min 380) to fit the taller LCD.
- **Removed dead code**: `render_vu_meters` method, `state_label_color` helper.
- Clean compile, 0 warnings, 0 errors.

### LCD layout refinement: two state squares + right-aligned timer
- **Two state squares** (was: one square shown only when stopped). Now always visible, stacked in a column above the timer:
  - Square 1 (top) = "ON" / ready — lit green (#39ff14) when state is Playing/Paused/Connecting/Buffering/Decoding, dim otherwise
  - Square 2 (bottom) = "OFF" / stopped — lit red (#cc3300) when state is Stopped, dim otherwise
- **Play triangle** moved out of the sidebar into its own column to the right of the squares. Its right edge sits 4px to the left of the spectrum left rule (visually "aligned" with the rule).
- **Sidebar** now contains only the 6 stacked letters (CORTOC when playing, OAITDV when idle) — full height, vertically centered.
- **Timer** is now right-aligned to the inner panel's right edge. `draw_timer` signature changed from `(origin: Pos2)` to `(right_edge: f32, top: f32)` and computes the text's own width (digits 22px + 3px gap, colon 8px + 6px gap, minus 12px + 3px gap) to position the start.
- `is_off` and `is_ready` booleans derived from `info.state` to drive the square colors.
- Clean compile, 0 warnings, 0 errors.

### Seek bugfix + console window
- **Seek bug**: `drag_stopped()` fires when `dragged()` is already `false`, so outer `if resp.dragged() || resp.clicked()` never entered on release → added `|| resp.drag_stopped()` to the condition
- **Console window**: Added togglable (`C` key or console button) bottom panel within the same window, same width, following winamp palette. Logs play/pause/stop/seek actions.
- Fixed E0502 borrow conflict from `self.push_log()` inside `self.engine.lock()` scope

## Completed

### Dart `audio_engine.dart`
- Replaced 26x `_ensureAlive()` + `_handle!` pattern with `_h` getter (single chokepoint for disposed checks)
- Removed `_ensureAlive()` method entirely
- Removed `lastError` getter (Rust function was an alias for `loadError`)
- Simplified `youtubeGetStreamUrl` — no longer creates/destroys `YoutubeServiceHandle` per call

### Dart `tunes4r_player_ffi.dart`
- Removed 11 dead FFI bindings: `playStreamFromBytes`, `fetchAndPipe`, `setStreamError`, `playStreamWithDownloader`, `getPipeSeekOffset`, `pollPipeSeekByteOffset`, `clearPipeSeekRequest`, `setPipeTotalBytes`, `youtubeSearch`, `youtubeGetVideoInfo`, `youtubeDownloadAudio`
- Removed `youtubeServiceCreate` / `youtubeServiceDestroy` (handle was unused by the underlying FFI)
- Removed `getLastError` (Rust function was an alias for `getLoadError`)
- Updated `youtubeGetStreamUrl` signature (no longer takes a handle)

### Rust `ffi.rs`
- Removed dead `SpectrumAnalyzer` field from `AudioEngineHandle` (field was written but never read)
- Removed `audio_engine_analyze_spectrum` / `spectrum_data_free` (used the dead field)
- Removed `audio_engine_get_last_error` (alias for `get_load_error`)
- Removed `audio_engine_skip_forward` / `audio_engine_skip_backward` (unused)
- Removed `audio_engine_get_buffered_bytes` / `audio_engine_get_total_bytes` (unused)
- Removed `audio_engine_get_pipe_url_for_seek` (unused)
- Removed `audio_engine_play_stream_with_downloader` (unused)
- Removed `YoutubeServiceHandle`, `youtube_service_create`, `youtube_service_destroy` (handle was unused)
- Removed `youtube_search`, `youtube_get_video_info`, `youtube_download_audio` (unused)
- Simplified `youtube_get_stream_url` — no longer takes an ignored handle parameter

### Rust `lib.rs`
- Added re-exports: `EngineEvent`, `ENGINE_EVENT_NONE`, `ENGINE_EVENT_STATE_CHANGED`, `ENGINE_EVENT_SEEK_STARTED`, `ENGINE_EVENT_SEEK_COMPLETED`, `ENGINE_EVENT_END_OF_STREAM`, `ENGINE_EVENT_POSITION_RESET`, `ENGINE_EVENT_ERROR`, `ENGINE_EVENT_SEEK_QUEUED`

### Rust `commands.rs` — Seek Fix
- **Bug**: `seek()` never emitted `ENGINE_EVENT_SEEK_COMPLETED` — the Dart side saw `SEEK_STARTED` but never `SEEK_COMPLETED`, leaving the UI slider stuck.
- **Fix**: Added `push_seek_completed(clamped_position)` in all 4 seek paths: Stream (after `source.open` + decode thread spawn), File (after prebuffer wait), Pipe (after decode thread spawn), Live (after decode thread spawn).
- **Stream seek fix**: Added `set_state(Connecting)` before the Range-request reconnect, then `set_state(Buffering { .. })` after the new decode thread spawns — previously the state stayed in Playing throughout the seek, giving no visual feedback.

### Rust `tests/seek_streaming.rs` — New test file
- `file_seek_emits_started_and_completed_events` — validates SEEK_STARTED → SEEK_COMPLETED event lifecycle and ordering for file seeks
- `live_seek_within_buffer_emits_both_events` — validates STARTED + COMPLETED for live seek within buffered region
- `live_seek_beyond_buffer_clamps_event_param` — validates that live seek beyond buffer clamps the target and carries the clamped value in event params

## Session 7 — Code review implementation (2026-06-08)

### Done
- **Dart unit tests** (`test/models_test.dart`): 32 tests covering `PlaybackState.fromValue`, `EngineConfig`, `AdaptiveRingBuffer` (all branches of `availableMs`, `contains`, `endMs`, `endMsClamped`, `isFullyBuffered`, `toString`), `Tunes4rErrorCode` extension, and `Tunes4rEngineException`
- **Extracted `_EnginePoller`** from `AudioEngine` (audio_engine.dart): Dedicated class owns all 5 StreamControllers + 4 Timers; `AudioEngine` delegates polling via `_poller.start(handle)` / `_poller.stop()` / `_poller.dispose()`
- **Named polling constants**: `_spectrumPollIntervalMs (100)`, `_positionPollIntervalMs (16)`, `_eventPollIntervalMs (16)`, `_bufferPollIntervalMs (200)` — top-level consts in audio_engine.dart
- **Removed `flutter_rust_bridge`** dependency (Cargo.toml + lib.rs annotations + `init_app`/`get_next_free_id`/classifier FRB wrappers). Kept helper functions as regular Rust API. Cleaned up unused re-exports.
- **Deprecated `playStream`**: `@Deprecated('Use play() instead')` on the method
- **Removed `cacheDir` parameter** from `playYoutube()` Dart API (Rust FFI keeps the parameter for ABI stability)
- **Fixed example `lastError` compile error**: replaced `_engine?.loadError ?? _engine?.lastError ?? ''` with `_engine?.loadError ?? ''`
- **Extracted `formatMs`**: deduplicated from `_Tunes4rPlayerExampleAppState` and `_BufferedSliderState` into a top-level function in `example/lib/main.dart`
- **Added `Tunes4rErrorCode` extension** on `int` in `models.dart`: constants (`ffiSuccess`, `ffiNullHandleOrUri`, `ffiInvalidUtf8`, `ffiEngineLockError`, `ffiPlaybackError`, `ffiInternalPanic`) + `isFfiError` + `ffiErrorMessage` getters
- **Deprecated global `tunes4rFFI`**: `@Deprecated('Prefer dependency injection via AudioEngine.create(ffi:)')` 
- **`Tunes4rEngineException`** now carries optional `errorCode`
- **Fixed typo**: `Aduio` → `Audio` in ffi.dart comment
- **Fixed duplicate `#[cfg(test)]`** in ffi.rs
- **Removed broken examples**: `winamp_ui.rs` (referenced missing `winamp_shared.rs`) and `winamp_tui copy.rs` (space in filename)

### Verification
- `flutter analyze lib/` — 0 issues
- `flutter test` — 32/32 pass
- `cargo check --workspace --lib --examples --tests` — 0 errors
- `cargo test --test ffi_contract` — 9/9 pass
- `cargo test -p tunes4r-core --lib` — 111/111 pass

## Findings (Code Review — 2026-06-07)

### Dead Code
- `PlaybackContext` struct + impl in `context.rs` (183 lines) — created for Arc-refactor; never used anywhere
- `play_stream_with_downloader` in `commands.rs` — only called from its own test; FFI binding already removed

### Functional Bugs
- `set_volume` / `get_volume` are no-ops — volume clamps to [0,1] in Dart but Rust `commands.rs:1054` only logs; never applied to cpal output

### Code Quality
- `catch_unwind(AssertUnwindSafe(|| ...))` appears in 50+ FFI functions — candidate for macro
- `commands.rs:1101-1110` clones 10 Arc fields individually per thread spawn — `PlaybackContext` was created for this but never integrated
- `handling.rs` (1410 lines) is a monolithic `#[cfg(not(target_os = "android"))]` block — hard to maintain

### Edge Cases
- `state.rs:77` — `current_ms = (raw_samples * 1000) / (rate * ch)` — `raw_samples * 1000` can overflow u64 on very long files
- `http.rs:30` — `rx.recv().expect(...)` panics with poor message if async task silently dies

## Key Decisions (this session)
- `_h` getter replaces `_ensureAlive()` as single chokepoint
- `YoutubeServiceHandle` was a wrapper that added no value (handle never used by FFI) — removed entirely
- `SpectrumAnalyzer` field in AudioEngineHandle was dead (written by never-read Rust code; `getSpectrum` reads from GLOBAL_SPECTRUM)

## Verification
- `cargo check --workspace --lib --examples --tests` — 0 warnings
- `cargo test --test ffi_contract` — 9/9 pass
- `cargo test -p tunes4r-core --lib` — 89/89 pass
- `flutter analyze lib/` — 0 issues

## Session 4 — winamptest_ui example (505px Winamp clone)

### Done
- Created `rust/examples/winamptest_ui.rs` — full Winamp Classic clone, initially 540px, later 505×215
- Color palette from spec, 32-bar spectrum analyzer with 5-zone color rendering + peak hold markers
- 7-segment LCD timer (MM:SS / -MM:SS), scrolling title marquee, dotted boundary rules
- Custom frameless title bar, transport buttons (⏮ ▶ ⏸ ⏹ ⏭ ⏏), shuffle/repeat toggles with green LEDs
- Volume slider with RGB-lerp color, balance slider (always green), seek bar with gold fill
- Spectrum uses frequency curve formula + LCG pseudo-random (no external rand dep)
- Keyboard: Space (pause/resume), S (stop), arrows (scrub), Enter (seek), Escape (cancel)

### Session 5 — 7-segment bug, thinner segments, H-gradient background, slider fix
- **7-segment bug fix**: segment pattern arrays reordered to match polys draw order `[a, f, b, g, e, c, d]`; unit tests added
- **Window**: 540×220 → 505×215
- **Gradient**: Full-window horizontal gradient `#13121c → #363654 → #13121c` — all panel fills removed
- **Segments**: Thinner (`seg_h=3`, `vert_w=2`)
- **Sliders**: `draw_slider` takes `&Response` directly — no `ui.interact` call, eliminates ID clash error
- **Logo**: Moved from title bar to bottom-right corner of main window
- **Logo replaced with PNG**: `logo-rustamp.png` (34×34) via `egui::include_image!`

### Session 6 — Ghost window fix + draw_slider groove
- **Ghost window drag**: Replaced manual `ViewportCommand::OuterPosition` software drag with `ViewportCommand::StartDrag` — uses native OS window dragging (smooth, no ghost outline)
- **Removed unused** `window_pos` field from `WinampTestApp`
- **Updated** `draw_slider` with authentic Winamp VOL/BAL groove: vertical gradient (darker top, vibrant mid, darker bottom), top shadow + dark line, bottom highlight, pixel-rounded ends

### Verification
- `cargo check --example winamp_ui --example winamptest_ui` — 0 errors, 0 warnings

---

## Session 3 — Stream decorator code review fixes

### Goal
Fix the three remaining issues from the code review of the stream decorator module.

### Done
1. **Seek lost in CacheDecorator cached path**: Created `ReadSeek: Read + Seek` trait in `source/mod.rs` with blanket impl. Changed `StreamSource::open()` return type from `Box<dyn Read + Send + Sync + 'static>` to `Box<dyn ReadSeek + Send + Sync + 'static>`. Updated all 9+ implementations and downstream type annotations (~27 locations across 15 files). Added `NonSeekable<R>` wrapper for HTTP/live-stream sources that cannot seek.
2. **Aspirational doc comment**: Updated `CachedReader` doc comment to reflect current design accurately.
3. **Race condition on filler thread stop**: Replaced `stop_bg: Arc<AtomicBool>` with `bg_gen: Arc<AtomicU64>` generation counter. `stop_background()` increments the generation; the filler thread checks `gen != bg_gen.load()` each iteration, eliminating the race where a new thread clears the shared stop flag before the old thread sees it.

### Verification
- `cargo check --workspace --lib --tests` — 0 errors, 0 warnings
- `cargo test -p tunes4r-core --lib` — 111/111 pass
- `cargo test -p tunes4r --test seek_streaming` — 10/10 pass

## Session 8 — Code review follow-up (2026-06-08)

### Done
- **Removed `cacheDir` parameter entirely** from Rust `audio_engine_play_youtube` FFI function + Dart FFI binding + AudioEngine.playYoutube()
- **Added `EngineEventType` enum** (`models.dart`): typed replacement for raw `engineEvent*` int constants. `EngineEvent.eventType` is now `EngineEventType`. `_EnginePoller` and example converted to use it.
- **Documented `_EnginePoller.start()`**: explains why stale handles are silently ignored
- **Documented `AudioEngine.dispose()`**: explains intentional order (pollers first, engine last)
- **Documented `lib.rs` helper functions**: marked as Rust-native convenience API for examples/tests, not used by FFI
- **Fixed `audio_http_fetch.rs`**: `info!` → `error!` for fetch errors
- **Fixed `ffi_contract.rs`**: `0u8 as c_char` → `0i8 as c_char`
- **Added `EngineEventType` tests** (6) + `EngineEvent` test (1): 36 tests total

### Verification
- `flutter analyze lib/ example/lib/` — 0 issues
- `flutter test` — 36/36 pass
- `cargo check --workspace --lib --examples --tests` — 0 errors
- `cargo test --test ffi_contract` — 9/9 pass
- `cargo test -p tunes4r-core --lib` — 111/111 pass
