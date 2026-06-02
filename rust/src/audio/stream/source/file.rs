//! FileSource — local audio files with full seek support.

use crate::audio::error::PlaybackError;
use crate::models::StreamType;

use super::{Capability, SourceInfo, SourceKind, StreamSource};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub struct FileSource {
    info: SourceInfo,
}

impl FileSource {
    pub fn new(path: &str) -> Self {
        let total = File::open(path).ok().and_then(|f| f.metadata().ok()).map(|m| m.len());
        Self {
            info: SourceInfo {
                kind: SourceKind::File,
                stream_type: StreamType::Seekable {
                    total_bytes: total.unwrap_or(0),
                },
                uri: path.to_string(),
                title: Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string()),
            },
        }
    }
}

impl StreamSource for FileSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, capability: Capability) -> bool {
        matches!(capability, Capability::Seek | Capability::Download)
    }

    fn as_any(&self) -> &dyn std::any::Any { self }

    fn open(
        &self,
        _seek_to: Option<u64>,
    ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
        let file = File::open(&self.info.uri).map_err(|e| PlaybackError::FileOpen {
            path: self.info.uri.clone(),
            source: e,
        })?;

        // Seek is handled by the decode loop via fast_forward_stream_seek;
        // we just provide the raw file. The decoder will fast-forward packets.
        Ok(Box::new(file))
    }
}
