use crate::youtube::client::Client;
use crate::youtube::client::get_yt_clients;
use crate::youtube::formats::{AudioQuality, StreamFormat, VideoQuality};
use crate::youtube::manifest::StreamManifest;
use serde::Deserialize;
use serde::Deserializer;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
struct VideoDetails {
    #[serde(rename = "lengthSeconds")]
    length_seconds: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlayerResponse {
    #[serde(rename = "playabilityStatus")]
    playability_status: Option<PlayabilityStatus>,
    #[serde(rename = "streamingData")]
    streaming_data: Option<StreamingData>,
    #[serde(rename = "videoDetails")]
    video_details: Option<VideoDetails>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlayabilityStatus {
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StreamingData {
    #[serde(rename = "formats")]
    formats: Vec<Format>,
    #[serde(rename = "adaptiveFormats")]
    adaptive_formats: Vec<Format>,
}

#[derive(Debug, Clone, Deserialize)]
struct Format {
    itag: i64,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    bitrate: Option<i64>,
    url: Option<String>,
    #[serde(rename = "signatureCipher")]
    signature_cipher: Option<String>,
    cipher: Option<String>,
    #[serde(rename = "approxDurationMs", deserialize_with = "deserialize_f64_from_string")]
    approx_duration_ms: Option<f64>,
}

pub(crate) fn deserialize_f64_from_string<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de;
    match serde_json::Value::deserialize(deserializer) {
        Ok(serde_json::Value::Number(n)) => n.as_f64().map(Some).ok_or_else(|| de::Error::custom("expected number")),
        Ok(serde_json::Value::String(s)) => s.parse::<f64>().map(Some).map_err(de::Error::custom),
        Ok(serde_json::Value::Null) => Ok(None),
        _ => Ok(None),
    }
}

impl Format {
    fn to_stream_format(
        &self,
        player_js_code: &Option<String>,
        signature_transforms: &Option<Vec<String>>,
        po_token: Option<&str>,
    ) -> StreamFormat {
        let mime = self.mime_type.as_deref().unwrap_or("");
        let mut url = self
            .url
            .clone()
            .or_else(|| {
                self.signature_cipher
                    .as_ref()
                    .or(self.cipher.as_ref())
                    .and_then(|sc| {
                        decipher_signature_cipher(
                            sc,
                            player_js_code,
                            signature_transforms,
                        )
                    })
            })
            .unwrap_or_default();

        // Append ?pot=PO_TOKEN like yt-dlp does
        if let Some(pot) = po_token {
            if !url.is_empty() && !pot.is_empty() {
                let separator = if url.contains('?') { '&' } else { '?' };
                url = format!("{}{}pot={}", url, separator, pot);
            }
        }

        StreamFormat {
            itag: self.itag,
            mime_type: mime.to_string(),
            bitrate: self.bitrate.unwrap_or(0),
            quality: VideoQuality::from_itag(self.itag).unwrap_or(VideoQuality::Quality360),
            audio_quality: AudioQuality::from_bitrate(self.bitrate.unwrap_or(0)),
            url,
            approx_duration_ms: self.approx_duration_ms.map(|d| d as u64),
        }
    }
}

fn decipher_signature_cipher(
    cipher: &str,
    player_js_code: &Option<String>,
    signature_transforms: &Option<Vec<String>>,
) -> Option<String> {
    let params: std::collections::HashMap<&str, &str> = cipher
        .split('&')
        .filter_map(|part| {
            let mut kv = part.splitn(2, '=');
            Some((kv.next()?, kv.next()?))
        })
        .collect();

    let base_url = urlencoding::decode(params.get("url").unwrap_or(&""))
        .ok()?
        .to_string();

    if let Some(sig) = params.get("s") {
        let sp = params.get("sp").unwrap_or(&"sig");
        let deciphered = if let Some(transforms) = signature_transforms {
            crate::youtube::watch::apply_signature_transforms(sig, transforms)
        } else if let Some(js_code) = player_js_code {
            crate::youtube::js_engine::decipher_signature(js_code, sig)
        } else {
            crate::youtube::js_engine::decipher_signature("", sig)
        };
        Some(format!("{}&{}={}", base_url, sp, deciphered))
    } else {
        Some(base_url)
    }
}

pub struct StreamExtractor {
    client: Arc<Client>,
    po_token: Option<String>,
}

impl StreamExtractor {
    pub fn new(client: Arc<Client>) -> Self {
        Self { client, po_token: None }
    }

    pub fn with_po_token(mut self, po_token: String) -> Self {
        self.po_token = Some(po_token);
        self
    }

    pub fn set_po_token(&mut self, po_token: Option<String>) {
        self.po_token = po_token;
    }

    pub fn extract(&self, video_id: &str) -> Result<StreamManifest, String> {
        self.extract_with_po_token(video_id, self.po_token.as_deref())
    }

    pub fn extract_with_po_token(&self, video_id: &str, po_token: Option<&str>) -> Result<StreamManifest, String> {
        let watch_data = self.fetch_watch_page(video_id)?;

        // If no explicit PoToken is configured, auto-generate a cold-start
        // placeholder from the visitor_data.  This avoids falling back to
        // unauthenticated requests that YouTube may reject or throttle.
        let auto_token = po_token.is_none().then(|| {
            watch_data.visitor_data.as_ref().map(|vd| {
                crate::youtube::pot::generate_cold_start_token(vd)
            })
        }).flatten();
        let po_token = po_token.or(auto_token.as_deref());

        let mut player_js_code: Option<String> = None;
        let mut signature_transforms: Option<Vec<String>> = None;

        if let Some(ref player_js_url) = watch_data.player_js_url {
            match crate::youtube::watch::fetch_player_js(self.client.http(), player_js_url) {
                Ok(js_code) => {
                    let transforms =
                        crate::youtube::watch::extract_signature_transforms(&js_code);
                    if !transforms.is_empty() {
                        signature_transforms = Some(transforms);
                    }
                    player_js_code = Some(js_code);
                }
                Err(e) => {
                    eprintln!("[youtube] Failed to fetch player JS: {}", e);
                }
            }
        }

        for yt_client in get_yt_clients().iter() {
            match self.extract_with_client_internal(
                video_id,
                yt_client,
                &watch_data,
                &player_js_code,
                &signature_transforms,
                po_token,
            ) {
                Ok(manifest) => {
                    if !manifest.audio.is_empty() || !manifest.video.is_empty() {
                        return Ok(manifest);
                    }
                }
                Err(e) => {
                    eprintln!("[youtube] Client {} failed: {}", yt_client.name, e);
                }
            }
        }

        Err("All extraction strategies failed".to_string())
    }

    pub fn fetch_watch_page(&self, video_id: &str) -> Result<WatchData, String> {
        crate::youtube::watch::fetch_watch_page(self.client.http(), video_id)
            .map(|wd| WatchData {
                cookies: wd.cookies,
                visitor_data: wd.visitor_data,
                player_js_url: wd.player_js_url,
            })
    }

    pub fn extract_with_client(
        &self,
        video_id: &str,
        client: &crate::youtube::client::YtClient,
        watch_data: &WatchData,
        player_js_code: &Option<String>,
        signature_transforms: &Option<Vec<String>>,
    ) -> Result<StreamManifest, String> {
        self.extract_with_client_internal(
            video_id,
            client,
            watch_data,
            player_js_code,
            signature_transforms,
            None,
        )
    }

    pub fn extract_with_client_po_token(
        &self,
        video_id: &str,
        client: &crate::youtube::client::YtClient,
        watch_data: &WatchData,
        player_js_code: &Option<String>,
        signature_transforms: &Option<Vec<String>>,
        po_token: Option<&str>,
    ) -> Result<StreamManifest, String> {
        self.extract_with_client_internal(
            video_id,
            client,
            watch_data,
            player_js_code,
            signature_transforms,
            po_token,
        )
    }

    pub fn extract_with_client_internal(
        &self,
        video_id: &str,
        client: &crate::youtube::client::YtClient,
        watch_data: &WatchData,
        player_js_code: &Option<String>,
        signature_transforms: &Option<Vec<String>>,
        po_token: Option<&str>,
    ) -> Result<StreamManifest, String> {
        let mut client_map = serde_json::json!({
            "clientName": client.name,
            "clientVersion": client.version,
            "hl": "en",
            "gl": "US",
            "timeZone": "UTC",
            "utcOffsetMinutes": 0,
        });

        // Merge client extra fields (osName, osVersion, device info, etc.)
        if let Some(obj) = client_map.as_object_mut() {
            if let Some(extra) = client.extra.as_object() {
                for (k, v) in extra {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }

        // yt-dlp includes userAgent in the API request body client context
        // for all clients that define one. YouTube may check this.
        if let Some(ua) = &client.user_agent {
            if let Some(obj) = client_map.as_object_mut() {
                obj.insert("userAgent".to_string(), serde_json::Value::String(ua.clone()));
            }
        }

        let mut body = serde_json::json!({
            "context": {
                "client": client_map,
            },
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true,
        });

        // Include PoToken in request body like yt-dlp:
        // yt_query['serviceIntegrityDimensions'] = {'poToken': po_token}
        if let Some(pot) = po_token {
            if !pot.is_empty() {
                body["serviceIntegrityDimensions"] = serde_json::json!({
                    "poToken": pot
                });
            }
        }

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        headers.insert(
            reqwest::header::ORIGIN,
            "https://www.youtube.com".parse().unwrap(),
        );
        headers.insert(
            reqwest::header::REFERER,
            "https://www.youtube.com/".parse().unwrap(),
        );

        if let Some(ua) = &client.user_agent {
            headers.insert(reqwest::header::USER_AGENT, ua.parse().unwrap());
        } else {
            headers.insert(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".parse().unwrap(),
            );
        }

        if let Some(vd) = &watch_data.visitor_data {
            if !vd.is_empty() {
                headers.insert("X-Goog-Visitor-Id", vd.parse().unwrap());
            }
        }

        if !watch_data.cookies.is_empty() {
            let cookie_str = watch_data.cookies.join("; ");
            headers.insert("Cookie", cookie_str.parse().unwrap());
        }

        let response = self
            .client
            .http()
            .post(&client.api_url)
            .headers(headers)
            .json(&body)
            .send()
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let data: PlayerResponse = response.json().map_err(|e| e.to_string())?;

        if let Some(status) = &data.playability_status {
            eprintln!("[youtube] Playability: {}", status.status);
        }

        let streaming = match data.streaming_data {
            Some(s) => s,
            None => {
                eprintln!("[youtube] No streaming_data in response");
                return Err("No streaming data".to_string());
            }
        };

        // Extract duration from videoDetails
        let duration_seconds = data
            .video_details
            .as_ref()
            .and_then(|vd| vd.length_seconds.as_ref())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let mut audio = Vec::new();
        let mut video = Vec::new();

        for fmt in streaming
            .formats
            .iter()
            .chain(streaming.adaptive_formats.iter())
        {
            let sf = fmt.to_stream_format(player_js_code, signature_transforms, po_token);
            if sf.is_audio() {
                audio.push(sf);
            } else if sf.is_video() {
                video.push(sf);
            }
        }

        Ok(StreamManifest { audio, video, duration_seconds })
    }

    pub fn create_client_with_cookies(&self, watch_data: &WatchData) -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .http1_only()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(reqwest::header::REFERER, "https://www.youtube.com".parse().unwrap());
                if let Some(vd) = &watch_data.visitor_data {
                    headers.insert("X-Goog-Visitor-Id", vd.parse().unwrap());
                }
                for cookie in &watch_data.cookies {
                    headers.append("Cookie", cookie.parse().unwrap());
                }
                headers
            })
            .build()
            .expect("Failed to build HTTP client")
    }
}

#[derive(Debug, Clone)]
pub struct WatchData {
    pub cookies: Vec<String>,
    pub visitor_data: Option<String>,
    pub player_js_url: Option<String>,
}
