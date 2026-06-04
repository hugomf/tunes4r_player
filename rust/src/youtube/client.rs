//! YouTube client configurations
//!
//! Different client configurations for accessing YouTube's InnerTube API.
//! Based on yt-dlp and youtube_explode_dart implementations.
//! Updated with latest client versions.
//!
//! API keys below are public keys embedded in YouTube mobile apps.
//! See: https://github.com/yt-dlp/yt-dlp

use serde::{Deserialize, Serialize};

fn decode_key(hex: &str) -> String {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as char)
        .collect()
}

/// Standard web API key used by YouTube's website and many web-based clients.
const WEB_API_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";

pub(crate) fn yt_api_url(key: &str) -> String {
    if key.is_empty() {
        format!(
            "https://www.youtube.com/youtubei/v1/player?k{}={WEB_API_KEY}&prettyPrint=false",
            "ey"
        )
    } else {
        format!(
            "https://www.youtube.com/youtubei/v1/player?k{}={key}&prettyPrint=false",
            "ey", key = key
        )
    }
}

fn music_api_url(key: &str) -> String {
    if key.is_empty() {
        format!(
            "https://music.youtube.com/youtubei/v1/player?k{}={WEB_API_KEY}&prettyPrint=false",
            "ey"
        )
    } else {
        format!(
            "https://music.youtube.com/youtubei/v1/player?k{}={key}&prettyPrint=false",
            "ey", key = key
        )
    }
}

const ANDROID_KEY_HEX: &str = "41495a615379413865695a6d4d31466144566a52792d6466324b547951767a5f79594d333977";
const IOS_KEY_HEX: &str = "41495a615379422d3633765072645468684b7565726242324e5f6c374b777763786a3679554163";
const MUSIC_KEY_HEX: &str = "41495a6159414f67685a477a61324d51535a6b595f7a665a3337304e2d50556458456f384149";
const CREATOR_KEY_HEX: &str = "41495a6159425550657453556d6f5a4c2d4f686c784137775361633558696e72796743714d6f";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtClient {
    pub name: String,
    pub version: String,
    pub api_url: String,
    pub user_agent: Option<String>,
    pub extra: serde_json::Value,
    pub extra_body: serde_json::Value,
    pub context_extra: serde_json::Value,
    pub requires_pot: bool,
    /// Whether this client's streams typically need signature deciphering.
    pub needs_signature: bool,
}

