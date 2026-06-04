//! Public commands (methods) for the PlaybackEngine.

use super::types::{PlaybackEngine, PlaybackType};
use crate::audio::error::PlaybackError;
use crate::audio::stream::queue_source::AudioBuffer;
use crate::models::PlaybackState;
use log::{debug, error, info, warn};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(target_os = "android")]
use reqwest::Client as AsyncClient;

#[cfg(not(target_os = "android"))]
use reqwest::blocking::Client as AsyncClient;

impl PlaybackEngine {
    /// Play a URI with auto-detected source type and auto-configured pipeline.
    pub fn play(&mut self, uri: &str) -> Result<(), PlaybackError> {
        use crate::audio::stream::source;
        let pipeline = source::from_uri(uri, self.http_client.clone(), None)?;
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
        {
            let mut q = self.audio_queue.lock();
            q.clear();
        }
        if let Some(handle) = self.playback_handle.take() {
            Self::join_with_timeout(handle, "play");
        }

        self.state = PlaybackState::Connecting;
        self.load_error.lock().clear();
        self.stream_url = Some(pipeline.info().uri.clone());
        self.samples_played.store(0, Ordering::Relaxed);
        self.total_duration_ms.store(0, Ordering::Relaxed);
        self.buffer_ready.store(false, Ordering::Relaxed);
        self.seek_target_ms.store(0, Ordering::Relaxed);

        // Extract pipe writer (works through decorator layers)
        self.stream_pipe = pipeline.pipe_writer();

        // Open the reader
        let reader = pipeline.open(None)?;

        // Set playback_type for backward compat
        let kind = pipeline.info().kind;
        let uri_for_playback = pipeline.info().uri.clone();
        #[cfg(target_os = "android")]
        let file_path_for_android = uri_for_playback.clone();
        #[cfg(target_os = "android")]
        let source_kind_for_android = kind;
        #[cfg(not(target_os = "android"))]
        let file_path_for_thread = uri_for_playback.clone();
        let source_kind = kind;
        self.playback_type = Some(match kind {
            crate::audio::stream::source::SourceKind::File => {
                PlaybackType::File { path: uri_for_playback }
            }
            crate::audio::stream::source::SourceKind::Radio => PlaybackType::Stream {
                url: uri_for_playback,
                seek_byte_offset: 0,
            },
            crate::audio::stream::source::SourceKind::YouTube => PlaybackType::AdaptiveBuffer {
                url: uri_for_playback,
                video_id: pipeline.info().title.clone(),
                cache_dir: String::new(),
            },
            crate::audio::stream::source::SourceKind::Pipe => PlaybackType::Pipe {
                url: uri_for_playback,
                video_id: None,
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
                            should_stop,
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error,
                            seek_target_ms,
                        );
                    }
                    _ => {
                        crate::audio::stream::handling::decode_and_play_from_read(
                            reader,
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
                    }
                }
                #[cfg(target_os = "android")]
                match source_kind_for_android {
                    crate::audio::stream::source::SourceKind::File => {
                        crate::audio::decoder::android_file_decoder::play_file_internal(
                            file_path_for_android,
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
                    }
                    _ => {
                        crate::audio::decoder::android_file_decoder::decode_and_play_from_read(
                            reader,
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
                    }
                }
            })
            .map_err(|e| PlaybackError::ThreadSpawn {
                operation: "play".into(),
                detail: e.to_string(),
            })?;

        self.playback_handle = Some(handle);
        self.state = PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        };

        // Wait for initial buffer
        let timeout = Duration::from_secs(30);
        let start = std::time::Instant::now();
        while !self.buffer_ready.load(Ordering::Relaxed)
            && start.elapsed() < timeout
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !self.buffer_ready.load(Ordering::Relaxed) {
            let err = self.load_error.lock().clone();
            if err.is_empty() {
                warn!("[engine] Playback failed to start within timeout");
            } else {
                error!("[engine] Playback error: {}", err);
            }
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
        let http_client = Arc::new(AsyncClient::new());

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

        self.state = PlaybackState::Connecting;
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
        let play_fn = crate::audio::decoder::android_file_decoder::play_file_internal;
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
        self.state = PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        };

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
        }

