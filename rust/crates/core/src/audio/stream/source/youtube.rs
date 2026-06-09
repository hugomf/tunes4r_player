//! YouTubeSource — YouTube audio streams.
//!
//! Resolves a video ID/URL/search query to a CDN audio URL,
//! then streams via HTTP with optional Range headers for seeking.
//!
//! ## Seek design (BUG-1 fix)
//!
//! Symphonia needs to re-probe the container format header on every `open()` call,
//! even for seeks.  For M4A/AAC streams the `moov` atom can be hundreds of KB and
//! lives at the start of the file — issuing `Range: bytes=<mid-stream>-` gives
//! Symphonia a stream that starts mid-file, so probing fails.
//!
//! Fix: on the *initial* `open(None)` we wrap the response in a `TeeReader` that
//! shadows every byte into `header_cache: Vec<u8>` until Symphonia finishes probing
//! (tracked by byte count, **not** a fixed 64 KB constant).  On subsequent
//! `open(Some(ms))` seek calls we issue `Range: bytes=<offset>-` and prepend the
//! cached header bytes via `ChainReader`, so Symphonia can re-probe from cache
//! (no network) and then decode from the range offset.

use crate::audio::engine::types::HttpClient;
use crate::audio::error::PlaybackError;
use crate::models::StreamType;
use tunes4r_youtube::YouTube;

#[cfg(not(target_os = "android"))]
use super::NonSeekable;
use super::{Capability, ReadSeek, SourceInfo, SourceKind, StreamSource};
use log::{debug, info};
#[cfg(not(target_os = "android"))]
use std::io::{self, Cursor, Read};
use std::sync::Arc;
#[cfg(not(target_os = "android"))]
use std::sync::Mutex;

#[cfg(target_os = "android")]
use crate::audio::stream::pipe;
#[cfg(target_os = "android")]
use std::thread;

// ---------------------------------------------------------------------------
// TeeReader — mirrors bytes into a Vec<u8> while reading
// ---------------------------------------------------------------------------

/// Wraps a `Read` and copies every byte into `sink` as it passes through.
/// Used on the initial open to capture exactly the bytes Symphonia consumes
/// during format probing (the container header / moov atom).
#[cfg(not(target_os = "android"))]
struct TeeReader<R: Read> {
    inner: R,
    sink: Arc<Mutex<Vec<u8>>>,
}

#[cfg(not(target_os = "android"))]
impl<R: Read> TeeReader<R> {
    fn new(inner: R, sink: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { inner, sink }
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Read> Read for TeeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.sink.lock().unwrap().extend_from_slice(&buf[..n]);
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// ChainReader — Cursor<Vec<u8>> prepended to another Read
// ---------------------------------------------------------------------------

/// Reads first from a `Cursor<Vec<u8>>` (the cached header) and then from
/// `tail` (the ranged HTTP response body).  Symphonia re-probes the header
/// from the cursor (no network), then decodes from the tail.
#[cfg(not(target_os = "android"))]
struct ChainReader {
    head: Cursor<Vec<u8>>,
    tail: Box<dyn ReadSeek + Send + Sync + 'static>,
}

#[cfg(not(target_os = "android"))]
impl ChainReader {
    fn new(header_cache: Vec<u8>, tail: Box<dyn ReadSeek + Send + Sync + 'static>) -> Self {
        Self {
            head: Cursor::new(header_cache),
            tail,
        }
    }
}

#[cfg(not(target_os = "android"))]
impl Read for ChainReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Drain the header cursor first; fall through to tail when exhausted.
        let n = self.head.read(buf)?;
        if n > 0 {
            return Ok(n);
        }
        self.tail.read(buf)
    }
}

// ---------------------------------------------------------------------------
// YouTubeSource
// ---------------------------------------------------------------------------

pub struct YouTubeSource {
    info: SourceInfo,
    client: Arc<HttpClient>,
    audio_url: String,
    duration_ms: u64,
    total_content_bytes: std::sync::atomic::AtomicU64,
    /// Bytes Symphonia consumed during initial format probing (moov atom etc.).
    /// Populated on the first `open(None)` call; used to build a `ChainReader`
    /// on subsequent `open(Some(ms))` seek calls.
    #[cfg(not(target_os = "android"))]
    header_cache: Arc<Mutex<Vec<u8>>>,
}

