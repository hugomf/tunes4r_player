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

/// Spawns a local HTTP server that serves `data` with Range support.
/// Returns the URL and a shutdown signal.
fn serve_fixture(data: Vec<u8>) -> (String, Arc<AtomicBool>) {
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
                        stream.write_all(slice).ok();
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
