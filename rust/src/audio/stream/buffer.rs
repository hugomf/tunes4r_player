use log::{debug, info};

use crate::models::StreamType;
use parking_lot::Mutex;
use std::sync::Arc;

const MIN_READ_BUFFER: usize = 64 * 1024;

pub const DETECT_HEAD_TIMEOUT_MS: u64 = 10_000;
pub const DETECT_RANGE_TIMEOUT_MS: u64 = 10_000;
pub const DETECT_MAX_RETRIES: u32 = 3;
pub const DETECT_RETRY_DELAY_MS: u64 = 500;
pub const PREFILL_SEEKABLE_BYTES: usize = 128 * 1024;
pub const PREFILL_LIVE_BYTES: usize = 128 * 1024;
pub const PREFILL_SEEKABLE_TIMEOUT_MS: u128 = 15_000;
pub const PREFILL_LIVE_TIMEOUT_MS: u128 = 10_000;
pub const READ_WAIT_MS: u64 = 1;
pub const LIVE_MIN_READ_BYTES: usize = 4096;
pub const LIVE_MAX_LAG_BYTES: usize = 64 * 1024;
pub const LIVE_KEEP_AHEAD_BYTES: usize = 512 * 1024;
pub const LIVE_RECONNECT_DELAY_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkQuality {
    Excellent,
    Good,
    Moderate,
    Poor,
    Unknown,
}

pub const NETWORK_QUALITY_EXCELLENT_THRESHOLD: f64 = 2_000_000.0;
pub const NETWORK_QUALITY_GOOD_THRESHOLD: f64 = 1_000_000.0;
pub const NETWORK_QUALITY_MODERATE_THRESHOLD: f64 = 500_000.0;
pub const NETWORK_QUALITY_POOR_THRESHOLD: f64 = 100_000.0;

#[derive(Debug, Clone)]
pub struct BufferConfig {
    pub prefill_bytes: usize,
    pub prefill_timeout_ms: u128,
    pub live_buffer_bytes: usize,
    pub max_buffer_bytes: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            prefill_bytes: PREFILL_SEEKABLE_BYTES,
            prefill_timeout_ms: PREFILL_SEEKABLE_TIMEOUT_MS,
            live_buffer_bytes: 20 * 1024 * 1024,
            max_buffer_bytes: 50 * 1024 * 1024,
        }
    }
}

pub struct AdaptiveBuffer {
    config: BufferConfig,
    samples: Vec<BufferSample>,
    network_quality: NetworkQuality,
}

#[derive(Debug, Clone, Copy)]
struct BufferSample {
    bytes: usize,
    elapsed_ms: u128,
}

impl AdaptiveBuffer {
    pub fn new() -> Self {
        Self {
            config: BufferConfig::default(),
            samples: Vec::with_capacity(10),
            network_quality: NetworkQuality::Unknown,
        }
    }

    pub fn record_sample(&mut self, bytes: usize, elapsed_ms: u128) {
        self.samples.push(BufferSample { bytes, elapsed_ms });
        if self.samples.len() > 10 {
            self.samples.remove(0);
        }
        self.network_quality = self.assess_network_quality();
        self.update_config();
    }

    fn assess_network_quality(&self) -> NetworkQuality {
        if self.samples.is_empty() {
            return NetworkQuality::Unknown;
        }

        let avg_speed = self
            .samples
            .iter()
            .map(|s| s.bytes as f64 / (s.elapsed_ms as f64 / 1000.0).max(1.0))
            .sum::<f64>()
            / self.samples.len() as f64;

        match avg_speed {
            s if s > NETWORK_QUALITY_EXCELLENT_THRESHOLD => NetworkQuality::Excellent,
            s if s > NETWORK_QUALITY_GOOD_THRESHOLD => NetworkQuality::Good,
            s if s > NETWORK_QUALITY_MODERATE_THRESHOLD => NetworkQuality::Moderate,
            s if s > NETWORK_QUALITY_POOR_THRESHOLD => NetworkQuality::Poor,
            _ => NetworkQuality::Unknown,
        }
    }

