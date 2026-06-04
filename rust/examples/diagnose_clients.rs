//! Diagnose each YouTube client configuration.
//!
//! Run: cargo run --example diagnose_clients [video-id]
//! Tests each client individually and shows the playability status + format count.

use tunes4r::youtube::client::get_yt_clients;
use tunes4r::youtube::client::YtClient;
use tunes4r::youtube::watch::fetch_watch_page;
use tunes4r::youtube::pot;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let video_id = args.get(1).map(|s| s.as_str()).unwrap_or("dQw4w9WgXcQ");

    let http = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap();

    // Fetch watch page once for visitor data & cookies
    let watch = fetch_watch_page(&http, video_id).unwrap_or_default();
    let po_token = watch.visitor_data.as_ref().map(|vd| pot::generate_cold_start_token(vd));
    println!("Testing {} clients on video: {video_id}\n", get_yt_clients().len());

    for client in get_yt_clients().iter() {
        let status = test_client(&http, &client, video_id, &watch, po_token.as_deref());
        println!("  {:<25} {}", format!("{} v{}", client.name, client.version), status);
    }
}

fn test_client(
    http: &reqwest::blocking::Client,
    client: &YtClient,
    video_id: &str,
    watch: &tunes4r::youtube::watch::WatchData,
    po_token: Option<&str>,
) -> String {
    let client_map = serde_json::json!({
        "clientName": client.name,
        "clientVersion": client.version,
        "hl": "en",
        "gl": "US",
        "timeZone": "UTC",
        "utcOffsetMinutes": 0,
    });

    let mut body = serde_json::json!({
        "context": { "client": client_map },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });

    if let Some(pot) = po_token {
        if !pot.is_empty() {
            body["serviceIntegrityDimensions"] = serde_json::json!({"poToken": pot});
        }
    }

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("Origin", "https://www.youtube.com".parse().unwrap());
    if let Some(vd) = &watch.visitor_data {
        if !vd.is_empty() {
            headers.insert("X-Goog-Visitor-Id", vd.parse().unwrap());
        }
    }

    let resp = match http
        .post(&client.api_url)
        .headers(headers)
        .json(&body)
        .send()
    {
        Ok(r) => r,
        Err(e) => return format!("HTTP error: {e}"),
    };

    let status_code = resp.status();
    let body_text = match resp.text() {
        Ok(t) => t,
        Err(e) => return format!("Body error: {e}"),
    };

    let data: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => return format!("{status_code} JSON error: {e} — preview: {}..", &body_text[..body_text.len().min(100)]),
    };

    let playability = data
        .pointer("/playabilityStatus/status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let formats = data
        .pointer("/streamingData/formats")
        .map(|a| a.as_array().map(|a| a.len()).unwrap_or(0))
        .unwrap_or(0);
    let adaptive = data
        .pointer("/streamingData/adaptiveFormats")
        .map(|a| a.as_array().map(|a| a.len()).unwrap_or(0))
        .unwrap_or(0);

    if playability == "OK" {
        let total = formats + adaptive;
        format!("{status_code} {playability} — {total} formats ({formats}+{adaptive})")
    } else {
        let reason = data
            .pointer("/playabilityStatus/reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        format!("{status_code} {playability} — {reason}")
    }
}
