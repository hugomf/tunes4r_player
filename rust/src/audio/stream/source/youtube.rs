//! YouTubeSource — YouTube audio streams.
//!
//! Resolves a video ID/URL/search query to a CDN audio URL,
//! then streams via HTTP with optional Range headers for seeking.

use crate::audio::engine::types::HttpClient;
use crate::audio::error::PlaybackError;
use crate::models::StreamType;
use crate::youtube::YouTube;

use super::{Capability, SourceInfo, SourceKind, StreamSource};
use log::{debug, info};
use std::io::Read;
use std::sync::Arc;

#[cfg(target_os = "android")]
use crate::audio::stream::pipe;
#[cfg(target_os = "android")]
use std::thread;

pub struct YouTubeSource {
    info: SourceInfo,
    client: Arc<HttpClient>,
    audio_url: String,
    total_content_bytes: std::sync::atomic::AtomicU64,
}

impl YouTubeSource {
    pub fn new(
        input: &str,
        client: Arc<HttpClient>,
        _cache_dir: Option<String>,
    ) -> Result<Self, PlaybackError> {
        info!("[youtube-source] Resolving: {}", input);

        // --- resolve to audio CDN URL ---
        let (audio_url, video_id) = match resolve_youtube_audio(input) {
            Ok(result) => result,
            Err(e) => {
                return Err(PlaybackError::HttpStream {
                    operation: "resolve".into(),
                    detail: format!("YouTube resolution failed: {}", e),
                });
            }
        };

        info!(
            "[youtube-source] Resolved video_id={}, audio_url length={}",
            video_id,
            audio_url.len()
        );

        let title = video_id.clone(); // placeholder; real title would come from video info

        Ok(Self {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: input.to_string(),
                title: Some(title),
            },
            client,
            audio_url,
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
        })
    }

    #[cfg_attr(target_os = "android", allow(dead_code))]
    fn estimate_byte_offset(&self, seek_ms: u64, content_length: u64) -> u64 {
        if content_length == 0 {
            return 0;
        }
        // Assume ~300s total for estimation (fine-tuned by decoder fast-forward)
        let estimated_total_ms = 300_000u64;
        let ratio = (seek_ms as f64 / estimated_total_ms as f64).min(0.99);
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
    ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
        #[cfg(not(target_os = "android"))]
        {
            let mut req = self
                .client
                .get(&self.audio_url)
                .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .header("Accept", "audio/*, text/plain, application/octet-stream")
                .header("Referer", "https://www.youtube.com")
                .header("Origin", "https://www.youtube.com");

            if let Some(ms) = seek_to {
                let content_length = self.total_content_bytes.load(std::sync::atomic::Ordering::Relaxed);
                let byte_offset = self.estimate_byte_offset(ms, content_length);
                if byte_offset > 0 {
                    debug!(
                        "[youtube-source] Seek to {}ms, byte offset ~{} (content_length={})",
                        ms, byte_offset, content_length
                    );
                    req = req.header("Range", format!("bytes={}-", byte_offset));
                }
            }

            let resp = req.send().map_err(|e| PlaybackError::HttpStream {
                operation: "GET".into(),
                detail: format!("YouTube HTTP request failed: {}", e),
            })?;

            let status = resp.status();
            if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
                return Err(PlaybackError::HttpStatus {
                    url: self.audio_url.clone(),
                    status_code: status.as_u16(),
                    detail: "YouTube stream request failed".into(),
                });
            }

            // Store content length if not yet known
            if self
                .total_content_bytes
                .load(std::sync::atomic::Ordering::Relaxed)
                == 0
            {
                if let Some(cl) = resp.content_length() {
                    self.total_content_bytes
                        .store(cl, std::sync::atomic::Ordering::Relaxed);
                }
            }

            Ok(Box::new(resp))
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
}

/// Resolve a YouTube video ID, URL, or search query to an audio CDN URL.
fn resolve_youtube_audio(input: &str) -> Result<(String, String), String> {
    let video_id = extract_video_id(input);

    match video_id {
        Some(id) => {
            debug!("[youtube-source] Extracted video_id: {}", id);
            let yt = YouTube::new();
            let manifest = yt.videos().stream(&id).map_err(|e| {
                format!("Failed to get YouTube stream: {}", e)
            })?;

            let audio = manifest.best_audio().ok_or_else(|| {
                "No audio stream found in YouTube manifest".to_string()
            })?;

            if audio.url.is_empty() {
                return Err("Extracted YouTube audio URL is empty".to_string());
            }

            Ok((audio.url.clone(), id))
        }
        None => {
            // Treat as a search query
            info!("[youtube-source] Treating input as search query: {}", input);
            let yt = YouTube::new();
            let search_client = yt.client().http();
            let results = crate::youtube::search::search(search_client, input, 1)
                .map_err(|e| format!("YouTube search failed: {}", e))?;

            let first = results.into_iter().next().ok_or_else(|| {
                format!("No YouTube results found for: {}", input)
            })?;

            info!(
                "[youtube-source] Search found: {} ({})",
                first.title, first.id
            );

            let manifest = yt.videos().stream(&first.id).map_err(|e| {
                format!("Failed to get YouTube stream for '{}': {}", first.id, e)
            })?;

            let audio = manifest.best_audio().ok_or_else(|| {
                "No audio stream found".to_string()
            })?;

            if audio.url.is_empty() {
                return Err("Extracted YouTube audio URL is empty".to_string());
            }

            Ok((audio.url.clone(), first.id.clone()))
        }
    }
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
    fn test_extract_video_id_v_path() {
        assert_eq!(
            extract_video_id("https://youtube.com/v/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn test_extract_video_id_invalid_id_too_short() {
        assert_eq!(extract_video_id("abc"), None);
    }

    #[test]
    fn test_extract_video_id_invalid_url() {
        assert_eq!(extract_video_id("not a url"), None);
    }

    #[test]
    fn test_extract_video_id_missing_v_param() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?list=123"),
            None
        );
    }

    #[test]
    fn test_extract_video_id_mobile_url() {
        assert_eq!(
            extract_video_id("https://m.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn test_estimate_byte_offset_zero_content() {
        let src = YouTubeSource {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: "test".into(),
                title: None,
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
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
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
        };
        // 150s / 300s = 0.5, so 0.5 * 10_000_000 = 5_000_000
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
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
        };
        // 350s / 300s > 1.0, should be capped at 0.99
        let offset = src.estimate_byte_offset(350_000, 1000);
        assert_eq!(offset, 990);
    }

    #[test]
    fn test_youtube_source_capabilities() {
        let src = YouTubeSource {
            info: SourceInfo {
                kind: SourceKind::YouTube,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: "test".into(),
                title: None,
            },
            client: Arc::new(crate::audio::engine::types::HttpClient::default()),
            audio_url: "http://example.com".into(),
            total_content_bytes: std::sync::atomic::AtomicU64::new(0),
        };
        assert!(src.supports(Capability::Seek));
        assert!(src.supports(Capability::Download));
        assert!(src.supports(Capability::Cache));
    }
}

/// Extract a YouTube video ID from a URL, or return the input if it looks like
/// an 11-character video ID.
fn extract_video_id(input: &str) -> Option<String> {
    let input = input.trim();

    // Direct 11-char video ID
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
        // Handle /v/ or /embed/ paths
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
