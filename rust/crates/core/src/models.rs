//! Domain models for the audio engine
//!
//! These are pure data structures with no business logic.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::Mutex;

/// Domain model for a song
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub file_path: String,
}

impl Song {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        file_path: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            file_path: file_path.into(),
        }
    }

    pub fn with_artist(mut self, artist: impl Into<String>) -> Self {
        self.artist = artist.into();
        self
    }

    pub fn with_album(mut self, album: impl Into<String>) -> Self {
        self.album = album.into();
        self
    }

    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }
}

/// Audio playback state
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    /// Resolving stream type via HEAD/Range probe
    Connecting,
    /// Download started, waiting for enough bytes to decode
    Buffering {
        buffered_bytes: u64,
        total_bytes: Option<u64>,
    },
    /// Decoder is being initialized (parsing headers/metadata)
    Decoding,
    Playing,
    Paused,
    /// Unrecoverable error — message is human-readable
    Error(String),
}

/// Stream metadata including content-type information
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StreamMetadata {
    pub total_bytes: Option<u64>,
    pub is_seekable: bool,
    pub stream_type: StreamType,
    pub content_type: Option<String>,
}

/// Stream type enumeration
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum StreamType {
    /// HTTP file with Content-Length + Accept-Ranges — full seeking supported
    Seekable { total_bytes: u64 },
    /// Live Icecast/Shoutcast — no duration, rolling buffer only
    Live { buffer_window_bytes: usize },
}

impl StreamType {
    pub fn total_bytes(&self) -> Option<u64> {
        match self {
            StreamType::Seekable { total_bytes } => Some(*total_bytes),
            StreamType::Live { .. } => None,
        }
    }
}

impl Default for StreamType {
    fn default() -> Self {
        Self::Live {
            buffer_window_bytes: 20 * 1024 * 1024,
        }
    }
}

impl PlaybackState {
    pub fn is_playing(&self) -> bool {
        matches!(self, Self::Playing)
    }

    pub fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped)
    }

    /// Convert to integer for FFI (0=Stopped, 1=Connecting, 2=Buffering, 3=Decoding, 4=Playing, 5=Paused, 6=Error)
    pub fn to_i32(&self) -> i32 {
        match self {
            Self::Stopped => 0,
            Self::Connecting => 1,
            Self::Buffering { .. } => 2,
            Self::Decoding => 3,
            Self::Playing => 4,
            Self::Paused => 5,
            Self::Error(_) => 6,
        }
    }
}

/// Equalizer band configuration
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct EqualizerBand {
    pub frequency: f32,
    pub gain_db: f32,
}

impl EqualizerBand {
    pub fn new(frequency: f32, gain_db: f32) -> Self {
        Self { frequency, gain_db }
    }
}

/// Spectrum analysis result
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct SpectrumData {
    pub frequencies: Vec<f32>,
    pub magnitudes: Vec<f32>,
}

/// Playback position information
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct PlaybackPosition {
    pub current_ms: u64,
    pub total_ms: u64,
}

impl PlaybackPosition {
    pub fn progress_ratio(&self) -> f32 {
        if self.total_ms == 0 {
            0.0
        } else {
            self.current_ms as f32 / self.total_ms as f32
        }
    }
}

/// C-compatible event emitted by the engine and consumed via FFI.
/// `int_param` carries the event's numeric payload (state value, position ms, etc.).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct EngineEvent {
    pub event_type: i32,
    pub int_param: i64,
}

pub const ENGINE_EVENT_NONE: i32 = 0;
pub const ENGINE_EVENT_STATE_CHANGED: i32 = 1;
pub const ENGINE_EVENT_SEEK_STARTED: i32 = 2;
pub const ENGINE_EVENT_SEEK_COMPLETED: i32 = 3;
pub const ENGINE_EVENT_END_OF_STREAM: i32 = 4;
pub const ENGINE_EVENT_POSITION_RESET: i32 = 5;
pub const ENGINE_EVENT_ERROR: i32 = 6;
pub const ENGINE_EVENT_SEEK_QUEUED: i32 = 7;

