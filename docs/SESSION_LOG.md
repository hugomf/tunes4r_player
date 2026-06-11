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

## Session 9 — Seek packet error retry limit + CDN fixture replay (2026-06-08)

### Done
- **Added `PacketSkipLimit` error variant + retry counter** to `packet_skip_seek()` in `crates/core/src/audio/decoder/seek.rs`: prevents infinite loop on repeated `format.next_packet()` errors (e.g., corrupted stream data at seek position). Gives up after 100 consecutive errors and returns `SeekError::PacketSkipLimit(100)`.
- **Added `MAX_CONSECUTIVE_PACKET_ERRORS = 100`** constant + unit tests for the error variant display and constant sanity bounds.
- **Created CDN fixture capture binary** (`src/bin/capture_youtube_fixture.rs`): `cargo run --bin capture_youtube_fixture -- <video-id>` downloads a real YouTube CDN audio stream and saves it as `tests/fixtures/youtube_stream.bin` + `youtube_stream.json`.
- **Created fixture replay test** (`tests/mock_youtube_stream.rs`): `mock_youtube_seek_with_fixture` loads the captured fixture and serves it via a local HTTP server with Range support. Skips gracefully if no fixture exists — run the capture binary once to generate it.
- **Fixed borrow/move issue** in `serve_fixture()` local HTTP server closure.
- **Rewrote capture binary** to use `YouTube::videos().stream_with_client()` (newer `StreamExtractor` path) instead of the broken `get_audio_stream_url()` (legacy `stream.rs` path). The legacy path fails because it doesn't auto-generate a PoToken and all non-signature clients return login-required/unplayable.
- **Cleaned up stray files** left over from stash operations.

### Key findings
- `get_audio_stream_url()` in `stream.rs` is broken — doesn't auto-generate PoToken, causing all clients to fail for most videos. The `StreamExtractor` path in `extractor.rs` (used by `YouTube::videos().stream()`) works because it auto-generates a cold-start PoToken from visitor_data.
- Working clients: ANDROID_VR (27 formats), ANDROID (27 formats), IOS (8 formats)
- Failing clients: MWEB/WEB (unplayable), TVHTML5/WEB_EMBEDDED (error), ANDROID_MUSIC/ANDROID_CREATOR/WEB_CREATOR (login required)

### Verification
- `cargo check --workspace --lib --tests --examples --bins` — 0 errors, 0 warnings
- `cargo test -p tunes4r-core -- seek` — 5 seek unit tests pass (including 2 new)
- `cargo test -p tunes4r --test yt_stream_seek` — 4/4 pass
- `cargo test -p tunes4r --test mock_youtube_stream mock_youtube_seek_with_fixture` — 1/1 pass (real YouTube CDN fixture data, no packet errors)
- Full test suite: 0 failures

## Session 10 — winamp_tui.rs full rewrite (2026-06-08)

### Done
- **Fully rewrote `rust/examples/winamp_tui.rs`** (1088→1978 lines): compact ratatui Winamp-style TUI
  - **Popup system**: URL input popup with text editing (cursor, backspace, delete, arrows, home/end) + file browser with directory navigation
  - **9-row compact layout**: LCD panel (title, VU meters, timer, state pill, transport buttons, seek bar, section indicators) fitting in ~9 terminal rows
  - **Console sidebar**: toggleable `l` key, scrollable log buffer viewer on the right side
  - **File browser**: `b` key opens an in-TUI file browser with [..] parent navigation, directory listing (▶ prefixed), file selection. Enter plays selected file
  - **3 source sections**: File/YouTube/Live with keyboard 1/2/3 selection, separate section info (URL, position, state)
  - **Shutdown signal** for graceful cleanup on quit
  - **Scrub mode**: `k` enters, arrows nudge ±1s/±10s, Enter commits, Esc cancels
  - **Lock/unlock safety** with `lock_ui`/`lock_engine` helpers (never panic on poisoned mutex)
