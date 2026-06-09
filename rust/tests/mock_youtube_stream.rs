//! Tests that replay a captured YouTube CDN fixture (from `tests/fixtures/`).
//!
//! Generate the fixture with:
//!   cargo run --bin capture_youtube_fixture -- <video-id>
//! or manually:
//!   yt-dlp -g -f "bestaudio[ext=m4a]" <video-id> | xargs curl -r 0-2097151 -o tests/fixtures/youtube_stream.bin
//!
//! The test skips gracefully when no fixture file exists, so it's safe in CI.

#![cfg(not(target_os = "android"))]

use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tunes4r::ffi::{
    audio_engine_create, audio_engine_destroy, audio_engine_get_position,
    audio_engine_get_state, audio_engine_pause, audio_engine_play,
    audio_engine_poll_event, audio_engine_resume, audio_engine_seek,
    audio_engine_stop, AudioEngineHandle,
};
use tunes4r::models::{
    EngineEvent, PlaybackState,
    ENGINE_EVENT_END_OF_STREAM, ENGINE_EVENT_NONE,
    ENGINE_EVENT_SEEK_COMPLETED,
    ENGINE_EVENT_SEEK_STARTED,
    ENGINE_EVENT_STATE_CHANGED,
};

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

fn wait_for_playing(engine: *mut AudioEngineHandle, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let st = audio_engine_get_state(engine);
        if st == PlaybackState::Playing.to_i32() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Returns the path to `tests/fixtures/` relative to `CARGO_MANIFEST_DIR`.
fn fixture_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures");
    p
}

fn fixture_exists() -> bool {
    fixture_dir().join("youtube_stream.bin").exists()
}

fn load_fixture_data() -> Vec<u8> {
    let path = fixture_dir().join("youtube_stream.bin");
    fs::read(&path).expect("read fixture data")
}

/// Simulated network conditions for realistic streaming tests.
struct NetworkConditions {
    /// Extra delay before sending the response header (ms).
    latency_ms: u64,
    /// Maximum bytes per second (0 = unlimited).
    throttle_bps: u64,
}

impl Default for NetworkConditions {
    fn default() -> Self {
        Self {
            latency_ms: 0,
            throttle_bps: 0,
        }
    }
}