    fn update_config(&mut self) {
        let (prefill_mult, timeout_mult, buffer_mult) = match self.network_quality {
            NetworkQuality::Excellent => (0.5, 0.5, 0.5),
            NetworkQuality::Good => (0.75, 0.75, 0.75),
            NetworkQuality::Moderate => (1.0, 1.0, 1.0),
            NetworkQuality::Poor => (2.0, 2.0, 2.0),
            NetworkQuality::Unknown => (1.0, 1.0, 1.0),
        };

        self.config.prefill_bytes = (PREFILL_SEEKABLE_BYTES as f64 * prefill_mult) as usize;
        self.config.prefill_timeout_ms =
            (PREFILL_SEEKABLE_TIMEOUT_MS as f64 * timeout_mult) as u128;
        self.config.max_buffer_bytes = ((50 * 1024 * 1024) as f64 * buffer_mult) as usize;
    }

    pub fn config(&self) -> &BufferConfig {
        &self.config
    }

    pub fn network_quality(&self) -> NetworkQuality {
        self.network_quality
    }
}

impl Default for AdaptiveBuffer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StreamBuffer {
    pub data: Vec<u8>,
    pub base_offset: usize,
    pub download_complete: bool,
    pub total_bytes: Option<u64>,
    pub stream_type: StreamType,
    pub generation: u64,
    pub read_offset: usize,
    trim_count: usize,
    total_trimmed_bytes: usize,
    last_health_log: std::time::Instant,
}

impl StreamBuffer {
    pub fn new(stream_type: StreamType) -> Self {
        Self {
            data: Vec::new(),
            base_offset: 0,
            download_complete: false,
            total_bytes: None,
            stream_type,
            generation: 0,
            read_offset: 0,
            trim_count: 0,
            total_trimmed_bytes: 0,
            last_health_log: std::time::Instant::now(),
        }
    }

    fn buffer_window_bytes(&self) -> usize {
        match self.stream_type {
            StreamType::Live {
                buffer_window_bytes,
            } => buffer_window_bytes,
            StreamType::Seekable { .. } => usize::MAX,
        }
    }

    pub fn set_total_bytes(&mut self, total: u64) {
        self.total_bytes = Some(total);
    }

    pub fn window_start(&self) -> usize {
        self.base_offset
    }
    pub fn window_end(&self) -> usize {
        self.base_offset + self.data.len()
    }
    pub fn available_range(&self) -> (usize, usize) {
        (self.window_start(), self.window_end())
    }
    pub fn is_download_complete(&self) -> bool {
        self.download_complete
    }

    pub fn bytes_available(&self) -> usize {
        self.data.len()
    }

    #[allow(dead_code)]
    pub fn ensure_minimum(&mut self) -> usize {
        self.data.len().max(MIN_READ_BUFFER)
    }

    pub fn clamp_read_pos(&self, pos: usize) -> usize {
        pos.max(self.base_offset)
    }

    pub fn append(&mut self, chunk: &[u8]) {
        let chunk_size = chunk.len();

        self.data.extend_from_slice(chunk);
        self.trim_if_live_with_pos(self.read_offset);

        if let StreamType::Live {
            buffer_window_bytes,
        } = self.stream_type
        {
            if self.data.len() > buffer_window_bytes * 2 {
                info!(
                    "[buffer] WARNING: buffer exceeded 2x window: {} / {} bytes",
                    self.data.len(),
                    buffer_window_bytes
                );
            }

            if chunk_size > 65536 {
                debug!("[buffer] Large chunk received: {} bytes", chunk_size);
            }
        }
    }

    pub fn append_and_trim(&mut self, chunk: &[u8], reader_pos: usize) {
        let chunk_size = chunk.len();

        self.data.extend_from_slice(chunk);

        self.trim_if_live_with_pos(reader_pos);

        let window_end = self.window_end();
        let window_start = self.window_start();

        if reader_pos > window_end {
            debug!(
                "[buffer] WARNING: reader_pos {} beyond window_end {}, resetting to {}",
                reader_pos, window_end, window_start
            );
            self.read_offset = window_start;
        }

        if let StreamType::Live {
            buffer_window_bytes,
        } = self.stream_type
        {
            if self.data.len() > buffer_window_bytes * 2 {
                info!(
                    "[buffer] WARNING: buffer exceeded 2x window: {} / {} bytes",
                    self.data.len(),
                    buffer_window_bytes
                );
            }

            if chunk_size > 65536 {
                debug!("[buffer] Large chunk received: {} bytes", chunk_size);
            }
        }
    }