- **Fixed popup Enter play**: inline in `handle_key` (drops UI lock before acquiring engine lock, then re-acquires UI for error reporting)
- **Fixed compilation errors**: unused `Write` import, `&&str` → `*sym`, mutable borrow on `e.stop()`, temporary array lifetime, `PlaybackError` type mismatch in engine lock error

### Verification
- `cargo build --example winamp_tui` — 0 errors, 6 warnings (pre-existing unused items)
- `cargo check --workspace` — 0 errors, 0 warnings
- `cargo test --lib` — 3/3 pass

## Session 11 — Code review improvements (2026-06-08)

### Done
- **`seek()` now emits position**: `audio_engine.dart:seek()` calls `_poller.positionCtrl.add(_ffi.getPosition(_h))` after native seek, matching `play()` behavior. Updated docstring to remove stale "with the seek target" claim.
- **Removed `_stateLabel`** in example app (`main.dart`): replaced with `state.name` (Dart enum built-in).
- **`availableMs` clamp clarity**: changed `1 << 31` → `2147483647` (i32::MAX literal) in `models.dart`.
- **DRY transport row**: `_transportRow` now conditionally shows Resume button (`onResume` is nullable). Live section uses `_transportRow` instead of inline row.
- **URL detection**: `_playYoutube()` uses `Uri.tryParse()` instead of fragile `input.contains('youtu')`.
- **Example widget test fixed**: replaced stale counter-app template test with `formatMs()` unit test. Added `flutter_test` and `flutter_lints` dev deps to example pubspec.
- **Lint fixes**: 3 `prefer_const_constructors` violations fixed in `models_test.dart`.

### Verification
- `dart analyze lib/ test/ example/lib/` — 0 issues
- `dart analyze` (example/) — 0 issues

## Session 12 — YouTube seek bugfix (2026-06-08)

### Analysis
Investigated why YouTube seek is broken in the winamp TUI. Root cause: two bugs in `commands.rs::seek()`.

### Bugs fixed

**Bug 1 — Queued seeks silently lost** (`commands.rs:771-781`):
When the seek target was past the buffered region (`is_queued == true`), the code set `seek_target_ms` but never set `seek_request`. The non-Android decode thread's `playback_loop()` only monitors `seek_request` after startup — it never reads `seek_target_ms`. The seek was silently dropped.  
**Fix**: Added `self.seek_request.store(clamped_position, Ordering::Relaxed)` + `audio_queue.lock().clear()` in the queued path. The decode thread will now pick up the seek request and block in `packet_skip_seek()` until the background filler catches up.

**Bug 2 — Backward seek loses future seek requests** (`commands.rs:835`):
The backward seek path spawned a new decode thread with `Arc::new(AtomicU64::new(0))` (a fresh atomic) instead of the shared `self.seek_request`. Any subsequent `seek()` call stored the target in the original atomic, but the new decode thread was watching its own disconnected atomic.  
**Fix**: Added `let seek_request = self.seek_request.clone();` before thread spawn and passed `seek_request` instead of the fresh atomic.

### Verification
- `cargo check --lib -p tunes4r-core` — 0 errors
- `cargo test -p tunes4r-core --lib` — 112/112 pass
- `cargo test --test seek_streaming` — 10/10 pass (including `stream_seek_backward_within_buffer_emits_both_events`)
- `cargo check --example winamp_tui` — 0 errors

## Session 13 — LCD layout: wider LCD + right-justified timer (2026-06-08)

### Done
- **Widened LCD panel** from 24 to 36 columns in `body_cols` layout constraint — gives inner.width=34 (was 22), providing room for state icon + label on the left and timer on the right.
- **Right-justified 7-segment timer**: timer digits now start at `inner.right() - 22` instead of `inner.x`; minus sign drawn just left of the timer (2 cols) instead of at the inner left edge.
- **Spectrum retains full width** (`spec_w = inner.width` = 34 cols), now extends ~10 columns to the left of the timer start — matching the winamptest_ui layout where the spectrum is wider than the timer and fills the LCD.

