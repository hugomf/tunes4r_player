//! Common data structures, enums, and global state for the audio engine.

use crate::audio::stream::queue_source::AudioBuffer;
use crate::audio::stream::source::{SourceInfo, StreamSource};
use crate::models::{PlaybackPosition, PlaybackState};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::RwLock;
use std::thread;

/// Enum representing the type of playback.
#[derive(Clone, Debug)]
pub enum PlaybackType {
    /// Playback from a local file.
    File { path: String },
    /// Direct HTTP stream. `seek_byte_offset > 0` means a Range request will be made.
    #[allow(dead_code)]
    Stream { url: String, seek_byte_offset: u64 },
    /// Playback from a pipe (e.g., for streaming data).
    Pipe {
        url: String,
        video_id: Option<String>,
    },
    /// Adaptive buffer playback with caching.
    AdaptiveBuffer {
        url: String,
        video_id: Option<String>,
        cache_dir: String,
    },
}

/// HTTP client wrapper for cross-platform compatibility.
/// On Android, we use async reqwest; on other platforms, we use blocking reqwest.
#[cfg(target_os = "android")]
pub type HttpClient = reqwest::Client;
/// HTTP client wrapper for cross-platform compatibility.
#[cfg(not(target_os = "android"))]
pub type HttpClient = reqwest::blocking::Client;

/// The main audio playback engine.
pub struct PlaybackEngine {
    pub(crate) state: PlaybackState,
    pub(crate) position: PlaybackPosition,
    pub(crate) stream_url: Option<String>,
    pub(crate) http_client: Arc<HttpClient>,
    pub(crate) load_error: Arc<Mutex<String>>,
    pub(crate) band_count: usize,
    pub(crate) sample_rate: Arc<AtomicU64>,
    pub(crate) channels: Arc<AtomicU64>,
    pub(crate) total_duration_ms: Arc<AtomicU64>,
    pub(crate) pipe_total_bytes: Arc<AtomicU64>,
    pub(crate) pipe_bytes_sent: Arc<AtomicU64>,
    pub(crate) audio_queue: AudioBuffer,
    pub(crate) buffer_ready: Arc<AtomicBool>,
    pub(crate) is_playing_flag: Arc<AtomicBool>,
    pub(crate) should_stop: Arc<AtomicBool>,
    pub(crate) samples_played: Arc<AtomicU64>,
    pub(crate) playback_handle: Option<thread::JoinHandle<()>>,
    pub(crate) stream_pipe: Option<Arc<crate::audio::stream::pipe::PipeWriter>>,
    pub(crate) playback_type: Option<PlaybackType>,
    pub(crate) source: Option<Box<dyn StreamSource>>,
    pub(crate) seek_target_ms: Arc<AtomicU64>,
}

impl PlaybackEngine {
    pub fn state(&self) -> PlaybackState {
        self.state.clone()
    }

    pub fn position(&self) -> PlaybackPosition {
        self.position.clone()
    }

    pub fn stream_url(&self) -> Option<String> {
        self.stream_url.clone()
    }

    pub fn band_count(&self) -> usize {
        self.band_count
    }

    pub fn sample_rate(&self) -> u64 {
        self.sample_rate.load(Ordering::Relaxed)
    }

    pub fn channels(&self) -> u64 {
        self.channels.load(Ordering::Relaxed)
    }

    pub fn total_duration_ms(&self) -> u64 {
        self.total_duration_ms.load(Ordering::Relaxed)
    }

    pub fn load_error(&self) -> String {
        self.load_error.lock().clone()
    }

    pub fn set_band_count(&mut self, count: usize) {
        self.band_count = count;
        crate::audio::engine::set_band_count(count);
    }

    pub fn audio_queue(&self) -> &AudioBuffer {
        &self.audio_queue
    }

    pub fn seek_target_ms(&self) -> u64 {
        self.seek_target_ms.load(Ordering::Relaxed)
    }

    pub fn playback_type(&self) -> Option<PlaybackType> {
        self.playback_type.clone()
    }

    pub fn source_info(&self) -> Option<SourceInfo> {
        self.source.as_ref().map(|s| s.info().clone())
    }

    pub fn source_supports(&self, capability: crate::audio::stream::source::Capability) -> bool {
        self.source
            .as_ref()
            .map_or(false, |s| s.supports(capability))
    }

    pub fn pipe_bytes_sent(&self) -> u64 {
        self.pipe_bytes_sent.load(Ordering::Relaxed)
    }
}

/// Global spectrum state for Android (accessed via FFI).
pub static GLOBAL_SPECTRUM: LazyLock<RwLock<Vec<f32>>> =
    LazyLock::new(|| RwLock::new(vec![0.0; 16]));

/// Mutex to protect the spectrum band count.
static SPECTRUM_BAND_COUNT: Mutex<usize> = Mutex::new(16);

/// Update global spectrum data from Android decoder.
pub fn update_global_spectrum(data: Vec<f32>) {
    let expected_len = get_band_count();
    let mut spectrum = GLOBAL_SPECTRUM.write().unwrap();
    if data.len() != expected_len {
        let mut resized = vec![0.0f32; expected_len];
        let copy_len = data.len().min(expected_len);
        resized[..copy_len].copy_from_slice(&data[..copy_len]);
        *spectrum = resized;
    } else {
        *spectrum = data;
    }
}

/// Get the current band count for the spectrum analyzer.
pub fn get_band_count() -> usize {
    *SPECTRUM_BAND_COUNT.lock()
}

/// Set the band count for the spectrum analyzer.
pub fn set_band_count(count: usize) {
    let mut band_count = SPECTRUM_BAND_COUNT.lock();
    *band_count = count;
    if let Ok(mut spectrum) = GLOBAL_SPECTRUM.write() {
        spectrum.resize(count, 0.0);
    }
}