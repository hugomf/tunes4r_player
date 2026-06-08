//! Integration tests for seek behavior on file and live streams.
//!
//! Verifies the event lifecycle (SEEK_STARTED → SEEK_COMPLETED) and
//! live-stream buffer clamping via the FFI surface.

#![cfg(not(target_os = "android"))]

use std::ffi::CString;
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tunes4r::audio::stream::source::{Capability, ReadSeek, SourceInfo, SourceKind, StreamSource};
use tunes4r::ffi::{
    audio_engine_create, audio_engine_destroy, audio_engine_get_state,
    audio_engine_play, audio_engine_play_live, audio_engine_poll_event,
    audio_engine_seek, audio_engine_stop,
    AudioEngineHandle,
};
use tunes4r::models::{
    EngineEvent, PlaybackState, StreamType,
    ENGINE_EVENT_NONE, ENGINE_EVENT_SEEK_COMPLETED,
    ENGINE_EVENT_SEEK_STARTED,
};
use tunes4r::{PlaybackEngine, PlaybackError};

// ── Helpers ──────────────────────────────────────────────────────────

struct EngineGuard(*mut AudioEngineHandle);

impl Drop for EngineGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { audio_engine_destroy(self.0) };
        }
    }
}

fn make_engine() -> EngineGuard {
    let raw = audio_engine_create();
    assert!(!raw.is_null(), "audio_engine_create returned null");
    EngineGuard(raw)
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("CString::new")
}

fn drain_events(engine: *mut AudioEngineHandle) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    loop {
        let e = audio_engine_poll_event(engine);
        if e.event_type == ENGINE_EVENT_NONE {
            break;
        }
        events.push(e);
    }
    events
}

fn write_dummy_audio_file(path: &PathBuf, num_frames: u32) {
    let frame_size = 417;
    let mut data = Vec::with_capacity(num_frames as usize * frame_size);
    let frame_header: [u8; 4] = [0xFF, 0xFB, 0x90, 0x44];
    for _ in 0..num_frames {
        data.extend_from_slice(&frame_header);
        data.extend(std::iter::repeat(0u8).take(frame_size - 4));
    }
    let mut f = File::create(path).expect("create temp file");
    f.write_all(&data).expect("write temp file");
}

/// Multi-shot HTTP server that serves dummy MP3 data (383 frames).
/// Accepts up to `max_connections` requests, then stops.
fn serve_mp3_multi(max_connections: usize) -> (String, Arc<AtomicUsize>) {
    let frame_size = 417;
    let mut data = Vec::with_capacity(383 * frame_size);
    let frame_header: [u8; 4] = [0xFF, 0xFB, 0x90, 0x44];
    for _ in 0..383 {
        data.extend_from_slice(&frame_header);
        data.extend(std::iter::repeat(0u8).take(frame_size - 4));
    }
    let len = data.len();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::new(AtomicUsize::new(0));
    let served_clone = served.clone();

    thread::spawn(move || {
        for stream in listener.incoming() {
            if served_clone.fetch_add(1, Ordering::SeqCst) >= max_connections {
                break;
            }
            if let Ok(mut stream) = stream {
                let mut buf = [0u8; 4096];
                let _ = Read::read(&mut stream, &mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    len
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&data);
            }
        }
    });

    (format!("http://127.0.0.1:{}/test.mp3", port), served)
}

// ── Mock seekable stream source ─────────────────────────────────────────

/// A mock StreamSource that returns dummy MP3 data and supports seeking.
/// Routes through the Stream playback path (commands.rs: PlaybackType::Stream).
struct MockStreamSource {
    info: SourceInfo,
    data: Arc<Vec<u8>>,
}

impl StreamSource for MockStreamSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, cap: Capability) -> bool {
        matches!(cap, Capability::Seek | Capability::Download)
    }

    fn open(
        &self,
        _seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
        Ok(Box::new(Cursor::new((*self.data).clone())))
    }

    fn total_bytes(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }
}

