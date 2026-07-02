#![allow(clippy::missing_safety_doc)]

pub use tunes4r_player::Player;
pub use tunes4r_player::PlaybackState;
pub use ytex as youtube;
pub use ytex::{
    get_audio_stream_url, get_video_info, search_videos, YouTubeService,
};

pub mod ffi;

#[cfg(feature = "classifier")]
pub mod classifier;

// ---------------------------------------------------------------------------
// Rust-native convenience API
// ---------------------------------------------------------------------------
//
// The functions below are thin wrappers around `tunes4r_player::Player`.
// They serve as a convenient Rust-side API for CLI examples (`cargo run
// --example`) and integration tests.  They are NOT called by the FFI layer
// (`crate::ffi`) — that path goes directly through C-compatible extern
// functions for use from Dart.
// ---------------------------------------------------------------------------

/// Create a new player instance.
pub fn create_player() -> Player {
    Player::new()
}

/// Start playback from a URL (file path, HTTP URL, or YouTube URL/ID).
pub fn play(player: &mut Player, url: String) -> Result<(), String> {
    player
        .start(&url)
        .map_err(|e| format!("Play error: {}", e))
}

/// Start playback from a file path.
pub fn play_file(player: &mut Player, file_path: String) -> Result<(), String> {
    let url = format!("file://{}", file_path);
    player
        .start(&url)
        .map_err(|e| format!("Playback error: {}", e))
}

/// Start playback from an HTTP stream.
pub fn play_stream(player: &mut Player, url: String) -> Result<(), String> {
    player
        .start(&url)
        .map_err(|e| format!("Stream error: {}", e))
}

/// Pause playback.
pub fn pause(player: &Player) {
    player.set_paused(true);
}

/// Resume playback.
pub fn resume(player: &Player) {
    player.set_paused(false);
}

/// Stop playback.
pub fn stop(player: &mut Player) {
    player.stop();
}

/// Seek to position in milliseconds.
pub fn seek(player: &Player, position_ms: u64) {
    player.seek(position_ms);
}

/// Set volume gain (1.0 = normal).
pub fn set_volume_gain(player: &Player, gain: f32) {
    player.set_volume_gain(gain);
}

/// Get current volume gain.
pub fn get_volume_gain(player: &Player) -> f32 {
    player.volume_gain()
}

/// Get current playback position.
pub fn get_position(player: &Player) -> (u64, u64) {
    (player.position_ms(), player.total_duration_ms())
}

/// Get playback state.
pub fn get_playback_state(player: &Player) -> PlaybackState {
    player.state()
}

/// Check if currently playing.
pub fn is_playing(player: &Player) -> bool {
    player.state() == PlaybackState::Playing
}
