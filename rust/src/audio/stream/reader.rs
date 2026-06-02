use log::debug;

use crate::audio::stream::buffer::{spawn_download, StreamBuffer};
use crate::models::StreamType;

use parking_lot::Mutex;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ReadDiagnostics {
    total_bytes_read: usize,
    zero_read_count: usize,
    silence_injected_count: usize,
    trim_jump_count: usize,
    last_log_time: Instant,
}

impl Default for ReadDiagnostics {
    fn default() -> Self {
        Self {
            total_bytes_read: 0,
            zero_read_count: 0,
            silence_injected_count: 0,
            trim_jump_count: 0,
            last_log_time: Instant::now(),
        }
    }
}

impl ReadDiagnostics {
    #[allow(dead_code)]
    fn log_if_needed(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_log_time).as_secs() >= 5 {
            debug!(
                "[diag] reads: {} bytes, zeros: {}, silence: {}, jumps: {}",
                self.total_bytes_read,
                self.zero_read_count,
                self.silence_injected_count,
                self.trim_jump_count,
            );
            self.last_log_time = now;
        }
    }
}

pub struct SeekableStreamReader {
    pub url: String,
    pub client: Arc<reqwest::Client>,
    pub buffer: Arc<Mutex<StreamBuffer>>,
    pub read_pos: usize,
    pub download: Option<crate::audio::stream::buffer::DownloadHandle>,
    diag: ReadDiagnostics,
}

impl SeekableStreamReader {
    pub fn new(url: String, buffer: StreamBuffer, client: Arc<reqwest::Client>) -> Self {
        Self {
            url,
            client,
            buffer: Arc::new(Mutex::new(buffer)),
            read_pos: 0,
            download: None,
            diag: ReadDiagnostics::default(),
        }
    }

    pub fn start_download_from(&mut self, start_byte: usize) {
        println!("[download] starting download from byte {}", start_byte);
        self.download = None;

        let generation = self.buffer.lock().reset_for_seek(start_byte);
        self.read_pos = start_byte;

        self.download = Some(spawn_download(
            Arc::clone(&self.client),
            self.url.clone(),
            start_byte,
            Arc::clone(&self.buffer),
            generation,
        ));
    }

    pub fn buffer_info(&self) -> (usize, usize, bool) {
        let buf = self.buffer.lock();
        let (s, e) = buf.available_range();
        (s, e, buf.is_download_complete())
    }
}

impl Read for SeekableStreamReader {
    fn read(&mut self, out_buf: &mut [u8]) -> std::io::Result<usize> {
        let mut buffer = self.buffer.lock();
        let is_live = matches!(buffer.stream_type, StreamType::Live { .. });

        let window_start = buffer.window_start();
        let window_end = buffer.window_end();

        // CRITICAL: Always ensure we're within valid range
        if self.read_pos < window_start {
            // We're behind, jump forward
            self.read_pos = window_start;
            buffer.read_offset = window_start;
        }

        // Calculate available data
        let available = if self.read_pos < window_end {
            window_end - self.read_pos
        } else {
            0
        };

        // If we have data, ALWAYS read it first
        if available > 0 {
            let to_read = out_buf.len().min(available);
            let copied = buffer.read_at(self.read_pos, &mut out_buf[..to_read]);
            if copied > 0 {
                self.read_pos += copied;
                buffer.read_offset = self.read_pos;
                self.diag.total_bytes_read += copied;

                // Log less frequently
                if self.diag.total_bytes_read % 65536 == 0 {
                    debug!(
                        "[reader] Read {} bytes total, current pos: {}",
                        self.diag.total_bytes_read, self.read_pos
                    );
                }
                return Ok(copied);
            }
        }

        // No data available - handle underrun for live streams
        if is_live {
            // Instead of injecting silence, wait a tiny bit for data
            // But don't block too long
            drop(buffer); // Release lock before sleeping
            std::thread::sleep(std::time::Duration::from_millis(5));

            // Try one more time with fresh lock
            let mut buffer = self.buffer.lock();
            let window_start = buffer.window_start();
            let window_end = buffer.window_end();

            if self.read_pos < window_start {
                self.read_pos = window_start;
                buffer.read_offset = window_start;
            }

            let available = if self.read_pos < window_end {
                window_end - self.read_pos
            } else {
                0
            };

            if available > 0 {
                let to_read = out_buf.len().min(available);
                let copied = buffer.read_at(self.read_pos, &mut out_buf[..to_read]);
                if copied > 0 {
                    self.read_pos += copied;
                    buffer.read_offset = self.read_pos;
                    self.diag.total_bytes_read += copied;
                    return Ok(copied);
                }
            }

            // Still no data? Inject silence as last resort
            let silence_len = out_buf.len().min(1152); // MP3 frame size
            out_buf[..silence_len].fill(0);
            self.diag.silence_injected_count += 1;

            if self.diag.silence_injected_count <= 10
                || self.diag.silence_injected_count % 1000 == 0
            {
                debug!(
    "[reader] UNDERRUN #{}: injecting {}B silence (buffer: {}B, pos: {}/{}, available: {})",
    self.diag.silence_injected_count,
                    silence_len,
                    window_end - window_start,
                    self.read_pos,
                    window_end,
                    available
);
            }
            return Ok(silence_len);
        }

        // For seekable streams with no data, return 0 (EOF or retry)
        if buffer.is_download_complete() {
            Ok(0)
        } else {
            // Not complete yet, try again later
            std::thread::sleep(std::time::Duration::from_millis(10));
            Ok(0)
        }
    }
}

