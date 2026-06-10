//! RetryDecorator — wraps a StreamSource and reconnects on read errors
//! or EOF once some data has already been received.
//!
//! This handles radio streams that send a finite audio chunk, then end
//! the HTTP connection, requiring a reconnect to resume playback.

use crate::audio::error::PlaybackError;
use crate::audio::stream::pipe::new_pipe;
use crate::audio::stream::source::{Capability, ReadSeek, SourceInfo, StreamSource};
use parking_lot::Mutex;
use std::io::Read;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct RetryDecorator {
    inner: Arc<Mutex<Box<dyn StreamSource>>>,
    info: SourceInfo,
    max_retries: u32,
    retry_delay: Duration,
}

impl RetryDecorator {
    pub fn new(inner: Box<dyn StreamSource>) -> Self {
        Self::new_with_config(inner, 5, 2000)
    }

    pub fn new_with_config(
        inner: Box<dyn StreamSource>,
        max_retries: u32,
        retry_delay_ms: u64,
    ) -> Self {
        let info = inner.info().clone();
        Self {
            inner: Arc::new(Mutex::new(inner)),
            info,
            max_retries,
            retry_delay: Duration::from_millis(retry_delay_ms),
        }
    }

    pub fn with_config(
        inner: Box<dyn StreamSource>,
        max_retries: u32,
        retry_delay_ms: u64,
    ) -> Self {
        Self::new_with_config(inner, max_retries, retry_delay_ms)
    }
}

impl StreamSource for RetryDecorator {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, capability: Capability) -> bool {
        self.inner.lock().supports(capability)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn open(
        &self,
        seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
        let initial = self.inner.lock().open(seek_to)?;

        let inner = self.inner.clone();
        let (writer, reader) = new_pipe();
        let writer = Arc::new(writer);
        let retry_delay = self.retry_delay;
        let max_retries = self.max_retries;

        thread::Builder::new()
            .name("retry-prefetch".into())
            .spawn(move || {
                let mut current: Box<dyn ReadSeek + Send + Sync + 'static> = initial;
                let mut attempts: u32 = 0;
                let mut buf = [0u8; 32768];

                loop {
                    match current.read(&mut buf) {
                        Ok(0) => {
                            if attempts >= max_retries {
                                break;
                            }
                            attempts += 1;
                            log::warn!(
                                "[retry] EOF at connection end (attempt {}/{})",
                                attempts,
                                max_retries
                            );
                            thread::sleep(retry_delay);
                            match inner.lock().open(seek_to) {
                                Ok(new_reader) => current = new_reader,
                                Err(_) => break,
                            }
                        }
                        Ok(n) => {
                            attempts = 0;
                            writer.push(&buf[..n]);
                        }
                        Err(_) => {
                            if attempts >= max_retries {
                                break;
                            }
                            attempts += 1;
                            log::warn!("[retry] Read error (attempt {}/{})", attempts, max_retries);
                            thread::sleep(retry_delay);
                            match inner.lock().open(seek_to) {
                                Ok(new_reader) => current = new_reader,
                                Err(_) => break,
                            }
                        }
                    }
                }

                writer.end();
            })
            .map_err(|e| PlaybackError::ThreadSpawn {
                operation: "retry-prefetch".into(),
                detail: e.to_string(),
            })?;

        Ok(Box::new(reader))
    }

    fn total_bytes(&self) -> Option<u64> {
        self.inner.lock().total_bytes()
    }

    fn pipe_writer(&self) -> Option<Arc<crate::audio::stream::pipe::PipeWriter>> {
        self.inner.lock().pipe_writer()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::stream::source::SourceKind;
    use std::io::{Seek, SeekFrom};

    struct FickleSource {
        fail_every: usize,
        data: Vec<u8>,
        info: SourceInfo,
    }

    impl FickleSource {
        fn new(data: Vec<u8>, fail_every: usize) -> Self {
            Self {
                fail_every,
                data,
                info: SourceInfo {
                    kind: SourceKind::Radio,
                    stream_type: crate::models::StreamType::Live {
                        buffer_window_bytes: 4096,
                    },
                    uri: "test://fickle".into(),
                    title: None,
                    artist: None,
                    album: None,
                },
            }
        }
    }

    impl StreamSource for FickleSource {
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
            _seek_to: Option<u64>,
        ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
            let data = self.data.clone();
            let fail_every = self.fail_every;
            Ok(Box::new(FickleReader {
                data,
                pos: 0,
                fail_every,
            }))
        }
    }

    struct FickleReader {
        data: Vec<u8>,
        pos: usize,
        fail_every: usize,
    }

    impl Read for FickleReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            if self.fail_every > 0 && self.pos > 0 && self.pos.is_multiple_of(self.fail_every) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "simulated failure",
                ));
            }
            let remaining = self.data.len() - self.pos;
            let n = buf.len().min(remaining);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    impl Seek for FickleReader {
        fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "fickle reader is not seekable",
            ))
        }
    }

    #[test]
    fn test_retry_recovers_from_errors() {
        let data: Vec<u8> = (0..200).collect();
        let source = FickleSource::new(data.clone(), 50);
        let decorator = RetryDecorator::with_config(Box::new(source), 20, 10);

        let mut reader = decorator.open(None).unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, data);
    }

    #[test]
    fn test_retry_delegates_info() {
        let source = FickleSource::new(vec![], 0);
        let decorator = RetryDecorator::new(Box::new(source));
        assert_eq!(decorator.info().kind, SourceKind::Radio);
        assert_eq!(decorator.info().uri, "test://fickle");
    }
}