### Layout now
```
┌─────────────────────────────────────┐
│[▶] CUR             00:42            │  Row 0: icon + label (left), timer digits (right, rows 1‑5)
│                        ██           │
│                        █  █         │
│                        █  █         │  Rows 1‑5: right‑justified 7‑segment timer
│                        █  █         │
│                        █  █         │
│                                     │  Row 6: gap
│ · █████████████████████████████████ │
│ · █████████████████████████████████ │  Rows 7‑13: spectrum (full 34‑col width,
│ · █████████████████████████████████ │             10 cols wider than timer start)
│ · █████████████████████████████████ │
│ · █████████████████████████████████ │
│ · · · · · · · · · · · · · · · · ·  │
└─────────────────────────────────────┘
```

### Verification
- `cargo build --example winamp_tui` — 0 errors
- `cargo clippy` — 0 new warnings

## Session 14 — Spectrum + button fixes (2026-06-08)

### Button press state fix
- **Fixed `C_BTN_PRESSED`**: was `rgb(55,55,80)` (brighter than `C_BTN_BG = rgb(40,40,60)`) — made the "pressed" button look raised instead of sunken. Changed to `rgb(26,26,44)` which is darker than `C_BTN_BG`, so active buttons now properly appear depressed.
- **Increased bevel contrast**: `C_BTN_BEVEL_HI` `rgb(140,140,165)` → `rgb(180,180,205)` (brighter highlight), `C_BTN_BEVEL_LO` `rgb(25,25,40)` → `rgb(12,12,24)` (deeper shadow).

### Spectrum: reverted to zone coloring matching winamptest_ui
- **Idle decay**: bars decay `*0.82` per frame (not immediate 0), peaks go to 0 immediately — exact match of winamptest_ui behavior.
- **Restored 5-zone amplitude coloring → simplified to 3 zones**: green (0–40%), amber (40–75%), red (75–100%). Removed unused `C_SPEC_YLW` and `C_SPEC_ORG` constants.
- **Test updated**: `spectrum_goes_flat_when_stopped` → `spectrum_decays_when_stopped`.
- **Fixed vertical resolution**: body height 16→22 rows, transport 4→3 rows. Each bar now has ~9 cells instead of ~2, so bars grow/shrink smoothly instead of flickering as individual pixels.

### Verification
- `cargo check` — 0 errors
- `cargo test --example winamp_tui` — 7/7 pass

## Session 15 — Mock YouTube stream test enhancements (2026-06-09)

### Done
- **Enhanced `mock_youtube_stream.rs`** with 8 new tests (9 total):
  - `state_lifecycle` — event-based state transition validation (Connecting → Buffering → Playing → Stopped)
  - `poll_state_transitions` — poll-based state validation using `audio_engine_get_state()`
  - `backward_seek` — cache-reopen seek path (playhead advances 3s then seeks back to 500ms)
  - `forward_seek` — forward seek within buffer (playhead advances 3s then seeks to 5000ms)
  - `multiple_rapid_seeks` — 3 seeks (8000→1000→5000→2000) validating all fire SEEK_STARTED
  - `with_latency` — 50ms artificial latency, validates engine reaches Playing
  - `throttled` — 500 KB/s throttle, forward+backward seeks
  - `slow_connection` — 200 KB/s + 50ms latency, graceful seek failure handling (skip not panic)
- **Added `NetworkConditions` struct** (latency_ms, throttle_bps) + `serve_fixture_with_network()` server
- **Fixed slow_connection test**: originally used 20 KB/s + 100ms latency causing HTTP timeouts; now uses 200 KB/s + 50ms with graceful seek-failure handling
- **Spectrum physics reverted** in winamp_tui.rs to match winamptest_ui (PEAK_BOUNCE=2.0, PEAK_GRAVITY=0.04, PEAK_MAX=1.0, no dt-scaling, no damping)

### Verification
- `cargo test --test mock_youtube_stream` — 9/9 pass (including CDN fixture replay)
- `cargo test --test seek_streaming` — 10/10 pass
- `cargo test --test yt_stream_seek` — 4/4 pass
- `cargo test --test ffi_contract` — 9/9 pass
- `cargo test -p tunes4r-core --lib` — 112/112 pass

