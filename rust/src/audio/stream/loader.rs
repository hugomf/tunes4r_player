use log::{debug, info};

use crate::audio::decoder::symphonia_decoder::SymphoniaDecoder;
use crate::audio::error::{
    DETECT_HEAD_TIMEOUT_MS, DETECT_MAX_RETRIES, DETECT_RETRY_DELAY_MS,
    PREFILL_LIVE_BYTES, PREFILL_LIVE_TIMEOUT_MS,
};
use crate::audio::http::run_async;
use crate::audio::buffer::AdaptiveBuffer;
use crate::audio::stream::buffer::StreamBuffer;
use crate::audio::stream::reader::SeekableStreamReader;
use crate::models::{PlaybackState, StreamMetadata, StreamType};

use std::sync::Arc;
use std::time::Duration;

pub fn detect_stream_type(url: &str, client: &Arc<reqwest::Client>) -> StreamMetadata {
    debug!("[detect] checking stream: {}", url);
    let url_owned = url.to_string();
    let client_clone = Arc::clone(client);

    run_async(async move {
        let extract_content_type = |headers: &reqwest::header::HeaderMap| -> Option<String> {
            headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(';').next().unwrap_or(s).trim().to_lowercase())
        };

        let mut last_error = None;
        for attempt in 0..DETECT_MAX_RETRIES {
            if attempt > 0 {
                debug!(
    "[detect] retry attempt {}/{} after {}ms",
    attempt + 1,
                    DETECT_MAX_RETRIES,
                    DETECT_RETRY_DELAY_MS
);
                tokio::time::sleep(Duration::from_millis(DETECT_RETRY_DELAY_MS)).await;
            }

            let timeout = Duration::from_millis(DETECT_HEAD_TIMEOUT_MS);

            let get_resp = client_clone
                .get(&url_owned)
                .header("User-Agent", "Mozilla/5.0")
                .header("Range", "bytes=0-0")
                .timeout(timeout)
                .send()
                .await;

            match get_resp {
                Ok(resp) => {
                    info!(
                        "[detect] GET Range status: {} (attempt {})",
                        resp.status(),
                        attempt + 1
                    );

                    for (name, value) in resp.headers().iter() {
                        let name_str = name.as_str();
                        if name_str.eq_ignore_ascii_case("accept-ranges")
                            || name_str.eq_ignore_ascii_case("content-range")
                            || name_str.eq_ignore_ascii_case("content-length")
                            || name_str.eq_ignore_ascii_case("content-type")
                        {
                            debug!("[detect] Header {}: {:?}", name_str, value);
                        }
                    }

                    let ct = extract_content_type(resp.headers());
                    let accept_ranges = resp
                        .headers()
                        .get("accept-ranges")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.contains("bytes"))
                        .unwrap_or(false);
                    debug!("[detect] accept_ranges parsed: {}", accept_ranges);

                    let total_bytes_from_range =
                        if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                            resp.headers()
                                .get("content-range")
                                .and_then(|v| v.to_str().ok())
                                .and_then(|s| s.split('/').next_back())
                                .and_then(|s| s.parse::<u64>().ok())
                        } else {
                            None
                        };

                    let total_bytes_from_length = resp.content_length();
                    let total_bytes = total_bytes_from_range.or(total_bytes_from_length);

                    debug!(
    "[detect] total_bytes from range: {:?}, from length: {:?}, final: {:?}",
    total_bytes_from_range, total_bytes_from_length, total_bytes
);

                    let is_seekable = total_bytes.is_some();

                    if is_seekable {
                        let total = total_bytes.unwrap_or(0);
                        debug!(
    "[detect] stream is SEEKABLE, total_bytes: {}, accept_ranges: {}",
    total, accept_ranges
);
                        return StreamMetadata {
                            total_bytes: Some(total),
                            is_seekable: true,
                            stream_type: StreamType::Seekable { total_bytes: total },
                            content_type: ct,
                        };
                    } else {
                        debug!("[detect] stream is LIVE (no Content-Length, truly streaming)");
                        return StreamMetadata {
                            content_type: ct,
                            ..StreamMetadata::default()
                        };
                    }
                }
                Err(e) => {
                    debug!(
    "[detect] GET request failed (attempt {}): {:?}",
    attempt + 1,
                        e
);
                    last_error = Some(e);
                }
            }
        }

        debug!(
    "[detect] All {} detection attempts failed, last error: {:?}",
    DETECT_MAX_RETRIES, last_error
);
        debug!("[detect] Defaulting to LIVE stream (non-seekable)");

        StreamMetadata {
            ..StreamMetadata::default()
        }
    })
}