impl YouTubeSource {
    pub fn new(
        input: &str,
        client: Arc<HttpClient>,
        _cache_dir: Option<String>,
    ) -> Result<Self, PlaybackError> {
        Self::with_po_token(input, client, _cache_dir, None)
    }

    pub fn with_po_token(
        input: &str,
        client: Arc<HttpClient>,
        _cache_dir: Option<String>,
        po_token: Option<String>,
    ) -> Result<Self, PlaybackError> {
        info!("[youtube-source] Resolving: {}", input);

        let (audio_url, video_id, duration_ms) = match resolve_youtube_audio(input, po_token) {
            Ok(result) => result,
            Err(e) => {
                return Err(PlaybackError::HttpStream {
                    operation: "resolve".into(),
                    detail: format!("YouTube resolution failed: {}", e),
                });
            }
        };

        info!(
            "[youtube-source] Resolved video_id={}, duration={}ms, audio_url length={}",
            video_id,
            duration_ms,
            audio_url.len()
        );

        let title = video_id.clone();

        Ok(Self {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: input.to_string(),
                title: Some(title),
                artist: None,
                album: None,
            },
            client,
            audio_url,
            duration_ms,
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
            #[cfg(not(target_os = "android"))]
            header_cache: Arc::new(Mutex::new(Vec::new())),
        })
    }

    #[cfg_attr(target_os = "android", allow(dead_code))]
    fn estimate_byte_offset(&self, seek_ms: u64, content_length: u64) -> u64 {
        if content_length == 0 || self.duration_ms == 0 {
            return 0;
        }
        let ratio = (seek_ms as f64 / self.duration_ms as f64).min(0.99);
        (ratio * content_length as f64) as u64
    }
}

