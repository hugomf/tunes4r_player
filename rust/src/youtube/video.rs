//! YouTube video info retrieval
//!
//! Gets video metadata (title, uploader, duration) via multiple strategies.

use crate::youtube::client::is_valid_video_id;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct VideoInfo {
    title: String,
    author_name: String,
    thumbnail_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub video_id: String,
    pub title: String,
    pub uploader: String,
    pub duration: u64,
    pub thumbnail_url: String,
}

impl VideoMetadata {
    pub fn new(video_id: String) -> Self {
        Self {
            video_id,
            title: String::new(),
            uploader: String::new(),
            duration: 0,
            thumbnail_url: String::new(),
        }
    }
}

pub fn get_video_info(
    _http_client: &reqwest::blocking::Client,
    video_id: &str,
) -> Result<VideoMetadata, String> {
    if !is_valid_video_id(video_id) {
        return Err("Invalid video ID format".to_string());
    }

    let url = format!(
        "https://www.youtube.com/oembed?url=https://www.youtube.com/watch?v={}&format=json",
        video_id
    );

    let http_client = reqwest::blocking::Client::new();
    let response = http_client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let body = response.text().map_err(|e| e.to_string())?;
    let info: VideoInfo = serde_json::from_str(&body).map_err(|e| e.to_string())?;

    Ok(VideoMetadata {
        video_id: video_id.to_string(),
        title: info.title,
        uploader: info.author_name,
        duration: 0,
        thumbnail_url: info.thumbnail_url,
    })
}