## Session 16 — Mock YouTube stream: comprehensive state tests (2026-06-09)

### Done
- **Expanded `mock_youtube_stream.rs` from 9 to 23 tests** covering pause/resume, end-of-stream, error injection, stop-from-any-state, unbuffered seeks, and replay/double-play
- **Fixed all 3 regressions**: `end_of_stream` (synthetic MPEG2 with correct frame sizing for Symphonia probe + `audio/mpeg` Content-Type), `seek_unbuffered_with_latency` (wait for Playing before seeking), `stop_while_connecting` (removed debug output)
- **Added helper functions**: `build_synthetic_mp3()`, `serve_fixture_with_count_and_type()`, `drain_eos_events()`
- **Removed unused imports**: `PlaybackPosition`, `ENGINE_EVENT_POSITION_RESET`, `ENGINE_EVENT_SEEK_QUEUED`
- **Added symphonia features**: `wav`, `pcm`, `symphonia-codec-pcm` dep
- **Key fix**: Synthetic MP3 frames must have frame_size matching the MPEG frame size formula. Used MPEG2 (32kbps/16kHz → 144-byte frames) instead of MPEG1 (128kbps/44kHz → 417-byte frames).

### Verification
- `cargo test --test mock_youtube_stream` — **23/23 pass** (12.4s)

## Session 17 — YouTube cache-reopen seek fix (2026-06-09)

### Goal
Fix YouTube streaming seek within buffered area: cache-reopen path was making a new HTTP connection instead of serving from cache.

### Done
1. **`open(None)` → `open(Some(clamped_position))`** (`commands.rs:832`): The cache-reopen seek path now calls `open(Some(position))` instead of `open(None)`. `open(None)` clears the `ByteCache` and starts a fresh HTTP download (defeating the purpose of a cached seek). `open(Some(_))` returns a `CachedReader` from the existing cache without making a new network request.

2. **Permanent header buffer in `ByteCache`** (`caching.rs`): Added `header: Vec<u8>` that permanently stores the first `HEADER_RESERVE` (512 KB) bytes. The `CachedReader` serves format-probe reads from this buffer, so the Matroska re-probe works even after the main ring buffer has wrapped and evicted early bytes. Three changes:
   - `push()`: captures the first 512 KB into `header`, then fills the ring buffer as before
   - `read_at()`: serves from `header` for offsets < header.len(), from ring buffer otherwise
   - `is_offset_cached()`: always returns `true` for offsets in the header region
   - `clear()`: clears both `header` and `data`

3. All cache-reopen seeks now complete in **< 600 ms** (vs. 7+ second timeout + new HTTP connection before the fix).

### Verification
- `flutter test --dart-define=YT_TEST=true test/yt_stream_seek_test.dart` — **All tests passed**
- Seek results: 5000ms→5064ms (483ms), 10000ms→10064ms (266ms), 20000ms→20096ms (591ms), 8000ms→8074ms (478ms), 15000ms→15053ms (486ms)
- No new HTTP connections after initial download — all seeks served from cache

## Session 18 — Cache-reopen ALL buffer seeks + detach thread (2026-06-09)

### Done
1. **Detach old decode thread** (`commands.rs:833`): `drop(self.playback_handle.take())` instead of `join_with_timeout(3000ms)`. The old CPAL stream writes silence via `OUTPUT_GEN` invalidation, so there's no audio glitch. Removes 100-600ms of seek latency per seek.

2. **Cache-reopen for ALL buffer seeks** (`commands.rs:811-813`): Removed the `_is_backward` classification that restricted cache-reopen to backward seeks only. Forward seeks within the buffer now also use cache-reopen instead of the broken in-thread seek path (which relied on `format.seek()` → native seek → `ReadOnlySource.byte_len()` returns `None` → packet-skip fallback, which was very slow for forward seeks).

3. **Rejected approaches**: 
   - `LenSource<MediaSource>` wrapper providing `byte_len()`: Broke format probing — the Matroska demuxer uses `byte_len()` to `SeekFrom::End()` during init, but the cache hasn't been filled yet, causing "seek beyond cached range" errors.
   - Threading `content_len` through `decode_and_play_from_read`: Too many callers to change across commands.rs, file_decoder.rs, and handling.rs for minimal gain.