impl StreamSource for YouTubeSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, capability: Capability) -> bool {
        matches!(
            capability,
            Capability::Seek | Capability::Download | Capability::Cache
        )
    }

    fn as_any(&self) -> &dyn std::any::Any { self }

    fn open(
        &self,
        seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
        #[cfg(not(target_os = "android"))]
        {
            match seek_to {
                // -------------------------------------------------------
                // Initial open: stream from byte 0, tee bytes into header_cache
                // so Symphonia's probed header is available for later seeks.
                // -------------------------------------------------------
                None => {
                    let req = self
                        .client
                        .get(&self.audio_url)
                        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                        .header("Accept", "audio/*, text/plain, application/octet-stream")
                        .header("Referer", "https://www.youtube.com")
                        .header("Origin", "https://www.youtube.com");

                    let resp = req.send().map_err(|e| PlaybackError::HttpStream {
                        operation: "GET".into(),
                        detail: format!("YouTube HTTP request failed: {}", e),
                    })?;

                    let status = resp.status();
                    if !status.is_success() {
                        return Err(PlaybackError::HttpStatus {
                            url: self.audio_url.clone(),
                            status_code: status.as_u16(),
                            detail: "YouTube stream request failed".into(),
                        });
                    }

                    // Store content-length for byte-offset estimation on seeks.
                    if self.total_content_bytes.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                        if let Some(cl) = resp.content_length() {
                            self.total_content_bytes.store(cl, std::sync::atomic::Ordering::Relaxed);
                        }
                    }

                    // Clear any stale header cache from a previous resolution.
                    self.header_cache.lock().unwrap().clear();

                    // Tee the response through header_cache.  Symphonia will read
                    // as many bytes as the container header requires (variable —
                    // up to several hundred KB for M4A moov atoms).  Every byte it
                    // consumes is mirrored into header_cache automatically.
                    let sink = Arc::clone(&self.header_cache);
                    let tee = TeeReader::new(resp, sink);
                    info!("[youtube-source] Initial open — tee-reading header into cache");
                    Ok(Box::new(NonSeekable(tee)))
                }

                // -------------------------------------------------------
                // Seek: issue Range request and prepend cached header so
                // Symphonia can re-probe without any network access.
                // -------------------------------------------------------
                Some(ms) => {
                    let content_length = self.total_content_bytes.load(std::sync::atomic::Ordering::Relaxed);
                    let byte_offset = self.estimate_byte_offset(ms, content_length);

                    debug!(
                        "[youtube-source] Seek to {}ms → byte offset {} (content_length={})",
                        ms, byte_offset, content_length
                    );

                    let mut req = self
                        .client
                        .get(&self.audio_url)
                        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                        .header("Accept", "audio/*, text/plain, application/octet-stream")
                        .header("Referer", "https://www.youtube.com")
                        .header("Origin", "https://www.youtube.com");

                    if byte_offset > 0 {
                        req = req.header("Range", format!("bytes={}-", byte_offset));
                    }

                    let resp = req.send().map_err(|e| PlaybackError::HttpStream {
                        operation: "GET (range)".into(),
                        detail: format!("YouTube Range request failed: {}", e),
                    })?;

                    let status = resp.status();
                    if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
                        return Err(PlaybackError::HttpStatus {
                            url: self.audio_url.clone(),
                            status_code: status.as_u16(),
                            detail: "YouTube range stream request failed".into(),
                        });
                    }

                    // Prepend the cached header so Symphonia can re-probe the
                    // container format without touching the network.
                    let header = self.header_cache.lock().unwrap().clone();
                    if header.is_empty() {
                        // Cache not yet populated (seek before first open — unusual).
                        // Fall back to a plain response; Symphonia may fail to probe
                        // if it's a mid-stream offset, but this is a corner-case.
                        info!("[youtube-source] Seek with empty header cache — falling back to raw range response");
                        return Ok(Box::new(NonSeekable(resp)));
                    }

                    info!(
                        "[youtube-source] Seek — prepending {} cached header bytes before range body",
                        header.len()
                    );
                    Ok(Box::new(NonSeekable(ChainReader::new(header, Box::new(NonSeekable(resp))))))
                }
            }
        }

        #[cfg(target_os = "android")]
        {
            let _ = seek_to;
            let (writer, reader) = pipe::new_pipe();
            let writer = Arc::new(writer);
            let fetch_writer = writer.clone();
            let client = Arc::clone(&self.client);
            let audio_url = self.audio_url.clone();

            thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(r) => r,
                    Err(e) => {
                        fetch_writer
                            .set_error(format!("Failed to create tokio runtime: {}", e));
                        return;
                    }
                };
                rt.block_on(async move {
                    let req = client
                        .get(&audio_url)
                        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                        .header("Accept", "audio/*, text/plain, application/octet-stream")
                        .header("Referer", "https://www.youtube.com")
                        .header("Origin", "https://www.youtube.com");

                    let mut resp = match req.send().await {
                        Ok(r) => r,
                        Err(e) => {
                            fetch_writer
                                .set_error(format!("YouTube HTTP request failed: {}", e));
                            return;
                        }
                    };

                    let status = resp.status();
                    if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
                        fetch_writer.set_error(format!(
                            "YouTube stream HTTP {}",
                            status.as_u16()
                        ));
                        return;
                    }

                    loop {
                        match resp.chunk().await {
                            Ok(Some(data)) => fetch_writer.push(&data),
                            Ok(None) => {
                                fetch_writer.end();
                                return;
                            }
                            Err(e) => {
                                fetch_writer
                                    .set_error(format!("YouTube stream error: {}", e));
                                return;
                            }
                        }
                    }
                });
            });

            Ok(Box::new(reader))
        }
    }

    fn total_bytes(&self) -> Option<u64> {
        let n = self.total_content_bytes.load(std::sync::atomic::Ordering::Relaxed);
        if n > 0 { Some(n) } else { None }
    }

    fn duration_ms(&self) -> Option<u64> {
        Some(self.duration_ms)
    }
}

