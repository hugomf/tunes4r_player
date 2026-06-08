//! Tunes4R Audio Engine — FFI surface crate
//!
//! Re-exports from tunes4r-core (audio engine, DSP, models) and
//! tunes4r-youtube (YouTube extraction), plus the FFI bindings for
//! Flutter integration and the audio_http_fetch legacy module.

pub use tunes4r_core::audio::{self, PlaybackEngine, PlaybackError};
pub use tunes4r_core::dsp;
pub use tunes4r_core::models::{
    self, AdaptiveRingBuffer, DownloadBuffer, EngineEvent, PlaybackPosition, PlaybackState,
    ENGINE_EVENT_END_OF_STREAM, ENGINE_EVENT_ERROR, ENGINE_EVENT_NONE,
    ENGINE_EVENT_POSITION_RESET, ENGINE_EVENT_SEEK_COMPLETED, ENGINE_EVENT_SEEK_QUEUED,
    ENGINE_EVENT_SEEK_STARTED, ENGINE_EVENT_STATE_CHANGED,
};
pub use tunes4r_youtube as youtube;
pub use tunes4r_youtube::{
    get_audio_stream_url, get_video_info, search_videos, YouTubeService,
};

pub mod ffi;

#[cfg(feature = "classifier")]
pub mod classifier;

pub mod audio_http_fetch;

// ---------------------------------------------------------------------------
// Rust-native convenience API
// ---------------------------------------------------------------------------
//
// The functions below are thin wrappers around `tunes4r_core::PlaybackEngine`.
// They serve as a convenient Rust-side API for CLI examples (`cargo run
// --example`) and integration tests.  They are NOT called by the FFI layer
// (`crate::ffi`) — that path goes directly through C-compatible extern
// functions for use from Dart.
// ---------------------------------------------------------------------------

/// Create a new playback engine.
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

/// Play a local file.
pub fn play_file(engine: &mut PlaybackEngine, file_path: String) -> Result<(), String> {
    engine
        .play_file(&file_path)
        .map_err(|e| format!("Playback error: {}", e))
}

/// Play an HTTP stream.
pub fn play_stream(engine: &mut PlaybackEngine, url: String) -> Result<(), String> {
    engine
        .play_stream(&url)
        .map_err(|e| format!("Stream error: {}", e))
}

/// Pause playback.
pub fn pause(engine: &mut PlaybackEngine) {
    engine.pause();
}

/// Resume playback.
pub fn resume(engine: &mut PlaybackEngine) {
    engine.resume();
}

/// Stop playback.
pub fn stop(engine: &mut PlaybackEngine) {
    engine.stop();
}

/// Seek to position in milliseconds.
pub fn seek(engine: &mut PlaybackEngine, position_ms: u64) -> Result<(), String> {
    engine
        .seek(position_ms)
        .map_err(|e| format!("Seek error: {}", e))
}

/// Skip forward by milliseconds.
pub fn skip_forward(engine: &mut PlaybackEngine, ms: u64) -> Result<(), String> {
    engine
        .skip_forward(ms)
        .map_err(|e| format!("Skip error: {}", e))
}

/// Skip backward by milliseconds.
pub fn skip_backward(engine: &mut PlaybackEngine, ms: u64) -> Result<(), String> {
    engine
        .skip_backward(ms)
        .map_err(|e| format!("Skip error: {}", e))
}

/// Set volume (0.0 to 1.0).
pub fn set_volume(engine: &PlaybackEngine, volume: f32) {
    engine.set_volume(volume);
}

/// Get current volume.
pub fn get_volume(engine: &PlaybackEngine) -> f32 {
    engine.get_volume()
}

/// Set balance (0.0 = full left, 0.5 = center, 1.0 = full right).
pub fn set_balance(engine: &PlaybackEngine, balance: f32) {
    engine.set_balance(balance);
}

/// Get current balance.
pub fn get_balance(engine: &PlaybackEngine) -> f32 {
    engine.get_balance()
}

/// Check if playing.
pub fn is_playing(engine: &mut PlaybackEngine) -> bool {
    engine.is_playing()
}

/// Get playback state.
pub fn get_playback_state(engine: &mut PlaybackEngine) -> PlaybackState {
    engine.get_state()
}

/// Get current position.
pub fn get_position(engine: &mut PlaybackEngine) -> PlaybackPosition {
    engine.get_position()
}
