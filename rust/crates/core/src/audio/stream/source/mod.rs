//! StreamSource trait and implementations.
//!
//! Each source provides raw audio bytes via a unified interface.
//! Features (caching, adaptive buffering) are layered as decorators.

pub mod file;
pub mod live;
pub mod pipe;
pub mod pipeline;
pub mod radio;
pub mod youtube;

use crate::audio::error::PlaybackError;
use crate::audio::stream::decorator::caching::CachingDecorator;
use crate::models::StreamType;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

/// Default in-memory cache size for YouTube streaming (1 MB).
const YOUTUBE_CACHE_BYTES: usize = 1_048_576;

/// Explicit capability that a source may or may not support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Can seek to arbitrary positions (Range requests, file seek).
    Seek,
    /// Can be saved to a local file for offline playback.
    Download,
    /// Results should be cached on disk for repeat plays.
    Cache,
}

/// High-level classification of the source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Radio,
    YouTube,
    File,
    Pipe,
    Live,
}

/// Read-only metadata about the source.
#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub kind: SourceKind,
    pub stream_type: StreamType,
    pub uri: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// Combined `Read + Seek` trait that can be used as a single trait bound
/// in trait objects (Rust doesn't allow multiple non-auto traits in a dyn
/// type).  Use `Box<dyn ReadSeek + Send + Sync + 'static>` everywhere a
/// seek-capable reader is needed.
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// Wraps a `Read + Send + Sync` reader and provides a `Seek` impl that
/// always returns `Unsupported`.  Used when a source inherently cannot
/// seek (e.g. HTTP live streams) but the trait requires a `Seek` bound.
#[allow(dead_code)]
pub(crate) struct NonSeekable<R>(pub(crate) R);

impl<R: Read> Read for NonSeekable<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<R: Read> Seek for NonSeekable<R> {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "source is not seekable",
        ))
    }
}

// NonSeekable<R> implements ReadSeek because it implements Read + Seek

/// A stream source provides raw audio bytes from some origin.
///
/// Implementations are responsible for positioning the returned reader
/// at the correct byte offset when `seek_to` is provided.
pub trait StreamSource: Send + Sync {
    fn info(&self) -> &SourceInfo;

    fn supports(&self, capability: Capability) -> bool;

    /// For downcasting to concrete source types (e.g. PipeSource).
    fn as_any(&self) -> &dyn std::any::Any { unimplemented!() }

    /// Open a byte reader at the given position.
    ///
    /// `seek_to` is in milliseconds from the start. Sources that do not
    /// support seeking (radio) should ignore this and return a reader
    /// from the beginning.
    fn open(
        &self,
        seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError>;

    /// Total content length in bytes, if known.
    fn total_bytes(&self) -> Option<u64> {
        None
    }

    /// Duration of the content in milliseconds, if known.
    fn duration_ms(&self) -> Option<u64> {
        None
    }

    /// Access the pipe writer, if this source is a pipe-based source.
    fn pipe_writer(&self) -> Option<Arc<crate::audio::stream::pipe::PipeWriter>> {
        None
    }
}

/// Auto-detect the source type from a URI and create the appropriate source.
pub fn from_uri(
    uri: &str,
    client: Arc<crate::audio::engine::types::HttpClient>,
    cache_dir: Option<String>,
) -> Result<Box<dyn StreamSource>, PlaybackError> {
    let lower = uri.to_lowercase();

    // Android content:// URIs are not supported — they require Android
    // ContentResolver to open, which Rust's std::fs and Path::exists()
    // cannot handle. Reject early before falling through to YouTube.
    if lower.starts_with("content://") {
        return Err(PlaybackError::UnsupportedScheme {
            scheme: "content://".into(),
        });
    }

    let source: Box<dyn StreamSource> = if lower.contains("youtube.com") || lower.contains("youtu.be") {
        Box::new(CachingDecorator::new(
            Box::new(youtube::YouTubeSource::new(uri, client, None)?),
            YOUTUBE_CACHE_BYTES,
        ))
    } else if uri.starts_with("http://") || uri.starts_with("https://") {
        Box::new(radio::RadioSource::new(uri, client))
    } else if std::path::Path::new(uri).exists() {
        Box::new(file::FileSource::new(uri))
    } else {
        // Assume YouTube video ID or search query
        Box::new(CachingDecorator::new(
            Box::new(youtube::YouTubeSource::new(uri, client, None)?),
            YOUTUBE_CACHE_BYTES,
        ))
    };

    // Wrap with cache decorator when a cache directory is provided
    // and the source supports caching.
    if let Some(dir) = cache_dir {
        if source.supports(Capability::Cache) {
            return Ok(Box::new(crate::audio::stream::decorator::cache::CacheDecorator::new(source, &dir)));
        }
    }

    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::engine::types::HttpClient;

    fn test_client() -> Arc<HttpClient> {
        Arc::new(HttpClient::default())
    }

    #[test]
    fn test_source_kind_enum() {
        assert_ne!(SourceKind::Radio, SourceKind::YouTube);
        assert_ne!(SourceKind::File, SourceKind::Pipe);
    }

    #[test]
    fn test_from_uri_http_routes_to_radio() {
        let src = from_uri("http://example.com/stream.mp3", test_client(), None).unwrap();
        assert_eq!(src.info().kind, SourceKind::Radio);
    }

    #[test]
    fn test_from_uri_https_routes_to_radio() {
        let src = from_uri("https://icecast.example.com/live.mp3", test_client(), None).unwrap();
        assert_eq!(src.info().kind, SourceKind::Radio);
    }

    #[test]
    fn test_from_uri_file_routes_to_file() {
        let dir = std::env::temp_dir().join("test_source_file.txt");
        std::fs::write(&dir, "not really audio").ok();
        let src = from_uri(dir.to_str().unwrap(), test_client(), None).unwrap();
        assert_eq!(src.info().kind, SourceKind::File);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn test_radio_capabilities() {
        let src = radio::RadioSource::new("http://example.com/stream", test_client());
        assert!(src.supports(Capability::Download));
        assert!(!src.supports(Capability::Seek));
        assert!(!src.supports(Capability::Cache));
    }

    #[test]
    fn test_file_capabilities() {
        let dir = std::env::temp_dir().join("test_caps_file.txt");
        std::fs::write(&dir, "not audio").ok();
        let src = file::FileSource::new(dir.to_str().unwrap());
        assert!(src.supports(Capability::Seek));
        assert!(src.supports(Capability::Download));
        assert!(!src.supports(Capability::Cache));
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn test_pipe_capabilities() {
        let src = pipe::PipeSource::new("pipe://test");
        assert!(src.supports(Capability::Download));
        assert!(!src.supports(Capability::Seek));
        assert!(!src.supports(Capability::Cache));
    }

    #[test]
    fn test_radio_source_info() {
        let src = radio::RadioSource::new("http://example.com/stream", test_client());
        let info = src.info();
        assert_eq!(info.kind, SourceKind::Radio);
        assert_eq!(info.uri, "http://example.com/stream");
        assert!(info.title.is_none());
    }

    #[test]
    fn test_file_source_info() {
        let dir = std::env::temp_dir().join("test_info_file.txt");
        std::fs::write(&dir, "not audio").ok();
        let src = file::FileSource::new(dir.to_str().unwrap());
        let info = src.info();
        assert_eq!(info.kind, SourceKind::File);
        assert!(info.title.is_some());
        let _ = std::fs::remove_file(&dir);
    }
}