fn dummy_mp3_data(num_frames: u32) -> Vec<u8> {
    let frame_size = 417; // MPEG1 Layer3, 128kbps, 44100Hz, no padding
    let mut data = Vec::with_capacity(num_frames as usize * frame_size);
    let frame_header: [u8; 4] = [0xFF, 0xFB, 0x90, 0x44];
    for _ in 0..num_frames {
        data.extend_from_slice(&frame_header);
        data.extend(std::iter::repeat(0u8).take(frame_size - 4));
    }
    data
}

fn drain_events_rust(engine: &mut PlaybackEngine) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    loop {
        let e = engine.poll_event();
        if e.event_type == ENGINE_EVENT_NONE {
            break;
        }
        events.push(e);
    }
    events
}

// ── FFI safety tests ──────────────────────────────────────────────────

/// Seek with null handle must return -1.
#[test]
fn seek_null_handle_returns_error() {
    let rc = audio_engine_seek(std::ptr::null_mut(), 0);
    assert_eq!(rc, -1, "seek with null handle should return -1");
}

/// Seek on an idle (never played) engine: SEEK_STARTED is pushed but no
/// SEEK_COMPLETED (engine has no playback type to restart).
#[test]
fn seek_on_idle_engine_emits_only_started() {
    let engine = make_engine();
    drain_events(engine.0); // clear any init events

    let rc = audio_engine_seek(engine.0, 500);
    assert_eq!(rc, 0, "seek on idle engine should return 0");

    let events = drain_events(engine.0);
    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED on idle seek, got: {:?}", events);
    assert_eq!(started.unwrap().int_param, 500);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_none(), "no SEEK_COMPLETED expected on idle seek, got: {:?}", events);
}

/// Seek after stop: same as idle — SEEK_STARTED but no SEEK_COMPLETED.
#[test]
fn seek_after_stop_emits_only_started() {
    let engine = make_engine();
    drain_events(engine.0);

    // Stop immediately (nothing playing — just validates the code path).
    audio_engine_stop(engine.0);
    drain_events(engine.0);

    let rc = audio_engine_seek(engine.0, 500);
    assert_eq!(rc, 0, "seek after stop should return 0");

    let events = drain_events(engine.0);
    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED after stop, got: {:?}", events);
    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_none(), "no SEEK_COMPLETED expected after stop, got: {:?}", events);
}

// ── File seek tests ──────────────────────────────────────────────────

