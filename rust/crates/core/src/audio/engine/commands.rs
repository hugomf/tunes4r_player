//! Public commands (methods) for the PlaybackEngine.

use super::types::{PlaybackEngine, PlaybackType};
use crate::audio::error::PlaybackError;
use crate::audio::stream::cpal_source::AudioBuffer;
#[cfg(not(target_os = "android"))]
use crate::audio::stream::handling::ByteCountingRead;

use crate::models::{
    DownloadBuffer, EngineEvent, PlaybackState, ENGINE_EVENT_END_OF_STREAM,
    ENGINE_EVENT_STATE_CHANGED,
};
use log::{debug, error, info, warn};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

impl PlaybackEngine {
    /// Play a URI with auto-detected source type and auto-configured pipeline.
    ///
    /// `buffer_size_ms` — optional fixed ring buffer capacity in ms.
    /// When `None`, the buffer is adaptively sized based on connection speed.
    pub fn play(&mut self, uri: &str, buffer_size_ms: Option<u64>) -> Result<(), PlaybackError> {
        use crate::audio::stream::source;
        let pipeline = source::from_uri(uri, self.http_client.clone(), None)?;
        self.buffer_size_ms_fixed
            .store(buffer_size_ms.unwrap_or(0), Ordering::Relaxed);
        self.play_pipeline(pipeline)
    }

    /// Play from a pre-built pipeline — full control over source + decorators.
    pub fn play_pipeline(
        &mut self,
        pipeline: Box<dyn crate::audio::stream::source::StreamSource>,
    ) -> Result<(), PlaybackError> {
        info!("[engine] play_pipeline: {}", pipeline.info().uri);

        // Stop any current playback
        self.should_stop.store(true, Ordering::Relaxed);
        self.stream_ended.store(false, Ordering::Relaxed);
        {
            let mut q = self.audio_queue.lock();
            q.clear();
        }
        if let Some(handle) = self.playback_handle.take() {
            Self::join_with_timeout(handle, "play");
        }

        self.set_state(PlaybackState::Connecting);
        self.load_error.lock().clear();
        self.stream_url = Some(pipeline.info().uri.clone());
        self.samples_played.store(0, Ordering::Relaxed);
        self.total_duration_ms.store(0, Ordering::Relaxed);
        self.buffer_ready.store(false, Ordering::Relaxed);
        self.seek_target_ms.store(0, Ordering::Relaxed);
        self.reset_download_buffer();
        self.push_position_reset();

        // Extract pipe writer (works through decorator layers)
        self.stream_pipe = pipeline.pipe_writer();

        // Open the reader
        let reader = pipeline.open(None)?;

        // Feed the adaptive ring buffer with real download progress for
        // Read-based sources (YouTube, progressive HTTP). The buffer poller
        // maps pipe_bytes_sent / pipe_total_bytes into the ring buffer's
        // write_offset_ms — without this, the buffer would always show
        // 100% complete for Read-based sources.
        if let Some(total) = pipeline.total_bytes() {
            self.pipe_total_bytes.store(total, Ordering::Relaxed);
        }

        // Set playback_type for backward compat
        let kind = pipeline.info().kind;
        let uri_for_playback = pipeline.info().uri.clone();
        #[cfg(target_os = "android")]
        let file_path_for_android = uri_for_playback.clone();
        #[cfg(target_os = "android")]
        let source_kind_for_android = kind;
        #[cfg(not(target_os = "android"))]
        let file_path_for_thread = uri_for_playback.clone();
        #[allow(unused_variables)]
        let source_kind = kind;
        self.playback_type = Some(match kind {
            crate::audio::stream::source::SourceKind::File => PlaybackType::File {
                path: uri_for_playback,
            },
            crate::audio::stream::source::SourceKind::Radio => PlaybackType::Stream {
                url: uri_for_playback,
                seek_byte_offset: 0,
            },
            crate::audio::stream::source::SourceKind::YouTube => PlaybackType::Stream {
                url: uri_for_playback,
                seek_byte_offset: 0,
            },
            crate::audio::stream::source::SourceKind::Pipe => PlaybackType::Pipe {
                url: uri_for_playback,
                video_id: None,
            },
            crate::audio::stream::source::SourceKind::Live => PlaybackType::Live {
                url: uri_for_playback,
                cache_max_ms: 30 * 60 * 1000,
            },
        });

        // Store source
        self.source = Some(pipeline);

        self.should_stop.store(false, Ordering::Relaxed);

        let audio_queue = self.audio_queue.clone();
        let buffer_ready = self.buffer_ready.clone();
        let is_playing_flag = self.is_playing_flag.clone();
        let should_stop = self.should_stop.clone();
        let samples_played = self.samples_played.clone();
        let sample_rate = self.sample_rate.clone();
        let channels = self.channels.clone();
        let total_duration_ms = self.total_duration_ms.clone();
        let load_error = self.load_error.clone();
        let seek_target_ms = self.seek_target_ms.clone();
        let _pipe_bytes_sent = self.pipe_bytes_sent.clone();
        let _seek_request = self.seek_request.clone();
        let stream_ended = self.stream_ended.clone();
        let event_queue = self.event_queue.clone();

        let handle = thread::Builder::new()
            .name("playback-decode".into())
            .spawn(move || {
                #[cfg(not(target_os = "android"))]
                match source_kind {
                    crate::audio::stream::source::SourceKind::File => {
                        crate::audio::decoder::file_decoder::play_file_internal(
                            file_path_for_thread,
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop.clone(),
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error.clone(),
                            seek_target_ms,
                        );
                    }
                    _ => {
                        let reader = ByteCountingRead::new(reader, _pipe_bytes_sent);
                        crate::audio::stream::handling::decode_and_play_from_read(
                            Box::new(reader),
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop.clone(),
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error.clone(),
                            seek_target_ms,
                            _seek_request,
                        );
                    }
                }
                #[cfg(target_os = "android")]
                match source_kind_for_android {
                    crate::audio::stream::source::SourceKind::File => {
                        crate::audio::decoder::file_decoder::play_file_internal(
                            file_path_for_android,
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop.clone(),
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error.clone(),
                            seek_target_ms,
                        );
                    }
                    _ => {
                        crate::audio::stream::handling::decode_and_play_from_read(
                            reader,
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop.clone(),
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error.clone(),
                            seek_target_ms,
                        );
                    }
                }

                // If the decode function returned without should_stop and without
                // an error, the stream ended naturally — notify the engine.
                if !should_stop.load(Ordering::Relaxed) && load_error.lock().is_empty() {
                    stream_ended.store(true, Ordering::Relaxed);
                    event_queue.lock().push_back(EngineEvent {
                        event_type: ENGINE_EVENT_END_OF_STREAM,
                        int_param: 0,
                    });
                    event_queue.lock().push_back(EngineEvent {
                        event_type: ENGINE_EVENT_STATE_CHANGED,
                        int_param: PlaybackState::Stopped.to_i32() as i64,
                    });
                }
            })
            .map_err(|e| PlaybackError::ThreadSpawn {
                operation: "play".into(),
                detail: e.to_string(),
            })?;

        self.playback_handle = Some(handle);
        self.set_state(PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        });

