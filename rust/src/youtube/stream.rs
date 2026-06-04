//! YouTube stream extraction
//!
//! Extracts audio stream URLs from YouTube videos using multiple strategies.
//! Implements full authentication: visitor data, signature deciphering,
//! n-parameter throttle transforms, and proper InnerTube API requests.
//! Mirrors the youtube_explode_dart and yt-dlp approach.

use crate::youtube::client::{get_yt_clients, is_valid_video_id};
use crate::youtube::js_engine;
use crate::youtube::watch::{self, WatchData};
use crate::youtube::YouTubeService;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
struct StreamResponse {
    playability_status: Option<PlayabilityStatus>,
    streaming_data: Option<StreamingData>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlayabilityStatus {
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StreamingData {
    adaptive_formats: Option<Vec<Format>>,
    formats: Option<Vec<Format>>,
}

#[derive(Debug, Clone, Deserialize)]
struct Format {
    itag: Option<i64>,
    mime_type: Option<String>,
    bitrate: Option<u64>,
    /// Direct URL (available when no signature deciphering is needed).
    url: Option<String>,
    /// Legacy cipher field.
    cipher: Option<String>,
    /// Signature cipher (URL-encoded JSON with `s`, `sp`, `url` fields).
    /// When present, the stream URL must be deciphered.
    signature_cipher: Option<String>,
    /// Content length in bytes.
    #[allow(dead_code)]
    content_length: Option<u64>,
    /// Audio sample rate.
    #[allow(dead_code)]
    audio_sample_rate: Option<String>,
    /// Approximate duration from streaming data (string or number)
    #[serde(rename = "approxDurationMs", deserialize_with = "crate::youtube::extractor::deserialize_f64_from_string")]
    #[allow(dead_code)]
    approx_duration_ms: Option<f64>,
}

impl Format {
    fn is_audio(&self) -> bool {
        self.mime_type
            .as_ref()
            .map(|m| m.starts_with("audio/"))
            .unwrap_or(false)
    }

    /// Extract the stream URL, deciphering if necessary.
    /// Returns (url, requires_signature_deciphering).
    fn extract_url(&self) -> Option<(String, bool)> {
        // Direct URL (no deciphering needed)
        if let Some(ref url) = self.url {
            if !url.is_empty() {
                return Some((url.clone(), false));
            }
        }

        // Signature cipher (requires deciphering)
        if let Some(ref sc) = self.signature_cipher {
            if !sc.is_empty() {
                if let Ok(parsed) = parse_signature_cipher(sc) {
                    return Some(parsed);
                }
            }
        }

        // Legacy cipher field
        if let Some(ref cipher) = self.cipher {
            if !cipher.is_empty() {
                if let Ok(parsed) = parse_signature_cipher(cipher) {
                    return Some(parsed);
                }
            }
        }

        None
    }
}

/// Parse a signatureCipher/cipher string into (url, needs_deciphering).
/// The cipher is a URL-encoded query string with fields: url, sp, s.
fn parse_signature_cipher(cipher: &str) -> Result<(String, bool), ()> {
    let params: HashMap<&str, &str> = cipher
        .split('&')
        .filter_map(|part| {
            let mut kv = part.splitn(2, '=');
            Some((kv.next()?, kv.next()?))
        })
        .collect();

    let base_url = urlencoding::decode(params.get("url").unwrap_or(&""))
        .map(|s| s.to_string())
        .map_err(|_| ())?;

    if params.contains_key("s") {
        // Signature needs to be deciphered
        Ok((base_url, true))
    } else {
        Ok((base_url, false))
    }
}

/// Parse signatureCipher parameters from a URL-encoded string.
fn parse_cipher_params(cipher: &str) -> HashMap<String, String> {
    cipher
        .split('&')
        .filter_map(|part| {
            let mut kv = part.splitn(2, '=');
            let key = kv.next()?.to_string();
            let val = kv.next().unwrap_or("").to_string();
            Some((key, val))
        })
        .collect()
}

/// Build an InnerTube player request for a given client.
fn build_client_request(
    client: &crate::youtube::client::YtClient,
    video_id: &str,
    watch_data: &WatchData,
    po_token: Option<&str>,
) -> Result<reqwest::blocking::RequestBuilder, String> {
    let mut client_map = serde_json::json!({
        "clientName": client.name,
        "clientVersion": client.version,
        "hl": "en",
        "gl": "US",
        "timeZone": "UTC",
        "utcOffsetMinutes": 0,
    });
    if let Some(obj) = client_map.as_object_mut() {
        if let Some(extra) = client.extra.as_object() {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
        // yt-dlp includes userAgent in the API request body client context
        if let Some(ua) = &client.user_agent {
            obj.insert("userAgent".to_string(), serde_json::Value::String(ua.clone()));
        }
    }

    let mut context = serde_json::json!({
        "client": client_map,
    });
    if let Some(extra) = client.context_extra.as_object() {
        if let Some(obj) = context.as_object_mut() {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
    }

    let mut body = serde_json::json!({
        "context": context,
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    if let Some(extra) = client.extra_body.as_object() {
        if let Some(obj) = body.as_object_mut() {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
    }

    // Include PoToken in request body like yt-dlp:
    // yt_query['serviceIntegrityDimensions'] = {'poToken': po_token}
    if let Some(pot) = po_token {
        if !pot.is_empty() {
            body["serviceIntegrityDimensions"] = serde_json::json!({
                "poToken": pot
            });
        }
    }

    // ── Build headers ──
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

    // User-Agent: use client-specific UA or a default Chrome UA
    if let Some(ua) = &client.user_agent {
        headers.insert(reqwest::header::USER_AGENT, ua.parse().unwrap());
    } else {
        headers.insert(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
                .parse()
                .unwrap(),
        );
    }

    // Visitor data
    if let Some(vd) = &watch_data.visitor_data {
        if !vd.is_empty() {
            headers.insert("X-Goog-Visitor-Id", vd.parse().unwrap());
        }
    }

    // Cookies
    if !watch_data.cookies.is_empty() {
        let cookie_str = watch_data.cookies.join("; ");
        headers.insert(
            reqwest::header::COOKIE,
            cookie_str.parse().unwrap(),
        );
    }

    let request = reqwest::blocking::Client::new()
        .post(client.api_url.clone())
        .headers(headers)
        .json(&body);

    Ok(request)
}

/// Get the best audio stream URL for a video.
///
/// This is the main entry point. It:
/// 1. Fetches the watch page to get visitor data and cookies
/// 2. Iterates through InnerTube clients
/// 3. For each client, makes a player API request
/// 4. Deciphers signatures and n-parameters as needed
/// 5. Returns the best audio stream URL
pub fn get_audio_stream_url(
    service: &mut YouTubeService,
    video_id: &str,
) -> Result<String, String> {
    if !is_valid_video_id(video_id) {
        return Err("Invalid video ID format".to_string());
    }

    if service.is_blocked(video_id) {
        return Err("Video is known to be blocked".to_string());
    }

    if let Some(cached) = service.get_cached_url(video_id) {
        return Ok(cached);
    }

    // ── Step 1: Fetch the watch page ──
    eprintln!("[youtube] Fetching watch page for: {}", video_id);
    let watch_data = watch::fetch_watch_page(service.http_client(), video_id)?;

    // Update service with visitor data if we got it
    if let Some(ref vd) = watch_data.visitor_data {
        service.set_visitor_data(vd.clone());
        eprintln!("[youtube] Got visitor data: {}...", &vd[..vd.len().min(20)]);
    }
    eprintln!(
        "[youtube] Got {} cookies from watch page",
        watch_data.cookies.len()
    );

    // ── Step 2: Fetch and parse player JS for signature/throttle ──
    let mut player_js_code: Option<String> = None;
    let mut signature_transforms: Option<Vec<String>> = None;

    if let Some(ref player_js_url) = watch_data.player_js_url {
        eprintln!("[youtube] Player JS URL: {}", player_js_url);
        match watch::fetch_player_js(service.http_client(), player_js_url) {
            Ok(js_code) => {
                // Extract signature transforms
                let transforms = watch::extract_signature_transforms(&js_code);
                if !transforms.is_empty() {
                    eprintln!(
                        "[youtube] Extracted {} signature transforms",
                        transforms.len()
                    );
                    signature_transforms = Some(transforms);
                }
                player_js_code = Some(js_code);
            }
            Err(e) => {
                eprintln!("[youtube] Failed to fetch player JS: {}", e);
            }
        }
    }

    // ── Step 3: Try each InnerTube client ──
    let po_token = service.po_token().map(|s| s.as_str());
    for yt_client in get_yt_clients().iter() {
        let request = build_client_request(yt_client, video_id, &watch_data, po_token)
            .map_err(|e| format!("Failed to build request: {}", e))?;

        let response = match request.send() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[youtube] Client {} request failed: {}", yt_client.name, e);
                continue;
            }
        };

        if !response.status().is_success() {
            eprintln!(
                "[youtube] Client {} returned HTTP {}",
                yt_client.name,
                response.status()
            );
            continue;
        }

        let data: Result<StreamResponse, _> = response.json();
        let data = match data {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[youtube] Failed to parse response: {}", e);
                continue;
            }
        };

        if let Some(ref status) = data.playability_status {
            if status.status != "OK" {
                eprintln!(
                    "[youtube] Client {}: playabilityStatus={}",
                    yt_client.name, status.status
                );
                continue;
            }
        }

        if let Some(ref streaming) = data.streaming_data {
            let mut formats = streaming.adaptive_formats.clone().unwrap_or_default();
            formats.extend(streaming.formats.clone().unwrap_or_default());

            let mut audio_formats: Vec<_> = formats
                .into_iter()
                .filter(|f| f.is_audio())
                .filter(|f| f.extract_url().is_some())
                .collect();

            // Sort by bitrate (highest first)
            audio_formats.sort_by(|a, b| b.bitrate.cmp(&a.bitrate));

            for format in &audio_formats {
                if let Some((url, needs_decipher)) = format.extract_url() {
                    let final_url = if needs_decipher {
                        match decipher_stream_url(
                            &url,
                            format,
                            &player_js_code,
                            &signature_transforms,
                        ) {
                            Ok(u) => u,
                            Err(e) => {
                                eprintln!(
                                    "[youtube] Decipher failed for itag {:?}: {}",
                                    format.itag, e
                                );
                                continue;
                            }
                        }
                    } else {
                        url
                    };

                    // Apply n-parameter throttle transform
                    let final_url = apply_throttle_transform(
                        &final_url,
                        &player_js_code,
                    );

                    // Append ?pot=PO_TOKEN like yt-dlp does
                    let final_url = if let Some(pot) = po_token {
                        if !pot.is_empty() {
                            let sep = if final_url.contains('?') { '&' } else { '?' };
                            format!("{}{}pot={}", final_url, sep, pot)
                        } else {
                            final_url
                        }
                    } else {
                        final_url
                    };

                    eprintln!(
                        "[youtube] Client {} got stream: itag={:?}, mime={:?}, bitrate={:?}",
                        yt_client.name, format.itag, format.mime_type, format.bitrate
                    );

                    service.cache_url(
                        video_id.to_string(),
                        final_url.clone(),
                        yt_client.requires_pot,
                    );
                    return Ok(final_url);
                }
            }
        }
    }

    Err("All extraction strategies failed".to_string())
}

/// Decipher a stream URL that requires signature transformation.
fn decipher_stream_url(
    url: &str,
    format: &Format,
    player_js_code: &Option<String>,
    signature_transforms: &Option<Vec<String>>,
) -> Result<String, String> {
    // Parse the signature cipher parameters
    let cipher_params = if let Some(ref sc) = format.signature_cipher {
        parse_cipher_params(sc)
    } else if let Some(ref cipher) = format.cipher {
        parse_cipher_params(cipher)
    } else {
        // Try to extract from the URL itself (url might contain &s= parameter)
        let params: HashMap<String, String> = url
            .split('&')
            .filter_map(|part| {
                let mut kv = part.splitn(2, '=');
                Some((kv.next()?.to_string(), kv.next().unwrap_or("").to_string()))
            })
            .collect();
        params
    };

    // Get the base URL (from cipher params or the URL itself)
    let base_url = if let Some(encoded_url) = cipher_params.get("url") {
        urlencoding::decode(encoded_url)
            .map(|s| s.to_string())
            .unwrap_or_else(|_| encoded_url.clone())
    } else {
        // URL is already the base URL
        url.to_string()
    };

    // Get the signature
    let signature = cipher_params.get("s").cloned().or_else(|| {
        // Try &s= in URL
        url.split("&s=").nth(1).and_then(|s| {
            s.split('&').next().map(|v| v.to_string())
        })
    });

    let sp = cipher_params.get("sp").cloned().unwrap_or_else(|| "sig".to_string());

    if let Some(sig) = signature {
        if !sig.is_empty() {
            // Decipher the signature
            let deciphered_sig = if let Some(ref transforms) = signature_transforms {
                watch::apply_signature_transforms(&sig, transforms)
            } else if let Some(ref js_code) = player_js_code {
                js_engine::decipher_signature(js_code, &sig)
            } else {
                js_engine::decipher_signature("", &sig)
            };

            // Build the final URL
            let separator = if base_url.contains('?') { '&' } else { '?' };
            return Ok(format!(
                "{}{}{}={}",
                base_url, separator, sp, deciphered_sig
            ));
        }
    }

    // No signature needed, return base URL
    Ok(base_url)
}

/// Apply the n-parameter throttle transform to a URL.
fn apply_throttle_transform(url: &str, player_js_code: &Option<String>) -> String {
    // Extract the n parameter from the URL
    let n_value = match extract_n_parameter(url) {
        Some(v) => v,
        None => return url.to_string(),
    };

    let deciphered = if let Some(ref js_code) = player_js_code {
        js_engine::decipher_throttle(js_code, &n_value)
    } else {
        return url.to_string();
    };

    if deciphered == n_value {
        // No transformation happened
        return url.to_string();
    }

    // Replace the n parameter in the URL
    replace_url_parameter(url, "n", &deciphered)
}

/// Extract the `n` parameter value from a URL.
fn extract_n_parameter(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    for (key, value) in parsed.query_pairs() {
        if key == "n" {
            return Some(value.to_string());
        }
    }
    None
}

/// Replace a query parameter in a URL.
fn replace_url_parameter(url: &str, param: &str, new_value: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(url) {
        let mut pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        for &mut (ref mut k, ref mut v) in &mut pairs {
            if k == param {
                *v = new_value.to_string();
            }
        }

        // Rebuild query string
        let new_query: String = pairs
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        parsed.set_query(Some(&new_query));
        parsed.to_string()
    } else {
        // Fallback: regex replace
        let re = Regex::new(&format!(
            r"([?&]{}=)[^&]*",
            regex::escape(param)
        ))
        .unwrap();

        if re.is_match(url) {
            re.replace(url, format!("$1{}", new_value))
                .to_string()
        } else {
            let separator = if url.contains('?') { '&' } else { '?' };
            format!("{}{}{}={}", url, separator, param, new_value)
        }
    }
}

pub fn refresh_audio_stream_url(
    service: &mut YouTubeService,
    video_id: &str,
) -> Result<String, String> {
    service.clear_cache(video_id);
    get_audio_stream_url(service, video_id)
}