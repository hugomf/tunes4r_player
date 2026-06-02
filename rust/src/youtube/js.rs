//! JavaScript signature deciphering for YouTube
//!
//! Uses the custom deciphering algorithm to decipher signature-protected stream URLs.

use crate::youtube::js_engine;

pub fn decipher_signature(_action: &str, _video_id: &str, s: &str) -> String {
    js_engine::decipher_signature("", s)
}

pub fn fetch_player_js(video_id: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let watch_data = crate::youtube::watch::fetch_watch_page(&client, video_id)?;
    let player_url = watch_data
        .player_js_url
        .ok_or("No player JS URL found")?;

    crate::youtube::watch::fetch_player_js(&client, &player_url)
}

/// Decipher the n-parameter (throttle) using the player JavaScript.
pub fn decipher_throttle(js_code: &str, n_value: &str) -> String {
    js_engine::decipher_throttle(js_code, n_value)
}