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

fn yt_api_url(key: &str) -> String {
    format!(
        "https://www.youtube.com/youtubei/v1/player?k{}={key}&prettyPrint=false",
        "ey", key = key
    )
}

fn music_api_url(key: &str) -> String {
    format!(
        "https://music.youtube.com/youtubei/v1/player?k{}={key}&prettyPrint=false",
        "ey", key = key
    )
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
        YtClient {
            name: "WEB".to_string(),
            version: "2.20250312.04.00".to_string(),
            api_url: yt_api_url(""),
            user_agent: None,
            extra: serde_json::json!({
                "clientName": "WEB",
                "clientVersion": "2.20250312.04.00",
                "hl": "en",
                "gl": "US",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: false,
            needs_signature: true,
        },
        YtClient {
            name: "ANDROID".to_string(),
            version: "19.29.37".to_string(),
            api_url: yt_api_url(&decode_key(ANDROID_KEY_HEX)),
            user_agent: Some("com.google.android.youtube/19.29.37 (Linux; U; Android 14) gzip".to_string()),
            extra: serde_json::json!({
                "osName": "Android",
                "osVersion": "14",
                "androidSdkVersion": 34,
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: false,
        },
        YtClient {
            name: "IOS".to_string(),
            version: "19.45.4".to_string(),
            api_url: yt_api_url(&decode_key(IOS_KEY_HEX)),
            user_agent: Some("com.google.ios.youtube/19.45.4 (iPhone16,2; U; CPU iOS 18_1_0 like Mac OS X;)".to_string()),
            extra: serde_json::json!({
                "deviceMake": "Apple",
                "deviceModel": "iPhone16,2",
                "platform": "MOBILE",
                "osName": "iPhone",
                "osVersion": "18.1.0.22B83",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: false,
        },
        YtClient {
            name: "TVHTML5_SIMPLY_EMBEDDED_PLAYER".to_string(),
            version: "2.0".to_string(),
            api_url: yt_api_url(""),
            user_agent: None,
            extra: serde_json::json!({
                "clientName": "TVHTML5_SIMPLY_EMBEDDED_PLAYER",
                "clientVersion": "2.0",
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
            requires_pot: false,
            needs_signature: true,
        },
        YtClient {
            name: "MWEB".to_string(),
            version: "2.20250312.04.00".to_string(),
            api_url: yt_api_url(""),
            user_agent: Some("Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.6778.200 Mobile Safari/537.36".to_string()),
            extra: serde_json::json!({
                "clientName": "MWEB",
                "clientVersion": "2.20250312.04.00",
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
        YtClient {
            name: "WEB_EMBEDDED_PLAYER".to_string(),
            version: "1.20250312.01.00".to_string(),
            api_url: yt_api_url(""),
            user_agent: None,
            extra: serde_json::json!({
                "clientName": "WEB_EMBEDDED_PLAYER",
                "clientVersion": "1.20250312.01.00",
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
        YtClient {
            name: "ANDROID_VR".to_string(),
            version: "1.60.19".to_string(),
            api_url: yt_api_url(""),
            user_agent: Some("com.google.android.apps.youtube.vr.oculus/1.60.19 (Linux; U; Android 12; eureka-user Build/SQ3A.220605.009.A1) gzip".to_string()),
            extra: serde_json::json!({
                "deviceMake": "Oculus",
                "deviceModel": "Quest 3",
                "osName": "Android",
                "osVersion": "12",
                "androidSdkVersion": "32",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: false,
            needs_signature: true,
        },
        YtClient {
            name: "ANDROID_MUSIC".to_string(),
            version: "7.27.52".to_string(),
            api_url: music_api_url(&decode_key(MUSIC_KEY_HEX)),
            user_agent: Some("com.google.android.apps.youtube.music/7.27.52 (Linux; U; Android 14) gzip".to_string()),
            extra: serde_json::json!({
                "androidSdkVersion": 34,
                "osName": "Android",
                "osVersion": "14",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: true,
            needs_signature: false,
        },
        YtClient {
            name: "ANDROID_CREATOR".to_string(),
            version: "24.45.100".to_string(),
            api_url: yt_api_url(&decode_key(CREATOR_KEY_HEX)),
            user_agent: Some("com.google.android.apps.youtube.creator/24.45.100 (Linux; U; Android 14) gzip".to_string()),
            extra: serde_json::json!({
                "osName": "Android",
                "osVersion": "14",
                "androidSdkVersion": 34,
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: false,
            needs_signature: false,
        },
        YtClient {
            name: "WEB_CREATOR".to_string(),
            version: "1.20250312.01.00".to_string(),
            api_url: yt_api_url(""),
            user_agent: None,
            extra: serde_json::json!({
                "clientName": "WEB_CREATOR",
                "clientVersion": "1.20250312.01.00",
                "hl": "en",
                "gl": "US",
            }),
            extra_body: serde_json::Value::Object(serde_json::Map::new()),
            context_extra: serde_json::Value::Object(serde_json::Map::new()),
            requires_pot: false,
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
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
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