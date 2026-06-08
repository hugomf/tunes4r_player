//! RadioSource — Icecast / Shoutcast / plain HTTP audio streams.
//!
//! These are live streams with no seek support and no known content length.

use crate::audio::engine::types::HttpClient;
use crate::audio::error::PlaybackError;
use crate::models::StreamType;

use super::{Capability, NonSeekable, ReadSeek, SourceInfo, SourceKind, StreamSource};
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
        #[cfg(not(target_os = "android"))]
        {
            let resp = self
                .client
                .get(&self.info.uri)
                        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
                .header("Icy-MetaData", "0")
                .header("Connection", "close")
                .send()
                .map_err(|e| PlaybackError::HttpStream {
                    operation: "GET".into(),
                    detail: e.to_string(),
                })?;

            if !resp.status().is_success() {
                return Err(PlaybackError::HttpStatus {
                    url: self.info.uri.clone(),
                    status_code: resp.status().as_u16(),
                    detail: "radio stream returned error".into(),
                });
            }

            Ok(Box::new(NonSeekable(resp)))
        }

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
                        fetch_writer
                            .set_error(format!("Failed to create tokio runtime: {}", e));
                        return;
                    }
                };
                rt.block_on(async move {
                    let mut resp = match client
                        .get(&uri)
                .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                        .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
                        .header("Icy-MetaData", "0")
                        .header("Connection", "close")
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
                        fetch_writer
                            .set_error(format!("HTTP error: {}", resp.status()));
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
                                    .set_error(format!("Stream error: {}", e));
                                return;
                            }
                        }
                    }
                });
            });

            Ok(Box::new(reader))
        }
    }
}
