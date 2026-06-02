//! PipeSource — bytes fed from Dart via a byte pipe.
//!
//! The PipeWriter is held by the engine so Dart can call push_audio_bytes.
//! Each call to `open()` creates a new pipe pair and stores the writer
//! in the provided `Arc` slot.

use crate::audio::error::PlaybackError;
use crate::audio::stream::pipe::PipeWriter;
use crate::models::StreamType;

use super::{Capability, SourceInfo, SourceKind, StreamSource};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct PipeSource {
    info: SourceInfo,
    writer: Arc<std::sync::Mutex<Option<Arc<PipeWriter>>>>,
    seek_requested: Arc<AtomicBool>,
}

impl PipeSource {
    pub fn new(url: &str) -> Self {
        Self {
            info: SourceInfo {
                kind: SourceKind::Pipe,
                stream_type: StreamType::Seekable { total_bytes: 0 },
                uri: url.to_string(),
                title: None,
            },
            writer: Arc::new(std::sync::Mutex::new(None)),
            seek_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Return the current PipeWriter so Dart can push bytes into it.
    pub fn writer(&self) -> Option<Arc<PipeWriter>> {
        self.writer.lock().unwrap().clone()
    }

    /// Signal that a seek is pending — the next `open()` call will
    /// return a fresh pipe and the old writer should be replaced.
    pub fn request_seek(&self) {
        self.seek_requested.store(true, Ordering::Release);
    }

    /// Clear the seek-pending flag.
    pub fn clear_seek(&self) {
        self.seek_requested.store(false, Ordering::Release);
    }

    /// Check if a seek has been requested.
    pub fn is_seek_pending(&self) -> bool {
        self.seek_requested.load(Ordering::Acquire)
    }
}

impl StreamSource for PipeSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, cap: Capability) -> bool {
        matches!(cap, Capability::Download)
        // Seek is complex in pipe mode — handled externally via re-fetch.
    }

    fn as_any(&self) -> &dyn std::any::Any { self }

    fn pipe_writer(&self) -> Option<Arc<crate::audio::stream::pipe::PipeWriter>> {
        self.writer.lock().unwrap().clone()
    }

    fn open(
        &self,
        _seek_to: Option<u64>,
    ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
        let (writer, reader) = crate::audio::stream::pipe::new_pipe();
        *self.writer.lock().unwrap() = Some(Arc::new(writer));
        self.seek_requested.store(false, Ordering::Release);
        Ok(Box::new(reader))
    }
}