/// Download buffer state for progressive streams (YouTube, HTTP).
/// Tells the UI which range of the timeline has been downloaded and
/// can therefore be seeked into immediately.
///
/// `start_ms`     — playback position corresponding to the first buffered byte
/// Sliding-window ring buffer state for progressive streams (YouTube, HTTP).
///
/// The buffer is a fixed-size window that slides along the file as playback
/// progresses. The downloader fills ahead of the playhead up to
/// `capacity_ms`; older data is discarded.
///
/// Fields (all in milliseconds, file-relative):
/// - `capacity_ms`   — fixed ring size (e.g. 30 000 = 30 s of audio)
/// - `read_offset_ms` — playhead position in the file (= current position)
/// - `write_offset_ms` — how far into the file the downloader has reached
/// - `total_ms`       — total file duration (0 until known)
/// - `is_complete`    — true once `write_offset_ms >= total_ms`
///
/// The data available to the decoder is the window
/// `[read_offset_ms, read_offset_ms + min(capacity_ms, write_offset_ms - read_offset_ms)]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdaptiveRingBuffer {
    pub capacity_ms: u64,
    pub read_offset_ms: u64,
    pub write_offset_ms: u64,
    pub total_ms: u64,
    pub is_complete: bool,
}

impl Default for AdaptiveRingBuffer {
    fn default() -> Self {
        Self {
            capacity_ms: 30_000,
            read_offset_ms: 0,
            write_offset_ms: 0,
            total_ms: 0,
            is_complete: false,
        }
    }
}

impl AdaptiveRingBuffer {
    /// Position (in file ms) of the last buffered byte relative to the
    /// file start. This is where the decoder can read up to.
    pub fn end_ms(&self) -> u64 {
        self.read_offset_ms + self.available_ms()
    }

    /// UI-safe end position: never reads as less than the playhead.
    ///
    /// Between buffer poller ticks, the playhead can advance beyond the
    /// last known buffer end (e.g. the downloader is briefly behind, or
    /// the position stream polls at 60Hz but the buffer stream at 26Hz).
    /// Exposing the raw `end_ms()` to the UI would let the "buffered"
    /// region on the slider appear to lag behind the playhead thumb.
    ///
    /// This method clamps the result to `>= read_offset_ms` so the
    /// invariant `buffered_end >= playhead` holds by construction.
    /// Callers should use this for all UI-facing values.
    pub fn end_ms_clamped(&self) -> u64 {
        self.end_ms().max(self.read_offset_ms)
    }

    /// How many ms of audio are currently in the ring buffer, clamped
    /// to `[0, capacity_ms]`. Returns `total_ms` if the file is complete.
    pub fn available_ms(&self) -> u64 {
        if self.is_complete && self.total_ms > 0 {
            // File is fully downloaded; everything from playhead to end
            // is available, regardless of ring capacity.
            self.total_ms.saturating_sub(self.read_offset_ms)
        } else if self.write_offset_ms > self.read_offset_ms {
            let filled = self.write_offset_ms - self.read_offset_ms;
            filled.min(self.capacity_ms)
        } else {
            0
        }
    }
}

/// Backwards-compat alias during the rename. New code should use
/// [`AdaptiveRingBuffer`].
pub type DownloadBuffer = AdaptiveRingBuffer;

/// A thread-safe byte ring buffer for caching live stream audio data.
///
/// Bytes are pushed sequentially and the buffer wraps around when it
/// reaches `max_bytes`. This allows seeking backward within the cached
/// window. The `total_written` counter gives the absolute byte position
/// of the last written byte, enabling offset calculations.
pub struct LiveByteRing {
    data: VecDeque<u8>,
    max_bytes: usize,
    total_written: u64,
}

