//! Integration tests for the streaming download-tracking mechanism.
//!
//! These tests verify that the pieces work together correctly:
//!
//! 1. `ByteCountingRead` — wraps a `Read` source and updates an `AtomicU64`
//!    on each read. This is what feeds `pipe_bytes_sent` for Read-based
//!    sources (YouTube, progressive HTTP).
//!
//! 2. `AdaptiveRingBuffer` — maps the pipe counters (`pipe_bytes_sent` /
//!    `pipe_total_bytes`) plus the playhead into a ring buffer that the UI
//!    uses to show the "downloaded" region on the timeline.
//!
//! 3. End-to-end with a local HTTP server — serves a real file over
//!    `127.0.0.1`, downloads it through `reqwest`, and verifies the ring
//!    buffer shows partial download (not 100%) mid-stream and complete
//!    after EOF.
//!
//! We deliberately do NOT touch the audio output stack (cpal/AAudio) —
//! these tests only exercise the download-tracking layer.

use std::io::{Cursor, Read};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(not(target_os = "android"))]
use tunes4r::audio::stream::handling::ByteCountingRead;
use tunes4r::AdaptiveRingBuffer;

// ── 1. ByteCountingRead ────────────────────────────────────────────

#[cfg(not(target_os = "android"))]
#[test]
fn byte_counting_read_updates_counter_per_read() {
    let data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
    let counter = Arc::new(AtomicU64::new(0));
    let mut reader = ByteCountingRead::new(Cursor::new(data.clone()), counter.clone());

    let mut buf = [0u8; 100];
    let n1 = reader.read(&mut buf).unwrap();
    assert_eq!(n1, 100);
    assert_eq!(counter.load(Ordering::Relaxed), 100);

    let n2 = reader.read(&mut buf).unwrap();
    assert_eq!(n2, 100);
    assert_eq!(counter.load(Ordering::Relaxed), 200);

    // Read the rest in one go.
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).unwrap();
    assert_eq!(rest.len(), 800);
    assert_eq!(counter.load(Ordering::Relaxed), 1000);
}

#[cfg(not(target_os = "android"))]
#[test]
fn byte_counting_read_handles_eof() {
    let data = vec![1u8, 2, 3, 4, 5];
    let counter = Arc::new(AtomicU64::new(0));
    let mut reader = ByteCountingRead::new(Cursor::new(data), counter.clone());

    let mut buf = [0u8; 10];
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 5); // EOF after 5 bytes
    assert_eq!(counter.load(Ordering::Relaxed), 5);

    // Second read returns 0 and does not increment.
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 0);
    assert_eq!(counter.load(Ordering::Relaxed), 5);
}

#[cfg(not(target_os = "android"))]
#[test]
fn byte_counting_read_handles_zero_len_buffer() {
    let data = vec![1u8, 2, 3];
    let counter = Arc::new(AtomicU64::new(0));
    let mut reader = ByteCountingRead::new(Cursor::new(data), counter.clone());

    let mut buf = [];
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 0);
    assert_eq!(counter.load(Ordering::Relaxed), 0);
}

// ── 2. AdaptiveRingBuffer state transitions during streaming ──────

/// Simulates the buffer-poller's output for a progressive download:
/// maps (pipe_bytes_sent, pipe_total_bytes, playhead_ms, total_ms) into
/// an `AdaptiveRingBuffer`. This is the same logic that lives in
/// `commands.rs::play_pipeline`'s buffer-poller closure.
fn compute_ring_buffer(
    pipe_bytes_sent: u64,
    pipe_total_bytes: u64,
    playhead_ms: u64,
    total_ms: u64,
) -> AdaptiveRingBuffer {
    let write_ms = if pipe_total_bytes > 0 && total_ms > 0 {
        ((pipe_bytes_sent as f64 / pipe_total_bytes as f64) * total_ms as f64) as u64
    } else {
        0
    };
    let write_ms = write_ms.min(total_ms);
    let is_complete = pipe_total_bytes > 0 && pipe_bytes_sent >= pipe_total_bytes;
    let read_offset = playhead_ms.min(total_ms);
    AdaptiveRingBuffer {
        capacity_ms: 30_000,
        read_offset_ms: read_offset,
        write_offset_ms: write_ms.max(read_offset),
        total_ms,
        is_complete,
    }
}

#[test]
fn ring_buffer_shows_partial_download_during_streaming() {
    // 60s file, downloaded 20s worth of bytes, playhead at 0.
    let buf = compute_ring_buffer(/* sent */ 1_000_000, /* total */ 3_000_000, 0, 60_000);

    assert!(!buf.is_complete);
    // 1M/3M * 60s = 20_000 ms written.
    assert_eq!(buf.write_offset_ms, 20_000);
    assert_eq!(buf.read_offset_ms, 0);
    assert_eq!(buf.available_ms(), 20_000);
    assert_eq!(buf.end_ms(), 20_000);
}