    fn trim_if_live_with_pos(&mut self, reader_pos: usize) {
        let buffer_window_bytes = match self.stream_type {
            StreamType::Live {
                buffer_window_bytes,
            } => buffer_window_bytes,
            StreamType::Seekable { .. } => return,
        };

        if self.data.len() > buffer_window_bytes {
            let excess = self.data.len() - buffer_window_bytes;
            let read_ahead = reader_pos.saturating_sub(self.base_offset);
            let safe_drop = read_ahead.min(excess);

            if safe_drop > 0 {
                self.trim_count += 1;
                self.total_trimmed_bytes += safe_drop;

                if self.trim_count % 100 == 0 {
                    info!(
                        "[buffer] TRIM #{}: dropping {} bytes (buffer: {} -> {})",
                        self.trim_count,
                        safe_drop,
                        self.data.len(),
                        self.data.len() - safe_drop
                    );
                }

                self.data.drain(..safe_drop);
                self.base_offset += safe_drop;

                if self.read_offset >= self.base_offset {
                    if self.read_offset > self.window_end() {
                        self.read_offset = self.window_end();
                    }
                } else {
                    self.read_offset = self.base_offset;
                }

                if self.trim_count % 1000 == 0 {
                    info!(
                        "[buffer] TRIM STATS: total={} trims, {} bytes trimmed, avg={} bytes/trim",
                        self.trim_count,
                        self.total_trimmed_bytes,
                        self.total_trimmed_bytes / self.trim_count.max(1)
                    );
                }
            } else if self.data.len() > buffer_window_bytes * 3 {
                let emergency_drop = self.data.len() - buffer_window_bytes;
                debug!("[buffer] EMERGENCY trim: dropping {} bytes", emergency_drop);
                self.data.drain(..emergency_drop);
                self.base_offset += emergency_drop;
                self.read_offset = self.base_offset;
            }
        }

        let now = std::time::Instant::now();
        if now.duration_since(self.last_health_log).as_secs() >= 30 {
            let gap = self.read_offset.saturating_sub(self.base_offset);
            let buffer_health = if !self.data.is_empty() {
                (gap as f32 / self.data.len() as f32) * 100.0
            } else {
                0.0
            };

            info!(
                "[buffer] HEALTH: size={} bytes, window={} bytes, pos={} ({}%), avail={} bytes",
                self.data.len(),
                buffer_window_bytes,
                gap,
                buffer_health,
                self.window_end() - self.read_offset
            );
            self.last_health_log = now;
        }
    }

    pub fn trim_if_live(&mut self) {
        self.trim_if_live_with_pos(self.read_offset);
    }

    pub fn read_at(&self, position: usize, buf: &mut [u8]) -> usize {
        if position < self.base_offset {
            return 0;
        }
        let local = position - self.base_offset;
        if local >= self.data.len() {
            return 0;
        }
        let n = buf.len().min(self.data.len() - local);
        buf[..n].copy_from_slice(&self.data[local..local + n]);
        n
    }

    pub fn reset_for_seek(&mut self, new_base: usize) -> u64 {
        if new_base > self.base_offset {
            let drop_n = (new_base - self.base_offset).min(self.data.len());
            self.data.drain(..drop_n);
        } else if new_base < self.base_offset {
            self.data.clear();
        }
        self.base_offset = new_base;
        self.read_offset = new_base;
        self.download_complete = false;
        self.generation += 1;
        self.generation
    }
}