impl Seek for SeekableStreamReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let (buf_start, buf_end, _) = self.buffer_info();
        let total_bytes = self.buffer.lock().total_bytes;

        debug!(
            "[seek] requested {:?}, buffer: [{}, {}), total_bytes: {:?}",
            pos, buf_start, buf_end, total_bytes
        );

        let target = match pos {
            SeekFrom::Start(n) => n as usize,
            SeekFrom::Current(n) => {
                let p = self.read_pos as i64 + n;
                if p < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek before start",
                    ));
                }
                p as usize
            }
            SeekFrom::End(n) => {
                let total = total_bytes.ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek from end: size unknown",
                    )
                })?;
                let p = total as i64 + n;
                if p < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek before start",
                    ));
                }
                p as usize
            }
        };

        let stream_type = self.buffer.lock().stream_type.clone();

        match stream_type {
            StreamType::Live { .. } => {
                if target >= buf_start && target <= buf_end {
                    self.read_pos = target;
                    self.buffer.lock().read_offset = target;
                } else if target < buf_start {
                    self.read_pos = buf_start;
                    self.buffer.lock().read_offset = buf_start;
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "cannot seek past live edge",
                    ));
                }
                Ok(self.read_pos as u64)
            }
            StreamType::Seekable { .. } => {
                if target >= buf_start && target <= buf_end {
                    debug!(
                        "[seek] target {} in buffer [{}, {}), moving cursor",
                        target, buf_start, buf_end
                    );
                    self.read_pos = target;
                    self.buffer.lock().read_offset = target;
                    Ok(target as u64)
                } else {
                    debug!(
                        "[seek] target {} outside buffer [{}, {}), starting new download from {}",
                        target, buf_start, buf_end, target
                    );
                    self.start_download_from(target);

                    const MIN_SEEK_PREFILL: usize = 8 * 1024;
                    let deadline = std::time::Instant::now();
                    let timeout = Duration::from_millis(2000);

                    loop {
                        let (start, end, complete) = self.buffer_info();
                        let buffered = end.saturating_sub(start);

                        if buffered >= MIN_SEEK_PREFILL || complete {
                            debug!(
                                "[seek] prefill complete: {} bytes available at target {}",
                                buffered, target
                            );
                            break;
                        }

                        if deadline.elapsed() >= timeout {
                            debug!(
                                "[seek] prefill timeout after {:?}, proceeding with {} bytes",
                                timeout, buffered
                            );
                            break;
                        }

                        std::thread::sleep(Duration::from_millis(10));
                    }

                    self.read_pos = target;
                    self.buffer.lock().read_offset = target;
                    Ok(target as u64)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::StreamType;
    use std::sync::Arc;

    #[test]
    fn test_reader_initial_state() {
        let reader = SeekableStreamReader::new(
            "http://example.com/test.mp3".to_string(),
            StreamBuffer::new(StreamType::Seekable { total_bytes: 1000 }),
            Arc::new(reqwest::Client::new()),
        );
        assert_eq!(reader.read_pos, 0);
    }

    #[test]
    fn test_reader_buffer_info() {
        let reader = SeekableStreamReader::new(
            "http://example.com/test.mp3".to_string(),
            StreamBuffer::new(StreamType::Seekable { total_bytes: 1000 }),
            Arc::new(reqwest::Client::new()),
        );
        let (start, end, complete) = reader.buffer_info();
        assert_eq!(start, 0);
        assert_eq!(end, 0);
        assert!(!complete);
    }
}