        // Spawn the buffer poller: maps the playhead (read_offset) and
        // download progress (write_offset) into the AdaptiveRingBuffer
        // for the UI. Also measures download throughput to adaptively
        // size the ring capacity — faster connections get a larger
        // buffer (more seek range), slower ones get a smaller buffer
        // (less wasted time buffering data that won't be reached).
        // Runs at 5Hz; stops when should_stop is set.
        let buffer_poller = {
            let download_buffer = self.download_buffer.clone();
            let pipe_bytes_sent = self.pipe_bytes_sent.clone();
            let pipe_total_bytes = self.pipe_total_bytes.clone();
            let total_duration_ms = self.total_duration_ms.clone();
            let samples_played = self.samples_played.clone();
            let sample_rate = self.sample_rate.clone();
            let channels = self.channels.clone();
            let should_stop = self.should_stop.clone();
            let buffer_size_ms = self.buffer_size_ms_fixed.clone();

            thread::Builder::new()
                .name("buffer-poller".into())
                .spawn(move || {
                    let mut last = DownloadBuffer::default();
                    // Throughput tracking: exponential moving average of
                    // bytes/sec over the last ~5 seconds (25 samples @ 200ms).
                    let mut ema_bps: f64 = 0.0;
                    let mut last_sent: u64 = 0;
                    let mut last_tick = std::time::Instant::now();
                    const EMA_ALPHA: f64 = 0.15; // smoothing factor

                    while !should_stop.load(Ordering::Relaxed) {
                        let now = std::time::Instant::now();
                        let dt = now.duration_since(last_tick).as_secs_f64();
                        last_tick = now;

                        let total_ms = total_duration_ms.load(Ordering::Relaxed);
                        let sent = pipe_bytes_sent.load(Ordering::Relaxed);
                        let total = pipe_total_bytes.load(Ordering::Relaxed);
                        let sp = samples_played.load(Ordering::Relaxed);
                        let sr = sample_rate.load(Ordering::Relaxed).max(1);
                        let ch = channels.load(Ordering::Relaxed).max(1);
                        // Playhead in file-ms (clamped to total).
                        let playhead_ms = ((sp as f64 / (sr as f64 * ch as f64)) * 1000.0) as u64;

                        // Update throughput EMA (only when bytes increased).
                        if sent > last_sent && dt > 0.0 {
                            let instant_bps = (sent - last_sent) as f64 / dt;
                            ema_bps = if ema_bps > 0.0 {
                                EMA_ALPHA * instant_bps + (1.0 - EMA_ALPHA) * ema_bps
                            } else {
                                instant_bps
                            };
                        }
                        last_sent = sent;

                        // Adaptive capacity based on throughput.
                        // When buffer_size_ms is set (>0), use that fixed value.
                        // Otherwise, dynamically size based on connection speed:
                        // Tiers (conservative — assume ~128 kbps audio):
                        //   >  5 MB/s  → 60 s buffer (fast, prefetch a lot)
                        //   >  1 MB/s  → 30 s buffer (broadband)
                        //   > 200 KB/s → 15 s buffer (3G / weak WiFi)
                        //   otherwise  →  8 s buffer (slow, don't waste time)
                        let fixed = buffer_size_ms.load(Ordering::Relaxed);
                        let capacity_ms = if fixed > 0 {
                            fixed
                        } else if ema_bps > 5_000_000.0 {
                            60_000
                        } else if ema_bps > 1_000_000.0 {
                            30_000
                        } else if ema_bps > 200_000.0 {
                            15_000
                        } else {
                            8_000
                        };

                        let new_buf = if total > 0 && total_ms > 0 {
                            // Progressive stream: map download bytes to ms.
                            let write_ms = ((sent as f64 / total as f64) * total_ms as f64) as u64;
                            let write_ms = write_ms.min(total_ms);
                            let is_complete = sent >= total;
                            // read_offset = playhead (the ring slides with playback).
                            let read_offset = playhead_ms.min(total_ms);
                            DownloadBuffer {
                                capacity_ms,
                                read_offset_ms: read_offset,
                                write_offset_ms: write_ms.max(read_offset),
                                total_ms,
                                is_complete,
                            }
                        } else if total_ms > 0 {
                            // Local file (or fully cached): entire duration
                            // is seekable from the start.
                            let read_offset = playhead_ms.min(total_ms);
                            DownloadBuffer {
                                capacity_ms,
                                read_offset_ms: read_offset,
                                write_offset_ms: total_ms,
                                total_ms,
                                is_complete: true,
                            }
                        } else {
                            // Duration not yet known — keep last known.
                            last
                        };

                        if new_buf != last {
                            *download_buffer.lock() = new_buf;
                            last = new_buf;
                        }

                        thread::sleep(std::time::Duration::from_millis(39));
                    }
                })
                .map_err(|e| PlaybackError::ThreadSpawn {
                    operation: "buffer-poller".into(),
                    detail: e.to_string(),
                })?
        };
        self.buffer_poller_handle = Some(buffer_poller);

