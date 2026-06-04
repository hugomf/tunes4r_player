use std::time::Duration;

use log::{info, warn};
use stream_download::http::reqwest::Client as ReqwestAsyncClient;
use stream_download::storage::temp::TempStorageProvider;
use stream_download::{Settings as StreamSettings, StreamDownload};

use crate::audio::stream::pipe::{PipeReader, PipeWriter};

pub struct StreamDownloader {
    pub pipe_reader: PipeReader,
    pub pipe_writer: PipeWriter,
}

impl StreamDownloader {
    pub fn new() -> Self {
        let (pipe_writer, pipe_reader) = crate::audio::stream::pipe::new_pipe();
        Self { pipe_reader, pipe_writer }
    }

    pub fn fetch_stream(
        url: &str,
        _buffer_bytes: u64,
    ) -> Result<(PipeReader, u64), String> {
        Self::fetch_stream_with_cache(url, None)
    }

    pub fn fetch_stream_with_cache(
        url: &str,
        _cache_dir: Option<&str>,
    ) -> Result<(PipeReader, u64), String> {
        let (pipe_writer, pipe_reader) = crate::audio::stream::pipe::new_pipe();

        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| format!("Failed to create tokio runtime: {}", e))?;

        let pipe_writer_clone = pipe_writer.clone();
        let url_owned = url.to_string();

        rt.spawn(async move {
            let _client = ReqwestAsyncClient::new();
            let settings = StreamSettings::default();

            let stream_dl = match StreamDownload::new_http(url_owned.parse().unwrap(), TempStorageProvider::with_prefix(".stream_download_"), settings).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("Failed to create stream download: {:?}", e);
                    pipe_writer_clone.end();
                    return;
                }
            };

            let mut buf = [0u8; 8192];
            let mut reader = stream_dl;
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(n) => {
                        if n > 0 {
                            pipe_writer_clone.push(&buf[..n]);
                        } else {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                    Err(e) => {
                        warn!("Stream download error: {}", e);
                        break;
                    }
                }
            }
            pipe_writer_clone.end();
        });

        let content_length = 0u64;
        Ok((pipe_reader, content_length))
    }
}

impl Default for StreamDownloader {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_stream_downloader() -> StreamDownloader {
    info!("[stream_download] Creating StreamDownloader");
    StreamDownloader::new()
}

pub async fn fetch_stream_async(
    url: &str,
) -> Result<PipeReader, String> {
    let (pipe_writer, pipe_reader) = crate::audio::stream::pipe::new_pipe();

    let _client = ReqwestAsyncClient::new();
    let settings = StreamSettings::default();

    let stream_dl = match StreamDownload::new_http(url.parse().unwrap(), TempStorageProvider::with_prefix(".stream_download_"), settings).await {
        Ok(s) => s,
        Err(e) => {
            return Err(format!("Failed to create stream download: {:?}", e));
        }
    };

    let pipe_writer_clone = pipe_writer.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        let mut reader = stream_dl;
        while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
            if n > 0 {
                pipe_writer_clone.push(&buf[..n]);
            } else {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        pipe_writer_clone.end();
    });

    Ok(pipe_reader)
}