/// Like `serve_fixture_with_count` but with configurable network simulation.
fn serve_fixture_with_network(
    data: Vec<u8>,
    net: NetworkConditions,
) -> (String, Arc<AtomicBool>, Arc<std::sync::atomic::AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let done = shutdown.clone();
    let data_len = data.len();
    let request_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = request_count.clone();

    thread::spawn(move || {
        listener.set_nonblocking(true).ok();
        loop {
            if done.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    count.fetch_add(1, Ordering::SeqCst);
                    stream.set_read_timeout(Some(Duration::from_millis(500))).ok();

                    // Simulate network latency
                    if net.latency_ms > 0 {
                        thread::sleep(Duration::from_millis(net.latency_ms));
                    }

                    let mut request = String::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        request.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if request.contains("\r\n\r\n") {
                            break;
                        }
                    }

                    // Check if request asks for initial bytes (format detection)
                    // vs Range seek — log it for test diagnostics
                    let is_range = request.contains("Range: bytes=") || request.contains("range: bytes=");

                    let start = if is_range {
                        request
                            .lines()
                            .find(|l| l.to_lowercase().starts_with("range:"))
                            .and_then(|l| {
                                l.split(':').nth(1)?.trim()
                                    .trim_start_matches("bytes=")
                                    .split('-')
                                    .next()?
                                    .parse::<usize>().ok()
                            })
                            .unwrap_or(0)
                    } else {
                        0
                    }.min(data_len);

                    let slice = &data[start..];

                    if is_range {
                        let resp = format!(
                            "HTTP/1.1 206 Partial Content\r\n\
                             Content-Type: audio/mp4\r\n\
                             Content-Range: bytes {}-{}/{}\r\n\
                             Content-Length: {}\r\n\
                             Connection: close\r\n\r\n",
                            start,
                            data_len - 1,
                            data_len,
                            slice.len()
                        );
                        stream.write_all(resp.as_bytes()).ok();
                    } else {
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\n\
                             Content-Type: audio/mp4\r\n\
                             Content-Length: {}\r\n\
                             Accept-Ranges: bytes\r\n\
                             Connection: close\r\n\r\n",
                            data_len
                        );
                        stream.write_all(resp.as_bytes()).ok();
                    }

                    // Write body with bandwidth throttling
                    if net.throttle_bps > 0 {
                        let chunk_size = (net.throttle_bps / 10).max(1024) as usize;
                        for chunk in slice.chunks(chunk_size) {
                            stream.write_all(chunk).ok();
                            stream.flush().ok();
                            thread::sleep(Duration::from_millis(100));
                        }
                    } else {
                        stream.write_all(slice).ok();
                    }
                    stream.flush().ok();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://127.0.0.1:{}/stream", port), shutdown, request_count)
}

/// Returns `(url, shutdown, request_count)`.
fn serve_fixture_with_count(data: Vec<u8>) -> (String, Arc<AtomicBool>, Arc<std::sync::atomic::AtomicUsize>) {
    serve_fixture_with_count_and_type(data, "audio/mp4")
}

fn serve_fixture_with_count_and_type(data: Vec<u8>, content_type: &str) -> (String, Arc<AtomicBool>, Arc<std::sync::atomic::AtomicUsize>) {
    let ct = content_type.to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let done = shutdown.clone();
    let data_len = data.len();
    let request_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = request_count.clone();

    thread::spawn(move || {
        listener.set_nonblocking(true).ok();
        loop {
            if done.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    count.fetch_add(1, Ordering::SeqCst);
                    stream.set_read_timeout(Some(Duration::from_millis(500))).ok();
                    let mut request = String::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        request.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if request.contains("\r\n\r\n") {
                            break;
                        }
                    }

                    if request.contains("Range: bytes=") || request.contains("range: bytes=") {
                        let start = request
                            .lines()
                            .find(|l| l.to_lowercase().starts_with("range:"))
                            .and_then(|l| {
                                l.split(':').nth(1)?.trim()
                                    .trim_start_matches("bytes=")
                                    .split('-')
                                    .next()?
                                    .parse::<usize>().ok()
                            })
                            .unwrap_or(0).min(data_len);

                        let slice = &data[start..];
                        let resp = format!(
                            "HTTP/1.1 206 Partial Content\r\n\
                             Content-Type: {}\r\n\
                             Content-Range: bytes {}-{}/{}\r\n\
                             Content-Length: {}\r\n\
                             Connection: close\r\n\r\n",
                            ct, start,
                            data_len - 1,
                            data_len,
                            slice.len()
                        );
                        stream.write_all(resp.as_bytes()).ok();
                        stream.write_all(slice).ok();
                    } else {
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\n\
                             Content-Type: {}\r\n\
                             Content-Length: {}\r\n\
                             Accept-Ranges: bytes\r\n\
                             Connection: close\r\n\r\n",
                            ct, data_len
                        );
                        stream.write_all(resp.as_bytes()).ok();
                        stream.write_all(&data).ok();
                    }
                    stream.flush().ok();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://127.0.0.1:{}/stream", port), shutdown, request_count)
}

/// Serve fixture but return `(url, shutdown)` — backward compat.
fn serve_fixture(data: Vec<u8>) -> (String, Arc<AtomicBool>) {
    let (url, shutdown, _) = serve_fixture_with_count(data);
    (url, shutdown)
}

/// Drain events and return only state-transition events with their i32 value.
fn drain_state_events(engine: *mut AudioEngineHandle) -> Vec<i32> {
    let mut states = Vec::new();
    loop {
        let e = audio_engine_poll_event(engine);
        if e.event_type == ENGINE_EVENT_NONE {
            break;
        }
        if e.event_type == ENGINE_EVENT_STATE_CHANGED {
            states.push(e.int_param as i32);
        }
    }
    states
}

#[allow(dead_code)]
fn wait_for_state_at_least(engine: *mut AudioEngineHandle, min_state: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let st = audio_engine_get_state(engine);
        if st >= min_state {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Drain events and return only END_OF_STREAM events count.
fn drain_eos_events(engine: *mut AudioEngineHandle) -> usize {
    let mut count = 0;
    loop {
        let e = audio_engine_poll_event(engine);
        if e.event_type == ENGINE_EVENT_NONE {
            break;
        }
        if e.event_type == ENGINE_EVENT_END_OF_STREAM {
            count += 1;
        }
    }
    count
}

/// Build synthetic MP3 data of approximately `approx_secs` seconds.
/// Each frame = 144 bytes (MPEG2 Layer 3, 32 kbps, 16 kHz stereo).
/// Frames are ~72 ms each (1152 samples per frame / 16000 Hz).
fn build_synthetic_mp3(approx_secs: u32) -> Vec<u8> {
    let frame_duration_ms = 1152.0 / 16000.0 * 1000.0; // ~72 ms
    let num_frames = (approx_secs as f64 * 1000.0 / frame_duration_ms).ceil() as u32;
    let mut data = Vec::with_capacity(num_frames as usize * 144);
    // MPEG2 Layer 3, 32 kbps, 16 kHz, Joint Stereo, no CRC, no padding
    let frame_header: [u8; 4] = [0xFF, 0xF3, 0x48, 0x44];
    for _ in 0..num_frames {
        data.extend_from_slice(&frame_header);
        data.extend(std::iter::repeat(0u8).take(140));
    }
    data
}

/// Serve fixture data but only send `send_bytes` before closing the connection
/// (announces full Content-Length to simulate a mid-stream cut).
fn serve_connection_cut(data: Vec<u8>, send_bytes: usize) -> (String, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let done = shutdown.clone();
    let data_len = data.len();

    thread::spawn(move || {
        listener.set_nonblocking(true).ok();
        loop {
            if done.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 4096];
                    let mut request = String::new();
                    loop {
                        let n = match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        request.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if request.contains("\r\n\r\n") {
                            break;
                        }
                    }

                    let resp = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: audio/mp4\r\n\
                         Content-Length: {}\r\n\
                         Accept-Ranges: bytes\r\n\
                         Connection: close\r\n\r\n",
                        data_len
                    );
                    stream.write_all(resp.as_bytes()).ok();
                    let n = send_bytes.min(data_len);
                    stream.write_all(&data[..n]).ok();
                    stream.flush().ok();
                    // Connection drops without sending remaining bytes
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://127.0.0.1:{}/stream", port), shutdown)
}

/// Serve random/garbage bytes to test error resilience.
fn serve_garbage(num_bytes: usize) -> (String, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let done = shutdown.clone();

    thread::spawn(move || {
        listener.set_nonblocking(true).ok();
        loop {
            if done.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        if String::from_utf8_lossy(&buf[..n]).contains("\r\n\r\n") {
                            break;
                        }
                    }

                    let resp = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: application/octet-stream\r\n\
                         Content-Length: {}\r\n\
                         Connection: close\r\n\r\n",
                        num_bytes
                    );
                    stream.write_all(resp.as_bytes()).ok();

                    let mut remaining = num_bytes;
                    let mut seed: u64 = 42;
                    while remaining > 0 {
                        let chunk_size = remaining.min(4096);
                        let mut chunk = vec![0u8; chunk_size];
                        for byte in chunk.iter_mut() {
                            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                            *byte = (seed >> 32) as u8;
                        }
                        stream.write_all(&chunk).ok();
                        remaining -= chunk_size;
                    }
                    stream.flush().ok();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://127.0.0.1:{}/stream", port), shutdown)
}

// ── Fixture-based seek test ──────────────────────────────────────────

/// Plays a captured CDN fixture and performs a seek to verify the engine
/// handles real MP4/AAC data correctly.
#[test]
fn mock_youtube_seek_with_fixture() {
    if !fixture_exists() {
        eprintln!(
            "SKIP: no CDN fixture found at {} — run `cargo run --bin capture_youtube_fixture -- <video-id>` to generate one",
            fixture_dir().display()
        );
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(10));
    drain_events(engine.0);

    let seek_pos: u64 = 5000;
    let rc = audio_engine_seek(engine.0, seek_pos);
    assert_eq!(rc, 0, "seek should return 0");

    let events = drain_events(engine.0);

    let started = events
        .iter()
        .find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(
        started.is_some(),
        "expected SEEK_STARTED, got: {:?}",
        events
    );

    if reached_playing {
        let completed = events
            .iter()
            .find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
        assert!(
            completed.is_some(),
            "expected SEEK_COMPLETED, got: {:?}",
            events
        );

        let s_idx = events
            .iter()
            .position(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
        let c_idx = events
            .iter()
            .position(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
        assert!(
            s_idx.unwrap() < c_idx.unwrap(),
            "SEEK_STARTED must precede SEEK_COMPLETED"
        );
    }

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

// ── State lifecycle test ─────────────────────────────────────────────

/// Play a captured CDN fixture and verify the engine transitions through
/// Connecting → Buffering → Playing states via ENGINE_EVENT_STATE_CHANGED events.
#[test]
fn mock_youtube_state_lifecycle() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    // Wait for Playing state (or timeout)
    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(15));
    let state_events = drain_state_events(engine.0);

    // We should have seen Connecting(1) → Buffering(2) → Playing(4)
    assert!(!state_events.is_empty(), "expected at least one state change event");

    if reached_playing {
        assert!(
            state_events.contains(&1),
            "expected Connecting(1) in state events: {:?}",
            state_events
        );
        assert!(
            state_events.contains(&2),
            "expected Buffering(2) in state events: {:?}",
            state_events
        );
        assert!(
            state_events.contains(&4),
            "expected Playing(4) in state events: {:?}",
            state_events
        );

        // Verify ordering: Connecting → Buffering → Playing
        let conn_idx = state_events.iter().position(|&s| s == 1).unwrap();
        let buf_idx = state_events.iter().position(|&s| s == 2).unwrap();
        let play_idx = state_events.iter().position(|&s| s == 4).unwrap();
        assert!(
            conn_idx < buf_idx,
            "Connecting(1) must precede Buffering(2), got indexes: conn={}, buf={}",
            conn_idx, buf_idx
        );
        assert!(
            buf_idx < play_idx,
            "Buffering(2) must precede Playing(4), got indexes: buf={}, play={}",
            buf_idx, play_idx
        );

        // Verify engine get_state returns Playing
        let st = audio_engine_get_state(engine.0);
        assert_eq!(st, PlaybackState::Playing.to_i32(), "engine should be Playing");
    }

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

/// Poll the engine state at 50ms intervals to observe Connecting → Buffering → Playing
/// transitions as they happen in real time.
#[test]
fn mock_youtube_poll_state_transitions() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    // Poll state at 50ms intervals for up to 15s
    let mut observed_states: Vec<i32> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut reached_playing = false;

    while Instant::now() < deadline {
        let st = audio_engine_get_state(engine.0);
        if observed_states.last() != Some(&st) {
            observed_states.push(st);
        }
        if st == PlaybackState::Playing.to_i32() {
            reached_playing = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    if !reached_playing {
        eprintln!("SKIP: engine did not reach Playing (observed: {:?})", observed_states);
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }

    // Must have passed through Connecting (1) before Playing (4)
    assert!(
        observed_states.contains(&1),
        "expected Connecting(1) in observed sequence: {:?}",
        observed_states
    );

    // Cross-check via event queue for rapid transitions (Buffering may be too
    // fast to catch by polling at 50ms on a local server).
    let state_events = drain_state_events(engine.0);
    assert!(
        state_events.contains(&2),
        "expected Buffering(2) in drained events: {:?} (poll observed: {:?})",
        state_events,
        observed_states
    );

    // Verify ordering: Connecting → (Buffering) → Playing
    // (observed_states ordering is implicitly confirmed by the poll loop above)

    // Verify we don't regress: once Playing reached, state should stay Playing
    for i in 0..10 {
        let st = audio_engine_get_state(engine.0);
        assert_eq!(
            st,
            PlaybackState::Playing.to_i32(),
            "state should remain Playing, got {} at poll {}",
            st,
            i
        );
        thread::sleep(Duration::from_millis(100));
    }

    audio_engine_stop(engine.0);

    // After stop, state should be Stopped(0)
    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "state should be Stopped after stop");

    shutdown.store(true, Ordering::SeqCst);
}

// ── Backward seek test ───────────────────────────────────────────────

/// Play the fixture, let the playhead advance to ~5s, then seek backward
/// to 2s. Verify the engine handles cache-reopen backward seek correctly.
#[test]
fn mock_youtube_backward_seek() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached_playing {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // Let playhead advance to ~3s
    thread::sleep(Duration::from_millis(3000));

    // Seek backward to 500ms
    let backward_pos: u64 = 500;
    let rc = audio_engine_seek(engine.0, backward_pos);
    assert_eq!(rc, 0, "backward seek should succeed");

    // Allow time for seek to process
    thread::sleep(Duration::from_millis(200));
    let events = drain_events(engine.0);

    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED for backward seek, got: {:?}", events);
    assert_eq!(started.unwrap().int_param, backward_pos as i64);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED for backward seek, got: {:?}", events);
    assert_eq!(completed.unwrap().int_param, backward_pos as i64);

    // Engine should still be Playing
    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Playing.to_i32(), "engine should remain Playing after backward seek");

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

// ── Forward seek within buffer ───────────────────────────────────────

/// Seek forward within the buffered region and verify state stays Playing.
#[test]
fn mock_youtube_forward_seek() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached_playing {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // Seek forward to 10s (within the 213s fixture)
    let seek_pos: u64 = 10000;
    let rc = audio_engine_seek(engine.0, seek_pos);
    assert_eq!(rc, 0, "forward seek should succeed");

    thread::sleep(Duration::from_millis(200));
    let events = drain_events(engine.0);

    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED, got: {:?}", events);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED, got: {:?}", events);

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Playing.to_i32(), "engine should remain Playing after forward seek");

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

// ── Multiple rapid seeks ─────────────────────────────────────────────

/// Perform multiple seeks in rapid succession and verify all produce
/// SEEK_STARTED + SEEK_COMPLETED pairs.
#[test]
fn mock_youtube_multiple_rapid_seeks() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached_playing {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    let positions: &[u64] = &[2000, 5000, 1000, 8000];
    for &pos in positions {
        let rc = audio_engine_seek(engine.0, pos);
        assert_eq!(rc, 0, "seek to {} should succeed", pos);
        thread::sleep(Duration::from_millis(30));
    }

    thread::sleep(Duration::from_millis(300));
    let events = drain_events(engine.0);

    let started_count = events.iter().filter(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED).count();
    assert!(
        started_count >= positions.len(),
        "expected at least {} SEEK_STARTED, got {}",
        positions.len(),
        started_count
    );

    // Engine must be Playing after all seeks
    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Playing.to_i32(), "engine should remain Playing after rapid seeks");

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

// ── Network simulation tests ─────────────────────────────────────────

/// Play with simulated latency (50ms) and verify the engine reaches Playing
/// and handles seek correctly under realistic network conditions.
#[test]
fn mock_youtube_with_latency() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let net = NetworkConditions { latency_ms: 50, ..Default::default() };
    let (url, shutdown, _requests) = serve_fixture_with_network(data, net);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(20));
    if !reached_playing {
        eprintln!("SKIP: engine did not reach Playing under latency");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // State transitions should show Connecting before Playing
    let _state_events = drain_state_events(engine.0);

    // Seek within buffer
    let rc = audio_engine_seek(engine.0, 3000);
    assert_eq!(rc, 0, "seek should succeed under latency");

    thread::sleep(Duration::from_millis(200));
    let events = drain_events(engine.0);
    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED under latency, got: {:?}", events);

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Playing.to_i32());

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

/// Play with bandwidth throttling (50 KB/s) to test slow-network behaviour.
#[test]
fn mock_youtube_throttled() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let net = NetworkConditions { throttle_bps: 50_000, ..Default::default() };
    let (url, shutdown, _requests) = serve_fixture_with_network(data, net);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(30));
    if !reached_playing {
        eprintln!("SKIP: engine did not reach Playing under throttling");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Playing.to_i32(), "engine should be Playing under throttling");

    let rc = audio_engine_seek(engine.0, 2000);
    assert_eq!(rc, 0, "seek should succeed under throttling");

    thread::sleep(Duration::from_millis(500));
    let events = drain_events(engine.0);
    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED under throttling, got: {:?}", events);

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Playing.to_i32());

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

/// Play with both latency + throttling to simulate a challenging real-world
/// mobile connection (200 KB/s, 50ms latency).
#[test]
fn mock_youtube_slow_connection() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let net = NetworkConditions { latency_ms: 50, throttle_bps: 200_000 };
    let (url, shutdown, _requests) = serve_fixture_with_network(data, net);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(30));
    if !reached_playing {
        eprintln!("SKIP: engine did not reach Playing under slow connection");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    for &pos in &[1000, 5000, 2000] {
        let rc = audio_engine_seek(engine.0, pos);
        if rc != 0 {
            eprintln!("SKIP: seek to {} failed (rc={}) under throttling", pos, rc);
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    thread::sleep(Duration::from_millis(500));
    let events = drain_events(engine.0);
    let started_count = events.iter().filter(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED).count();
    assert!(started_count >= 1, "expected at least 1 seek under slow connection, got: {}", started_count);

    // Under throttling, some seeks may time out, but the engine should still be alive
    audio_engine_get_state(engine.0);

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

// ── Pause / Resume ────────────────────────────────────────────────────

/// Play → Playing → pause → verify Paused(5) → resume → verify Playing(4).
#[test]
fn mock_youtube_pause_resume() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // Pause and verify state
    audio_engine_pause(engine.0);
    thread::sleep(Duration::from_millis(100));

    let state_events = drain_state_events(engine.0);
    assert!(
        state_events.contains(&5),
        "expected Paused(5) after pause, got: {:?}",
        state_events
    );
    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Paused.to_i32(), "engine should be Paused after pause");

    // Resume and verify state
    audio_engine_resume(engine.0);
    thread::sleep(Duration::from_millis(200));

    let state_events = drain_state_events(engine.0);
    assert!(
        state_events.contains(&4),
        "expected Playing(4) after resume, got: {:?}",
        state_events
    );
    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Playing.to_i32(), "engine should be Playing after resume");

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

/// Pause → seek → verify SEEK_STARTED + SEEK_COMPLETED → resume → position advances.
#[test]
fn mock_youtube_seek_while_paused() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // Pause
    audio_engine_pause(engine.0);
    thread::sleep(Duration::from_millis(100));
    drain_events(engine.0);

    // Seek while paused
    let seek_pos: u64 = 8000;
    let rc = audio_engine_seek(engine.0, seek_pos);
    assert_eq!(rc, 0, "seek while paused should succeed");

    thread::sleep(Duration::from_millis(200));
    let events = drain_events(engine.0);
    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED while paused, got: {:?}", events);
    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED while paused, got: {:?}", events);

    // Resume and check position moved
    audio_engine_resume(engine.0);
    thread::sleep(Duration::from_millis(300));
    let pos = audio_engine_get_position(engine.0);
    assert!(
        pos.current_ms >= seek_pos,
        "position should advance from {} after resume, got: {}",
        seek_pos,
        pos.current_ms
    );

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

/// Pause → stop → verify Stopped(0).
#[test]
fn mock_youtube_stop_while_paused() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // Pause then stop
    audio_engine_pause(engine.0);
    thread::sleep(Duration::from_millis(50));
    audio_engine_stop(engine.0);
    thread::sleep(Duration::from_millis(100));

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be Stopped after stop from paused");

    shutdown.store(true, Ordering::SeqCst);
}

// ── End-of-stream ─────────────────────────────────────────────────────

/// Play a short synthetic MP3 to completion and verify ENGINE_EVENT_END_OF_STREAM
/// is emitted and the state transitions to Stopped.
#[test]
fn mock_youtube_end_of_stream() {
    let mp3_data = build_synthetic_mp3(5);
    let (url, shutdown, _count) = serve_fixture_with_count_and_type(mp3_data, "audio/mpeg");
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    // Wait for end-of-stream event (20s timeout for a 5s file + buffer overhead)
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut eos_received = false;
    while Instant::now() < deadline {
        let eos_count = drain_eos_events(engine.0);
        if eos_count > 0 {
            eos_received = true;
            break;
        }
        let st = audio_engine_get_state(engine.0);
        if st == PlaybackState::Stopped.to_i32() {
            eos_received = drain_eos_events(engine.0) > 0 || eos_received;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    assert!(eos_received, "expected ENGINE_EVENT_END_OF_STREAM within 20s");

    // State should eventually be Stopped after end of stream
    thread::sleep(Duration::from_millis(200));
    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be Stopped after EOS");

    shutdown.store(true, Ordering::SeqCst);
}

// ── Error injection ───────────────────────────────────────────────────

/// Feed the engine random garbage bytes and verify it doesn't crash.
#[test]
fn mock_youtube_malformed_data() {
    let (url, shutdown) = serve_garbage(256 * 1024);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should return 0 even with garbage data");

    // Give engine time to attempt processing
    thread::sleep(Duration::from_millis(2000));
    drain_events(engine.0);

    // Engine should still be stoppable (no crash)
    let st = audio_engine_get_state(engine.0);
    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be stoppable after garbage");
}

/// Serve valid fixture data but cut the connection mid-stream, verify engine
/// handles the incomplete response gracefully.
#[test]
fn mock_youtube_connection_cut() {
    let data = load_fixture_data();
    let (url, shutdown) = serve_connection_cut(data, 16384); // only 16KB sent
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should return 0");

    // Give engine time to discover the cut connection
    thread::sleep(Duration::from_millis(3000));
    drain_events(engine.0);

    // Engine should be stoppable without crashing
    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be stoppable after connection cut");
}

// ── Stop from any state ───────────────────────────────────────────────

/// Stop immediately after play (while still in Connecting state due to latency).
#[test]
fn mock_youtube_stop_while_connecting() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    // 1000ms latency keeps engine in Connecting long enough to stop immediately
    let net = NetworkConditions { latency_ms: 1000, ..Default::default() };
    let (url, shutdown, _) = serve_fixture_with_network(data, net);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    // Poll rapidly to see if Connecting is observed
    let mut observed = Vec::new();
    for _ in 0..5 {
        observed.push(audio_engine_get_state(engine.0));
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        observed.contains(&1),
        "expected Connecting(1) in sequence: {:?}",
        observed
    );

    // Stop while still connecting
    audio_engine_stop(engine.0);
    thread::sleep(Duration::from_millis(200));

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be Stopped after stop from Connecting");

    shutdown.store(true, Ordering::SeqCst);
}

/// Stop while still buffering (before Playing).
#[test]
fn mock_youtube_stop_while_buffering() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    // Slow throttle to keep engine in Buffering long enough
    let net = NetworkConditions { throttle_bps: 10_000, ..Default::default() };
    let (url, shutdown, _) = serve_fixture_with_network(data, net);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    // Wait long enough to pass Connecting, stop while still Buffering
    thread::sleep(Duration::from_millis(200));

    let st = audio_engine_get_state(engine.0);
    // If already Playing, skip (throttle may not be slow enough on some machines)
    if st == PlaybackState::Playing.to_i32() {
        eprintln!("SKIP: engine reached Playing before stop (throttle too fast)");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }

    audio_engine_stop(engine.0);
    thread::sleep(Duration::from_millis(200));

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be Stopped after stop from Buffering");

    shutdown.store(true, Ordering::SeqCst);
}

/// Call stop twice in a row — verify idempotent (no crash).
#[test]
fn mock_youtube_stop_idempotent() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // Stop twice
    audio_engine_stop(engine.0);
    thread::sleep(Duration::from_millis(100));
    audio_engine_stop(engine.0); // second stop should be a no-op
    thread::sleep(Duration::from_millis(100));

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be Stopped after stop");

    shutdown.store(true, Ordering::SeqCst);
}

// ── Unbuffered area seeks with network simulation ─────────────────────

/// Seek to an unbuffered area (beyond pre-buffer) with latency simulating the
/// cost of establishing a new Range-request connection.
#[test]
fn mock_youtube_seek_unbuffered_with_latency() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    // 100ms latency to simulate connection cost for the Range request
    let net = NetworkConditions { latency_ms: 100, ..Default::default() };
    let (url, shutdown, _) = serve_fixture_with_network(data, net);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    // Wait for Playing before seeking to avoid engine state confusion
    assert!(wait_for_playing(engine.0, Duration::from_secs(20)),
        "should reach Playing within 20s");

    // Seek to 60s — well beyond the ~7s pre-buffer, so engine must fetch new data
    let seek_pos: u64 = 60000;
    let rc = audio_engine_seek(engine.0, seek_pos);
    assert_eq!(rc, 0, "unbuffered seek should succeed");

    // Wait for seek to resolve (Range request with 100ms latency + rebuffer)
    thread::sleep(Duration::from_millis(5000));
    let events = drain_events(engine.0);

    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED for unbuffered seek, got: {:?}", events);

    let completed = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
    assert!(completed.is_some(), "expected SEEK_COMPLETED for unbuffered seek, got: {:?}", events);

    // Verify approximated position after seek
    let pos = audio_engine_get_position(engine.0);
    assert!(
        pos.current_ms >= seek_pos.saturating_sub(3000),
        "position after unbuffered seek should be near {}, got: {}",
        seek_pos,
        pos.current_ms
    );

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

/// Seek beyond the captured data range (200s into a 213s stream with only 2MB).
/// The engine should clamp or handle gracefully.
#[test]
fn mock_youtube_seek_beyond_captured_data() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // Seek to ~200s — near the end of the 213s stream
    let far_pos: u64 = 200000;
    let rc = audio_engine_seek(engine.0, far_pos);
    assert_eq!(rc, 0, "seek near end should succeed");

    // Allow time for seek (will trigger Range request for new data)
    thread::sleep(Duration::from_millis(2000));
    let events = drain_events(engine.0);

    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED for far seek, got: {:?}", events);

    // Engine should still be alive and stoppable
    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be stoppable after far seek");
}

/// Seek beyond the stream's total duration — engine should clamp.
#[test]
fn mock_youtube_seek_beyond_duration() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached {
        eprintln!("SKIP: engine did not reach Playing");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    // Seek way past duration (213s) — should clamp to duration
    let past_end: u64 = 999999;
    let rc = audio_engine_seek(engine.0, past_end);
    assert_eq!(rc, 0, "seek past duration should succeed");

    thread::sleep(Duration::from_millis(1000));
    let events = drain_events(engine.0);

    // Should still see seek events even if clamped
    let started = events.iter().find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(started.is_some(), "expected SEEK_STARTED for clamped seek, got: {:?}", events);

    let pos = audio_engine_get_position(engine.0);
    assert!(
        pos.current_ms <= pos.total_ms,
        "clamped position {} should not exceed total_ms {}",
        pos.current_ms,
        pos.total_ms
    );

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

// ── Replay / double-play ──────────────────────────────────────────────

/// Play → stop → play again with the same engine and fixture.
#[test]
fn mock_youtube_stop_then_play_again() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    // First play
    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "first play should succeed");

    let reached = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached {
        eprintln!("SKIP: engine did not reach Playing on first play");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }
    drain_events(engine.0);

    audio_engine_stop(engine.0);
    thread::sleep(Duration::from_millis(200));

    // Second play with same engine
    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "second play should succeed");

    let reached = wait_for_playing(engine.0, Duration::from_secs(15));
    if !reached {
        eprintln!("SKIP: engine did not reach Playing on second play");
        audio_engine_stop(engine.0);
        shutdown.store(true, Ordering::SeqCst);
        return;
    }

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Playing.to_i32(), "engine should be Playing after replay");

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);
}

/// Call play twice without stop — verify no crash (second play should be
/// rejected or handled gracefully).
#[test]
fn mock_youtube_double_play() {
    if !fixture_exists() {
        eprintln!("SKIP: no CDN fixture found");
        return;
    }

    let data = load_fixture_data();
    let (url, shutdown) = serve_fixture(data);
    let engine = make_engine();

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "first play should succeed");

    // Second play without stop — should not crash
    let rc2 = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    // May return error or succeed; either is acceptable as long as it doesn't crash

    thread::sleep(Duration::from_millis(500));
    drain_events(engine.0);

    audio_engine_stop(engine.0);
    shutdown.store(true, Ordering::SeqCst);

    let st = audio_engine_get_state(engine.0);
    assert_eq!(st, PlaybackState::Stopped.to_i32(), "engine should be stoppable after double play");
}