        // Wait for initial buffer (10s max — if audio hasn't started by
        // then, something is fundamentally wrong).
        let timeout = Duration::from_secs(10);
        let start = std::time::Instant::now();
        while !self.buffer_ready.load(Ordering::Relaxed) && start.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(10));
        }
        if !self.buffer_ready.load(Ordering::Relaxed) {
            let err = self.load_error.lock().clone();
            if err.is_empty() {
                warn!("[engine] Playback failed to start within timeout");
            } else {
                error!("[engine] Playback error: {}", err);
            }
        } else {
            self.set_state(PlaybackState::Playing);
        }

        Ok(())
    }

    fn create_internal(headless: bool) -> Result<Self, PlaybackError> {
        if headless {
            debug!("[engine] Creating engine in headless mode");
        } else {
            info!("[engine] Initializing audio engine...");
        }

        let _ = crate::audio::http::get_runtime();

        #[cfg(target_os = "android")]
        let http_client = Arc::new(reqwest::Client::new());

        #[cfg(not(target_os = "android"))]
        let http_client = {
            let client = crate::audio::http::build_blocking_http_client();
            Arc::new(client)
        };

        let audio_queue: AudioBuffer = Arc::new(parking_lot::Mutex::new(VecDeque::new()));

        Ok(Self {
            state: PlaybackState::Stopped,
            position: crate::models::PlaybackPosition::default(),
            stream_url: None,
            http_client,
            load_error: Arc::new(Mutex::new(String::new())),
            band_count: 16,
            audio_queue,
            buffer_ready: Arc::new(AtomicBool::new(false)),
            is_playing_flag: Arc::new(AtomicBool::new(false)),
            should_stop: Arc::new(AtomicBool::new(false)),
            samples_played: Arc::new(AtomicU64::new(0)),
            sample_rate: Arc::new(AtomicU64::new(44100)),
            channels: Arc::new(AtomicU64::new(2)),
            total_duration_ms: Arc::new(AtomicU64::new(0)),
            pipe_total_bytes: Arc::new(AtomicU64::new(0)),
            pipe_bytes_sent: Arc::new(AtomicU64::new(0)),
            playback_handle: None,
            stream_pipe: None,
            playback_type: None,
            source: None,
            seek_target_ms: Arc::new(AtomicU64::new(0)),
            download_buffer: Arc::new(Mutex::new(DownloadBuffer::default())),
            buffer_poller_handle: None,
            event_queue: Arc::new(Mutex::new(VecDeque::new())),
            live_ring: None,
            buffer_size_ms_fixed: Arc::new(AtomicU64::new(0)),
            live_start_time: Arc::new(std::sync::Mutex::new(None)),
            seek_request: Arc::new(AtomicU64::new(0)),
            stream_ended: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Creates a new PlaybackEngine.
    pub fn new() -> Result<Self, PlaybackError> {
        Self::create_internal(false)
    }

    /// Creates a new PlaybackEngine without an audio device (headless mode).
    pub fn new_without_device() -> Result<Self, PlaybackError> {
        Self::create_internal(true)
    }

    /// Plays an audio file from the given path.
    pub fn play_file(&mut self, path: &str) -> Result<(), PlaybackError> {
        info!("[playback] play_file: loading from {}", path);

        self.should_stop.store(true, Ordering::Relaxed);

        self.set_state(PlaybackState::Connecting);
        self.load_error.lock().clear();
        let path_owned = path.to_string();
        self.stream_url = Some(path_owned.clone());
        self.samples_played.store(0, Ordering::Relaxed);
        self.total_duration_ms.store(0, Ordering::Relaxed);
        {
            let mut q = self.audio_queue.lock();
            q.clear();
        }
        self.buffer_ready.store(false, Ordering::Relaxed);
        self.seek_target_ms.store(0, Ordering::Relaxed);
        self.playback_type = Some(PlaybackType::File {
            path: path_owned.clone(),
        });

        if let Some(handle) = self.playback_handle.take() {
            debug!("[playback] Stopping previous playback thread");
            Self::join_with_timeout(handle, "play_file");
            debug!("[playback] Previous playback thread stopped");
        }

        self.should_stop.store(false, Ordering::Relaxed);

        let audio_queue = self.audio_queue.clone();
        let buffer_ready = self.buffer_ready.clone();
        let is_playing_flag = self.is_playing_flag.clone();
        let should_stop = self.should_stop.clone();
        let samples_played = self.samples_played.clone();
        let sample_rate = self.sample_rate.clone();
        let channels = self.channels.clone();
        let total_duration_ms = self.total_duration_ms.clone();
        let load_error = self.load_error.clone();
        let seek_target_ms = self.seek_target_ms.clone();

        let _playback_type = self.playback_type.clone();

        #[cfg(target_os = "android")]
        let play_fn = crate::audio::decoder::file_decoder::play_file_internal;
        #[cfg(not(target_os = "android"))]
        let play_fn = crate::audio::decoder::file_decoder::play_file_internal;

        let handle = thread::spawn(move || {
            play_fn(
                path_owned,
                audio_queue,
                buffer_ready,
                is_playing_flag,
                should_stop,
                samples_played,
                sample_rate,
                channels,
                total_duration_ms,
                load_error,
                seek_target_ms,
            );
        });

        self.playback_handle = Some(handle);
        self.set_state(PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        });

        let start_time = std::time::Instant::now();
        while !self.buffer_ready.load(Ordering::Relaxed)
            && start_time.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(50));
        }

        if !self.buffer_ready.load(Ordering::Relaxed) {
            let err = self.load_error.lock().clone();
            if err.is_empty() {
                warn!("[playback] File playback failed to start within timeout");
            } else {
                error!("[playback] File playback error: {}", err);
            }
        } else {
            self.set_state(PlaybackState::Playing);
        }

        Ok(())
    }

    /// Plays an HTTP stream — delegates to `play()` for pipeline-based playback.
    /// The legacy `play_stream_internal` path is retired; `play()` auto-detects
    /// the source type via `source::from_uri` and builds the correct pipeline
    /// (YouTubeSource caches the CDN URL, radio uses RadioSource, etc.).
    pub fn play_stream(&mut self, url: &str) -> Result<(), PlaybackError> {
        info!("[playback] play_stream: delegating to play() for {}", url);
        self.play(url, None)
    }

    /// Play a live internet stream with backward seek via ring buffer.
    pub fn play_live(&mut self, url: &str, cache_max_ms: u64) -> Result<(), PlaybackError> {
        info!("[engine] play_live: {} cache={}ms", url, cache_max_ms);
        self.should_stop.store(true, Ordering::Relaxed);
        self.set_state(PlaybackState::Connecting);
        self.load_error.lock().clear();
        let url_owned = url.to_string();
        self.stream_url = Some(url_owned.clone());
        self.samples_played.store(0, Ordering::Relaxed);
        self.total_duration_ms
            .store(cache_max_ms, Ordering::Relaxed);
        {
            let mut q = self.audio_queue.lock();
            q.clear();
        }
        self.buffer_ready.store(false, Ordering::Relaxed);
        self.playback_type = Some(PlaybackType::Live {
            url: url_owned.clone(),
            cache_max_ms,
        });

        // Create shared ring buffer for live stream caching (persists across seeks).
        let ring = std::sync::Arc::new(std::sync::Mutex::new(crate::models::LiveByteRing::new(
            cache_max_ms,
            128_000,
        )));
        self.live_ring = Some(Arc::clone(&ring));

        // BUG-3 fix: set self.source so source_supports(Capability::Seek) returns true
        // for live streams. Without this, engine.source is None and canSeek stays false.
        self.source = Some(Box::new(
            crate::audio::stream::source::live::LiveSource::new(
                url,
                self.http_client.clone(),
                cache_max_ms,
            ),
        ));

        if let Some(handle) = self.playback_handle.take() {
            Self::join_with_timeout(handle, "play_live");
        }
        self.should_stop.store(false, Ordering::Relaxed);

        let audio_queue = self.audio_queue.clone();
        let buffer_ready = self.buffer_ready.clone();
        let is_playing_flag = self.is_playing_flag.clone();
        let should_stop = self.should_stop.clone();
        let samples_played = self.samples_played.clone();
        let sample_rate = self.sample_rate.clone();
        let channels = self.channels.clone();
        let total_duration_ms = self.total_duration_ms.clone();
        let load_error = self.load_error.clone();
        let _client = self.http_client.clone();
        let seek_target_ms = self.seek_target_ms.clone();
        let pipe_bytes_sent = self.pipe_bytes_sent.clone();
        let pipe_total_bytes = self.pipe_total_bytes.clone();

        // Record the monotonic start time so the buffer poller can compute
        // write_offset_ms = min(elapsed_since_start_ms, cache_max_ms).
        *self.live_start_time.lock().unwrap() = Some(std::time::Instant::now());

        #[cfg(not(target_os = "android"))]
        let handle = thread::spawn(move || {
            crate::audio::stream::handling::play_live_internal(
                url_owned,
                _client,
                audio_queue,
                buffer_ready,
                is_playing_flag,
                should_stop,
                samples_played,
                sample_rate,
                channels,
                total_duration_ms,
                load_error,
                seek_target_ms,
                pipe_bytes_sent,
                pipe_total_bytes,
                cache_max_ms,
                ring,
                0, // cache_head_ms = 0 on initial play
            );
        });

        #[cfg(target_os = "android")]
        let handle = thread::spawn(move || {
            crate::audio::decoder::file_decoder::play_live_internal(
                url_owned,
                audio_queue,
                buffer_ready,
                is_playing_flag,
                should_stop,
                samples_played,
                sample_rate,
                channels,
                total_duration_ms,
                load_error,
                seek_target_ms,
                pipe_bytes_sent,
                pipe_total_bytes,
                cache_max_ms,
                ring,
                0, // cache_head_ms = 0 on initial play
            );
        });
        self.playback_handle = Some(handle);
        self.set_state(PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        });

        // Spawn buffer poller for live stream: fills the DownloadBuffer
        // using elapsed wall-clock time rather than pipe bytes, so the UI
        // shows the buffer filling progressively up to cache_max_ms.
        let buf_poller_download_buffer = self.download_buffer.clone();
        let buf_poller_should_stop = self.should_stop.clone();
        let buf_poller_samples_played = self.samples_played.clone();
        let buf_poller_sample_rate = self.sample_rate.clone();
        let buf_poller_channels = self.channels.clone();
        let buf_poller_total_ms = self.total_duration_ms.clone();
        let buf_poller_start_time = self.live_start_time.clone();
        let buf_poller = thread::Builder::new()
            .name("buffer-poller".into())
            .spawn(move || {
                let mut last = DownloadBuffer::default();
                while !buf_poller_should_stop.load(Ordering::Relaxed) {
                    let total_ms = buf_poller_total_ms.load(Ordering::Relaxed);
                    let sp = buf_poller_samples_played.load(Ordering::Relaxed);
                    let sr = buf_poller_sample_rate.load(Ordering::Relaxed).max(1);
                    let ch = buf_poller_channels.load(Ordering::Relaxed).max(1);
                    let playhead_ms = ((sp as f64 / (sr as f64 * ch as f64)) * 1000.0) as u64;

                    let elapsed = {
                        let guard = buf_poller_start_time.lock().unwrap();
                        guard.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0)
                    };

                    let capacity_ms = total_ms;
                    let read_offset = playhead_ms.min(total_ms);
                    let write_offset = elapsed.min(total_ms);
                    let new_buf = DownloadBuffer {
                        capacity_ms,
                        read_offset_ms: read_offset,
                        write_offset_ms: write_offset.max(read_offset),
                        total_ms,
                        is_complete: false,
                    };
                    if new_buf != last {
                        *buf_poller_download_buffer.lock() = new_buf;
                        last = new_buf;
                    }
                    thread::sleep(std::time::Duration::from_millis(200));
                }
            })
            .map_err(|e| PlaybackError::ThreadSpawn {
                operation: "buffer-poller".into(),
                detail: e.to_string(),
            })?;
        self.buffer_poller_handle = Some(buf_poller);

        // Wait for initial buffer (30s max).
        let start_time = std::time::Instant::now();
        while !self.buffer_ready.load(Ordering::Relaxed)
            && start_time.elapsed() < Duration::from_secs(30)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        if self.buffer_ready.load(Ordering::Relaxed) {
            self.set_state(PlaybackState::Playing);
        } else {
            let err = self.load_error.lock().clone();
            if err.is_empty() {
                warn!("[playback] Live stream failed to start within timeout");
            } else {
                error!("[playback] Live stream error: {}", err);
            }
        }

        Ok(())
    }

    /// Stops playback.
    pub fn stop(&mut self) {
        info!("[engine] Stopping playback");
        if let Some(pipe) = self.stream_pipe.take() {
            pipe.end();
        }
        self.should_stop.store(true, Ordering::Relaxed);
        self.buffer_ready.store(false, Ordering::Relaxed);
        {
            let mut q = self.audio_queue.lock();
            q.clear();
        }
        self.set_state(PlaybackState::Stopped);
        self.seek_target_ms.store(0, Ordering::Relaxed);
        self.playback_type = None;

        if let Some(handle) = self.playback_handle.take() {
            let _ = handle.join();
            debug!("[engine] Playback thread joined");
        }

        if let Some(handle) = self.buffer_poller_handle.take() {
            let _ = handle.join();
            debug!("[engine] Buffer poller joined");
        }
    }

    /// Pauses playback.
    pub fn pause(&mut self) {
        debug!("[engine] Pausing playback");
        self.buffer_ready.store(false, Ordering::Relaxed);
        self.is_playing_flag.store(false, Ordering::Relaxed);
        self.set_state(PlaybackState::Paused);
    }

    /// Resumes playback.
    pub fn resume(&mut self) {
        debug!("[engine] Resuming playback");
        self.buffer_ready.store(true, Ordering::Relaxed);
        self.is_playing_flag.store(true, Ordering::Relaxed);
        self.set_state(PlaybackState::Playing);
    }

    /// Seeks to position in milliseconds.
    ///
    /// If `position_ms` is past the current download buffer end (and the
    /// download is not complete), the seek is queued: `SEEK_STARTED` is
    /// emitted with the target, `SEEK_QUEUED` is emitted to inform the
    /// client the seek is waiting for the downloader to catch up, and the
    /// existing pipeline will apply the seek once enough data has been
    /// buffered. For local files and completed downloads, the seek is
    /// applied immediately as before.
    pub fn seek(&mut self, position_ms: u64) -> Result<(), PlaybackError> {
        info!("[engine] Seeking to {} ms", position_ms);
        let buffer = self.get_download_buffer();
        let end_ms = buffer.end_ms();
        let is_live = matches!(self.playback_type, Some(PlaybackType::Live { .. }));

        // Live streams cannot seek beyond the current buffer (future data
        // doesn't exist yet). Soft-clamp the target to the buffer end.
        let clamped_position = if is_live && position_ms > end_ms && end_ms > 0 {
            warn!(
                "[engine] Live seek target {} ms clamped to buffer end {} ms",
                position_ms, end_ms
            );
            end_ms
        } else {
            position_ms
        };

        let is_queued = clamped_position > end_ms && !buffer.is_complete && end_ms > 0;

        self.seek_target_ms
            .store(clamped_position, Ordering::Relaxed);
        self.push_seek_started(clamped_position);

        if is_queued {
            // Seek is past the buffered region. Forward the request to the
            // decode thread anyway — it will block in packet-skip until the
            // background filler catches up. Without this the seek is silently
            // lost because the non-Android playback_loop only monitors
            // seek_request after startup, not seek_target_ms.
            self.seek_request.store(clamped_position, Ordering::Relaxed);
            {
                let mut q = self.audio_queue.lock();
                q.clear();
            }
            self.push_event(crate::models::EngineEvent {
                event_type: crate::models::ENGINE_EVENT_SEEK_QUEUED,
                int_param: clamped_position as i64,
            });
            return Ok(());
        }

        // If we're playing a stream, we might need to restart it with a Range header
        let current_playback_type = self.playback_type.clone();
        if let Some(playback_type) = current_playback_type {
            match playback_type {
                PlaybackType::Stream { url, .. } => {
                    #[cfg(not(target_os = "android"))]
                    {
                        let _ = &url;
                        info!("[engine] Cache-reopen seek to {} ms", clamped_position);
                        // Invalidate old CPAL streams so they write silence.
                        crate::audio::stream::cpal_source::OUTPUT_GEN
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        // Signal old decode thread to stop.
                        self.should_stop.store(true, Ordering::Relaxed);
                        self.buffer_ready.store(false, Ordering::Relaxed);
                        {
                            let mut q = self.audio_queue.lock();
                            q.clear();
                        }
                        self.samples_played.store(0, Ordering::Relaxed);
                        // Detach the old decode thread instead of joining it.
                        // join_with_timeout waited up to 3 seconds for CoreAudio's
                        // stream drop() to complete — that's the single biggest
                        // contributor to seek latency.  With OUTPUT_GEN invalidation
                        // the old CPAL callback writes silence, so there's no audio
                        // glitch even while the old thread is still exiting.
                        drop(self.playback_handle.take());
                        self.should_stop.store(false, Ordering::Relaxed);
                        // Open source from cache — open(Some(...)) tells CachingDecorator
                        // to return a CachedReader from the existing ByteCache without
                        // making a new HTTP connection.  open(None) would clear the cache
                        // and start a fresh download, which defeats the purpose of a
                        // cache-reopen seek.
                        let reader = self.source.as_ref().unwrap().open(Some(clamped_position))?;
                        let counting = ByteCountingRead::new(reader, self.pipe_bytes_sent.clone());
                        // Clone Arcs for the decode thread.
                        let audio_queue = self.audio_queue.clone();
                        let buffer_ready = self.buffer_ready.clone();
                        let is_playing_flag = self.is_playing_flag.clone();
                        let should_stop = self.should_stop.clone();
                        let samples_played = self.samples_played.clone();
                        let sample_rate = self.sample_rate.clone();
                        let channels = self.channels.clone();
                        let total_duration_ms = self.total_duration_ms.clone();
                        let load_error = self.load_error.clone();
                        let seek_target_ms = self.seek_target_ms.clone();
                        let seek_request = self.seek_request.clone();
                        let handle = thread::Builder::new()
                            .name("playback-decode".into())
                            .spawn(move || {
                                crate::audio::stream::handling::decode_and_play_from_read(
                                    Box::new(counting),
                                    audio_queue,
                                    buffer_ready,
                                    is_playing_flag,
                                    should_stop,
                                    samples_played,
                                    sample_rate,
                                    channels,
                                    total_duration_ms,
                                    load_error,
                                    seek_target_ms,
                                    seek_request,
                                );
                            })
                            .map_err(|e| PlaybackError::ThreadSpawn {
                                operation: "seek".into(),
                                detail: e.to_string(),
                            })?;
                        self.playback_handle = Some(handle);
                        // Wait for prebuffer.
                        let seek_start = std::time::Instant::now();
                        while !self.buffer_ready.load(Ordering::Relaxed)
                            && seek_start.elapsed() < Duration::from_secs(5)
                        {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        if !self.buffer_ready.load(Ordering::Relaxed) {
                            let err = self.load_error.lock().clone();
                            if err.is_empty() {
                                warn!("[engine] Cache-reopen seek prebuffer failed to start within timeout");
                            } else {
                                error!("[engine] Cache-reopen seek prebuffer error: {}", err);
                            }
                        }
                        self.push_seek_completed(clamped_position);
                        return Ok(());
                    }
                    // Android: use the legacy play_stream path (re-download stream)
                    #[cfg(target_os = "android")]
                    {
                        self.set_state(PlaybackState::Connecting);
                        self.play_stream(&url)?;
                    }
                }
                PlaybackType::File { path } => {
                    // For files, we need to restart the decode thread with the new position
                    info!(
                        "[engine] Restarting file playback for seek to {} ms",
                        position_ms
                    );
                    crate::audio::stream::cpal_source::OUTPUT_GEN
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.should_stop.store(true, Ordering::Relaxed);
                    self.buffer_ready.store(false, Ordering::Relaxed);
                    {
                        let mut q = self.audio_queue.lock();
                        q.clear();
                    }
                    self.samples_played.store(0, Ordering::Relaxed);
                    self.seek_target_ms
                        .store(clamped_position, Ordering::Relaxed);

                    // Stop old decode thread
                    if let Some(handle) = self.playback_handle.take() {
                        Self::join_with_timeout(handle, "file-seek-restart");
                    }

                    self.should_stop.store(false, Ordering::Relaxed);

                    let audio_queue = self.audio_queue.clone();
                    let buffer_ready = self.buffer_ready.clone();
                    let is_playing_flag = self.is_playing_flag.clone();
                    let should_stop = self.should_stop.clone();
                    let samples_played = self.samples_played.clone();
                    let sample_rate = self.sample_rate.clone();
                    let channels = self.channels.clone();
                    let total_duration_ms = self.total_duration_ms.clone();
                    let load_error = self.load_error.clone();
                    let seek_target = self.seek_target_ms.clone();
                    let stream_ended = self.stream_ended.clone();
                    let event_queue = self.event_queue.clone();

                    #[cfg(target_os = "android")]
                    let play_fn = crate::audio::decoder::file_decoder::play_file_internal;
                    #[cfg(not(target_os = "android"))]
                    let play_fn = crate::audio::decoder::file_decoder::play_file_internal;

                    let handle = thread::spawn(move || {
                        play_fn(
                            path,
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop.clone(),
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error.clone(),
                            seek_target,
                        );

                        if !should_stop.load(Ordering::Relaxed) && load_error.lock().is_empty() {
                            stream_ended.store(true, Ordering::Relaxed);
                            event_queue.lock().push_back(EngineEvent {
                                event_type: ENGINE_EVENT_END_OF_STREAM,
                                int_param: 0,
                            });
                            event_queue.lock().push_back(EngineEvent {
                                event_type: ENGINE_EVENT_STATE_CHANGED,
                                int_param: PlaybackState::Stopped.to_i32() as i64,
                            });
                        }
                    });
                    self.playback_handle = Some(handle);

                    // Wait for prebuffer to complete (same as play_file)
                    let seek_start = std::time::Instant::now();
                    while !self.buffer_ready.load(Ordering::Relaxed)
                        && seek_start.elapsed() < Duration::from_secs(5)
                    {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    if !self.buffer_ready.load(Ordering::Relaxed) {
                        let err = self.load_error.lock().clone();
                        if err.is_empty() {
                            warn!("[engine] Seek prebuffer failed to start within timeout");
                        } else {
                            error!("[engine] Seek prebuffer error: {}", err);
                        }
                    }
                    self.push_seek_completed(clamped_position);
                }
                PlaybackType::Pipe { url: _, .. } => {
                    // For pipes, we need to restart both the fetch and decode threads
                    info!(
                        "[engine] Restarting pipe playback for seek to {} ms",
                        position_ms
                    );
                    crate::audio::stream::cpal_source::OUTPUT_GEN
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.should_stop.store(true, Ordering::Relaxed);
                    self.buffer_ready.store(false, Ordering::Relaxed);
                    {
                        let mut q = self.audio_queue.lock();
                        q.clear();
                    }
                    self.samples_played.store(0, Ordering::Relaxed);
                    self.seek_target_ms
                        .store(clamped_position, Ordering::Relaxed);

                    // Signal fetch thread to reconnect
                    if let Some(pipe) = &self.stream_pipe {
                        let byte_offset = self.calculate_byte_offset_for_seek(clamped_position);
                        pipe.set_seek_offset(byte_offset);
                    }

                    // Stop old decode thread
                    if let Some(handle) = self.playback_handle.take() {
                        Self::join_with_timeout(handle, "seek-restart");
                    }

                    // Create new pipe and restart decode thread
                    let (pipe_writer, pipe_reader) = crate::audio::stream::pipe::new_pipe();
                    self.stream_pipe = Some(Arc::new(pipe_writer));

                    self.should_stop.store(false, Ordering::Relaxed);

                    let audio_queue = self.audio_queue.clone();
                    let buffer_ready = self.buffer_ready.clone();
                    let is_playing_flag = self.is_playing_flag.clone();
                    let should_stop = self.should_stop.clone();
                    let samples_played = self.samples_played.clone();
                    let sample_rate = self.sample_rate.clone();
                    let channels = self.channels.clone();
                    let total_duration_ms = self.total_duration_ms.clone();
                    let load_error = self.load_error.clone();
                    let stream_ended = self.stream_ended.clone();
                    let event_queue = self.event_queue.clone();

                    #[cfg(target_os = "android")]
                    let play_fn =
                        crate::audio::decoder::file_decoder::play_stream_from_pipe_internal;
                    #[cfg(not(target_os = "android"))]
                    let play_fn = crate::audio::stream::handling::play_stream_from_pipe_internal;

                    let handle = thread::spawn(move || {
                        play_fn(
                            pipe_reader,
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop.clone(),
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error.clone(),
                        );

                        if !should_stop.load(Ordering::Relaxed) && load_error.lock().is_empty() {
                            stream_ended.store(true, Ordering::Relaxed);
                            event_queue.lock().push_back(EngineEvent {
                                event_type: ENGINE_EVENT_END_OF_STREAM,
                                int_param: 0,
                            });
                            event_queue.lock().push_back(EngineEvent {
                                event_type: ENGINE_EVENT_STATE_CHANGED,
                                int_param: PlaybackState::Stopped.to_i32() as i64,
                            });
                        }
                    });
                    self.playback_handle = Some(handle);
                    self.push_seek_completed(clamped_position);
                }
                PlaybackType::Live { url, cache_max_ms } => {
                    info!(
                        "[engine] Restarting live stream for seek to {} ms (cache={}ms)",
                        position_ms, cache_max_ms
                    );
                    self.should_stop.store(true, Ordering::Relaxed);
                    self.buffer_ready.store(false, Ordering::Relaxed);
                    {
                        let mut q = self.audio_queue.lock();
                        q.clear();
                    }
                    self.samples_played.store(0, Ordering::Relaxed);
                    self.seek_target_ms
                        .store(clamped_position, Ordering::Relaxed);

                    if let Some(handle) = self.playback_handle.take() {
                        Self::join_with_timeout(handle, "live-seek-restart");
                    }

                    self.should_stop.store(false, Ordering::Relaxed);

                    let audio_queue = self.audio_queue.clone();
                    let buffer_ready = self.buffer_ready.clone();
                    let is_playing_flag = self.is_playing_flag.clone();
                    let should_stop = self.should_stop.clone();
                    let samples_played = self.samples_played.clone();
                    let sample_rate = self.sample_rate.clone();
                    let channels = self.channels.clone();
                    let total_duration_ms = self.total_duration_ms.clone();
                    let load_error = self.load_error.clone();
                    let _client = self.http_client.clone();
                    let seek_target_ms = self.seek_target_ms.clone();
                    let pipe_bytes_sent = self.pipe_bytes_sent.clone();
                    let pipe_total_bytes = self.pipe_total_bytes.clone();
                    let seek_url = url.clone();
                    let ring = self.live_ring.clone().unwrap();
                    let cache_head_ms = {
                        let guard = self.live_start_time.lock().unwrap();
                        guard
                            .map(|start| {
                                let elapsed = start.elapsed().as_millis() as u64;
                                elapsed.min(cache_max_ms)
                            })
                            .unwrap_or(0)
                    };

                    #[cfg(not(target_os = "android"))]
                    let handle = thread::spawn(move || {
                        crate::audio::stream::handling::play_live_internal(
                            seek_url,
                _client,
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop,
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error,
                            seek_target_ms,
                            pipe_bytes_sent,
                            pipe_total_bytes,
                            cache_max_ms,
                            ring,
                            cache_head_ms,
                        );
                    });

                    #[cfg(target_os = "android")]
                    let handle = thread::spawn(move || {
                        crate::audio::decoder::file_decoder::play_live_internal(
                            seek_url,
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop,
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error,
                            seek_target_ms,
                            pipe_bytes_sent,
                            pipe_total_bytes,
                            cache_max_ms,
                            ring,
                            cache_head_ms,
                        );
                    });
                    self.playback_handle = Some(handle);
                    self.push_seek_completed(clamped_position);
                }
                PlaybackType::AdaptiveBuffer { url, .. } => {
                    // Retired path: delegate to play() which auto-detects source
                    // via pipeline (YouTubeSource caches CDN URL for efficient
                    // Range-based seeks).  The old adaptive buffer path re-resolved
                    // the YouTube manifest on every seek (BUG-4).
                    info!(
                        "[engine] AdaptiveBuffer seek retired, delegating to play() for {} ms",
                        position_ms
                    );
                    return self.play(&url, None);
                }
            }
        }
        Ok(())
    }

    /// Skips forward by milliseconds.
    pub fn skip_forward(&mut self, ms: u64) -> Result<(), PlaybackError> {
        info!("[engine] Skipping forward {} ms", ms);
        let current = self.get_position().current_ms;
        let total = self.total_duration_ms.load(Ordering::Relaxed);
        let new_position = if total > 0 {
            (current + ms).min(total.saturating_sub(500))
        } else {
            current + ms
        };
        self.seek(new_position)
    }

    /// Skips backward by milliseconds.
    pub fn skip_backward(&mut self, ms: u64) -> Result<(), PlaybackError> {
        info!("[engine] Skipping backward {} ms", ms);
        let current = self.get_position().current_ms;
        self.seek(current.saturating_sub(ms))
    }

    /// Sets the volume (0.0 to 1.0).
    pub fn set_volume(&self, volume: f32) {
        debug!("[engine] Setting volume to {}", volume);
        crate::audio::stream::cpal_source::set_volume_gain(volume.clamp(0.0, 1.0));
    }

    /// Gets the current volume.
    pub fn get_volume(&self) -> f32 {
        debug!("[engine] Getting volume");
        crate::audio::stream::cpal_source::get_volume_gain()
    }

    /// Sets the balance / pan (0.0 = full left, 0.5 = center, 1.0 = full right).
    pub fn set_balance(&self, balance: f32) {
        debug!("[engine] Setting balance to {}", balance);
        crate::audio::stream::cpal_source::set_balance_gain(balance.clamp(0.0, 1.0));
    }

    /// Gets the current balance.
    pub fn get_balance(&self) -> f32 {
        debug!("[engine] Getting balance");
        crate::audio::stream::cpal_source::get_balance_gain()
    }

    /// Play a stream from bytes (internal method for FFI).
    pub fn play_stream_from_bytes_internal(&mut self, url: &str) -> Result<(), PlaybackError> {
        info!(
            "[playback] play_stream_from_bytes_internal: loading from {}",
            url
        );
        self.should_stop.store(true, Ordering::Relaxed);
        self.set_state(PlaybackState::Connecting);
        self.load_error.lock().clear();
        let url_owned = url.to_string();
        self.stream_url = Some(url_owned.clone());
        self.samples_played.store(0, Ordering::Relaxed);
        self.total_duration_ms.store(0, Ordering::Relaxed);
        {
            let mut q = self.audio_queue.lock();
            q.clear();
        }
        self.buffer_ready.store(false, Ordering::Relaxed);
        self.playback_type = Some(PlaybackType::Pipe {
            url: url_owned.clone(),
            video_id: None,
        });

        let (pipe_writer, pipe_reader) = crate::audio::stream::pipe::new_pipe();
        self.stream_pipe = Some(Arc::new(pipe_writer));

        if let Some(handle) = self.playback_handle.take() {
            debug!("[playback] Stopping previous playback thread");
            Self::join_with_timeout(handle, "play_stream_from_bytes_internal");
            debug!("[playback] Previous playback thread stopped");
        }
        self.should_stop.store(false, Ordering::Relaxed);

        let audio_queue = self.audio_queue.clone();
        let buffer_ready = self.buffer_ready.clone();
        let is_playing_flag = self.is_playing_flag.clone();
        let should_stop = self.should_stop.clone();
        let samples_played = self.samples_played.clone();
        let sample_rate = self.sample_rate.clone();
        let channels = self.channels.clone();
        let total_duration_ms = self.total_duration_ms.clone();
        let load_error = self.load_error.clone();
        let _seek_target_ms = self.seek_target_ms.clone();

        #[cfg(target_os = "android")]
        let play_fn = crate::audio::decoder::file_decoder::play_stream_from_pipe_internal;
        #[cfg(not(target_os = "android"))]
        let play_fn = crate::audio::stream::handling::play_stream_from_pipe_internal;

        let handle = thread::spawn(move || {
            play_fn(
                pipe_reader,
                audio_queue,
                buffer_ready,
                is_playing_flag,
                should_stop,
                samples_played,
                sample_rate,
                channels,
                total_duration_ms,
                load_error,
            );
        });

        self.playback_handle = Some(handle);
        self.set_state(PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        });

        let start_time = std::time::Instant::now();
        while !self.buffer_ready.load(Ordering::Relaxed)
            && start_time.elapsed() < Duration::from_secs(30)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !self.buffer_ready.load(Ordering::Relaxed) {
            let err = self.load_error.lock().clone();
            if err.is_empty() {
                warn!("[playback] Stream from bytes failed to start within timeout");
            } else {
                error!("[playback] Stream from bytes error: {}", err);
            }
        } else {
            self.set_state(PlaybackState::Playing);
        }
        Ok(())
    }

    /// Legacy — delegates to `play()` which auto-detects source via pipeline.
    /// The adaptive buffer path is retired because it re-resolved the YouTube
    /// manifest on every seek (BUG-4).  `play()` + `YouTubeSource` cache the
    /// CDN URL and use Range headers for efficient seeks.
    pub fn play_adaptive_buffer(
        &mut self,
        url: &str,
        _cache_dir: &str,
    ) -> Result<(), PlaybackError> {
        info!(
            "[playback] play_adaptive_buffer: delegating to play() for {}",
            url
        );
        self.play(url, None)
    }

    /// Plays a stream — delegates to `play()` for pipeline-based playback.
    /// The legacy `play_stream_internal` / `play_stream_with_downloader_internal`
    /// paths are retired in favour of `play()` which auto-detects the source.
    pub fn play_stream_with_downloader(&mut self, url: &str) -> Result<(), PlaybackError> {
        info!(
            "[playback] play_stream_with_downloader: delegating to play() for {}",
            url
        );
        self.play(url, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The live buffer poller must spawn when play_live() is called and
    /// must produce write_offset_ms that tracks elapsed wall-clock time
    /// (capped at cache_max_ms), with is_complete = false (a live stream
    /// is never finished).
    #[test]
    fn test_live_buffer_poller_progressive_fill() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let cache_max_ms = 30_000; // 30 s window

        // play_live starts a background download thread; that thread
        // will fail to connect because the URL is fake, but the buffer
        // poller thread is independent and starts running immediately.
        let _ = engine.play_live("http://127.0.0.1:1/nonexistent", cache_max_ms);

        // live_start_time must be set by play_live.
        {
            let guard = engine.live_start_time.lock().unwrap();
            assert!(
                guard.is_some(),
                "live_start_time should be Some after play_live"
            );
        }

        // The buffer poller must have been spawned.
        assert!(
            engine.buffer_poller_handle.is_some(),
            "buffer_poller_handle should be Some after play_live"
        );

        // Give the buffer poller a chance to tick once (200 ms interval).
        thread::sleep(Duration::from_millis(300));

        // After a brief wait, the buffer should show:
        //   - write_offset_ms > 0 (elapsed time)
        //   - write_offset_ms <= cache_max_ms (clamped)
        //   - is_complete == false (live is never complete)
        let buf = {
            let g = engine.download_buffer.lock();
            *g
        };
        assert!(
            buf.write_offset_ms > 0,
            "write_offset_ms should advance after a tick, got {}",
            buf.write_offset_ms
        );
        assert!(
            buf.write_offset_ms <= cache_max_ms,
            "write_offset_ms should be capped at cache_max_ms ({})",
            cache_max_ms
        );
        assert!(!buf.is_complete, "live buffer should never be complete");
        assert!(
            buf.write_offset_ms >= buf.read_offset_ms,
            "write_offset_ms ({}) >= read_offset_ms ({}) invariant violated",
            buf.write_offset_ms,
            buf.read_offset_ms
        );

        // Clean up.
        engine.stop();
    }

    // ── BUG-3 regression: play_live() must set self.source so          ──
    // source_supports(Seek) returns true for live streams.
    #[test]
    fn test_play_live_sets_source_for_can_seek() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let _ = engine.play_live("http://127.0.0.1:1/nonexistent", 30_000);

        assert!(
            engine.source.is_some(),
            "source should be Some after play_live"
        );
        assert!(
            engine.source_supports(crate::audio::stream::source::Capability::Seek),
            "source_supports(Seek) should be true after play_live (BUG-3)"
        );

        engine.stop();
    }

    // ── BUG-4 regression: deprecated functions must delegate to play() ──
    // without crashing or returning Err.  These tests call the methods
    // with a fake URL — the pipeline will fail to connect, but the
    // delegation itself must not produce an unexpected error.

    #[test]
    fn test_play_stream_delegates_to_play() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let result = engine.play_stream("http://127.0.0.1:1/nonexistent");
        // The method should accept the call (delegation) and return the
        // same error that play() would produce for a non-existent URL.
        assert!(
            result.is_err(),
            "play_stream should delegate to play() and return its error, got {:?}",
            result
        );
        engine.stop();
    }

    #[test]
    fn test_play_stream_with_downloader_delegates_to_play() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let result = engine.play_stream_with_downloader("http://127.0.0.1:1/nonexistent");
        assert!(
            result.is_err(),
            "play_stream_with_downloader should delegate to play()"
        );
        engine.stop();
    }

    #[test]
    fn test_play_adaptive_buffer_delegates_to_play() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let result = engine.play_adaptive_buffer("http://127.0.0.1:1/nonexistent", "/tmp/cache");
        assert!(
            result.is_err(),
            "play_adaptive_buffer should delegate to play()"
        );
        engine.stop();
    }

    // ── Seek within buffered range must work without re-fetch ──
    // The seek() method must:
    //   1. Set seek_target_ms for any position (buffered or not)
    //   2. Return early with is_queued=true for positions beyond the buffer
    //   3. Fall through to the playback-type handler for in-buffer seeks
    //   4. Never crash regardless of playback thread state

    #[test]
    fn test_seek_sets_seek_target_ms_for_live_stream() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let _ = engine.play_live("http://127.0.0.1:1/nonexistent", 60_000);

        // Give the buffer poller time to tick so download_buffer has a
        // non-zero write_offset, making the seek "within buffered range".
        std::thread::sleep(Duration::from_millis(400));

        let buf = { *engine.download_buffer.lock() };
        assert!(
            buf.write_offset_ms > 0,
            "buffer should have write_offset > 0 after 400ms, got {}",
            buf.write_offset_ms
        );

        // Seek to a position within the buffered range
        let seek_pos = buf.write_offset_ms / 2; // halfway into buffer
        let result = engine.seek(seek_pos);
        assert!(
            result.is_ok(),
            "seek within buffered range should succeed, got {:?}",
            result
        );

        // seek_target_ms must be set
        assert_eq!(
            engine.seek_target_ms.load(Ordering::Relaxed),
            seek_pos,
            "seek_target_ms should be set to {} after seek( {} )",
            seek_pos,
            seek_pos
        );

        engine.stop();
    }

    #[test]
    fn test_seek_is_queued_when_beyond_buffer() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let _ = engine.play_live("http://127.0.0.1:1/nonexistent", 60_000);

        // Wait for buffer to have some data
        std::thread::sleep(Duration::from_millis(400));
        let (end_ms, far_pos) = {
            let buf = engine.download_buffer.lock();
            let e = buf.end_ms();
            let f = buf.write_offset_ms * 100;
            (e, f)
        };
        assert!(end_ms > 0, "buffer must have some data after 400ms");

        // Seek way beyond the buffer for a LIVE stream — the target should
        // be clamped to end_ms (live can't re-fetch via Range header).
        let result = engine.seek(far_pos);
        assert!(
            result.is_ok(),
            "seek beyond buffer should not error, got {:?}",
            result
        );

        // seek_target_ms must be clamped to end_ms for live streams
        assert_eq!(
            engine.seek_target_ms.load(Ordering::Relaxed),
            end_ms,
            "seek_target_ms should be clamped to end_ms for live streams"
        );

        engine.stop();
    }

    #[test]
    fn test_seek_within_buffer_does_not_clear_buffer_ready_for_live() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let _ = engine.play_live("http://127.0.0.1:1/nonexistent", 60_000);

        std::thread::sleep(Duration::from_millis(400));

        let buf = { *engine.download_buffer.lock() };
        let seek_pos = buf.write_offset_ms / 2;
        engine.seek(seek_pos).expect("in-buffer seek");

        // The buffer poller should still be running after seek
        assert!(
            engine.buffer_poller_handle.is_some(),
            "buffer poller must continue running after seek"
        );

        engine.stop();
    }

    #[test]
    fn test_seek_does_not_crash_without_playback_handle() {
        // seek() should handle the case where no playback thread exists
        // without panicking. It will spawn a new thread that fails later.
        let mut engine = PlaybackEngine::new().expect("create engine");

        engine.playback_type = Some(PlaybackType::File {
            path: "/nonexistent/file.mp3".into(),
        });
        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 30000,
            read_offset_ms: 0,
            write_offset_ms: 30000,
            total_ms: 30000,
            is_complete: true,
        };

        // seek() should succeed even though no thread exists — it
        // will spawn a new decode thread that fails silently.
        let result = engine.seek(5000);
        assert!(result.is_ok(), "seek without handle must not panic");

        // seek_target_ms must be set
        assert_eq!(engine.seek_target_ms.load(Ordering::Relaxed), 5000);

        // Playback handle must now exist (new thread was spawned)
        assert!(
            engine.playback_handle.is_some(),
            "seek without handle should create a new playback handle"
        );

        engine.stop();
    }

    #[test]
    fn test_live_seek_preserves_live_start_time() {
        // The live_start_time must survive a seek so that the buffer
        // poller continues to compute correct write_offset_ms after
        // the seek restarts the decode thread.
        let mut engine = PlaybackEngine::new().expect("create engine");
        let _ = engine.play_live("http://127.0.0.1:1/nonexistent", 60_000);

        let start_time = {
            let guard = engine.live_start_time.lock().unwrap();
            *guard
        };
        assert!(start_time.is_some(), "live_start_time must be set");

        std::thread::sleep(Duration::from_millis(300));

        // Seek within buffer
        let buf = { *engine.download_buffer.lock() };
        let seek_pos = buf.write_offset_ms / 2;
        engine.seek(seek_pos).expect("seek");

        // live_start_time must NOT be reset during seek
        let after_seek = {
            let guard = engine.live_start_time.lock().unwrap();
            *guard
        };
        assert!(
            after_seek.is_some(),
            "live_start_time must persist across seek"
        );
        // Live buffer poller should continue to advance
        std::thread::sleep(Duration::from_millis(300));
        let buf2 = { *engine.download_buffer.lock() };
        assert!(
            buf2.write_offset_ms > buf.write_offset_ms,
            "write_offset must continue advancing after seek: was {} now {}",
            buf.write_offset_ms,
            buf2.write_offset_ms
        );

        engine.stop();
    }

    #[test]
    fn test_live_seek_beyond_buffer_clamps_to_end_ms() {
        // Live seeks beyond the buffered region must be clamped to the
        // buffer end instead of queued (which would never resolve since
        // live streams don't re-fetch via HTTP Range).
        let mut engine = PlaybackEngine::new().expect("create engine");
        engine.playback_type = Some(PlaybackType::Live {
            url: "http://127.0.0.1:1/nonexistent".into(),
            cache_max_ms: 60000,
        });
        engine.live_ring = Some(std::sync::Arc::new(std::sync::Mutex::new(
            crate::models::LiveByteRing::new(60000, 128_000),
        )));
        *engine.live_start_time.lock().unwrap() = Some(std::time::Instant::now());

        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 60000,
            read_offset_ms: 5000,
            write_offset_ms: 15000,
            total_ms: 0,
            is_complete: false,
        };

        // end_ms = read_offset_ms + (write_offset_ms - read_offset_ms) = 15000
        engine.seek(50000).expect("seek beyond buffer");

        // Seek target must be clamped to end_ms, not the requested 50000
        assert_eq!(
            engine.seek_target_ms.load(Ordering::Relaxed),
            15000,
            "live seek beyond buffer should clamp to end_ms"
        );

        engine.stop();
    }

    #[test]
    fn test_live_seek_within_buffer_passes_through() {
        // Live seeks within the buffered region should NOT be clamped
        // — they should pass through to the original target.
        let mut engine = PlaybackEngine::new().expect("create engine");
        engine.playback_type = Some(PlaybackType::Live {
            url: "http://127.0.0.1:1/nonexistent".into(),
            cache_max_ms: 60000,
        });
        engine.live_ring = Some(std::sync::Arc::new(std::sync::Mutex::new(
            crate::models::LiveByteRing::new(60000, 128_000),
        )));
        *engine.live_start_time.lock().unwrap() = Some(std::time::Instant::now());

        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 60000,
            read_offset_ms: 5000,
            write_offset_ms: 15000,
            total_ms: 0,
            is_complete: false,
        };

        // end_ms = 15000, seek to 10000 which is within buffer
        engine.seek(10000).expect("seek within buffer");

        assert_eq!(
            engine.seek_target_ms.load(Ordering::Relaxed),
            10000,
            "live seek within buffer should not clamp"
        );

        engine.stop();
    }

    #[test]
    fn test_seek_beyond_buffer_for_stream_is_queued_not_clamped() {
        // Non-live streams (YouTube/progressive) seeking beyond buffer
        // should still be queued (HTTP Range will fetch the data).
        let mut engine = PlaybackEngine::new().expect("create engine");
        engine.playback_type = Some(PlaybackType::Stream {
            url: "http://127.0.0.1:1/nonexistent".into(),
            seek_byte_offset: 0,
        });

        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 60000,
            read_offset_ms: 5000,
            write_offset_ms: 15000,
            total_ms: 0,
            is_complete: false,
        };

        // end_ms = 15000, seek to 50000 which is beyond buffer
        let far_pos = 50000;
        engine.seek(far_pos).expect("seek beyond buffer");

        // Seek target must NOT be clamped (YouTube streams use Range header)
        assert_eq!(
            engine.seek_target_ms.load(Ordering::Relaxed),
            far_pos,
            "stream seek beyond buffer should keep original target (queued)"
        );

        engine.stop();
    }

    // ═══════════════════════════════════════════════════════════════════
    // YouTube / Seek-capable stream seek tests
    // ═══════════════════════════════════════════════════════════════════

    use crate::audio::stream::source::{Capability, ReadSeek, SourceInfo, SourceKind, StreamSource};

    /// A seek-capable stream source backed by an in-memory byte buffer,
    /// used to test the YouTube stream seek path. Reports `SourceKind::YouTube`
    /// and supports `Capability::Seek`, so the engine routes through the
    /// seeking-source branch in `seek()` rather than the Radio fallback.
    struct TestStreamSource {
        data: Vec<u8>,
        info: SourceInfo,
    }

    impl TestStreamSource {
        fn new(data: Vec<u8>) -> Self {
            Self {
                data,
                info: SourceInfo {
                    kind: SourceKind::YouTube,
                    stream_type: crate::models::StreamType::Seekable { total_bytes: 0 },
                    uri: "test://stream".into(),
                    title: Some("Test Stream".into()),
                    artist: None,
                    album: None,
                },
            }
        }
    }

    impl StreamSource for TestStreamSource {
        fn info(&self) -> &SourceInfo {
            &self.info
        }

        fn supports(&self, capability: Capability) -> bool {
            matches!(capability, Capability::Seek | Capability::Download)
        }

        fn open(
            &self,
            _seek_to: Option<u64>,
        ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
            Ok(Box::new(std::io::Cursor::new(self.data.clone())))
        }

        fn total_bytes(&self) -> Option<u64> {
            Some(self.data.len() as u64)
        }
    }

    /// A wrapper that records every `open()` call so tests can verify
    /// whether `open(None)` vs `open(Some(ms))` was used during seek.
    struct TrackingSource {
        inner: TestStreamSource,
        open_calls: Arc<std::sync::Mutex<Vec<Option<u64>>>>,
    }

    impl StreamSource for TrackingSource {
        fn info(&self) -> &SourceInfo {
            self.inner.info()
        }

        fn supports(&self, capability: Capability) -> bool {
            self.inner.supports(capability)
        }

        fn open(
            &self,
            seek_to: Option<u64>,
        ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
            self.open_calls.lock().unwrap().push(seek_to);
            self.inner.open(seek_to)
        }

        fn total_bytes(&self) -> Option<u64> {
            self.inner.total_bytes()
        }
    }

    fn make_mp3_data(frame_count: u32) -> Vec<u8> {
        let mut data = Vec::with_capacity(frame_count as usize * 144);
        let frame_header: [u8; 4] = [0xFF, 0xFB, 0x90, 0x44];
        for _ in 0..frame_count {
            data.extend_from_slice(&frame_header);
            data.extend(std::iter::repeat(0u8).take(140));
        }
        data
    }

    /// YouTube stream seek within buffered area must call `source.open(None)`
    /// (not `source.open(Some(ms))`), because opening with a byte-offset
    /// Range request puts the reader mid-Cluster for WebM containers, causing
    /// EBML parser errors. The actual seeking is handled by `seek_to_position`
    /// in `decode_and_play_from_read`.
    #[test]
    fn test_youtube_stream_seek_within_buffer_calls_open_none() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let mp3 = make_mp3_data(383);
        let calls: Arc<std::sync::Mutex<Vec<Option<u64>>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));

        let source = TrackingSource {
            inner: TestStreamSource::new(mp3),
            open_calls: calls.clone(),
        };
        engine.source = Some(Box::new(source));
        engine.playback_type = Some(PlaybackType::Stream {
            url: "test://stream".into(),
            seek_byte_offset: 0,
        });

        // Set buffer state so the seek target is within the buffered area.
        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 60000,
            read_offset_ms: 500,
            write_offset_ms: 20000,
            total_ms: 60000,
            is_complete: false,
        };

        // Seek to 5000ms — well within the [0, 20000] buffer window.
        let rc = engine.seek(5000);
        assert!(
            rc.is_ok(),
            "seek within buffer should succeed, got {:?}",
            rc
        );

        // With in-thread seek, no open() call should be made.
        let recorded_calls = calls.lock().unwrap();
        assert!(
            recorded_calls.is_empty(),
            "in-thread seek must not call source.open(), got {:?}",
            *recorded_calls
        );

        // seek_target_ms must be set to the requested position.
        assert_eq!(
            engine.seek_target_ms.load(Ordering::Relaxed),
            5000,
            "seek_target_ms should be 5000"
        );

        engine.stop();
    }

    /// YouTube stream seek within buffered area must emit SEEK_STARTED
    /// with the correct position and must not error.
    #[test]
    fn test_youtube_stream_seek_within_buffer_emits_started() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let mp3 = make_mp3_data(383);

        let source = TestStreamSource::new(mp3);
        engine.source = Some(Box::new(source));
        engine.playback_type = Some(PlaybackType::Stream {
            url: "test://stream".into(),
            seek_byte_offset: 0,
        });

        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 60000,
            read_offset_ms: 500,
            write_offset_ms: 20000,
            total_ms: 60000,
            is_complete: false,
        };

        // Drain any stale events.
        loop {
            let e = engine.event_queue.lock().pop_front();
            if e.is_none() {
                break;
            }
        }

        let seek_pos: u64 = 5000;
        let rc = engine.seek(seek_pos);
        assert!(rc.is_ok(), "seek should succeed");

        // Check that SEEK_STARTED was emitted.
        let mut found_started = false;
        while let Some(event) = engine.event_queue.lock().pop_front() {
            if event.event_type == crate::models::ENGINE_EVENT_SEEK_STARTED {
                assert_eq!(event.int_param as u64, seek_pos);
                found_started = true;
                break;
            }
        }
        assert!(found_started, "expected SEEK_STARTED event");

        engine.stop();
    }

    /// Seek within buffered area on a YouTube stream must NOT be queued
    /// (it should be applied immediately via the source reopen path).
    #[test]
    fn test_youtube_stream_seek_within_buffer_not_queued() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let mp3 = make_mp3_data(383);

        let source = TestStreamSource::new(mp3);
        engine.source = Some(Box::new(source));
        engine.playback_type = Some(PlaybackType::Stream {
            url: "test://stream".into(),
            seek_byte_offset: 0,
        });

        // Buffer is fully available (complete) — seek is always immediate.
        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 60000,
            read_offset_ms: 500,
            write_offset_ms: 60000,
            total_ms: 60000,
            is_complete: true,
        };

        // Drain stale events.
        loop {
            let e = engine.event_queue.lock().pop_front();
            if e.is_none() {
                break;
            }
        }

        let rc = engine.seek(5000);
        assert!(rc.is_ok(), "seek should succeed");

        // Must NOT emit SEEK_QUEUED since target is within buffer.
        let mut found_queued = false;
        while let Some(event) = engine.event_queue.lock().pop_front() {
            if event.event_type == crate::models::ENGINE_EVENT_SEEK_QUEUED {
                found_queued = true;
                break;
            }
        }
        assert!(!found_queued, "in-buffer seek must not be queued");

        engine.stop();
    }

    /// YouTube stream seek beyond buffered area must be queued
    /// (the engine waits for the download to reach the target).
    #[test]
    fn test_youtube_stream_seek_beyond_buffer_is_queued() {
        let mut engine = PlaybackEngine::new().expect("create engine");
        let mp3 = make_mp3_data(383);

        let source = TestStreamSource::new(mp3);
        engine.source = Some(Box::new(source));
        engine.playback_type = Some(PlaybackType::Stream {
            url: "test://stream".into(),
            seek_byte_offset: 0,
        });

        // Buffer only covers [0, 20000), seek target 50000 is beyond.
        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 60000,
            read_offset_ms: 500,
            write_offset_ms: 20000,
            total_ms: 60000,
            is_complete: false,
        };

        // Drain stale events.
        loop {
            let e = engine.event_queue.lock().pop_front();
            if e.is_none() {
                break;
            }
        }

        let far_pos: u64 = 50000;
        let rc = engine.seek(far_pos);
        assert!(rc.is_ok(), "seek beyond buffer should succeed");

        // Must emit SEEK_QUEUED since target is beyond buffer.
        let mut found_queued = false;
        while let Some(event) = engine.event_queue.lock().pop_front() {
            if event.event_type == crate::models::ENGINE_EVENT_SEEK_QUEUED {
                found_queued = true;
                break;
            }
        }
        assert!(found_queued, "beyond-buffer seek should be queued");

        // seek_target_ms must keep the original (unclamped) target.
        assert_eq!(
            engine.seek_target_ms.load(Ordering::Relaxed),
            far_pos,
            "stream seek beyond buffer should keep original target"
        );

        engine.stop();
    }

    /// Live stream seeking within buffer must call the Live reopen path,
    /// not the Stream path, even when source.supports(Seek) is true.
    #[test]
    fn test_live_seek_within_buffer_uses_live_path() {
        let mut engine = PlaybackEngine::new().expect("create engine");

        // Set up a live-stream-like state.
        engine.playback_type = Some(PlaybackType::Live {
            url: "http://127.0.0.1:1/nonexistent".into(),
            cache_max_ms: 60000,
        });
        engine.live_ring = Some(std::sync::Arc::new(std::sync::Mutex::new(
            crate::models::LiveByteRing::new(60000, 128_000),
        )));
        *engine.live_start_time.lock().unwrap() = Some(std::time::Instant::now());

        // Buffer with some data.
        *engine.download_buffer.lock() = DownloadBuffer {
            capacity_ms: 60000,
            read_offset_ms: 5000,
            write_offset_ms: 15000,
            total_ms: 0,
            is_complete: false,
        };

        // Drain stale events.
        loop {
            let e = engine.event_queue.lock().pop_front();
            if e.is_none() {
                break;
            }
        }

        let seek_pos: u64 = 10000;
        let rc = engine.seek(seek_pos);
        assert!(rc.is_ok(), "live seek within buffer should succeed");

        // Live seek must NOT emit SEEK_QUEUED.
        let mut found_queued = false;
        while let Some(event) = engine.event_queue.lock().pop_front() {
            if event.event_type == crate::models::ENGINE_EVENT_SEEK_QUEUED {
                found_queued = true;
                break;
            }
        }
        assert!(!found_queued, "in-buffer live seek must not be queued");

        // seek_target_ms must be the original (unclamped) position.
        assert_eq!(
            engine.seek_target_ms.load(Ordering::Relaxed),
            seek_pos,
            "live seek within buffer should keep original target"
        );

        engine.stop();
    }
}