        Ok(())
    }

    /// Plays an HTTP stream.
    pub fn play_stream(&mut self, url: &str) -> Result<(), PlaybackError> {
        info!("[playback] play_stream: loading from {}", url);
        self.should_stop.store(true, Ordering::Relaxed);
        self.state = PlaybackState::Connecting;
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
        self.playback_type = Some(PlaybackType::Stream {
            url: url_owned.clone(),
            seek_byte_offset: 0,
        });

        if let Some(handle) = self.playback_handle.take() {
            debug!("[playback] Stopping previous playback thread");
            Self::join_with_timeout(handle, "play_stream");
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
        let client = self.http_client.clone();
        let seek_target_ms = self.seek_target_ms.clone();
        let _playback_type = self.playback_type.clone();

        #[cfg(target_os = "android")]
        let play_fn = crate::audio::decoder::android_file_decoder::play_stream_internal;
        #[cfg(not(target_os = "android"))]
        let play_fn = crate::audio::stream::handling::play_stream_internal;

        let handle = thread::spawn(move || {
            play_fn(
                url_owned,
                client,
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
                0,
            );
        });

        self.playback_handle = Some(handle);
        self.state = PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        };

        let start_time = std::time::Instant::now();
        while !self.buffer_ready.load(Ordering::Relaxed)
            && start_time.elapsed() < Duration::from_secs(30)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !self.buffer_ready.load(Ordering::Relaxed) {
            let err = self.load_error.lock().clone();
            if err.is_empty() {
                warn!("[playback] Stream failed to start within timeout");
            } else {
                error!("[playback] Stream error: {}", err);
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
        self.state = PlaybackState::Stopped;
        self.seek_target_ms.store(0, Ordering::Relaxed);
        self.playback_type = None;

        if let Some(handle) = self.playback_handle.take() {
            let _ = handle.join();
            debug!("[engine] Playback thread joined");
        }
    }

    /// Pauses playback.
    pub fn pause(&mut self) {
        debug!("[engine] Pausing playback");
        self.buffer_ready.store(false, Ordering::Relaxed);
        self.is_playing_flag.store(false, Ordering::Relaxed);
        self.state = PlaybackState::Paused;
    }

    /// Resumes playback.
    pub fn resume(&mut self) {
        debug!("[engine] Resuming playback");
        self.buffer_ready.store(true, Ordering::Relaxed);
        self.is_playing_flag.store(true, Ordering::Relaxed);
        self.state = PlaybackState::Playing;
    }

    /// Seeks to position in milliseconds.
    pub fn seek(&mut self, position_ms: u64) -> Result<(), PlaybackError> {
        info!("[engine] Seeking to {} ms", position_ms);
        self.seek_target_ms.store(position_ms, Ordering::Relaxed);

        // If we're playing a stream, we might need to restart it with a Range header
        let current_playback_type = self.playback_type.clone();
        if let Some(playback_type) = current_playback_type {
            match playback_type {
                PlaybackType::Stream { url, .. } => {
                    // For streams, we need to restart the stream from the new position
                    self.play_stream(&url)?;
                }
                PlaybackType::File { path } => {
                    // For files, we need to restart the decode thread with the new position
                    info!(
                        "[engine] Restarting file playback for seek to {} ms",
                        position_ms
                    );
                    self.should_stop.store(true, Ordering::Relaxed);
                    self.buffer_ready.store(false, Ordering::Relaxed);
                    {
                        let mut q = self.audio_queue.lock();
                        q.clear();
                    }
                    self.samples_played.store(0, Ordering::Relaxed);
                    self.seek_target_ms.store(position_ms, Ordering::Relaxed);

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

                    #[cfg(target_os = "android")]
                    let play_fn =
                        crate::audio::decoder::android_file_decoder::play_file_internal;
                    #[cfg(not(target_os = "android"))]
                    let play_fn = crate::audio::decoder::file_decoder::play_file_internal;

                    let handle = thread::spawn(move || {
                        play_fn(
                            path,
                            audio_queue,
                            buffer_ready,
                            is_playing_flag,
                            should_stop,
                            samples_played,
                            sample_rate,
                            channels,
                            total_duration_ms,
                            load_error,
                            seek_target,
                        );
                    });
                    self.playback_handle = Some(handle);
                }
                PlaybackType::Pipe { url: _, .. } => {
                    // For pipes, we need to restart both the fetch and decode threads
                    info!(
                        "[engine] Restarting pipe playback for seek to {} ms",
                        position_ms
                    );
                    self.should_stop.store(true, Ordering::Relaxed);
                    self.buffer_ready.store(false, Ordering::Relaxed);
                    {
                        let mut q = self.audio_queue.lock();
                        q.clear();
                    }
                    self.samples_played.store(0, Ordering::Relaxed);
                    self.seek_target_ms.store(position_ms, Ordering::Relaxed);

                    // Signal fetch thread to reconnect
                    if let Some(pipe) = &self.stream_pipe {
                        let byte_offset = self.calculate_byte_offset_for_seek(position_ms);
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

                    #[cfg(target_os = "android")]
                    let play_fn =
                        crate::audio::decoder::android_file_decoder::play_stream_from_pipe_internal;
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
                }
                PlaybackType::AdaptiveBuffer { url, cache_dir, .. } => {
                    // For adaptive buffer, we need to restart both the fetch and decode threads
                    info!(
                        "[engine] Restarting adaptive buffer playback for seek to {} ms",
                        position_ms
                    );
                    self.should_stop.store(true, Ordering::Relaxed);
                    self.buffer_ready.store(false, Ordering::Relaxed);
                    {
                        let mut q = self.audio_queue.lock();
                        q.clear();
                    }
                    self.samples_played.store(0, Ordering::Relaxed);
                    self.seek_target_ms.store(position_ms, Ordering::Relaxed);

                    // Get the existing pipe writer from stream_pipe
                    let pipe_writer = self.stream_pipe.clone().unwrap();

                    // Stop old decode thread
                    if let Some(handle) = self.playback_handle.take() {
                        Self::join_with_timeout(handle, "adaptive-buffer-seek-restart");
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
                    let seek_target_ms = Arc::new(AtomicU64::new(position_ms));

                    #[cfg(target_os = "android")]
                    let play_fn =
                        crate::audio::decoder::android_file_decoder::play_adaptive_buffer_internal;
                    #[cfg(not(target_os = "android"))]
                    let play_fn = crate::audio::stream::handling::play_adaptive_buffer_internal;

                    let handle = thread::spawn(move || {
                        play_fn(
                            pipe_writer,
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
                            url,
                            cache_dir,
                        );
                    });
                    self.playback_handle = Some(handle);
                }
            }
        }
        Ok(())
    }

    /// Skips forward by milliseconds.
    pub fn skip_forward(&mut self, ms: u64) -> Result<(), PlaybackError> {
        info!("[engine] Skipping forward {} ms", ms);
        let new_position = self.position.current_ms + ms;
        self.seek(new_position)
    }

    /// Skips backward by milliseconds.
    pub fn skip_backward(&mut self, ms: u64) -> Result<(), PlaybackError> {
        info!("[engine] Skipping backward {} ms", ms);
        let new_position = self.position.current_ms.saturating_sub(ms);
        self.seek(new_position)
    }

    /// Sets the volume (0.0 to 1.0).
    pub fn set_volume(&self, volume: f32) {
        debug!("[engine] Setting volume to {}", volume);
        // In a real implementation, this would control the actual audio output volume
        // For now, we'll just store it if needed
    }

    /// Gets the current volume.
    pub fn get_volume(&self) -> f32 {
        debug!("[engine] Getting volume");
        // In a real implementation, this would return the actual audio output volume
        // For now, we'll return a default value
        1.0
    }

    /// Play a stream from bytes (internal method for FFI).
    pub fn play_stream_from_bytes_internal(&mut self, url: &str) -> Result<(), PlaybackError> {
        info!(
            "[playback] play_stream_from_bytes_internal: loading from {}",
            url
        );
        self.should_stop.store(true, Ordering::Relaxed);
        self.state = PlaybackState::Connecting;
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
        let play_fn = crate::audio::decoder::android_file_decoder::play_stream_from_pipe_internal;
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
        self.state = PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        };

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
        }
        Ok(())
    }

    /// Plays a stream with adaptive buffering and caching.
    pub fn play_adaptive_buffer(
        &mut self,
        url: &str,
        cache_dir: &str,
    ) -> Result<(), PlaybackError> {
        info!(
            "[playback] play_adaptive_buffer: loading from {} with cache at {}",
            url, cache_dir
        );
        self.should_stop.store(true, Ordering::Relaxed);
        self.state = PlaybackState::Connecting;
        self.load_error.lock().clear();
        let url_owned = url.to_string();
        let cache_dir_owned = cache_dir.to_string();
        self.stream_url = Some(url_owned.clone());
        self.samples_played.store(0, Ordering::Relaxed);
        self.total_duration_ms.store(0, Ordering::Relaxed);
        {
            let mut q = self.audio_queue.lock();
            q.clear();
        }
        self.buffer_ready.store(false, Ordering::Relaxed);
        self.playback_type = Some(PlaybackType::AdaptiveBuffer {
            url: url_owned.clone(),
            video_id: None,
            cache_dir: cache_dir_owned.clone(),
        });

        let (pipe_writer, _pipe_reader) = crate::audio::stream::pipe::new_pipe();
        let pipe_writer = Arc::new(pipe_writer);
        self.stream_pipe = Some(pipe_writer.clone());

        if let Some(handle) = self.playback_handle.take() {
            debug!("[playback] Stopping previous playback thread");
            Self::join_with_timeout(handle, "play_adaptive_buffer");
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

        #[cfg(target_os = "android")]
        let play_fn = crate::audio::decoder::android_file_decoder::play_adaptive_buffer_internal;
        #[cfg(not(target_os = "android"))]
        let play_fn = crate::audio::stream::handling::play_adaptive_buffer_internal;

        let handle = thread::spawn(move || {
            play_fn(
                pipe_writer,
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
                url_owned,
                cache_dir_owned,
            );
        });

        self.playback_handle = Some(handle);
        self.state = PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        };

        let start_time = std::time::Instant::now();
        while !self.buffer_ready.load(Ordering::Relaxed)
            && start_time.elapsed() < Duration::from_secs(30)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !self.buffer_ready.load(Ordering::Relaxed) {
            let err = self.load_error.lock().clone();
            if err.is_empty() {
                warn!("[playback] Adaptive buffer playback failed to start within timeout");
            } else {
                error!("[playback] Adaptive buffer playback error: {}", err);
            }
        }
        Ok(())
    }

    /// Plays a stream using stream_download crate for progressive download.
    pub fn play_stream_with_downloader(&mut self, url: &str) -> Result<(), PlaybackError> {
        info!(
            "[playback] play_stream_with_downloader: loading from {}",
            url
        );
        self.should_stop.store(true, Ordering::Relaxed);
        self.state = PlaybackState::Connecting;
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
        self.playback_type = Some(PlaybackType::Stream {
            url: url_owned.clone(),
            seek_byte_offset: 0,
        });

        if let Some(handle) = self.playback_handle.take() {
            debug!("[playback] Stopping previous playback thread");
            Self::join_with_timeout(handle, "play_stream_with_downloader");
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

        let handle = thread::spawn(move || {
            #[cfg(target_os = "android")]
            crate::audio::decoder::android_file_decoder::play_stream_with_downloader_internal(
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
                0,
            );
            #[cfg(not(target_os = "android"))]
            crate::audio::stream::handling::play_stream_internal(
                url_owned,
                Arc::new(AsyncClient::new()),
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
                0,
            );
        });

        self.playback_handle = Some(handle);
        self.state = PlaybackState::Buffering {
            buffered_bytes: 0,
            total_bytes: None,
        };

        let start_time = std::time::Instant::now();
        while !self.buffer_ready.load(Ordering::Relaxed)
            && start_time.elapsed() < Duration::from_secs(30)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !self.buffer_ready.load(Ordering::Relaxed) {
            let err = self.load_error.lock().clone();
            if err.is_empty() {
                warn!("[playback] Stream with downloader failed to start within timeout");
            } else {
                error!("[playback] Stream with downloader error: {}", err);
            }
        }
        Ok(())
    }
}
