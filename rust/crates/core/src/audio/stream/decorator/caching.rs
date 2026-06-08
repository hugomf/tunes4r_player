use crate::audio::error::PlaybackError;
use crate::audio::stream::source::{Capability, ReadSeek, SourceInfo, StreamSource};
use log::{debug, info, warn};
use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Thread-safe byte ring buffer for caching stream data.
///
/// Stores the most recent `max_bytes` bytes of the stream. New bytes are
/// pushed with `push()`, oldest bytes are evicted when capacity is reached.
/// `read_at()` reads from an absolute byte offset, returning 0 if the
/// offset has been evicted or is past the write head.
pub(crate) struct ByteCache {
    data: VecDeque<u8>,
    max_bytes: usize,
    total_written: u64,
}

impl ByteCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            data: VecDeque::with_capacity(max_bytes.min(100_000_000)),
            max_bytes,
            total_written: 0,
        }
    }

    fn push(&mut self, buf: &[u8]) {
        let room = self.max_bytes.saturating_sub(self.data.len());
        if buf.len() > room {
            let excess = buf.len() - room;
            self.data.drain(..excess.min(self.data.len()));
        }
        self.data.extend(buf);
        self.total_written = self.total_written.wrapping_add(buf.len() as u64);
    }

    fn total_written(&self) -> u64 {
        self.total_written
    }

    fn read_at(&self, abs_offset: u64, buf: &mut [u8]) -> usize {
        if self.data.is_empty() {
            return 0;
        }
        let ring_len = self.data.len() as u64;
        if abs_offset >= self.total_written {
            return 0;
        }
        if self.total_written - abs_offset > ring_len {
            return 0;
        }
        let start = (ring_len - (self.total_written - abs_offset)) as usize;
        let available = self.data.len() - start;
        let to_read = buf.len().min(available);
        for (i, b) in self.data.range(start..start + to_read).enumerate() {
            buf[i] = *b;
        }
        to_read
    }

    fn clear(&mut self) {
        self.data.clear();
        self.total_written = 0;
    }

    fn is_offset_cached(&self, abs_offset: u64) -> bool {
        if self.data.is_empty() {
            return false;
        }
        let ring_len = self.data.len() as u64;
        abs_offset < self.total_written && self.total_written - abs_offset <= ring_len
    }
}

/// A `Read`+`Seek` impl that reads from a `ByteCache`.
///
/// Blocks until data is available or the stream is exhausted. The
/// background filler thread downloads into the cache concurrently;
/// `read()` polls every 200ms until data arrives or EOF is signaled.
struct CachedReader {
    cache: Arc<Mutex<ByteCache>>,
    read_offset: u64,
    exhausted: bool,
    eof: Arc<AtomicBool>,
}

impl CachedReader {
    fn new(cache: Arc<Mutex<ByteCache>>, start_offset: u64, eof: Arc<AtomicBool>) -> Self {
        Self {
            cache,
            read_offset: start_offset,
            exhausted: false,
            eof,
        }
    }
}

impl Read for CachedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.exhausted || self.eof.load(Ordering::Acquire) {
            return Ok(0);
        }
        loop {
            let cache = self.cache.lock().unwrap();
            let n = cache.read_at(self.read_offset, buf);
            if n > 0 {
                self.read_offset += n as u64;
                return Ok(n);
            }
            if self.eof.load(Ordering::Acquire) {
                drop(cache);
                self.exhausted = true;
                return Ok(0);
            }
            drop(cache);
            thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Seek for CachedReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let cache = self.cache.lock().unwrap();
        match pos {
            SeekFrom::Start(offset) => {
                if cache.is_offset_cached(offset) || offset == cache.total_written() {
                    self.read_offset = offset;
                    self.exhausted = false;
                    Ok(offset)
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek beyond cached range",
                    ))
                }
            }
            SeekFrom::Current(offset) => {
                let new_pos = self.read_offset as i64 + offset;
                if new_pos < 0 {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek before beginning",
                    ))
                } else {
                    let new_pos = new_pos as u64;
                    if cache.is_offset_cached(new_pos) || new_pos == cache.total_written() {
                        self.read_offset = new_pos;
                        self.exhausted = false;
                        Ok(new_pos)
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "seek beyond cached range",
                        ))
                    }
                }
            }
            SeekFrom::End(offset) => {
                let tw = cache.total_written();
                let new_pos = tw as i64 + offset;
                if new_pos < 0 {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek before beginning",
                    ))
                } else {
                    let new_pos = new_pos as u64;
                    if cache.is_offset_cached(new_pos) || new_pos == tw {
                        self.read_offset = new_pos;
                        self.exhausted = false;
                        Ok(new_pos)
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "seek beyond cached range",
                        ))
                    }
                }
            }
        }
    }
}

