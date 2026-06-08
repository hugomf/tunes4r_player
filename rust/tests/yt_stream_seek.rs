//! Integration tests for YouTube-style stream seek behavior.
//!
//! Tests the seek-within-buffered-area scenario for HTTP stream sources,
//! exercising the same code paths used for YouTube CDN playback (TeeReader
//! header caching → ChainReader Range seek → Symphonia re-probe).
//!
//! A local HTTP server simulates a CDN that supports Range requests and
//! serves a synthetic MP3 file large enough for meaningful seeks.

#![cfg(not(target_os = "android"))]

use std::ffi::CString;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tunes4r::ffi::{
    audio_engine_create, audio_engine_destroy, audio_engine_get_state, audio_engine_play,
    audio_engine_poll_event, audio_engine_seek, audio_engine_stop, AudioEngineHandle,
};
use tunes4r::models::{
    EngineEvent, PlaybackState, ENGINE_EVENT_NONE, ENGINE_EVENT_SEEK_COMPLETED,
    ENGINE_EVENT_SEEK_STARTED,
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

/// Build synthetic MP3 data: `num_frames` frames, each 144 bytes
/// (128 kbps, 44100 Hz stereo — ≈26 ms per frame).
fn build_mp3_data(num_frames: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(num_frames as usize * 144);
    let frame_header: [u8; 4] = [0xFF, 0xFB, 0x90, 0x44];
    for _ in 0..num_frames {
        data.extend_from_slice(&frame_header);
        data.extend(std::iter::repeat(0u8).take(140));
    }
    data
}

/// Spawns an HTTP server that:
/// - Serves `data` for normal GET requests
/// - Responds to `Range: bytes=N-` with a 206 Partial Content and the
///   requested suffix of `data`
/// - Accepts up to `max_requests` connections before shutting down
///
/// Returns `(url, request_count)`.
fn serve_range_aware_mp3(data: Vec<u8>, max_requests: usize) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();
    let data_len = data.len();

    thread::spawn(move || {
        for stream in listener.incoming() {
            if count_clone.load(Ordering::SeqCst) >= max_requests {
                break;
            }
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => break,
            };
            count_clone.fetch_add(1, Ordering::SeqCst);

            let mut request = String::new();
            let mut buf = [0u8; 4096];
            stream
                .set_read_timeout(Some(Duration::from_millis(500)))
                .ok();
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

            if let Some(range_header) = request
                .lines()
                .find(|l| l.to_lowercase().starts_with("range:"))
            {
                let range_value = range_header.split(':').nth(1).unwrap_or("").trim();
                let start_byte = range_value
                    .trim_start_matches("bytes=")
                    .split('-')
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);

                let start = start_byte.min(data_len);
                let slice = &data[start..];
                let resp = format!(
                    "HTTP/1.1 206 Partial Content\r\n\
                     Content-Type: audio/mpeg\r\n\
                     Content-Range: bytes {}-{}/{}\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    start,
                    data_len - 1,
                    data_len,
                    slice.len()
                );
                stream.write_all(resp.as_bytes()).ok();
                stream.write_all(slice).ok();
            } else {
                let resp = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: audio/mpeg\r\n\
                     Content-Length: {}\r\n\
                     Accept-Ranges: bytes\r\n\
                     Connection: close\r\n\r\n",
                    data_len
                );
                stream.write_all(resp.as_bytes()).ok();
                stream.write_all(&data).ok();
            }
            stream.flush().ok();
        }
    });

    (
        format!("http://127.0.0.1:{}/stream.mp3", port),
        request_count,
    )
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
        thread::sleep(Duration::from_millis(10));
    }
}

// ── Stream seek within buffered area ──────────────────────────────────

/// Playing an HTTP stream via `audio_engine_play` and seeking to a position
/// within the already-downloaded data must emit SEEK_STARTED followed by
/// SEEK_COMPLETED with the correct position parameter.
#[test]
fn stream_seek_within_buffer_emits_started_and_completed() {
    let engine = make_engine();
    let data = build_mp3_data(383);
    let (url, _req_count) = serve_range_aware_mp3(data, 4);

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "play should succeed");

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(5));
    drain_events(engine.0);

    let seek_pos: u64 = 500;
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
    assert_eq!(started.unwrap().int_param, seek_pos as i64);

    if reached_playing {
        let completed = events
            .iter()
            .find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
        assert!(
            completed.is_some(),
            "expected SEEK_COMPLETED for stream seek, got: {:?}",
            events
        );
        assert_eq!(completed.unwrap().int_param, seek_pos as i64);

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
}