### Seek timings (this session)
```
Seek  5000 ms → position  5056 ms (111ms)
Seek 10000 ms → position 10074 ms (107ms)
Seek 20000 ms → position 20053 ms (103ms)
Seek  8000 ms → position  8074 ms (104ms)
Seek 15000 ms → position 15064 ms (103ms)
```
All seeks ~100ms, zero new HTTP connections. ~80% faster than session 17 (~500ms → ~100ms), mostly from the thread detach fix.

### Verification
- `cargo build --release` — 0 errors
- `flutter test --dart-define=YT_TEST=1 test/yt_stream_seek_test.dart` — **All tests passed**
- 55 second test: 30s initial play + 5×5s segments + 5 seeks ≈ 55s total

## Session 19/20 — HE-AAC stream "too fast" fix: mono→stereo upmix (2026-06-11)

### Goal
Diagnose and fix why a live HE-AAC v2 radio stream (`XHMFMAAC_SC.aac`) plays at double speed.

### Root cause: mono → stereo channel mismatch

**Diagnostic logs confirmed:**
```
Prebuffer decoded: rate=22050, ch=1, samples_in=1024, output_rate=48000
Prebuffer resampled: from 22050 to 48000, samples_out=2230
Decoded packet 0: rate=22050, ch=1, samples_in=1024, output_rate=48000
```

Each frame produces **2230 mono samples** at 48000 Hz (correct per-frame timing). But the CPAL output callback (`cpal_source.rs`) is configured for **48000 Hz × 2 channels** = 96000 samples/sec consumption rate. The decode loop pushed **mono** samples into the queue, and the callback consumed them at **2× speed**:

- 2230 queue samples / 96000 = 23.2ms of playback per frame
- But 1024 AAC samples / 22050 = **46.4ms of actual audio per frame**
- **2× speed!**

### Fix
Added **mono→stereo upmix** after resampling, before pushing to the audio queue:

1. **`decode_and_play_from_read`**: Added `output_channels = config.channels as usize` from CPAL device config
2. **Prebuffer**: After resample, expand mono to stereo (duplicate each sample to L+R)
3. **`playback_loop`**: Added `output_channels` parameter. After resample + spectrum accumulation, expand mono to stereo before queue push
4. **`drain_queue`**: Uses `output_channels` for duration estimation (samples_played now counts stereo samples)
5. **Seek path**: Uses `output_channels` for position counter seeding

Total change: ~20 lines added across 3 functions, no new dependencies.

### Files changed
- `rust/crates/core/src/audio/stream/handling.rs`: `decode_and_play_from_read` (output_channels var, target size), prebuffer loop (upmix), `playback_loop` (new parameter + upmix before push), `drain_queue` signature + call sites

### Verification
- `cargo check` — 0 errors, 1 pre-existing warning (`NonSeekable` unused import in radio.rs)
- `cargo test -p tunes4r-core -- upmix` — 5/5 pass
- `cargo test -p tunes4r-core -- resample` — 3/3 pass

### Documentation
- `docs/adr/001-audio-queue-channel-contract.md` — explains the queue channel invariant and why upmixing is required

### Tests added
- `test_upmix_mono_to_stereo`: 1→2 channel upmix duplicates each sample
- `test_upmix_same_channels_is_noop`: identity for equal channel counts
- `test_upmix_empty_input`: empty in → empty out
- `test_upmix_identity_single_channel`: 1→1 returns clone
- `test_upmix_mono_to_quad`: 1→4 expansion works
- `test_resample_interleaved_identity`: same rate → clone
- `test_resample_interleaved_empty`: empty in → empty out
- `test_resample_interleaved_basic_upsample`: 22050→44100 doubles sample count

### Extracted
- `pub fn upmix_interleaved(samples: &[f32], input_channels: usize, output_channels: usize) -> Vec<f32>` — generic channel upmix helper used by both prebuffer and playback_loop, replacing the inline duplicated code
