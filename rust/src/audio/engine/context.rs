use crate::audio::engine::types::HttpClient;
use crate::audio::stream::queue_source::AudioBuffer;
use crate::models::PlaybackPosition;
use crate::models::PlaybackState;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

/// Context structure to hold shared data for playback threads
/// This reduces the need to clone multiple Arc fields individually
pub struct PlaybackContext {
    /// Queue to hold audio samples before playback
    pub audio_queue: AudioBuffer,
    /// Flag indicating if the audio buffer is ready
    pub buffer_ready: Arc<AtomicBool>,
    /// Flag indicating if audio is currently playing
    pub is_playing_flag: Arc<AtomicBool>,
    /// Flag to signal the playback thread to stop
    pub should_stop: Arc<AtomicBool>,
    /// Number of samples played so far
    pub samples_played: Arc<AtomicU64>,
    /// Sample rate of the audio output
    pub sample_rate: Arc<AtomicU64>,
    /// Number of audio channels
    pub channels: Arc<AtomicU64>,
    /// Total duration of the audio in milliseconds
    pub total_duration_ms: Arc<AtomicU64>,
    /// Stores any error encountered during loading
    pub load_error: Arc<Mutex<String>>,
    /// Seek target in milliseconds — checked by decode threads to skip to position
    pub seek_target_ms: Arc<AtomicU64>,
    /// HTTP client for making network requests
    pub http_client: Option<Arc<HttpClient>>,
    /// Handle to the playback thread
    pub playback_handle: Option<thread::JoinHandle<()>>,
    /// The current playback position
    pub position: PlaybackPosition,
    /// The current playback state
    pub state: PlaybackState,
    /// The URL of the current stream, if any
    pub stream_url: Option<String>,
    /// Pipe writer for Dart-piped streaming (YouTube 403 workaround)
    pub stream_pipe: Option<Arc<crate::audio::stream::pipe::PipeWriter>>,
    /// Last playback type for restart after seek
    pub playback_type: Option<crate::audio::engine::types::PlaybackType>,
}

impl PlaybackContext {
    /// Creates a new playback context
    pub fn new(
        audio_queue: AudioBuffer,
        buffer_ready: Arc<AtomicBool>,
        is_playing_flag: Arc<AtomicBool>,
        should_stop: Arc<AtomicBool>,
        samples_played: Arc<AtomicU64>,
        sample_rate: Arc<AtomicU64>,
        channels: Arc<AtomicU64>,
        total_duration_ms: Arc<AtomicU64>,
        load_error: Arc<Mutex<String>>,
        seek_target_ms: Arc<AtomicU64>,
        http_client: Option<Arc<HttpClient>>,
        position: PlaybackPosition,
        state: PlaybackState,
        stream_url: Option<String>,
        stream_pipe: Option<Arc<crate::audio::stream::pipe::PipeWriter>>,
        playback_type: Option<crate::audio::engine::types::PlaybackType>,
    ) -> Self {
        Self {
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
            http_client,
            playback_handle: None,
            position,
            state,
            stream_url,
            stream_pipe,
            playback_type,
        }
    }

    /// Creates a basic playback context with default values
    pub fn new_basic(audio_queue: AudioBuffer) -> Self {
        Self {
            audio_queue,
            buffer_ready: Arc::new(AtomicBool::new(false)),
            is_playing_flag: Arc::new(AtomicBool::new(false)),
            should_stop: Arc::new(AtomicBool::new(false)),
            samples_played: Arc::new(AtomicU64::new(0)),
            sample_rate: Arc::new(AtomicU64::new(44100)),
            channels: Arc::new(AtomicU64::new(2)),
            total_duration_ms: Arc::new(AtomicU64::new(0)),
            load_error: Arc::new(Mutex::new(String::new())),
            seek_target_ms: Arc::new(AtomicU64::new(0)),
            http_client: None,
            playback_handle: None,
            position: PlaybackPosition::default(),
            state: PlaybackState::Stopped,
            stream_url: None,
            stream_pipe: None,
            playback_type: None,
        }
    }

    /// Updates the context with new playback type
    pub fn set_playback_type(&mut self, playback_type: crate::audio::engine::types::PlaybackType) {
        self.playback_type = Some(playback_type);
    }

    /// Updates the context with new stream URL
    pub fn set_stream_url(&mut self, url: String) {
        self.stream_url = Some(url);
    }

    /// Updates the context with new position
    pub fn set_position(&mut self, position: PlaybackPosition) {
        self.position = position;
    }

    /// Updates the context with new state
    pub fn set_state(&mut self, state: PlaybackState) {
        self.state = state;
    }

    /// Updates the context with new stream pipe
    pub fn set_stream_pipe(&mut self, pipe: Option<Arc<crate::audio::stream::pipe::PipeWriter>>) {
        self.stream_pipe = pipe;
    }

    /// Updates the context with new playback handle
    pub fn set_playback_handle(&mut self, handle: Option<thread::JoinHandle<()>>) {
        self.playback_handle = handle;
    }

    /// Clears the audio queue
    pub fn clear_audio_queue(&mut self) {
        let mut q = self.audio_queue.lock();
        q.clear();
    }

    /// Clears the load error
    pub fn clear_load_error(&mut self) {
        self.load_error.lock().clear();
    }

    /// Sets the buffer ready flag
    pub fn set_buffer_ready(&mut self, ready: bool) {
        self.buffer_ready.store(ready, Ordering::Relaxed);
    }

    /// Sets the is playing flag
    pub fn set_is_playing(&mut self, playing: bool) {
        self.is_playing_flag.store(playing, Ordering::Relaxed);
    }

    /// Sets the should stop flag
    pub fn set_should_stop(&mut self, stop: bool) {
        self.should_stop.store(stop, Ordering::Relaxed);
    }

    /// Sets the samples played counter
    pub fn set_samples_played(&mut self, count: u64) {
        self.samples_played.store(count, Ordering::Relaxed);
    }

    /// Sets the total duration in milliseconds
    pub fn set_total_duration_ms(&mut self, duration: u64) {
        self.total_duration_ms.store(duration, Ordering::Relaxed);
    }

    /// Sets the seek target in milliseconds
    pub fn set_seek_target_ms(&mut self, target: u64) {
        self.seek_target_ms.store(target, Ordering::Relaxed);
    }
}