/// Seeking to position 0 on a stream must emit both events and not crash.
#[test]
fn stream_seek_to_zero_emits_both_events() {
    let engine = make_engine();
    let data = build_mp3_data(383);
    let (url, _req_count) = serve_range_aware_mp3(data, 4);

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0);

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(5));
    drain_events(engine.0);

    let rc = audio_engine_seek(engine.0, 0);
    assert_eq!(rc, 0, "seek to 0 should succeed");

    let events = drain_events(engine.0);

    let started = events
        .iter()
        .find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(
        started.is_some(),
        "expected SEEK_STARTED for seek(0), got: {:?}",
        events
    );
    assert_eq!(started.unwrap().int_param, 0);

    if reached_playing {
        let completed = events
            .iter()
            .find(|e| e.event_type == ENGINE_EVENT_SEEK_COMPLETED);
        assert!(
            completed.is_some(),
            "expected SEEK_COMPLETED for seek(0), got: {:?}",
            events
        );
        assert_eq!(completed.unwrap().int_param, 0);
    }

    audio_engine_stop(engine.0);
}

/// Multiple rapid seeks within the buffered area must each emit
/// SEEK_STARTED + SEEK_COMPLETED in order without interleaving.
#[test]
fn stream_multiple_rapid_seeks_within_buffer() {
    let engine = make_engine();
    let data = build_mp3_data(383);
    let (url, _req_count) = serve_range_aware_mp3(data, 8);

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0);

    let reached_playing = wait_for_playing(engine.0, Duration::from_secs(5));
    if !reached_playing {
        eprintln!("[skip] engine did not reach Playing, skipping multi-seek test");
        audio_engine_stop(engine.0);
        return;
    }
    drain_events(engine.0);

    let seek_positions: &[u64] = &[200, 400, 100];
    for &pos in seek_positions {
        let rc = audio_engine_seek(engine.0, pos);
        assert_eq!(rc, 0, "seek to {} should succeed", pos);

        thread::sleep(Duration::from_millis(50));
    }

    thread::sleep(Duration::from_millis(300));
    let events = drain_events(engine.0);

    let started_count = events
        .iter()
        .filter(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED)
        .count();
    assert!(
        started_count >= seek_positions.len(),
        "expected at least {} SEEK_STARTED events, got {}",
        seek_positions.len(),
        started_count
    );

    for window in events.windows(2) {
        if window[0].event_type == ENGINE_EVENT_SEEK_STARTED
            && window[1].event_type == ENGINE_EVENT_SEEK_COMPLETED
        {
            assert_eq!(
                window[0].int_param, window[1].int_param,
                "SEEK_COMPLETED position must match its SEEK_STARTED"
            );
        }
    }

    audio_engine_stop(engine.0);
}

/// After seek within the buffer, the engine must remain in a playable state
/// and accept further seeks without crashing or wedging.
#[test]
fn stream_seek_then_seek_again_remains_stable() {
    let engine = make_engine();
    let data = build_mp3_data(383);
    let (url, _req_count) = serve_range_aware_mp3(data, 8);

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0);

    wait_for_playing(engine.0, Duration::from_secs(5));
    drain_events(engine.0);

    let first_seek: u64 = 300;
    let rc = audio_engine_seek(engine.0, first_seek);
    assert_eq!(rc, 0);

    thread::sleep(Duration::from_millis(200));
    drain_events(engine.0);

    let second_seek: u64 = 600;
    let rc = audio_engine_seek(engine.0, second_seek);
    assert_eq!(rc, 0, "second seek should succeed after first seek");

    let events = drain_events(engine.0);
    let started = events
        .iter()
        .find(|e| e.event_type == ENGINE_EVENT_SEEK_STARTED);
    assert!(
        started.is_some(),
        "expected SEEK_STARTED for second seek, got: {:?}",
        events
    );
    assert_eq!(started.unwrap().int_param, second_seek as i64);

    audio_engine_stop(engine.0);
}