/// A `StreamSource` decorator that caches downloaded bytes in a ring buffer.
///
/// On `open(None)` (initial play):
/// 1. Opens the inner source to get a byte reader.
/// 2. Spawns a background thread that reads from the inner reader and writes
///    every byte into the ring cache.
/// 3. Returns a `CachedReader` starting at byte 0.
///
/// On `open(Some(_))` (seek):
/// - If the cache has data (i.e., the background filler is running), returns
///   a `CachedReader` from byte 0.  The engine's `seek_target_ms` handles
///   positioning via packet-skip.  The `CachedReader` implements `Seek` for
///   direct use (and `open()` now exposes it via the `Seek` trait bound),
///   but `handling.rs` wraps the reader in `ReadOnlySource` which reports
///   `is_seekable() = false`, so Symphonia never calls `seek()` on it.
/// - If the cache is empty (before initial open or after a full eviction),
///   delegates to the inner source's open, which creates a Range request.
pub struct CachingDecorator {
    inner: Box<dyn StreamSource>,
    info: SourceInfo,
    cache: Arc<Mutex<ByteCache>>,
    content_length: Arc<AtomicU64>,
    /// Monotonically increasing generation counter.  Each filler thread
    /// records its generation at spawn and stops when the global gen
    /// advances (on `stop_background`).  This avoids races where a new
    /// thread clears a shared `stop` flag before the old thread sees it.
    bg_gen: Arc<AtomicU64>,
    eof: Arc<AtomicBool>,
    bg_handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl CachingDecorator {
    /// Create a new caching decorator.
    ///
    /// `capacity_bytes` — max bytes to retain in the ring cache (e.g. 30s
    /// at 128kbps ≈ 480 KB).
    pub fn new(inner: Box<dyn StreamSource>, capacity_bytes: usize) -> Self {
        let info = inner.info().clone();
        Self {
            inner,
            info,
            cache: Arc::new(Mutex::new(ByteCache::new(capacity_bytes))),
            content_length: Arc::new(AtomicU64::new(0)),
            bg_gen: Arc::new(AtomicU64::new(0)),
            eof: Arc::new(AtomicBool::new(false)),
            bg_handle: Mutex::new(None),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn cache(&self) -> Arc<Mutex<ByteCache>> {
        self.cache.clone()
    }

    pub fn has_cached_data(&self) -> bool {
        self.cache.lock().unwrap().data.len() > 0
    }

    fn start_background(&self, reader: Box<dyn ReadSeek + Send + Sync + 'static>) {
        self.stop_background();
        self.eof.store(false, Ordering::Release);
        let cache = self.cache.clone();
        let bg_gen = self.bg_gen.clone();
        let gen = bg_gen.load(Ordering::Relaxed);
        let eof = self.eof.clone();

        let handle = thread::Builder::new()
            .name("caching-filler".into())
            .spawn(move || {
                let mut reader = reader;
                let mut buf = [0u8; 65536];
                loop {
                    if gen != bg_gen.load(Ordering::Acquire) {
                        debug!("[caching] Background filler stopped (generation advance)");
                        break;
                    }
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            info!("[caching] Background filler reached end of stream");
                            eof.store(true, Ordering::Release);
                            break;
                        }
                        Ok(n) => {
                            cache.lock().unwrap().push(&buf[..n]);
                        }
                        Err(e) => {
                            warn!("[caching] Background filler read error: {}", e);
                            thread::sleep(Duration::from_millis(500));
                        }
                    }
                }
            })
            .expect("Failed to spawn caching filler thread");
        *self.bg_handle.lock().unwrap() = Some(handle);
    }

    fn stop_background(&self) {
        self.bg_gen.fetch_add(1, Ordering::Release);
        if let Ok(mut handle) = self.bg_handle.lock() {
            if let Some(h) = handle.take() {
                let deadline = Instant::now() + Duration::from_millis(50);
                while Instant::now() < deadline {
                    if h.is_finished() {
                        let _ = h.join();
                        return;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                warn!("[caching] Background filler did not stop within 50ms — continuing");
            }
        }
    }
}

impl StreamSource for CachingDecorator {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, capability: Capability) -> bool {
        self.inner.supports(capability)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn duration_ms(&self) -> Option<u64> {
        self.inner.duration_ms()
    }

    fn open(
        &self,
        seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
        match seek_to {
            None => {
                self.stop_background();
                self.cache.lock().unwrap().clear();
                let reader = self.inner.open(None)?;
                if let Some(total) = self.inner.total_bytes() {
                    self.content_length.store(total, Ordering::Relaxed);
                }
                self.start_background(reader);
                Ok(Box::new(CachedReader::new(self.cache.clone(), 0, self.eof.clone())))
            }
            Some(ms) => {
                // If the cache already has data (background filler running),
                // serve from cache — the engine handles positioning via seek_target_ms.
                if self.has_cached_data() {
                    info!("[caching] Serving seek from cache ({} ms)", ms);
                    return Ok(Box::new(CachedReader::new(
                        self.cache.clone(),
                        0,
                        self.eof.clone(),
                    )));
                }
                // Cache is empty — re-open the inner source with Range request.
                info!("[caching] Cache empty for seek ({} ms), re-opening inner", ms);
                self.stop_background();
                self.cache.lock().unwrap().clear();
                let reader = self.inner.open(seek_to)?;
                if let Some(total) = self.inner.total_bytes() {
                    self.content_length.store(total, Ordering::Relaxed);
                }
                self.start_background(reader);
                Ok(Box::new(CachedReader::new(self.cache.clone(), 0, self.eof.clone())))
            }
        }
    }

    fn total_bytes(&self) -> Option<u64> {
        let cl = self.content_length.load(Ordering::Relaxed);
        if cl > 0 { Some(cl) } else { self.inner.total_bytes() }
    }

    fn pipe_writer(&self) -> Option<Arc<crate::audio::stream::pipe::PipeWriter>> {
        self.inner.pipe_writer()
    }
}
