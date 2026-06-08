//! AdaptiveBufferDecorator — wraps a StreamSource with a background pre-fetch pipe.
//!
//! The inner source's `open()` is called on first access; bytes are pulled into a
//! bounded buffer in a background thread so the decoder never blocks on I/O.

use crate::audio::error::PlaybackError;
use crate::audio::stream::pipe::new_pipe;
use crate::audio::stream::source::{Capability, ReadSeek, SourceInfo, StreamSource};
use std::io::Read;
use std::sync::Arc;
use std::thread;

/// Wraps a source with a background read-ahead buffer.
pub struct AdaptiveBufferDecorator {
    inner: Box<dyn StreamSource>,
}

impl AdaptiveBufferDecorator {
    pub fn new(inner: Box<dyn StreamSource>) -> Self {
        Self { inner }
    }
}

impl StreamSource for AdaptiveBufferDecorator {
    fn info(&self) -> &SourceInfo {
        self.inner.info()
    }

    fn supports(&self, capability: Capability) -> bool {
        self.inner.supports(capability)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn open(
        &self,
        seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
        let mut inner_reader = self.inner.open(seek_to)?;
        let (writer, reader) = new_pipe();
        let writer = Arc::new(writer);

        thread::Builder::new()
            .name("adaptive-prefetch".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match inner_reader.read(&mut buf) {
                        Ok(0) => {
                            log::info!("[adaptive] Inner source ended");
                            break;
                        }
                        Ok(n) => {
                            writer.push(&buf[..n]);
                        }
                        Err(e) => {
                            log::error!("[adaptive] Read error: {}", e);
                            writer.end();
                            return;
                        }
                    }
                }
                writer.end();
            })
            .map_err(|e| PlaybackError::ThreadSpawn {
                operation: "adaptive-prefetch".into(),
                detail: e.to_string(),
            })?;

        Ok(Box::new(reader))
    }

    fn total_bytes(&self) -> Option<u64> {
        self.inner.total_bytes()
    }

    fn pipe_writer(&self) -> Option<std::sync::Arc<crate::audio::stream::pipe::PipeWriter>> {
        self.inner.pipe_writer()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::stream::source::SourceKind;

    struct SilentReader {
        info: SourceInfo,
    }

    impl SilentReader {
        fn new() -> Self {
            Self {
                info: SourceInfo {
                    kind: SourceKind::Radio,
                    stream_type: crate::models::StreamType::Seekable { total_bytes: 0 },
                    uri: "test://silent".into(),
                    title: None,
                    artist: None,
                    album: None,
                },
            }
        }
    }

    impl StreamSource for SilentReader {
        fn info(&self) -> &SourceInfo {
            &self.info
        }
        fn supports(&self, _: Capability) -> bool {
            false
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn open(
            &self,
            _: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
            Ok(Box::new(std::io::Cursor::new(vec![0u8; 100])))
        }
    }

    #[test]
    fn test_adaptive_delegates_info() {
        let decorator = AdaptiveBufferDecorator::new(Box::new(SilentReader::new()));
        assert_eq!(decorator.info().uri, "test://silent");
        assert_eq!(decorator.info().kind, SourceKind::Radio);
    }

    #[test]
    fn test_adaptive_returns_data() {
        let decorator = AdaptiveBufferDecorator::new(Box::new(SilentReader::new()));
        let mut reader = decorator.open(None).unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf.len(), 100);
    }
}