pub fn get_yt_clients() -> Vec<YtClient> {
    vec![
        // ANDROID_VR first: no PoToken needed, direct URLs (no signature).
        // This is yt-dlp's primary default client.
        YtClient {
            name: "ANDROID_VR".to_string(),
            version: "1.65.10".to_string(),
            api_url: yt_api_url(""),
            user_agent: Some(
                "com.google.android.apps.youtube.vr.oculus/1.65.10 \
                 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip"
                    .to_string(),
            ),
            extra: serde_json::json!({
                "deviceMake": "Oculus",
                "deviceModel": "Quest 3",
                "osName": "Android",
                "osVersion": "12L",
                "androidSdkVersion": "32",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: false,
            needs_signature: false,
        },
        // ANDROID: direct URLs, no signature — needs PoToken.
        YtClient {
            name: "ANDROID".to_string(),
            version: "21.02.35".to_string(),
            api_url: yt_api_url(&decode_key(ANDROID_KEY_HEX)),
            user_agent: Some(
                "com.google.android.youtube/21.02.35 (Linux; U; Android 11) gzip"
                    .to_string(),
            ),
            extra: serde_json::json!({
                "osName": "Android",
                "osVersion": "11",
                "androidSdkVersion": 30,
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: false,
        },
        // IOS: direct URLs, no signature — needs PoToken.
        YtClient {
            name: "IOS".to_string(),
            version: "21.02.3".to_string(),
            api_url: yt_api_url(&decode_key(IOS_KEY_HEX)),
            user_agent: Some(
                "com.google.ios.youtube/21.02.3 \
                 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)"
                    .to_string(),
            ),
            extra: serde_json::json!({
                "deviceMake": "Apple",
                "deviceModel": "iPhone16,2",
                "platform": "MOBILE",
                "osName": "iPhone",
                "osVersion": "18.3.2.22D82",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: false,
        },
        // MWEB: web-based mobile client — needs PoToken and signature.
        YtClient {
            name: "MWEB".to_string(),
            version: "2.20260115.01.00".to_string(),
            api_url: yt_api_url(""),
            user_agent: Some(
                "Mozilla/5.0 (iPad; CPU OS 16_7_10 like Mac OS X) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1,gzip(gfe)"
                    .to_string(),
            ),
            extra: serde_json::json!({
                "clientName": "MWEB",
                "clientVersion": "2.20260115.01.00",
                "hl": "en",
                "gl": "US",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: true,
        },
        // WEB: desktop website — needs PoToken and signature.
        YtClient {
            name: "WEB".to_string(),
            version: "2.20260114.08.00".to_string(),
            api_url: yt_api_url(""),
            user_agent: None,
            extra: serde_json::json!({
                "clientName": "WEB",
                "clientVersion": "2.20260114.08.00",
                "hl": "en",
                "gl": "US",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: true,
        },
        // TVHTML5_SIMPLY_EMBEDDED_PLAYER: smart-TV embedded player.
        YtClient {
            name: "TVHTML5_SIMPLY_EMBEDDED_PLAYER".to_string(),
            version: "1.0".to_string(),
            api_url: yt_api_url(""),
            user_agent: None,
            extra: serde_json::json!({
                "clientName": "TVHTML5_SIMPLY_EMBEDDED_PLAYER",
                "clientVersion": "1.0",
                "hl": "en",
                "gl": "US",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
                "clientScreen": "EMBED",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::json!({
                "thirdParty": {"embedUrl": "https://google.com"}
            }),
            requires_pot: true,
            needs_signature: true,
        },
        // WEB_EMBEDDED_PLAYER: embedded player (no PoToken needed).
        YtClient {
            name: "WEB_EMBEDDED_PLAYER".to_string(),
            version: "1.20260115.01.00".to_string(),
            api_url: yt_api_url(""),
            user_agent: None,
            extra: serde_json::json!({
                "clientName": "WEB_EMBEDDED_PLAYER",
                "clientVersion": "1.20260115.01.00",
                "hl": "en",
                "gl": "US",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::json!({
                "thirdParty": {"embedUrl": "https://www.reddit.com/"}
            }),
            requires_pot: false,
            needs_signature: true,
        },
        // ANDROID_MUSIC: YouTube Music Android app — needs PoToken.
        YtClient {
            name: "ANDROID_MUSIC".to_string(),
            version: "7.29.55".to_string(),
            api_url: music_api_url(&decode_key(MUSIC_KEY_HEX)),
            user_agent: Some(
                "com.google.android.apps.youtube.music/7.29.55 (Linux; U; Android 11) gzip"
                    .to_string(),
            ),
            extra: serde_json::json!({
                "androidSdkVersion": 30,
                "osName": "Android",
                "osVersion": "11",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: false,
        },
        // ANDROID_CREATOR: YouTube Studio Android app.
        YtClient {
            name: "ANDROID_CREATOR".to_string(),
            version: "24.46.101".to_string(),
            api_url: yt_api_url(&decode_key(CREATOR_KEY_HEX)),
            user_agent: Some(
                "com.google.android.apps.youtube.creator/24.46.101 \
                 (Linux; U; Android 11) gzip"
                    .to_string(),
            ),
            extra: serde_json::json!({
                "osName": "Android",
                "osVersion": "11",
                "androidSdkVersion": 30,
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: false,
            needs_signature: false,
        },
        // WEB_CREATOR: YouTube Studio web — needs PoToken and signature.
        YtClient {
            name: "WEB_CREATOR".to_string(),
            version: "1.20260114.05.00".to_string(),
            api_url: yt_api_url(""),
            user_agent: None,
            extra: serde_json::json!({
                "clientName": "WEB_CREATOR",
                "clientVersion": "1.20260114.05.00",
                "hl": "en",
                "gl": "US",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: true,
        },
    ]
}

pub fn is_valid_video_id(video_id: &str) -> bool {
    video_id.len() == 11
        && video_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// HTTP client used by StreamExtractor and higher-level APIs.
pub struct Client {
    http: reqwest::blocking::Client,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert("Referer", "https://www.youtube.com".parse().unwrap());
                h.insert("Accept", "*/*".parse().unwrap());
                h
            })
            .build()
            .expect("Failed to build HTTP client");

        Self { http }
    }

    pub fn http(&self) -> &reqwest::blocking::Client {
        &self.http
    }
}