impl LiveByteRing {
    pub fn new(cache_max_ms: u64, avg_bitrate: u32) -> Self {
        // Estimate max_bytes from duration × bitrate (128 kbps default).
        let bytes_per_ms = (avg_bitrate.max(128_000) / 8) as u64;
        let max_bytes = (cache_max_ms * bytes_per_ms) as usize;
        Self {
            data: VecDeque::with_capacity(max_bytes.min(100_000_000)),
            max_bytes,
            total_written: 0,
        }
    }

    /// Push bytes into the ring buffer, evicting oldest data as needed.
    pub fn push(&mut self, buf: &[u8]) {
        for &b in buf {
            if self.data.len() >= self.max_bytes {
                self.data.pop_front();
            }
            self.data.push_back(b);
        }
        self.total_written = self.total_written.wrapping_add(buf.len() as u64);
    }

    /// Total bytes written since the stream started (may wrap).
    pub fn total_written(&self) -> u64 {
        self.total_written
    }

    /// Current number of bytes in the ring buffer.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the ring buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Read bytes from `offset` bytes before the write head.
    /// `offset` is measured from `total_written`.
    pub fn read_at(&self, abs_offset: u64, buf: &mut [u8]) -> usize {
        if self.data.is_empty() {
            return 0;
        }
        let ring_len = self.data.len() as u64;
        let start = if abs_offset >= self.total_written {
            return 0; // past the write head
        } else if self.total_written - abs_offset > ring_len {
            return 0; // data was evicted
        } else {
            (ring_len - (self.total_written - abs_offset)) as usize
        };
        let available = self.data.len() - start;
        let to_read = buf.len().min(available);
        for (i, b) in self.data.range(start..start + to_read).enumerate() {
            buf[i] = *b;
        }
        to_read
    }

    /// Returns a contiguous slice of all cached bytes (for seeking).
    pub fn as_slice(&self) -> Vec<u8> {
        self.data.iter().copied().collect()
    }
}

/// A Read implementation over a shared LiveByteRing, positioned at a
/// specific absolute byte offset. Used by the decode thread after a seek.
pub struct LiveByteReader {
    ring: Arc<Mutex<LiveByteRing>>,
    read_offset: u64,
    exhausted: bool,
}

impl LiveByteReader {
    pub fn new(ring: Arc<Mutex<LiveByteRing>>, abs_offset: u64) -> Self {
        Self {
            ring,
            read_offset: abs_offset,
            exhausted: false,
        }
    }
}

impl Read for LiveByteReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.exhausted {
            return Ok(0);
        }
        let ring = self.ring.lock().unwrap();
        let n = ring.read_at(self.read_offset, buf);
        if n > 0 {
            self.read_offset += n as u64;
        } else {
            drop(ring);
            std::thread::sleep(std::time::Duration::from_millis(100));
            let ring = self.ring.lock().unwrap();
            let n = ring.read_at(self.read_offset, buf);
            if n > 0 {
                self.read_offset += n as u64;
                return Ok(n);
            }
            self.exhausted = true;
            return Ok(0);
        }
        Ok(n)
    }
}

impl Seek for LiveByteReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match pos {
            SeekFrom::Start(offset) => {
                self.read_offset = offset;
                self.exhausted = false;
                Ok(offset)
            }
            SeekFrom::Current(delta) => {
                let new_pos = self.read_offset as i64 + delta;
                if new_pos < 0 {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek before beginning",
                    ))
                } else {
                    let new_pos = new_pos as u64;
                    self.read_offset = new_pos;
                    self.exhausted = false;
                    Ok(new_pos)
                }
            }
            SeekFrom::End(_) => {
                // Ring buffer is live — end position is not fixed.
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "SeekFrom::End not supported on live stream",
                ))
            }
        }
    }
}

/// A Read wrapper that caches all bytes read into a LiveByteRing.
/// Used by `play_live_internal` to simultaneously feed the decoder
/// and fill the ring buffer for backward seek.
pub struct LiveByteCacheReader<R: Read> {
    inner: R,
    ring: Arc<Mutex<LiveByteRing>>,
}