pub struct DownloadHandle {
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for DownloadHandle {
    fn drop(&mut self) {
        drop(self.stop_tx.take());
    }
}

use crate::audio::http::get_runtime;
use futures_util::StreamExt;
use std::time::Duration;

pub fn spawn_download(
    client: Arc<reqwest::Client>,
    url: String,
    start_byte: usize,
    buffer: Arc<Mutex<StreamBuffer>>,
    generation: u64,
) -> DownloadHandle {
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let is_live_stream = {
        let buf = buffer.lock();
        matches!(buf.stream_type, StreamType::Live { .. })
    };

    get_runtime().spawn(async move {
        let mut current_start = start_byte;

        loop {
            debug!("[download] starting GET request to: {}", url);

            let mut req = client
                .get(&url)
                .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
                .header("Icy-MetaData", "0")
                .header("Connection", "close");

            if current_start > 0 && !is_live_stream {
                req = req.header("Range", format!("bytes={}-", current_start));
            }

            let download_result = {
                let req = req;
                tokio::select! {
                    _ = &mut stop_rx => {
                        debug!("[download] stop signal, aborting");
                        return;
                    }
                    result = req.send() => result
                }
            };

            match download_result {
                Err(e) => {
                    if e.is_timeout() {
                        debug!("[audio] download timed out: {:?}", e);
                    } else if e.is_connect() {
                        debug!("[audio] download connection failed: {:?}", e);
                    } else if e.is_request() {
                        debug!("[audio] download request error: {:?}", e);
                    } else {
                        debug!("[audio] download error: {:?}", e);
                    }
                    if !is_live_stream {
                        buffer.lock().download_complete = true;
                    }
                }
                Ok(resp) => {
                    debug!("[download] GET response status: {}", resp.status());
                    if let Some(cl) = resp.content_length() {
                        debug!("[download] Content-Length: {}", cl);
                    }

                    if current_start > 0 && !is_live_stream && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                        if resp.status().is_success() {
                            debug!("[audio] Range request ignored (got {} instead of 206), resetting buffer to byte 0", resp.status());
                            let mut buf = buffer.lock();
                            buf.reset_for_seek(0);
                        } else {
                            debug!("[audio] Range request failed with status: {}", resp.status());
                            buffer.lock().download_complete = true;
                            return;
                        }
                    }

                    let mut stream = resp.bytes_stream();

                    loop {
                        tokio::select! {
                            _ = &mut stop_rx => {
                                debug!("[download] stop signal, aborting");
                                return;
                            }
                            chunk = stream.next() => {
                                match chunk {
                                    Some(Ok(data)) => {
                                        let reader_pos = {
                                            let b = buffer.lock();
                                            if b.generation != generation {
                                                debug!("[download] stale generation, aborting");
                                                return;
                                            }
                                            b.read_offset
                                        };

                                        let window_bytes = {
                                            let b = buffer.lock();
                                            b.buffer_window_bytes()
                                        };

                                        let can_append = {
                                            let buf = buffer.lock();
                                            buf.data.len() + data.len() <= window_bytes * 2
                                        };

                                        if !can_append {
                                            debug!("[download] buffer full, skipping chunk");
                                            continue;
                                        }

                                        {
                                            let mut buf = buffer.lock();
                                            buf.append_and_trim(&data, reader_pos);
                                        }
                                        current_start = {
                                            let b = buffer.lock();
                                            b.window_end()
                                        };
                                    }
                                    Some(Err(e)) => {
                                        debug!("[audio] download read error: {}", e);
                                        if !is_live_stream {
                                            buffer.lock().download_complete = true;
                                        }
                                        break;
                                    }
                                    None => {
                                        debug!("[download] stream complete - server closed connection");
                                        if is_live_stream {
                                            break;
                                        }
                                        buffer.lock().download_complete = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !is_live_stream {
                return;
            }
            debug!("[download] retrying live stream in {}ms", LIVE_RECONNECT_DELAY_MS);
            tokio::time::sleep(Duration::from_millis(LIVE_RECONNECT_DELAY_MS)).await;
        }
    });

    DownloadHandle {
        stop_tx: Some(stop_tx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::StreamType;

    #[test]
    fn test_download_complete_default() {
        let buf = StreamBuffer::new(StreamType::Seekable { total_bytes: 1000 });
        assert!(!buf.download_complete);
    }

    #[test]
    fn test_download_complete_settable() {
        let mut buf = StreamBuffer::new(StreamType::Seekable { total_bytes: 1000 });
        assert!(!buf.download_complete);
        buf.download_complete = true;
        assert!(buf.download_complete);
    }

    #[test]
    fn test_live_buffer_trims() {
        let mut buf = StreamBuffer::new(StreamType::Live {
            buffer_window_bytes: 100,
        });
        buf.read_offset = 100;
        buf.base_offset = 0;
        buf.append_and_trim(&[1u8; 200], 100);
        assert_eq!(buf.bytes_available(), 100);
        assert_eq!(buf.base_offset, 100);
    }

    #[test]
    fn test_seekable_buffer_no_trim() {
        let mut buf = StreamBuffer::new(StreamType::Seekable { total_bytes: 1000 });
        buf.append_and_trim(&[1u8; 200], 0);
        assert_eq!(buf.bytes_available(), 200);
        assert_eq!(buf.base_offset, 0);
    }

    #[test]
    fn test_clamp_read_pos() {
        let buf = StreamBuffer::new(StreamType::Live {
            buffer_window_bytes: 100,
        });
        assert_eq!(buf.clamp_read_pos(50), 50);
        assert_eq!(buf.clamp_read_pos(0), 0);

        let mut buf = StreamBuffer::new(StreamType::Live {
            buffer_window_bytes: 100,
        });
        buf.base_offset = 100;
        assert_eq!(buf.clamp_read_pos(50), 100);
        assert_eq!(buf.clamp_read_pos(150), 150);
    }
}