//! YouTube service for audio extraction
//!
//! Provides functionality to search YouTube and extract audio stream URLs.
//! Uses HTML scraping (primary) and Invidious instances (fallback) for search,
//! and InnerTube API + player JS deciphering for stream extraction.

pub mod client;
pub mod extractor;
pub mod formats;
pub mod js;
pub mod js_engine;
pub mod manifest;
pub mod search;
pub mod stream;
pub mod video;
pub mod watch;

pub use client::{Client, YtClient};
pub use extractor::StreamExtractor;
pub use formats::{AudioQuality, StreamFormat, VideoQuality};
pub use manifest::StreamManifest;
pub use search::search;
pub use search::{search_via_invidious, search_videos, SearchResult};
pub use stream::{get_audio_stream_url, refresh_audio_stream_url};
pub use video::{get_video_info, VideoMetadata};

use std::collections::HashMap;
use std::sync::Arc;

const CDN_URL_TTL_SECS: u64 = 4 * 60 * 60;

#[derive(Debug, Clone)]
struct CachedUrl {
    url: String,
    fetched_at: std::time::Instant,
    #[allow(dead_code)]
    requires_pot: bool,
}

impl CachedUrl {
    fn new(url: String, requires_pot: bool) -> Self {
        Self {
            url,
            fetched_at: std::time::Instant::now(),
            requires_pot,
        }
    }

    fn is_expired(&self) -> bool {
        self.fetched_at.elapsed().as_secs() > CDN_URL_TTL_SECS
    }
}

pub struct YouTubeService {
    http_client: Arc<reqwest::blocking::Client>,
    cdn_cache: HashMap<String, CachedUrl>,
    visitor_data: Option<String>,
    known_blocked: std::collections::HashSet<String>,
    cookies: Option<String>,
}

impl Default for YouTubeService {
    fn default() -> Self {
        Self::new()
    }
}

impl YouTubeService {
    pub fn new() -> Self {
        Self::builder().build()
    }

    pub fn builder() -> YouTubeServiceBuilder {
        YouTubeServiceBuilder::default()
    }

    pub fn http_client(&self) -> &reqwest::blocking::Client {
        &self.http_client
    }

    pub fn get_cached_url(&self, video_id: &str) -> Option<String> {
        self.cdn_cache
            .get(video_id)
            .filter(|c| !c.is_expired())
            .map(|c| c.url.clone())
    }

    pub fn cache_url(&mut self, video_id: String, url: String, _requires_pot: bool) {
        self.cdn_cache.insert(video_id, CachedUrl::new(url, false));
    }

    pub fn clear_cache(&mut self, video_id: &str) {
        self.cdn_cache.remove(video_id);
    }

    pub fn is_blocked(&self, video_id: &str) -> bool {
        self.known_blocked.contains(video_id)
    }

    pub fn mark_blocked(&mut self, video_id: String) {
        self.known_blocked.insert(video_id);
    }

    pub fn set_visitor_data(&mut self, visitor_data: String) {
        self.visitor_data = Some(visitor_data);
    }

    pub fn visitor_data(&self) -> Option<&String> {
        self.visitor_data.as_ref()
    }

    pub fn cookies(&self) -> Option<&String> {
        self.cookies.as_ref()
    }
}

#[derive(Default)]
pub struct YouTubeServiceBuilder {
    user_agent: Option<String>,
    cookies: Option<String>,
    timeout: Option<std::time::Duration>,
    proxy: Option<String>,
}

impl YouTubeServiceBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn user_agent(mut self, ua: String) -> Self {
        self.user_agent = Some(ua);
        self
    }

    pub fn cookies(mut self, cookies: String) -> Self {
        self.cookies = Some(cookies);
        self
    }

    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn proxy(mut self, proxy: String) -> Self {
        self.proxy = Some(proxy);
        self
    }

    pub fn build(self) -> YouTubeService {
        let mut builder = reqwest::blocking::Client::builder();

        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }

        if let Some(ua) = &self.user_agent {
            builder = builder.user_agent(ua);
        }

        if let Some(proxy_url) = &self.proxy {
            let proxy = reqwest::Proxy::all(proxy_url).expect("Invalid proxy URL");
            builder = builder.proxy(proxy);
        }

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::REFERER,
            "https://www.youtube.com".parse().unwrap(),
        );

        if let Some(cookies) = &self.cookies {
            headers.insert(
                reqwest::header::COOKIE,
                reqwest::header::HeaderValue::from_str(cookies).unwrap(),
            );
        }

        builder = builder.default_headers(headers);

        let http_client = Arc::new(builder.build().expect("Failed to build HTTP client"));

        YouTubeService {
            http_client,
            cdn_cache: HashMap::new(),
            visitor_data: None,
            known_blocked: std::collections::HashSet::new(),
            cookies: self.cookies,
        }
    }
}

impl Clone for YouTubeService {
    fn clone(&self) -> Self {
        Self {
            http_client: self.http_client.clone(),
            cdn_cache: HashMap::new(),
            visitor_data: self.visitor_data.clone(),
            known_blocked: std::collections::HashSet::new(),
            cookies: self.cookies.clone(),
        }
    }
}

/// High-level YouTube API (replaces youtube_explode::YouTube).
pub struct YouTube {
    client: Arc<Client>,
}

impl YouTube {
    pub fn new() -> Self {
        Self {
            client: Arc::new(Client::new()),
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn videos(&self) -> Videos {
        Videos {
            client: self.client.clone(),
        }
    }
}

impl Default for YouTube {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Videos {
    client: Arc<Client>,
}

impl Videos {
    pub fn get(&self, video_id: &str) -> Result<VideoInfo, String> {
        let url = format!(
            "https://www.youtube.com/oembed?url=https://www.youtube.com/watch?v={}&format=json",
            video_id
        );

        let response = self
            .client
            .http()
            .get(&url)
            .send()
            .map_err(|e| e.to_string())?;

        let data: serde_json::Value = response.json().map_err(|e| e.to_string())?;

        Ok(VideoInfo {
            id: video_id.to_string(),
            title: data
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            author: data
                .get("author_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            duration: 0,
        })
    }

    pub fn stream(&self, video_id: &str) -> Result<StreamManifest, String> {
        StreamExtractor::new(self.client.clone()).extract(video_id)
    }

    pub fn stream_with_client(
        &self,
        video_id: &str,
    ) -> Result<(StreamManifest, reqwest::blocking::Client), String> {
        let extractor = StreamExtractor::new(self.client.clone());
        let watch_data = extractor.fetch_watch_page(video_id)?;

        let mut player_js_code: Option<String> = None;
        let mut signature_transforms: Option<Vec<String>> = None;

        if let Some(ref player_js_url) = watch_data.player_js_url {
            if let Ok(js_code) =
                crate::youtube::watch::fetch_player_js(self.client.http(), player_js_url)
            {
                let transforms =
                    crate::youtube::watch::extract_signature_transforms(&js_code);
                if !transforms.is_empty() {
                    signature_transforms = Some(transforms);
                }
                player_js_code = Some(js_code);
            }
        }

        let clients = crate::youtube::client::get_yt_clients();
        let mut last_error = String::new();

        for yt_client in clients.iter() {
            match extractor.extract_with_client_internal(
                video_id,
                yt_client,
                &watch_data,
                &player_js_code,
                &signature_transforms,
            ) {
                Ok(manifest) => {
                    let http_client = extractor.create_client_with_cookies(&watch_data);
                    return Ok((manifest, http_client));
                }
                Err(e) => {
                    last_error = e;
                }
            }
        }

        Err(last_error)
    }
}

#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub author: String,
    pub duration: u64,
}
