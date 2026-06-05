//! StreamPipeline builder — compose sources + decorators into a pipeline.
//!
//! # Presets
//! - `radio(url, client)` — live HTTP stream, no caching
//! - `youtube(input, client)` — resolved YouTube audio, cache + adaptive by default
//! - `file(path)` — local file with seek
//! - `from_source(source)` — any custom source
//!
//! # Layers
//! - `.with_cache(dir)` — wrap in CacheDecorator
//! - `.with_adaptive_buffer()` — wrap in AdaptiveBufferDecorator
//! - `.with_retry()` — wrap in RetryDecorator (auto-reconnect on read errors)

use crate::audio::engine::types::HttpClient;
use crate::audio::stream::pipe::PipeWriter;

use crate::audio::stream::decorator::adaptive::AdaptiveBufferDecorator;
use crate::audio::stream::decorator::cache::CacheDecorator;
use crate::audio::stream::decorator::retry::RetryDecorator;
use super::file::FileSource;
use super::pipe::PipeSource;
use super::radio::RadioSource;
use super::youtube::YouTubeSource;
use super::{Capability, SourceInfo, SourceKind, StreamSource};
use crate::audio::error::PlaybackError;
use std::io::Read;
use std::sync::Arc;

/// A fully built stream pipeline — source + decorators assembled.
pub struct Pipeline {
    source: Box<dyn StreamSource>,
    kind: SourceKind,
    pipe_writer: Option<Arc<PipeWriter>>,
}

impl Pipeline {
    pub fn open(
        &self,
    ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
        self.source.open(None)
    }

    pub fn info(&self) -> &SourceInfo {
        self.source.info()
    }

    pub fn supports(&self, capability: Capability) -> bool {
        self.source.supports(capability)
    }

    pub fn total_bytes(&self) -> Option<u64> {
        self.source.total_bytes()
    }

    pub fn kind(&self) -> SourceKind {
        self.kind
    }

    pub fn pipe_writer(&self) -> Option<Arc<PipeWriter>> {
        self.pipe_writer.clone()
    }
}

/// Builder that composes a source with optional decorators.
pub struct PipelineBuilder {
    inner: Option<Box<dyn StreamSource>>,
    cache_dir: Option<String>,
    adaptive_buffer: bool,
    retry: bool,
    kind: SourceKind,
    pipe_writer: Option<Arc<PipeWriter>>,
}

impl PipelineBuilder {
    /// Live radio / Icecast stream.
    pub fn radio(url: &str, client: Arc<HttpClient>) -> Self {
        Self {
            inner: Some(Box::new(RadioSource::new(url, client))),
            cache_dir: None,
            adaptive_buffer: false,
            retry: true,
            kind: SourceKind::Radio,
            pipe_writer: None,
        }
    }

    /// YouTube video — resolved to CDN audio URL.
    pub fn youtube(
        input: &str,
        client: Arc<HttpClient>,
    ) -> Result<Self, PlaybackError> {
        let source = YouTubeSource::new(input, client, None)?;
        Ok(Self {
            inner: Some(Box::new(source)),
            cache_dir: None,
            adaptive_buffer: true, // enabled by default for YouTube
            retry: false,
            kind: SourceKind::YouTube,
            pipe_writer: None,
        })
    }

    /// Local file.
    pub fn file(path: &str) -> Self {
        Self {
            inner: Some(Box::new(FileSource::new(path))),
            cache_dir: None,
            adaptive_buffer: false,
            retry: false,
            kind: SourceKind::File,
            pipe_writer: None,
        }
    }

    /// Pipe (bytes fed from Dart).
    pub fn pipe(url: &str) -> Self {
        let inner = PipeSource::new(url);
        let writer = inner.writer();
        Self {
            inner: Some(Box::new(inner)),
            cache_dir: None,
            adaptive_buffer: false,
            retry: false,
            kind: SourceKind::Pipe,
            pipe_writer: writer,
        }
    }

    /// Wrap any existing source.
    pub fn from_source(source: Box<dyn StreamSource>) -> Self {
        let kind = source.info().kind;
        let pipe_writer = if kind == SourceKind::Pipe {
            source.as_any().downcast_ref::<PipeSource>().and_then(|ps| ps.writer())
        } else {
            None
        };
        Self {
            inner: Some(source),
            cache_dir: None,
            adaptive_buffer: false,
            retry: false,
            kind,
            pipe_writer,
        }
    }

    /// Enable disk caching for this pipeline.
    pub fn with_cache(mut self, dir: &str) -> Self {
        self.cache_dir = Some(dir.to_string());
        self
    }

    /// Enable background pre-fetch buffering.
    pub fn with_adaptive_buffer(mut self) -> Self {
        self.adaptive_buffer = true;
        self
    }

    /// Enable auto-reconnect on read errors (useful for live radio).
    pub fn with_retry(mut self) -> Self {
        self.retry = true;
        self
    }

    /// Build the pipeline — applies all decorators.
    pub fn build(mut self) -> Result<Pipeline, PlaybackError> {
        let mut source = self.inner.take().ok_or_else(|| PlaybackError::Cache {
            detail: "PipelineBuilder has no inner source".into(),
        })?;

        // Apply cache (only if source supports it)
        if let Some(dir) = self.cache_dir.take() {
            if source.supports(Capability::Cache) {
                source = Box::new(CacheDecorator::new(source, &dir));
            }
        }

        // Apply adaptive buffer
        if self.adaptive_buffer {
            source = Box::new(AdaptiveBufferDecorator::new(source));
        }

        // Apply retry (last — wraps all other layers so reconnects re-apply them)
        if self.retry {
            source = Box::new(RetryDecorator::new(source));
        }

        Ok(Pipeline {
            source,
            kind: self.kind,
            pipe_writer: self.pipe_writer,
        })
    }
}