#[test]
fn ring_buffer_playhead_advances_during_playback() {
    // Mid-stream: downloaded 50%, played 10s.
    let buf = compute_ring_buffer(1_500_000, 3_000_000, 10_000, 60_000);

    assert!(!buf.is_complete);
    assert_eq!(buf.write_offset_ms, 30_000);
    assert_eq!(buf.read_offset_ms, 10_000);
    // Available = write - read = 20s, but clamped to 30s capacity → 20s.
    assert_eq!(buf.available_ms(), 20_000);
    assert_eq!(buf.end_ms(), 30_000);
}

#[test]
fn ring_buffer_marks_complete_at_eof() {
    // All bytes downloaded, playhead at 30s.
    let buf = compute_ring_buffer(3_000_000, 3_000_000, 30_000, 60_000);

    assert!(buf.is_complete);
    // is_complete path: available = total - read = 30s (not clamped).
    assert_eq!(buf.available_ms(), 30_000);
    assert_eq!(buf.end_ms(), 60_000);
}

#[test]
fn ring_buffer_zero_bytes_downloaded_is_empty() {
    // Just started, nothing downloaded yet.
    let buf = compute_ring_buffer(0, 3_000_000, 0, 60_000);

    assert!(!buf.is_complete);
    assert_eq!(buf.write_offset_ms, 0);
    assert_eq!(buf.available_ms(), 0);
    assert_eq!(buf.end_ms(), 0);
}

#[test]
fn ring_buffer_playhead_past_download_is_empty() {
    // Seek forward past the downloaded region (shouldn't normally happen
    // with seek-queuing, but the math must hold). The buffer poller
    // intentionally clamps write_ms to read_offset so the ring never
    // moves backwards, and available_ms reports 0 because write == read.
    let buf = compute_ring_buffer(500_000, 3_000_000, 25_000, 60_000);

    // write_ms = 500k/3M * 60s = 10_000, but clamped up to read_offset.
    assert_eq!(buf.write_offset_ms, 25_000);
    // read_offset == write_offset → no buffered data ahead of playhead.
    assert_eq!(buf.available_ms(), 0);
    assert_eq!(buf.end_ms(), 25_000);
}

// ── 3. End-to-end with a local HTTP server ────────────────────────

/// Spin up a single-shot HTTP server on 127.0.0.1 that serves `body`
/// with the given `Content-Length`. Returns the URL.
fn serve_once(body: Vec<u8>, content_length_header: Option<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            use std::io::Write;
            // Read the request (don't care about its content).
            let mut req = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut req);

            let cl = content_length_header
                .unwrap_or_else(|| body.len().to_string());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                cl
            );
            stream.write_all(response.as_bytes()).unwrap();
            // Write body in small chunks with a tiny delay to simulate
            // progressive download.
            for chunk in body.chunks(64) {
                stream.write_all(chunk).unwrap();
                std::io::Write::flush(&mut stream).unwrap();
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    format!("http://127.0.0.1:{}/audio.mp3", port)
}

#[test]
fn http_download_fills_pipe_counters_progressively() {
    // 1 KB of data, served in 16 chunks of 64 bytes with 10ms delays.
    let body: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
    let url = serve_once(body.clone(), Some("1024".into()));

    let pipe_bytes_sent = Arc::new(AtomicU64::new(0));
    let pipe_total_bytes = Arc::new(AtomicU64::new(0));

    // Spawn a "reader" thread that simulates the engine's pipe
    // interaction: set total_bytes from Content-Length, then read
    // incrementally and update bytes_sent.
    let pbs = pipe_bytes_sent.clone();
    let ptb = pipe_total_bytes.clone();
    let url_clone = url.clone();
    let reader_handle = thread::spawn(move || {
        let resp = reqwest::blocking::get(&url_clone).unwrap();
        let total = resp.content_length().unwrap();
        ptb.store(total, Ordering::Relaxed);

        let mut stream = resp;
        let mut tmp = [0u8; 64];
        while let Ok(n) = stream.read(&mut tmp) {
            if n == 0 {
                break;
            }
            pbs.fetch_add(n as u64, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(5));
        }
    });

    // Poll the counters over ~200ms and check that bytes_sent increases
    // (proves it's streaming, not all-at-once).
    let mut samples: Vec<u64> = Vec::new();
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(20));
        samples.push(pipe_bytes_sent.load(Ordering::Relaxed));
    }
    reader_handle.join().unwrap();

    // total_bytes was set immediately from Content-Length.
    assert_eq!(pipe_total_bytes.load(Ordering::Relaxed), 1024);

    // We should have observed at least 2 distinct increasing values
    // (proves progressive download, not a single jump to 1024).
    let distinct: std::collections::HashSet<u64> = samples.iter().copied().collect();
    assert!(
        distinct.len() >= 2,
        "expected progressive download, got samples {:?}",
        samples
    );

    // Final value equals the body length.
    assert_eq!(pipe_bytes_sent.load(Ordering::Relaxed), 1024);
}

