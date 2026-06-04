//! LiveSource — live internet stream with backward seek via ring buffer.
//!
//! Downloads a live HTTP stream into a bounded byte ring buffer (`cache_max_ms`).
//! Seeking backward reads from the cached bytes; the last N minutes are
//! always available. MP3 live streams can re-probe from mid-stream; other
//! formats may fail to probe after seek.

use crate::audio::engine::types::HttpClient;
use crate::audio::error::PlaybackError;
use crate::models::{LiveByteRing, LiveByteReader, StreamType};

use super::{Capability, SourceInfo, SourceKind, StreamSource};
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
            },
            client,
            url: url.to_string(),
            cache_max_ms,
            ring,
            total_written,
        }
    }

    pub fn cache_max_ms(&self) -> u64 {
        self.cache_max_ms
    }

    /// Spawn a background download thread that feeds the ring buffer.
    pub fn start_download(&self) {
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
    ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
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
                // Estimate byte offset within the cache for the given ms position.
                let bytes_per_ms = (total_written as f64) / (self.cache_max_ms as f64);
                let byte_offset = (ms as f64 * bytes_per_ms) as u64;
                // Clamp to the beginning of the ring buffer.
                let ring_len = ring.lock().unwrap().len() as u64;
                if total_written > ring_len && total_written - ring_len > byte_offset {
                    total_written - ring_len // clamp to oldest cached byte
                } else {
                    total_written.saturating_sub(byte_offset)
                }
            }
            None => total_written, // start at live edge
        };

        Ok(Box::new(LiveByteReader::new(ring, abs_offset)))
    }

    fn total_bytes(&self) -> Option<u64> {
        Some(self.cache_max_ms * 16_000 / 1000) // estimate
    }
}