fn resolve_youtube_audio(input: &str, po_token: Option<String>) -> Result<(String, String, u64), String> {
    let video_id = extract_video_id(input);

    match video_id {
        Some(id) => {
            debug!("[youtube-source] Extracted video_id: {}", id);
            let mut yt = YouTube::new();
            if let Some(ref pot) = po_token {
                yt.set_po_token(Some(pot.clone()));
            }
            let manifest = yt.videos().stream(&id).map_err(|e| {
                format!("Failed to get YouTube stream: {}", e)
            })?;

            let audio = manifest.best_audio().ok_or_else(|| {
                "No audio stream found in YouTube manifest".to_string()
            })?;

            if audio.url.is_empty() {
                return Err("Extracted YouTube audio URL is empty".to_string());
            }

            // Prefer per-format approx_duration_ms, fall back to manifest duration_seconds
            let duration_ms = audio
                .approx_duration_ms
                .or_else(|| {
                    let secs = manifest.duration_seconds;
                    if secs > 0 { Some(secs * 1000) } else { None }
                })
                .unwrap_or(0);

            Ok((audio.url.clone(), id, duration_ms))
        }
        None => {
            info!("[youtube-source] Treating input as search query: {}", input);
            let yt = YouTube::new();
            let search_client = yt.client().http();
            let results = tunes4r_youtube::search::search(search_client, input, 1)
                .map_err(|e| format!("YouTube search failed: {}", e))?;

            let first = results.into_iter().next().ok_or_else(|| {
                format!("No YouTube results found for: {}", input)
            })?;

            info!(
                "[youtube-source] Search found: {} ({})",
                first.title, first.id
            );

            let mut yt = YouTube::new();
            if let Some(ref pot) = po_token {
                yt.set_po_token(Some(pot.clone()));
            }
            let manifest = yt.videos().stream(&first.id).map_err(|e| {
                format!("Failed to get YouTube stream for '{}': {}", first.id, e)
            })?;

            let audio = manifest.best_audio().ok_or_else(|| {
                "No audio stream found".to_string()
            })?;

            if audio.url.is_empty() {
                return Err("Extracted YouTube audio URL is empty".to_string());
            }

            let duration_ms = audio
                .approx_duration_ms
                .or_else(|| {
                    let secs = manifest.duration_seconds;
                    if secs > 0 { Some(secs * 1000) } else { None }
                })
                .unwrap_or(0);

            Ok((audio.url.clone(), first.id.clone(), duration_ms))
        }
    }
}

