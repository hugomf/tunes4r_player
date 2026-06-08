//! LiveSource — live internet stream with backward seek via ring buffer.
//!
//! Downloads a live HTTP stream into a bounded byte ring buffer (`cache_max_ms`).
//! Seeking backward reads from the cached bytes; the last N minutes are
//! always available. MP3 live streams can re-probe from mid-stream; other
//! formats may fail to probe after seek.

use crate::audio::engine::types::HttpClient;
use crate::audio::error::PlaybackError;
use crate::models::{LiveByteRing, LiveByteReader, StreamType};

use super::{Capability, ReadSeek, SourceInfo, SourceKind, StreamSource};
use log::info;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct LiveSource {
    info: SourceInfo,
    client: Arc<HttpClient>,
    url: String,
    cache_max_ms: u64,
    ring: Arc<Mutex<LiveByteRing>>,
    total_written: Arc<AtomicU64>,
    /// Monotonic timestamp recorded when `start_download()` is called.
    /// Used to compute `write_offset_ms = min(elapsed, cache_max_ms)` for
    /// the buffer poller, preventing the live buffer indicator from
    /// showing values past `cache_max_ms`.
    start_time: std::sync::Mutex<Option<std::time::Instant>>,
}

impl LiveSource {
    pub fn new(url: &str, client: Arc<HttpClient>, cache_max_ms: u64) -> Self {
        let ring = Arc::new(Mutex::new(LiveByteRing::new(cache_max_ms, 128_000)));
        let total_written = Arc::new(AtomicU64::new(0));
        Self {
            info: SourceInfo {
                kind: SourceKind::Live,
                stream_type: StreamType::Live {
                    buffer_window_bytes: 20 * 1024 * 1024,
                },
                uri: url.to_string(),
                title: Some("Live Stream".into()),
                artist: None,
                album: None,
            },
            client,
            url: url.to_string(),
            cache_max_ms,
            ring,
            total_written,
            start_time: std::sync::Mutex::new(None),
        }
    }

    pub fn cache_max_ms(&self) -> u64 {
        self.cache_max_ms
    }

    /// Returns the elapsed wall-clock time since `start_download()` was called,
    /// or 0 if the download has not started yet.
    pub fn elapsed_since_start_ms(&self) -> u64 {
        if let Ok(guard) = self.start_time.lock() {
            if let Some(instant) = *guard {
                return instant.elapsed().as_millis() as u64;
            }
        }
        0
    }