/// Seek on a file source must emit SEEK_STARTED followed by
/// SEEK_COMPLETED with the correct position parameter.
#[test]
fn file_seek_emits_started_and_completed_events() {
    let engine = make_engine();
    let mut path = std::env::temp_dir();
    path.push(format!("tunes4r_seek_events_{}.mp3", std::process::id()));
    write_dummy_audio_file(&path, 383);
    let uri = cstr(path.to_str().unwrap());

    let rc = unsafe { audio_engine_play(engine.0, uri.as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    // Wait for Playing state (or timeout).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let st = audio_engine_get_state(engine.0);
        if st == PlaybackState::Playing.to_i32() || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    // Drain startup events so seek events are clean.
    drain_events(engine.0);

    // Seek to 500ms.
    let seek_pos: u64 = 500;
    let rc = audio_engine_seek(engine.0, seek_pos);
    assert_eq!(rc, 0, "seek should return 0, got {}", rc);

    let events = drain_events(engine.0);

    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED, got: {:?}", events);
    assert_eq!(started.unwrap().int_param, seek_pos as i64);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED, got: {:?}", events);
    assert_eq!(completed.unwrap().int_param, seek_pos as i64);

    // ORDER: STARTED before COMPLETED.
    let s_idx = events.iter().position(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    let c_idx = events.iter().position(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(s_idx.unwrap() < c_idx.unwrap(), "STARTED must precede COMPLETED");

    audio_engine_stop(engine.0);
    let _ = std::fs::remove_file(&path);
}

/// Seek to position 0 on file: must not error and must emit events.
#[test]
fn file_seek_at_zero() {
    let engine = make_engine();
    let mut path = std::env::temp_dir();
    path.push(format!("tunes4r_seek_zero_{}.mp3", std::process::id()));
    write_dummy_audio_file(&path, 383);
    let uri = cstr(path.to_str().unwrap());

    unsafe { audio_engine_play(engine.0, uri.as_ptr(), -1) };

    // Wait a brief moment, drain startup events.
    thread::sleep(Duration::from_millis(100));
    drain_events(engine.0);

    let rc = audio_engine_seek(engine.0, 0);
    assert_eq!(rc, 0, "seek to 0 should succeed");

    let events = drain_events(engine.0);
    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED for seek(0), got: {:?}", events);
    assert_eq!(started.unwrap().int_param, 0);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED for seek(0), got: {:?}", events);
    assert_eq!(completed.unwrap().int_param, 0);

    audio_engine_stop(engine.0);
    let _ = std::fs::remove_file(&path);
}


// ── Live seek tests ──────────────────────────────────────────────────

/// Live seek within the buffered region emits STARTED + COMPLETED.
#[test]
fn live_seek_within_buffer_emits_both_events() {
    let engine = make_engine();
    // Allow up to 2 connections: initial play + seek restart.
    let (url, _served) = serve_mp3_multi(2);

    let rc = unsafe { audio_engine_play_live(engine.0, cstr(&url).as_ptr(), 60_000) };
    assert_eq!(rc, 0, "play_live should succeed");

    // Wait for Playing state (up to 5s).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let st = audio_engine_get_state(engine.0);
        if st == PlaybackState::Playing.to_i32() || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    drain_events(engine.0); // discard startup events

    // Seek to 100ms — should be within buffered region.
    let seek_pos: u64 = 100;
    let rc = audio_engine_seek(engine.0, seek_pos);
    assert_eq!(rc, 0, "live seek within buffer should succeed");

    let events = drain_events(engine.0);

    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(
        started.is_some(),
        "expected SEEK_STARTED in live seek, got: {:?}",
        events
    );
    assert_eq!(started.unwrap().int_param, seek_pos as i64);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(
        completed.is_some(),
        "expected SEEK_COMPLETED in live seek, got: {:?}",
        events
    );
    assert_eq!(completed.unwrap().int_param, seek_pos as i64);

    audio_engine_stop(engine.0);
}

/// Seek to position 0 on a live stream: same event contract.
#[test]
fn live_seek_at_zero() {
    let engine = make_engine();
    let (url, _served) = serve_mp3_multi(2);

    unsafe { audio_engine_play_live(engine.0, cstr(&url).as_ptr(), 60_000) };

    // Wait for playing state (or timeout).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let st = audio_engine_get_state(engine.0);
        if st == PlaybackState::Playing.to_i32() || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    drain_events(engine.0);

    let rc = audio_engine_seek(engine.0, 0);
    assert_eq!(rc, 0, "live seek to 0 should succeed");

    let events = drain_events(engine.0);
    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED for live seek(0), got: {:?}", events);
    assert_eq!(started.unwrap().int_param, 0);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED for live seek(0), got: {:?}", events);
    assert_eq!(completed.unwrap().int_param, 0);

    audio_engine_stop(engine.0);
}

/// Live seek beyond the buffered region clamps the target to buffer end.
/// SEEK_COMPLETED carries the clamped (not the requested) position.
#[test]
fn live_seek_beyond_buffer_clamps_event_param() {
    let engine = make_engine();
    let (url, _served) = serve_mp3_multi(2);

    let rc = unsafe { audio_engine_play_live(engine.0, cstr(&url).as_ptr(), 60_000) };
    assert_eq!(rc, 0);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let st = audio_engine_get_state(engine.0);
        if st == PlaybackState::Playing.to_i32() || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    drain_events(engine.0);

    // Seek to a position far past any plausible buffer head (120s).
    let far_pos: u64 = 120_000;
    let rc = audio_engine_seek(engine.0, far_pos);
    assert_eq!(rc, 0);

    let events = drain_events(engine.0);

    // STARTED should carry the clamped value.
    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(
        started.is_some(),
        "expected SEEK_STARTED for clamped live seek, got: {:?}",
        events
    );
    let clamped = started.unwrap().int_param as u64;
    assert!(
        clamped < far_pos,
        "seek target should be clamped from {far_pos} to {clamped}"
    );

    // SEEK_COMPLETED should carry the same clamped value.
    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(
        completed.is_some(),
        "expected SEEK_COMPLETED for clamped live seek, got: {:?}",
        events
    );
    assert_eq!(completed.unwrap().int_param as u64, clamped);

    audio_engine_stop(engine.0);
}

// ── Stream seek tests (PlaybackType::Stream) ───────────────────────────

/// Seek within the buffered area on a seekable stream source must emit
/// SEEK_STARTED followed by SEEK_COMPLETED with the correct position.
///
/// The test uses a headless engine with a mock source that supports Seek.
/// The event lifecycle is validated independently of audio decoding —
/// the seek path (commands.rs: ~line 780) opens the source, spawns a
/// decode thread, and pushes SEEK_COMPLETED regardless of decode success.
#[test]
fn stream_seek_within_buffer_emits_both_events() {
    let mut engine = PlaybackEngine::new_without_device().unwrap();
    let data = dummy_mp3_data(700); // ~10 seconds
    let total_bytes = data.len() as u64;

    let source = MockStreamSource {
        info: SourceInfo {
            kind: SourceKind::Radio,
            stream_type: StreamType::Seekable { total_bytes },
            uri: "mock://seek-test".into(),
            title: Some("Mock Seek Test".into()),
            artist: None,
            album: None,
        },
        data: Arc::new(data),
    };

    // play_pipeline sets playback_type + source before the decode
    // thread starts, so the seek path has everything it needs.
    let rc = engine.play_pipeline(Box::new(source));
    assert!(rc.is_ok(), "play_pipeline should succeed, got {:?}", rc);

    // Drain any startup events.
    drain_events_rust(&mut engine);

    // Seek to 500ms — within the ~10s track.
    let seek_pos: u64 = 500;
    let rc = engine.seek(seek_pos);
    assert!(rc.is_ok(), "seek should succeed, got {:?}", rc);

    let events = drain_events_rust(&mut engine);

    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(
        started.is_some(),
        "expected SEEK_STARTED in stream seek, got: {:?}",
        events
    );
    assert_eq!(started.unwrap().int_param, seek_pos as i64);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(
        completed.is_some(),
        "expected SEEK_COMPLETED in stream seek, got: {:?}",
        events
    );
    assert_eq!(completed.unwrap().int_param, seek_pos as i64);

    // ORDER: STARTED before COMPLETED.
    let s_idx = events.iter().position(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    let c_idx = events.iter().position(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(s_idx.unwrap() < c_idx.unwrap(), "STARTED must precede COMPLETED");

    engine.stop();
}

/// Backward seek within the buffered area on an HTTP stream must restart
/// the decode thread from cache and emit SEEK_STARTED → SEEK_COMPLETED.
#[test]
fn stream_seek_backward_within_buffer_emits_both_events() {
    let engine = make_engine();
    // Allow up to 2 connections: initial play + backward seek restart.
    let (url, _served) = serve_mp3_multi(2);

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    // Wait for Playing state (decode thread initializes).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let st = audio_engine_get_state(engine.0);
        if st == PlaybackState::Playing.to_i32() || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        audio_engine_get_state(engine.0),
        PlaybackState::Playing.to_i32(),
        "engine should reach Playing state within 5s"
    );

    // Allow some playhead advance so a backward seek makes sense.
    thread::sleep(Duration::from_millis(1500));

    // Drain startup events so seek events are clean.
    drain_events(engine.0);

    // Seek backward to 100ms (after playhead should have advanced past this).
    let seek_pos: u64 = 100;
    let rc = audio_engine_seek(engine.0, seek_pos);
    assert_eq!(rc, 0, "backward seek should succeed, got {}", rc);

    let events = drain_events(engine.0);

    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(
        started.is_some(),
        "expected SEEK_STARTED in backward seek, got: {:?}",
        events
    );
    assert_eq!(started.unwrap().int_param, seek_pos as i64);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(
        completed.is_some(),
        "expected SEEK_COMPLETED in backward seek, got: {:?}",
        events
    );
    assert_eq!(completed.unwrap().int_param, seek_pos as i64);

    let s_idx = events.iter().position(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    let c_idx = events.iter().position(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(s_idx.unwrap() < c_idx.unwrap(), "STARTED must precede COMPLETED");

    audio_engine_stop(engine.0);
}