pub enum AudioSource {
    Symphonia(SymphoniaDecoder),
}

impl AudioSource {
    pub fn total_duration(&self) -> Option<std::time::Duration> {
        match self {
            AudioSource::Symphonia(dec) => dec.total_duration(),
        }
    }
    pub fn into_source(self) -> Box<dyn rodio::Source<Item = f32>> {
        match self {
            AudioSource::Symphonia(dec) => Box::new(dec.into_source()),
        }
    }
}

pub struct AsyncLoadSuccess {
    pub source: AudioSource,
    pub metadata: StreamMetadata,
    pub url: String,
    pub total_ms: u64,
    pub seek_to_ms: Option<u64>,
}

pub enum StreamLoadResult {
    Phase(PlaybackState),
    Ready(Box<AsyncLoadSuccess>),
    Failed(String),
}

pub fn load_stream_background(
    url: String,
    client: Arc<reqwest::Client>,
    tx: std::sync::mpsc::SyncSender<StreamLoadResult>,
) {
    macro_rules! send_phase {
        ($state:expr) => {
            if tx.send(StreamLoadResult::Phase($state)).is_err() {
                debug!("[load] receiver dropped, aborting");
                return;
            }
        };
    }

    send_phase!(PlaybackState::Connecting);
    debug!("[load] detecting stream type: {}", url);
    let metadata = detect_stream_type(&url, &client);
    let stream_type = metadata.stream_type.clone();
    debug!(
    "[load] seekable={}, stream_type={:?}, content_type={:?}",
    metadata.is_seekable, stream_type, metadata.content_type
);

    // WebM/Opus is now supported via symphonia-adapter-libopus
    if let Some(ref ct) = metadata.content_type {
        debug!("[load] content-type: {}", ct);
    }

    let total_bytes_for_ui = metadata.total_bytes;
    send_phase!(PlaybackState::Buffering {
        buffered_bytes: 0,
        total_bytes: total_bytes_for_ui,
    });

    let mut reader = SeekableStreamReader::new(
        url.clone(),
        StreamBuffer::new(stream_type),
        Arc::clone(&client),
    );
    reader.start_download_from(0);
    if let Some(total) = metadata.total_bytes {
        reader.buffer.lock().set_total_bytes(total);
    }

    let mut adaptive_buffer = AdaptiveBuffer::new();
    let prefill_start = std::time::Instant::now();
    let timeout_ms = if metadata.is_seekable {
        adaptive_buffer.config().prefill_timeout_ms
    } else {
        PREFILL_LIVE_TIMEOUT_MS
    };

    let target_prefill = if metadata.is_seekable {
        adaptive_buffer.config().prefill_bytes
    } else {
        match metadata.content_type.as_deref() {
            Some("audio/mpeg") => 64 * 1024,
            Some("audio/ogg") | Some("audio/opus") => 128 * 1024,
            _ => PREFILL_LIVE_BYTES,
        }
    };

    info!(
        "[load] waiting for prefill: {} bytes (target: {} bytes, network: {:?})",
        target_prefill,
        target_prefill,
        adaptive_buffer.network_quality()
    );

    let mut last_buffered = 0usize;
    let mut stall_count = 0;
    let mut stall_warning_printed = false;

    loop {
        let (start, end, complete) = reader.buffer_info();
        let buffered = end.saturating_sub(start);

        if buffered == last_buffered {
            stall_count += 1;
            if stall_count >= 30 && !stall_warning_printed {
                debug!(
    "[load] WARNING: Buffer stalled at {} bytes for 3+ seconds",
    buffered
);
                stall_warning_printed = true;
            }
        } else {
            stall_count = 0;
            stall_warning_printed = false;
            last_buffered = buffered;
        }

        let elapsed = prefill_start.elapsed().as_millis();
        if elapsed > 0 && elapsed.is_multiple_of(500) {
            adaptive_buffer.record_sample(buffered, elapsed);
        }

        let _ = tx.send(StreamLoadResult::Phase(PlaybackState::Buffering {
            buffered_bytes: buffered as u64,
            total_bytes: total_bytes_for_ui,
        }));

        if buffered >= target_prefill {
            debug!("[load] prefill complete: {} bytes", buffered);
            break;
        }

        if buffered >= 16 * 1024 && stall_count >= 30 {
            debug!(
    "[load] Starting playback with {} bytes after stall",
    buffered
);
            break;
        }

        if complete && !metadata.is_seekable && buffered > 0 {
            debug!(
    "[load] Download complete with {} bytes, starting playback",
    buffered
);
            break;
        }

        if elapsed >= timeout_ms {
            if buffered > 0 {
                debug!(
    "[load] Timeout after {}ms with {} bytes, starting playback",
    elapsed, buffered
);
                break;
            } else {
                let _ = tx.send(StreamLoadResult::Failed(
                    "Server unreachable or returned no data".to_string(),
                ));
                return;
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    send_phase!(PlaybackState::Decoding);
    debug!("[load] creating decoder");

    let source = match SymphoniaDecoder::new(reader, metadata.clone()) {
        Ok((decoder, _)) => AudioSource::Symphonia(decoder),
        Err(e) => {
            let error_msg = if e.contains("WAV") {
                "Unsupported audio format: WAV files are not supported for streaming."
            } else if e.contains("unsupported") || e.contains("unknown") {
                "Unsupported audio format."
            } else {
                &format!("Unsupported audio format: {}", e)[..]
            };
            let _ = tx.send(StreamLoadResult::Failed(error_msg.to_string()));
            return;
        }
    };

    let total_ms = source
        .total_duration()
        .map(|d: std::time::Duration| d.as_millis() as u64)
        .unwrap_or(0);

    let _ = tx.send(StreamLoadResult::Ready(Box::new(AsyncLoadSuccess {
        source,
        metadata,
        url,
        total_ms,
        seek_to_ms: None,
    })));
}

pub fn seek_stream_background(
    url: String,
    client: Arc<reqwest::Client>,
    position_ms: u64,
    total_bytes: u64,
    saved_total_ms: u64,
    tx: std::sync::mpsc::SyncSender<StreamLoadResult>,
) {
    debug!("[seek-bg] === BACKGROUND SEEK WORKER STARTED ===");
    debug!(
    "[seek-bg] target: {} ms, total_bytes: {}, saved_total_ms: {}",
    position_ms, total_bytes, saved_total_ms
);

    macro_rules! send {
        ($msg:expr) => {
            if tx.send($msg).is_err() {
                debug!("[seek-bg] receiver dropped, aborting");
                return;
            }
        };
    }

    debug!("[seek-bg] starting backward seek to {} ms", position_ms);

    let mut reader = SeekableStreamReader::new(
        url,
        StreamBuffer::new(StreamType::Seekable { total_bytes }),
        Arc::clone(&client),
    );
    let start_byte = 0;
    reader.start_download_from(start_byte);
    reader.buffer.lock().set_total_bytes(total_bytes);

    const HEADER_PREFILL: usize = 64 * 1024;
    let deadline = std::time::Instant::now();
    let timeout_ms: u128 = 3_000;

    loop {
        let (start, end, complete) = reader.buffer_info();
        let buffered = end.saturating_sub(start) as u64;

        let _ = tx.send(StreamLoadResult::Phase(PlaybackState::Buffering {
            buffered_bytes: buffered,
            total_bytes: Some(total_bytes),
        }));

        if buffered >= HEADER_PREFILL as u64 {
            debug!("[seek-bg] header prefill done: {} bytes", buffered);
            break;
        }
        if complete || deadline.elapsed().as_millis() >= timeout_ms {
            if buffered == 0 {
                let _ = tx.send(StreamLoadResult::Failed(
                    "Server unreachable during seek".to_string(),
                ));
                return;
            }
            debug!("[seek-bg] partial prefill ({} bytes), proceeding", buffered);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    send!(StreamLoadResult::Phase(PlaybackState::Decoding));

    let mut symphonia_reader = SeekableStreamReader::new(
        String::new(),
        StreamBuffer::new(StreamType::Seekable { total_bytes }),
        Arc::clone(&client),
    );
    symphonia_reader.start_download_from(0);
    symphonia_reader.buffer.lock().set_total_bytes(total_bytes);

    let (decoder, metadata) = match SymphoniaDecoder::new(
        symphonia_reader,
        StreamMetadata {
            total_bytes: Some(total_bytes),
            is_seekable: true,
            stream_type: StreamType::Seekable { total_bytes },
            content_type: None,
        },
    ) {
        Ok(result) => result,
        Err(e) => {
            let _ = tx.send(StreamLoadResult::Failed(format!(
                "Decoder failed during seek: {}",
                e
            )));
            return;
        }
    };

    let total_ms = metadata.total_bytes.unwrap_or(saved_total_ms);

    let _ = tx.send(StreamLoadResult::Ready(Box::new(AsyncLoadSuccess {
        source: AudioSource::Symphonia(decoder),
        metadata,
        url: String::new(),
        total_ms,
        seek_to_ms: Some(position_ms),
    })));
}