impl<R: Read> LiveByteCacheReader<R> {
    pub fn new(inner: R, ring: Arc<Mutex<LiveByteRing>>) -> Self {
        Self { inner, ring }
    }
}

impl<R: Read> Read for LiveByteCacheReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            let mut ring = self.ring.lock().unwrap();
            ring.push(&buf[..n]);
        }
        Ok(n)
    }
}

impl<R: Read> Seek for LiveByteCacheReader<R> {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "live cache reader is not seekable",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Default ─────────────────────────────────────────────────────

    #[test]
    fn default_is_empty_with_30s_capacity() {
        let buf = AdaptiveRingBuffer::default();
        assert_eq!(buf.capacity_ms, 30_000);
        assert_eq!(buf.read_offset_ms, 0);
        assert_eq!(buf.write_offset_ms, 0);
        assert_eq!(buf.total_ms, 0);
        assert!(!buf.is_complete);
        assert_eq!(buf.available_ms(), 0);
        assert_eq!(buf.end_ms(), 0);
    }

    // ── available_ms / end_ms: download in progress ────────────────

    #[test]
    fn available_ms_partial_download_within_capacity() {
        // 60s file, playhead at 0, downloaded 20s, capacity 30s.
        // Available = min(capacity, write - read) = min(30000, 20000) = 20000.
        let buf = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 0,
            write_offset_ms: 20_000,
            total_ms: 60_000,
            is_complete: false,
        };
        assert_eq!(buf.available_ms(), 20_000);
        assert_eq!(buf.end_ms(), 20_000);
    }

    #[test]
    fn available_ms_clamps_to_capacity() {
        // Downloaded more than capacity: ring is full.
        let buf = AdaptiveRingBuffer {
            capacity_ms: 15_000,
            read_offset_ms: 0,
            write_offset_ms: 50_000,
            total_ms: 120_000,
            is_complete: false,
        };
        assert_eq!(buf.available_ms(), 15_000);
        assert_eq!(buf.end_ms(), 15_000);
    }

    #[test]
    fn available_ms_playhead_inside_buffered_region() {
        // Playhead at 10s, download reached 25s, capacity 30s.
        // Available = min(30000, 25000-10000) = 15000.
        let buf = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 10_000,
            write_offset_ms: 25_000,
            total_ms: 60_000,
            is_complete: false,
        };
        assert_eq!(buf.available_ms(), 15_000);
        assert_eq!(buf.end_ms(), 25_000);
    }

    #[test]
    fn available_ms_write_behind_read_means_empty() {
        // Should never happen in practice (write is monotonic), but
        // the code must not panic or wrap around.
        let buf = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 20_000,
            write_offset_ms: 5_000,
            total_ms: 60_000,
            is_complete: false,
        };
        assert_eq!(buf.available_ms(), 0);
    }

    // ── available_ms: complete download ─────────────────────────────

    #[test]
    fn available_ms_complete_returns_remaining_duration() {
        // File complete: available = total - read, regardless of capacity.
        let buf = AdaptiveRingBuffer {
            capacity_ms: 8_000, // tiny capacity
            read_offset_ms: 50_000,
            write_offset_ms: 120_000,
            total_ms: 120_000,
            is_complete: true,
        };
        // 120_000 - 50_000 = 70_000 (not clamped to 8_000)
        assert_eq!(buf.available_ms(), 70_000);
        assert_eq!(buf.end_ms(), 120_000);
    }

    #[test]
    fn available_ms_complete_at_end_of_file() {
        // Playhead at the end: available = 0.
        let buf = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 120_000,
            write_offset_ms: 120_000,
            total_ms: 120_000,
            is_complete: true,
        };
        assert_eq!(buf.available_ms(), 0);
        assert_eq!(buf.end_ms(), 120_000);
    }

    // ── Ring buffer sliding behavior ────────────────────────────────

    #[test]
    fn ring_slides_with_playhead() {
        // Simulate playback advancing while download continues.
        // Capacity 30s, total 120s.
        let mut buf = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 0,
            write_offset_ms: 30_000,
            total_ms: 120_000,
            is_complete: false,
        };
        // Initial: 30s buffered.
        assert_eq!(buf.available_ms(), 30_000);

        // Playhead advances 10s, download advances 10s.
        buf.read_offset_ms = 10_000;
        buf.write_offset_ms = 40_000;
        assert_eq!(buf.available_ms(), 30_000);
        assert_eq!(buf.end_ms(), 40_000);

        // Playhead advances 10s, download stalls.
        buf.read_offset_ms = 20_000;
        buf.write_offset_ms = 40_000;
        assert_eq!(buf.available_ms(), 20_000);
        assert_eq!(buf.end_ms(), 40_000);

        // Playhead advances past the downloaded region (seek-forward).
        buf.read_offset_ms = 40_000;
        buf.write_offset_ms = 40_000;
        assert_eq!(buf.available_ms(), 0);
    }

    // ── Equality ────────────────────────────────────────────────────

    #[test]
    fn two_buffers_with_same_state_are_equal() {
        let a = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 5_000,
            write_offset_ms: 20_000,
            total_ms: 60_000,
            is_complete: false,
        };
        let b = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 5_000,
            write_offset_ms: 20_000,
            total_ms: 60_000,
            is_complete: false,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn two_buffers_differing_in_one_field_are_unequal() {
        let a = AdaptiveRingBuffer::default();
        let mut b = a;
        b.write_offset_ms = 1;
        assert_ne!(a, b);
    }

    // ── FFI compatibility: #[repr(C)] ──────────────────────────────

    #[test]
    fn repr_c_field_order_matches_dart_binding() {
        // AdaptiveRingBufferStruct in tunes4r_player_ffi.dart must read
        // these fields in the same order. If you reorder fields here,
        // update the Dart side too.
        use std::mem;
        assert_eq!(mem::offset_of!(AdaptiveRingBuffer, capacity_ms), 0);
        assert_eq!(mem::offset_of!(AdaptiveRingBuffer, read_offset_ms), 8);
        assert_eq!(mem::offset_of!(AdaptiveRingBuffer, write_offset_ms), 16);
        assert_eq!(mem::offset_of!(AdaptiveRingBuffer, total_ms), 24);
        assert_eq!(mem::offset_of!(AdaptiveRingBuffer, is_complete), 32);
    }

    // ── end_ms_clamped: UI invariant (end >= read_offset) ───────────

    #[test]
    fn end_ms_clamped_never_less_than_playhead_when_downloader_lags() {
        // The downloader is briefly behind the playhead. The raw end_ms
        // would equal write_offset (clamped to read_offset), so they're
        // equal here — but if a stale buffer snapshot arrives at the UI
        // (e.g. between poller ticks), end_ms_clamped must still hold
        // the invariant.
        let buf = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 25_000, // playhead at 25s
            write_offset_ms: 25_000, // downloader just at playhead
            total_ms: 60_000,
            is_complete: false,
        };
        assert!(buf.end_ms_clamped() >= buf.read_offset_ms);
    }

    #[test]
    fn end_ms_clamped_holds_across_simulated_playback() {
        // Simulate a full playback session: downloader falls behind,
        // catches up, falls behind again. The clamped value must never
        // read as less than the playhead.
        let mut buf = AdaptiveRingBuffer {
            capacity_ms: 15_000,
            read_offset_ms: 0,
            write_offset_ms: 0,
            total_ms: 120_000,
            is_complete: false,
        };

        for tick in 0..600u64 {
            // Playhead advances ~100ms per tick (60s of playback in 600 ticks).
            let playhead: u64 = tick * 100;
            // Downloader alternates between ahead and behind the playhead.
            let downloader_ahead: u64 = match tick % 4 {
                0 | 1 => playhead + 2_000, // ahead
                2 => playhead + 500,        // slightly ahead
                _ => playhead.saturating_sub(500), // behind
            };
            buf.read_offset_ms = playhead.min(buf.total_ms);
            buf.write_offset_ms = downloader_ahead.max(buf.read_offset_ms).min(buf.total_ms);

            // The UI-facing invariant.
            assert!(
                buf.end_ms_clamped() >= buf.read_offset_ms,
                "tick {}: end_ms_clamped={} < read_offset_ms={}",
                tick,
                buf.end_ms_clamped(),
                buf.read_offset_ms
            );
        }
    }

    #[test]
    fn end_ms_clamped_equals_end_ms_when_downloader_is_ahead() {
        // Normal streaming: downloader well ahead of playhead.
        let buf = AdaptiveRingBuffer {
            capacity_ms: 30_000,
            read_offset_ms: 5_000,
            write_offset_ms: 20_000,
            total_ms: 60_000,
            is_complete: false,
        };
        assert_eq!(buf.end_ms_clamped(), buf.end_ms());
        assert_eq!(buf.end_ms_clamped(), 20_000);
    }

    // ── LiveByteRing + LiveByteReader: seek-in-ring tests ──
    // These verify that seeking within the ring buffer returns the
    // correct data without re-fetching from the network.

    #[test]
    fn ring_read_at_returns_written_data() {
        let mut ring = LiveByteRing::new(30_000, 128_000);
        ring.push(b"Hello, LiveByteRing!");
        let mut out = [0u8; 20];
        let n = ring.read_at(0, &mut out);
        assert_eq!(n, 20, "should read 20 bytes, got {}", n);
        assert_eq!(&out[..n], b"Hello, LiveByteRing!");
    }

    #[test]
    fn ring_read_at_offset_skips_bytes() {
        let mut ring = LiveByteRing::new(30_000, 128_000);
        ring.push(b"abcdefghijklmnopqrstuvwxyz");
        let mut out = [0u8; 10];
        let n = ring.read_at(5, &mut out);
        assert_eq!(n, 10, "should read 10 bytes at offset 5, got {}", n);
        assert_eq!(&out[..n], b"fghijklmno");
    }

    #[test]
    fn ring_read_at_past_total_returns_zero() {
        let mut ring = LiveByteRing::new(30_000, 128_000);
        ring.push(b"small data");
        let mut out = [0u8; 10];
        let n = ring.read_at(100, &mut out);
        assert_eq!(n, 0, "past total should return 0");
    }

    #[test]
    fn ring_read_at_evicted_data_returns_zero() {
        let mut ring = LiveByteRing::new(1, 128_000); // ring holds ~16KB
        // Fill with 32KB — older data gets evicted.
        for i in 0..32_000u16 {
            ring.push(&[(i % 256) as u8]);
        }
        // Ring holds only the last ~16KB. Offset 0 is well past eviction.
        let mut out = [0u8; 1];
        let n = ring.read_at(0, &mut out);
        assert_eq!(n, 0, "evicted data should return 0");
    }

    #[test]
    fn ring_wraparound_read_at_earliest_cached_byte() {
        let mut ring = LiveByteRing::new(1, 128_000); // ring holds ~16KB
        // Write 32KB; ring holds only the last ~16KB.
        for i in 0..32_000u16 {
            ring.push(&[(i % 256) as u8]);
        }
        // Ring holds bytes [16000..32000). Earliest readable offset is ~16000.
        // total_written = 32000, ring_len = 16000, oldest = 16000.
        let mut out = [0u8; 1];
        let n = ring.read_at(16_000, &mut out);
        assert_eq!(n, 1, "earliest cached byte should be readable");
        assert_eq!(out[0], (16_000 % 256) as u8, "byte should be 0x40 (64)");
    }

    #[test]
    fn live_byte_reader_reads_sequentially_after_seek() {
        // Fill ring with known data.
        let ring = Arc::new(Mutex::new(LiveByteRing::new(30_000, 128_000)));
        {
            let mut r = ring.lock().unwrap();
            let data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
            r.push(&data);
        }

        // Seek to offset 500 and verify we read from there.
        let mut reader = LiveByteReader::new(ring, 500);
        let mut buf = [0u8; 10];
        let n = reader.read(&mut buf).expect("read should succeed");
        assert_eq!(n, 10, "should read 10 bytes, got {}", n);
        // Data is (0..1000).map(|i| (i % 256) as u8),
        // so offset 500 → byte 244, then 245, 246...
        let expected: Vec<u8> = (500u16..510).map(|i| (i % 256) as u8).collect();
        assert_eq!(buf.to_vec(), expected,
            "read_at(500, 10) should match source at offset 500");
    }

    #[test]
    fn live_byte_reader_reads_past_ring_exhaustion() {
        // Reader should return Ok(0) when ring is empty.
        let ring = Arc::new(Mutex::new(LiveByteRing::new(30_000, 128_000)));
        let mut reader = LiveByteReader::new(ring, 0);
        let mut buf = [0u8; 10];
        let n = reader.read(&mut buf).expect("read on empty ring");
        // First read sleeps 100ms and retries; if still empty, returns 0.
        assert_eq!(n, 0, "empty ring should return 0 after retry");
    }

    #[test]
    fn live_byte_reader_sequential_reads_advance_position() {
        let ring = Arc::new(Mutex::new(LiveByteRing::new(30_000, 128_000)));
        {
            let mut r = ring.lock().unwrap();
            r.push(b"0123456789ABCDEF");
        }

        let mut reader = LiveByteReader::new(ring, 5);
        let mut buf = [0u8; 5];
        reader.read(&mut buf).expect("first read");
        assert_eq!(&buf, b"56789");

        // Second read should pick up where left off.
        let mut buf2 = [0u8; 5];
        reader.read(&mut buf2).expect("second read");
        assert_eq!(&buf2, b"ABCDE");
    }

    #[test]
    fn live_byte_seek_and_read_after_cache_reader_writes() {
        // Simulate real live-stream flow:
        // 1. HTTP data comes through LiveByteCacheReader into ring.
        // 2. Seek creates LiveByteReader at offset.
        // 3. Verify seek reads correct data from ring.
        let ring = Arc::new(Mutex::new(LiveByteRing::new(5, 128_000))); // tiny ring

        // Step 1: Write data via a cursor (simulates HTTP download).
        // Only need enough data to exceed ring capacity.
        let source_data: Vec<u8> = (0..200_000).map(|i| (i % 256) as u8).collect();
        let cursor = std::io::Cursor::new(source_data.clone());
        let mut cache_reader = LiveByteCacheReader::new(cursor, ring.clone());
        let mut tmp = [0u8; 8192];
        while cache_reader.read(&mut tmp).expect("cache read") > 0 {}

        // Verify ring has data.
        assert!(
            ring.lock().unwrap().len() > 0,
            "ring should have data after cache reader finishes"
        );

        // Step 2: Seek to a position near the end of cached data.
        // The ring holds only the last ~80KB of the 200KB source.
        // Seek to offset 150KB (within the cached window for a 5s ring).
        let seek_byte = 150_000u64;
        let mut reader = LiveByteReader::new(ring.clone(), seek_byte);

        // Step 3: Read bytes from seek position and verify.
        let mut read_buf = [0u8; 100];
        let n = reader.read(&mut read_buf).expect("seek read");
        assert!(n > 0, "should read data after seek, got {} bytes", n);

        // Verify the data matches the source at the same offset.
        for i in 0..n {
            assert_eq!(
                read_buf[i], source_data[seek_byte as usize + i],
                "byte mismatch at offset {} (seek {})", i, seek_byte
            );
        }
    }
}