    /// Spawn a background download thread that feeds the ring buffer.
    pub fn start_download(&self) {
        // Record the start time for write_offset_ms computation.
        *self.start_time.lock().unwrap() = Some(std::time::Instant::now());

        let client = Arc::clone(&self.client);
        let url = self.url.clone();
        let ring = Arc::clone(&self.ring);
        let total_written = self.total_written.clone();

        thread::spawn(move || {
            info!("[live-source] Starting download: {}", url);
            loop {
                match client
                    .get(&url)
                    .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
                    .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
                    .header("Icy-MetaData", "0")
                    .send()
                {
                    Ok(resp) if resp.status().is_success() => {
                        info!("[live-source] Connected");
                        let mut resp = resp.take(1024 * 1024 * 1024); // safety limit
                        let mut buf = [0u8; 32768];
                        loop {
                            match resp.read(&mut buf) {
                                Ok(0) => {
                                    info!("[live-source] Stream ended, reconnecting...");
                                    break;
                                }
                                Ok(n) => {
                                    let mut ring = ring.lock().unwrap();
                                    ring.push(&buf[..n]);
                                    let tw = ring.total_written();
                                    total_written.store(tw, Ordering::Relaxed);
                                }
                                Err(e) => {
                                    info!("[live-source] Read error: {}, reconnecting...", e);
                                    break;
                                }
                            }
                        }
                    }
                    Ok(resp) => {
                        info!("[live-source] HTTP {} retrying...", resp.status());
                    }
                    Err(e) => {
                        info!("[live-source] Connection failed: {}, retrying...", e);
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        });
    }
}

impl StreamSource for LiveSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, capability: Capability) -> bool {
        matches!(capability, Capability::Seek | Capability::Download)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn open(
        &self,
        seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
        let ring = Arc::clone(&self.ring);
        let total_written = self.total_written.load(Ordering::Relaxed);

        let abs_offset = match seek_to {
            Some(ms) => {
                if ms == 0 || total_written == 0 {
                    return Err(PlaybackError::HttpStream {
                        operation: "open".into(),
                        detail: "No data cached yet".into(),
                    });
                }
                // INVARIANT: ms is absolute position from playback start, not a delta from live edge.
                // ms_ago: how far back from the live edge the seek target is.
                let ms_ago = self.cache_max_ms.saturating_sub(ms);
                let bytes_per_ms = (total_written as f64) / (self.cache_max_ms as f64);
                let byte_offset_from_live_edge = (ms_ago as f64 * bytes_per_ms) as u64;
                // Clamp so we never go below the oldest byte still in the ring.
                let ring_len = ring.lock().unwrap().len() as u64;
                let oldest_byte = if total_written > ring_len { total_written - ring_len } else { 0 };
                let abs = total_written.saturating_sub(byte_offset_from_live_edge);
                abs.max(oldest_byte)
            }
            None => total_written, // start at live edge
        };

        Ok(Box::new(LiveByteReader::new(ring, abs_offset)))
    }

    fn total_bytes(&self) -> Option<u64> {
        Some(self.cache_max_ms * 16_000 / 1000) // estimate
    }
}

#[cfg(test)]
mod tests {

    // ── BUG-2 regression: live seek byte-offset formula ───────────────

    /// The live seek offset must place the reader correctly within the
    /// ring buffer: seeking to `cache_max_ms` (the live edge) should give
    /// `total_written` (skip all cached bytes); seeking to 0 should give
    /// the oldest byte still in the ring.
    fn live_seek_byte_offset(
        seek_pos_ms: u64,
        cache_max_ms: u64,
        total_written: u64,
        ring_len_bytes: u64,
    ) -> u64 {
        let ms_ago = cache_max_ms.saturating_sub(seek_pos_ms);
        let bytes_per_ms = (total_written as f64) / (cache_max_ms as f64);
        let byte_offset_from_live_edge = (ms_ago as f64 * bytes_per_ms) as u64;
        let oldest_byte = if total_written > ring_len_bytes {
            total_written - ring_len_bytes
        } else {
            0
        };
        let abs = total_written.saturating_sub(byte_offset_from_live_edge);
        abs.max(oldest_byte)
    }

    /// Seeking to the live edge (cache_max_ms) must return total_written.
    #[test]
    fn test_live_seek_byte_offset_at_live_edge() {
        let offset = live_seek_byte_offset(30_000, 30_000, 1_000_000, 500_000);
        assert_eq!(
            offset, 1_000_000,
            "seek to live edge should skip all cached bytes"
        );
    }

    /// Seeking to 0 must return the oldest byte still in the ring.
    #[test]
    fn test_live_seek_byte_offset_at_start() {
        // total_written = 1MB, ring holds 500KB → oldest = 500KB
        let offset = live_seek_byte_offset(0, 30_000, 1_000_000, 500_000);
        assert_eq!(
            offset, 500_000,
            "seek to 0 should land on oldest byte in ring"
        );
    }

    /// Seeking halfway through the cache window must land at ~50% of
    /// the total_written.
    #[test]
    fn test_live_seek_byte_offset_halfway() {
        // 30s window, seek to 15s (halfway) → ms_ago = 15s
        // total_written = 1MB → half = 500KB
        let offset = live_seek_byte_offset(15_000, 30_000, 1_000_000, 1_000_000);
        assert!(
            offset > 450_000 && offset < 550_000,
            "seek to halfway should land ~50% through total_written, got {offset}"
        );
    }

    /// Seeking past the cache window must clamp to total_written.
    #[test]
    fn test_live_seek_byte_offset_past_live_edge() {
        let offset = live_seek_byte_offset(60_000, 30_000, 1_000_000, 1_000_000);
        assert_eq!(
            offset, 1_000_000,
            "seek past live edge should clamp to total_written"
        );
    }

    /// Seeking before the ring has data must return oldest_byte (0).
    #[test]
    fn test_live_seek_byte_offset_empty_ring() {
        let offset = live_seek_byte_offset(0, 30_000, 0, 0);
        assert_eq!(offset, 0, "empty ring seek should return 0");
    }
}
