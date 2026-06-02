//! RetryDecorator — wraps a StreamSource and reconnects on read errors.
//!
//! When the underlying reader fails, the decorator sleeps briefly then
//! re-opens the inner source and resumes streaming. Useful for radio
//! streams where the TCP connection may drop intermittently.

use crate::audio::error::PlaybackError;
use crate::audio::stream::pipe::new_pipe;
use crate::audio::stream::source::{Capability, SourceInfo, StreamSource};
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
        RetryDecorator::with_config(inner, 5, 2000)
    }

    pub fn with_config(inner: Box<dyn StreamSource>, max_retries: u32, retry_delay_ms: u64) -> Self {
        let info = inner.info().clone();
        Self {
            inner: Arc::new(Mutex::new(inner)),
            info,
            max_retries,
            retry_delay: Duration::from_millis(retry_delay_ms),
        }
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
    ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
        let initial = self.inner.lock().open(seek_to)?;

        let inner = self.inner.clone();
        let (writer, reader) = new_pipe();
        let writer = Arc::new(writer);
        let retry_delay = self.retry_delay;
        let max_retries = self.max_retries;

        thread::Builder::new()
            .name("retry-prefetch".into())
            .spawn(move || {
                let mut current: Box<dyn Read + Send + Sync + 'static> = initial;
                let mut attempts: u32 = 0;
                let mut buf = [0u8; 8192];

                loop {
                    match current.read(&mut buf) {
                        Ok(0) => {
                            log::info!("[retry] Inner source ended");
                            break;
                        }
                        Ok(n) => {
                            attempts = 0;
                            writer.push(&buf[..n]);
                        }
                        Err(e) => {
                            attempts += 1;
                            log::warn!(
                                "[retry] Read error (attempt {}/{}): {}",
                                attempts,
                                max_retries,
                                e
                            );

                            if attempts > max_retries {
                                log::error!("[retry] Max retries ({}) exceeded", max_retries);
                                writer.end();
                                return;
                            }

                            thread::sleep(retry_delay);

                            match inner.lock().open(seek_to) {
                                Ok(new_reader) => {
                                    log::info!("[retry] Re-connected, resuming stream");
                                    current = new_reader;
                                }
                                Err(e) => {
                                    log::error!("[retry] Re-open failed: {}", e);
                                }
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
                    stream_type: crate::models::StreamType::Live { buffer_window_bytes: 4096 },
                    uri: "test://fickle".into(),
                    title: None,
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
        ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
            let data = self.data.clone();
            let fail_every = self.fail_every;
            Ok(Box::new(FickleReader { data, pos: 0, fail_every }))
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
            if self.fail_every > 0 && self.pos > 0 && self.pos % self.fail_every == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "simulated failure"));
            }
            let remaining = self.data.len() - self.pos;
            let n = buf.len().min(remaining);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn test_retry_recovers_from_errors() {
        let data: Vec<u8> = (0..200).collect();
        let source = FickleSource::new(data.clone(), 50);
        let decorator = RetryDecorator::with_config(Box::new(source), 10, 10);

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