#[test]
fn http_download_ring_buffer_completes_at_eof() {
    let body: Vec<u8> = (0..512).map(|i| (i % 256) as u8).collect();
    let url = serve_once(body.clone(), Some("512".into()));

    let pipe_bytes_sent = Arc::new(AtomicU64::new(0));
    let pipe_total_bytes = Arc::new(AtomicU64::new(0));

    // Total duration: pretend the 512 bytes = 10s of audio.
    // (Byte-to-ms ratio is set by the caller; in the engine it's derived
    // from the codec once the header is parsed.)
    let total_ms: u64 = 10_000;
    let playhead_ms: u64 = 0;

    let pbs = pipe_bytes_sent.clone();
    let ptb = pipe_total_bytes.clone();
    let url_clone = url.clone();
    let reader_handle = thread::spawn(move || {
        let resp = reqwest::blocking::get(&url_clone).unwrap();
        let total = resp.content_length().unwrap();
        ptb.store(total, Ordering::Relaxed);

        let mut stream = resp;
        let mut tmp = [0u8; 32];
        while let Ok(n) = stream.read(&mut tmp) {
            if n == 0 {
                break;
            }
            pbs.fetch_add(n as u64, Ordering::Relaxed);
        }
    });
    reader_handle.join().unwrap();

    // After EOF: ring buffer is complete.
    let buf = compute_ring_buffer(
        pipe_bytes_sent.load(Ordering::Relaxed),
        pipe_total_bytes.load(Ordering::Relaxed),
        playhead_ms,
        total_ms,
    );
    assert!(buf.is_complete);
    assert_eq!(buf.available_ms(), total_ms);
    assert_eq!(buf.end_ms(), total_ms);
}

#[test]
fn http_download_ring_buffer_partial_mid_stream() {
    // Larger body (4 KB) with chunked transfer, but we only wait for
    // the first ~half before snapshotting the ring buffer state.
    let body: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
    let url = serve_once(body, Some("4096".into()));

    let pipe_bytes_sent = Arc::new(AtomicU64::new(0));
    let pipe_total_bytes = Arc::new(AtomicU64::new(0));

    let pbs = pipe_bytes_sent.clone();
    let ptb = pipe_total_bytes.clone();
    let url_clone = url.clone();
    let reader_handle = thread::spawn(move || {
        let resp = reqwest::blocking::get(&url_clone).unwrap();
        let total = resp.content_length().unwrap();
        ptb.store(total, Ordering::Relaxed);

        let mut stream = resp;
        let mut tmp = [0u8; 64];
        // Read until we've got at least 1 KB, then stop early.
        while pbs.load(Ordering::Relaxed) < 1024 {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            pbs.fetch_add(n as u64, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(5));
        }
        // Drop the response — we're not draining the rest. This
        // simulates the decoder falling behind the downloader.
    });

    // Wait up to 2s for the reader to accumulate some bytes.
    let mut buf: Option<AdaptiveRingBuffer> = None;
    for _ in 0..40 {
        thread::sleep(Duration::from_millis(50));
        let sent = pipe_bytes_sent.load(Ordering::Relaxed);
        let total = pipe_total_bytes.load(Ordering::Relaxed);
        if sent > 0 && total > 0 {
            buf = Some(compute_ring_buffer(sent, total, 0, 60_000));
            if sent >= 1024 {
                break;
            }
        }
    }
    // Make sure the reader thread finishes (it will once the response
    // is dropped).
    let _ = reader_handle.join();

    let buf = buf.expect("never received any bytes from server");
    // total_ms = 60s, sent < total → not complete.
    assert!(!buf.is_complete);
    // We should have SOME download progress.
    assert!(buf.write_offset_ms > 0, "write_offset_ms should be > 0");
    // write_offset should be < total_ms.
    assert!(buf.write_offset_ms < 60_000);
    // end_ms (= read + available) should be > 0 and < total_ms.
    assert!(buf.end_ms() > 0);
    assert!(buf.end_ms() < 60_000);
}
