//! RadioSource — Icecast / Shoutcast / plain HTTP audio streams.
//!
//! These are live streams with no seek support and no known content length.

use crate::audio::engine::types::HttpClient;
use crate::audio::error::PlaybackError;
use crate::models::StreamType;

#[cfg(not(target_os = "android"))]
use super::NonSeekable;
use super::{Capability, ReadSeek, SourceInfo, SourceKind, StreamSource};
use std::sync::Arc;

pub struct RadioSource {
    info: SourceInfo,
    client: Arc<HttpClient>,
}

impl RadioSource {
    pub fn new(url: &str, client: Arc<HttpClient>) -> Self {
        Self {
            info: SourceInfo {
                kind: SourceKind::Radio,
                stream_type: StreamType::Live {
                    buffer_window_bytes: 20 * 1024 * 1024,
                },
                uri: url.to_string(),
                title: None,
                artist: None,
                album: None,
            },
            client,
        }
    }
}

impl StreamSource for RadioSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, capability: Capability) -> bool {
        matches!(capability, Capability::Download)
    }

    fn as_any(&self) -> &dyn std::any::Any { self }

    fn open(
        &self,
        _seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
        #[cfg(target_os = "android")]
        {
            use crate::audio::stream::pipe;
            use std::sync::Arc;
            use std::thread;

            let (writer, reader) = pipe::new_pipe();
            let writer = Arc::new(writer);
            let fetch_writer = writer.clone();
            let client = Arc::clone(&self.client);
            let uri = self.info.uri.clone();

            thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(r) => r,
                    Err(e) => {
                        fetch_writer.set_error(format!("tokio runtime failed: {}", e));
                        return;
                    }
                };
                rt.block_on(async move {
                    loop {
                        let mut resp = match client
                            .get(&uri)
                            .header("User-Agent", "Mozilla/5.0 (Android) AppleWebKit/537.36")
                            .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
                            .header("Icy-MetaData", "1")
                            .send()
                            .await
                        {
                            Ok(r) => r,
                            Err(e) => {
                                fetch_writer.set_error(format!("HTTP request failed: {}", e));
                                return;
                            }
                        };

                        if !resp.status().is_success() {
                            fetch_writer.set_error(format!("HTTP error: {}", resp.status()));
                            return;
                        }

                        let stream_done = loop {
                            match resp.chunk().await {
                                Ok(Some(data)) => fetch_writer.push(&data),
                                Ok(None) => {
                                    fetch_writer.end();
                                    break true;
                                }
                                Err(e) => {
                                    fetch_writer.set_error(format!("Stream error: {}", e));
                                    break false;
                                }
                            }
                        };

                        if stream_done { return; }
                        std::thread::sleep(std::time::Duration::from_secs(3));
                    }
                });
            });

            return Ok(Box::new(reader));
        }

        #[cfg(not(target_os = "android"))]
        {
            Ok(Box::new(ReconnectingRadioReader::new(
                self.client.clone(),
                self.info.uri.clone(),
            )))
        }
    }
}

#[cfg(not(target_os = "android"))]
struct ReconnectingRadioReader {
    client: std::sync::Arc<crate::audio::engine::types::HttpClient>,
    uri: String,
    resp: Option<reqwest::blocking::Response>,
}

#[cfg(not(target_os = "android"))]
impl ReconnectingRadioReader {
    fn new(
        client: std::sync::Arc<crate::audio::engine::types::HttpClient>,
        uri: String,
    ) -> Self {
        Self { client, uri, resp: None }
    }

    fn open_connection(&mut self) -> Result<(), String> {
        if self.resp.is_some() {
            return Ok(());
        }
        let resp = self
            .client
            .get(&self.uri)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
            .header("Icy-MetaData", "1")
            .send()
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            self.resp = None;
            return Err(format!("HTTP error: {}", resp.status()));
        }

        self.resp = Some(resp);
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
impl std::io::Read for ReconnectingRadioReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.resp.is_none() {
                if let Err(_) = self.open_connection() {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    continue;
                }
            }

            let n = self.resp.as_mut().unwrap().read(buf).map_err(|e| {
                self.resp = None;
                std::io::Error::new(std::io::ErrorKind::Other, e)
            })?;

            if n == 0 {
                self.resp = None;
                std::thread::sleep(std::time::Duration::from_secs(3));
                continue;
            }

            return Ok(n);
        }
    }
}

#[cfg(not(target_os = "android"))]
impl std::io::Seek for ReconnectingRadioReader {
    fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "radio streams do not support seeking",
        ))
    }
}
