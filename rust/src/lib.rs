//! Tunes4R Audio Engine — FFI surface crate
//!
//! Re-exports from tunes4r-core (audio engine, DSP, models) and
//! tunes4r-youtube (YouTube extraction), plus the FFI bindings for
//! Flutter integration and the audio_http_fetch legacy module.

pub use tunes4r_core::audio::{self, PlaybackEngine, PlaybackError};
pub use tunes4r_core::dsp::{self, Equalizer, RmsSpectrumAnalyzer, SpectrumAnalyzer, SpectrumConfig, DEFAULT_SPECTRUM_BANDS};
pub use tunes4r_core::models::{self, AdaptiveRingBuffer, DownloadBuffer, EqualizerBand, PlaybackPosition, PlaybackState, Song, SpectrumData};
pub use tunes4r_youtube as youtube;
pub use tunes4r_youtube::{get_audio_stream_url, get_video_info, search_videos, SearchResult, VideoMetadata, YouTubeService};

pub mod ffi;

#[cfg(feature = "classifier")]
pub mod classifier;

pub mod audio_http_fetch;

use flutter_rust_bridge::frb;

#[cfg(feature = "classifier")]
use classifier::Classifier;

/// Initialize logging
#[frb(init)]
pub fn init_app() {
    tracing_subscriber::fmt::init();
}

/// Initialize the search intent classifier
#[cfg(feature = "classifier")]
#[frb(sync)]
pub fn init_classifier(model_path: String, tokenizer_path: String) -> Result<(), String> {
    Classifier::init_global(&model_path, &tokenizer_path).map_err(|e| e.to_string())
}

/// Classify a search query
#[cfg(feature = "classifier")]
#[frb(sync)]
pub fn classify_query(query: String) -> Result<serde_json::Value, String> {
    let classifier =
        Classifier::global().ok_or_else(|| "Classifier not initialized".to_string())?;
    let mut guard = classifier.write().map_err(|e| e.to_string())?;
    guard
        .classify(&query)
        .map(|intent| {
            serde_json::json!({
                "label": intent.label(),
                "confidence": intent.confidence(),
            })
        })
        .map_err(|e| e.to_string())
}

/// Create a new playback engine
#[frb(sync)]
pub fn create_playback_engine() -> PlaybackEngine {
    PlaybackEngine::new().expect("Failed to create playback engine")
}

/// Unified play: auto-detect source type from URI and start playback.
pub fn play(engine: &mut PlaybackEngine, uri: String, buffer_size_ms: Option<u64>) -> Result<(), String> {
    engine
        .play(&uri, buffer_size_ms)
        .map_err(|e| format!("Play error: {}", e))
}

/// Check whether the current source supports seeking.
pub fn can_seek(engine: &mut PlaybackEngine) -> bool {
    engine.source_supports(tunes4r_core::audio::stream::source::Capability::Seek)
}

/// Check whether the current source supports downloading.
pub fn can_download(engine: &mut PlaybackEngine) -> bool {
    engine.source_supports(tunes4r_core::audio::stream::source::Capability::Download)
}

/// Play a local file
pub fn play_file(engine: &mut PlaybackEngine, file_path: String) -> Result<(), String> {
    engine
        .play_file(&file_path)
        .map_err(|e| format!("Playback error: {}", e))
}

/// Play an HTTP stream
pub fn play_stream(engine: &mut PlaybackEngine, url: String) -> Result<(), String> {
    engine
        .play_stream(&url)
        .map_err(|e| format!("Stream error: {}", e))
}

/// Pause playback
pub fn pause(engine: &mut PlaybackEngine) {
    engine.pause();
}

/// Resume playback
pub fn resume(engine: &mut PlaybackEngine) {
    engine.resume();
}

/// Stop playback
pub fn stop(engine: &mut PlaybackEngine) {
    engine.stop();
}

/// Seek to position in milliseconds
pub fn seek(engine: &mut PlaybackEngine, position_ms: u64) -> Result<(), String> {
    engine
        .seek(position_ms)
        .map_err(|e| format!("Seek error: {}", e))
}

/// Skip forward by milliseconds
pub fn skip_forward(engine: &mut PlaybackEngine, ms: u64) -> Result<(), String> {
    engine
        .skip_forward(ms)
        .map_err(|e| format!("Skip error: {}", e))
}

/// Skip backward by milliseconds
pub fn skip_backward(engine: &mut PlaybackEngine, ms: u64) -> Result<(), String> {
    engine
        .skip_backward(ms)
        .map_err(|e| format!("Skip error: {}", e))
}

/// Set volume (0.0 to 1.0)
pub fn set_volume(engine: &PlaybackEngine, volume: f32) {
    engine.set_volume(volume);
}

/// Get current volume
pub fn get_volume(engine: &PlaybackEngine) -> f32 {
    engine.get_volume()
}

/// Check if playing
pub fn is_playing(engine: &mut PlaybackEngine) -> bool {
    engine.is_playing()
}

/// Get playback state
pub fn get_playback_state(engine: &mut PlaybackEngine) -> PlaybackState {
    engine.get_state()
}

/// Get current position
pub fn get_position(engine: &mut PlaybackEngine) -> PlaybackPosition {
    engine.get_position()
}

/// Analyze spectrum (requires samples from audio callback)
pub fn analyze_spectrum(samples: Vec<f32>) -> SpectrumData {
    let mut analyzer = SpectrumAnalyzer::default();
    analyzer.analyze(&samples)
}

// Legacy compatibility
#[frb(sync)]
pub fn get_next_free_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}