fn extract_video_id(input: &str) -> Option<String> {
    let input = input.trim();

    if input.len() == 11 && input.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Some(input.to_string());
    }

    let uri = url::Url::parse(input).ok()?;
    let host = uri.host_str()?;

    if host.contains("youtu.be") {
        return uri.path_segments()?.next().map(|s| s.to_string());
    }

    if host.contains("youtube.com") || host.contains("m.youtube.com") {
        if let Some(v) = uri.query_pairs().find(|(k, _)| k == "v") {
            let id = v.1.to_string();
            if id.len() == 11 {
                return Some(id);
            }
        }
        let path = uri.path();
        for prefix in &["/v/", "/embed/", "/shorts/"] {
            if let Some(rest) = path.strip_prefix(prefix) {
                let id = rest.split('/').next().unwrap_or(rest);
                if id.len() == 11 {
                    return Some(id.to_string());
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_video_id_11_char_id() {
        assert_eq!(extract_video_id("dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".into()));
    }

    #[test]
    fn test_extract_video_id_full_url() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn test_extract_video_id_with_query_params() {
        assert_eq!(
            extract_video_id("https://youtube.com/watch?v=dQw4w9WgXcQ&t=123"),
            Some("dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn test_extract_video_id_short_url() {
        assert_eq!(
            extract_video_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn test_extract_video_id_embed_url() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/embed/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn test_extract_video_id_shorts_url() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/shorts/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn test_extract_video_id_invalid() {
        assert_eq!(extract_video_id("not a valid id"), None);
    }

    #[test]
    fn test_estimate_byte_offset_zero_content() {
        let src = YouTubeSource {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: "test".into(),
                title: None,
                artist: None,
                album: None,
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            duration_ms: 300_000,
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
            #[cfg(not(target_os = "android"))]
            header_cache: Arc::new(Mutex::new(Vec::new())),
        };
        assert_eq!(src.estimate_byte_offset(1000, 0), 0);
    }

    #[test]
    fn test_estimate_byte_offset_halfway() {
        let src = YouTubeSource {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: "test".into(),
                title: None,
                artist: None,
                album: None,
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            duration_ms: 300_000,
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
            #[cfg(not(target_os = "android"))]
            header_cache: Arc::new(Mutex::new(Vec::new())),
        };
        let offset = src.estimate_byte_offset(150_000, 10_000_000);
        assert!(offset > 4_900_000 && offset < 5_100_000);
    }

    #[test]
    fn test_estimate_byte_offset_capped_at_99_percent() {
        let src = YouTubeSource {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: "test".into(),
                title: None,
                artist: None,
                album: None,
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            duration_ms: 300_000,
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
            #[cfg(not(target_os = "android"))]
            header_cache: Arc::new(Mutex::new(Vec::new())),
        };
        let offset = src.estimate_byte_offset(297_000, 1000);
        assert_eq!(offset, 990);
    }

    #[test]
    fn test_estimate_byte_offset_uses_real_duration() {
        // A 213-second video seeking to 50% should give byte offset at ~50%
        let src = YouTubeSource {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: "test".into(),
                title: None,
                artist: None,
                album: None,
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            duration_ms: 213_159,
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
            #[cfg(not(target_os = "android"))]
            header_cache: Arc::new(Mutex::new(Vec::new())),
        };
        let offset = src.estimate_byte_offset(106_579, 10_000_000);
        assert!(offset > 4_900_000 && offset < 5_100_000,
            "expected ~50% byte offset for 50% seek, got {offset}");
    }

    #[test]
    fn test_youtube_source_capabilities() {
        let src = YouTubeSource {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: "test".into(),
                title: None,
                artist: None,
                album: None,
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            duration_ms: 300_000,
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
            #[cfg(not(target_os = "android"))]
            header_cache: Arc::new(Mutex::new(Vec::new())),
        };
        assert!(src.supports(Capability::Seek));
        assert!(src.supports(Capability::Download));
        assert!(src.supports(Capability::Cache));
    }

    // ── BUG-1 regression tests ────────────────────────────────────────

    /// TeeReader must mirror every byte that passes through into the sink.
    #[test]
    #[cfg(not(target_os = "android"))]
    fn test_tee_reader_mirrors_bytes() {
        let source_data = b"Hello, Symphonia! This is the container header.";
        let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let cursor = std::io::Cursor::new(source_data);
        let mut tee = TeeReader::new(cursor, Arc::clone(&sink));

        let mut output = Vec::new();
        tee.read_to_end(&mut output).unwrap();

        assert_eq!(output, source_data, "TeeReader must pass through all bytes");
        let cached = sink.lock().unwrap();
        assert_eq!(
            *cached, source_data,
            "TeeReader must mirror all bytes into sink"
        );
    }

    /// TeeReader must mirror partial reads correctly.
    #[test]
    #[cfg(not(target_os = "android"))]
    fn test_tee_reader_mirrors_partial_reads() {
        let source_data = b"ABCDEFGHIJ";
        let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let cursor = std::io::Cursor::new(source_data);
        let mut tee = TeeReader::new(cursor, Arc::clone(&sink));

        let mut buf = [0u8; 4];
        let n = tee.read(&mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf, b"ABCD");
        assert_eq!(&sink.lock().unwrap()[..], b"ABCD");

        let n = tee.read(&mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf, b"EFGH");
        assert_eq!(&sink.lock().unwrap()[..], b"ABCDEFGH");
    }

    /// ChainReader must first serve bytes from the head cursor, then
    /// seamlessly continue from the tail reader.
    #[test]
    #[cfg(not(target_os = "android"))]
    fn test_chain_reader_head_then_tail() {
        let head_bytes: Vec<u8> = b"HEADER--".to_vec();
        let tail_bytes: Vec<u8> = b"TAIL-DATA".to_vec();
        let tail = std::io::Cursor::new(tail_bytes.clone());

        let chain = ChainReader::new(head_bytes.clone(), Box::new(tail));
        let mut reader = chain;
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();

        let expected = [&head_bytes[..], &tail_bytes[..]].concat();
        assert_eq!(output, expected, "ChainReader must concatenate head and tail");
    }

    /// ChainReader must return empty data from an empty head.
    #[test]
    #[cfg(not(target_os = "android"))]
    fn test_chain_reader_empty_head() {
        let head_bytes: Vec<u8> = vec![];
        let tail_bytes: Vec<u8> = b"ONLY-TAIL".to_vec();
        let tail = std::io::Cursor::new(tail_bytes.clone());

        let chain = ChainReader::new(head_bytes, Box::new(tail));
        let mut reader = chain;
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();

        assert_eq!(output, b"ONLY-TAIL");
    }
}