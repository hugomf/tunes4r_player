//! YouTube service example
//!
//! Demonstrates the YouTube service API.
//!
//! Note: Actual stream extraction may fail due to:
//! - YouTube's anti-bot measures
//! - Expired client configurations  
//! - Missing JavaScript signature deciphering
//!
//! To fix: Update client configs in src/youtube/client.rs and implement
//! proper JS signature deciphering in src/youtube/js.rs

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" {
        eprintln!("Usage: cargo run --example yt_service <video-id|search-query>");
        eprintln!();
        eprintln!("YouTube service example.");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  cargo run --example yt_service dQw4w9WgXcQ");
        eprintln!("  cargo run --example yt_service \"never gonna give you up\"");
        eprintln!();
        eprintln!("API:");
        eprintln!("  tunes4r::YouTubeService - Service instance");
        eprintln!("  tunes4r::search_videos(&client, query, limit) - Search");
        eprintln!("  tunes4r::get_audio_stream_url(&mut service, video_id) - Get stream");
        eprintln!("  tunes4r::get_video_info(&client, video_id) - Get metadata");
        return;
    }

    let query = &args[1];
    let is_video_id = query.len() == 11
        && query
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    let http_client = reqwest::blocking::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/120.0.0.0 Safari/537.36",
        )
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("Referer", "https://www.youtube.com".parse().unwrap());
            h.insert("Accept", "*/*".parse().unwrap());
            h.insert("Accept-Encoding", "identity".parse().unwrap());
            h
        })
        .build()
        .expect("Failed to build HTTP client");

    let service = tunes4r::YouTubeService::new();

    if is_video_id {
        eprintln!("[youtube] Video ID: {}", query);
        match tunes4r::get_audio_stream_url(&mut service.clone(), query) {
            Ok(url) => {
                let len = url.len();
                eprintln!("[youtube] Stream URL: {} chars", len);
                eprintln!("[youtube] URL: {}", url);
            }
            Err(e) => eprintln!("[youtube] Error: {}", e),
        }
    } else {
        eprintln!("[youtube] Search: {}", query);
        match tunes4r::search_videos(&http_client, query, 5) {
            Ok(results) => {
                if results.is_empty() {
                    eprintln!("[youtube] No results");
                    return;
                }
                let count = results.len();
                eprintln!("[youtube] Found {} results:", count);
                for (i, result) in results.iter().enumerate() {
                    eprintln!("  [{}] {} ({})", i + 1, result.title, result.author);
                }
                if let Some(first) = results.first() {
                    eprintln!("[youtube] Getting stream for: {}", first.id);
                    match tunes4r::get_audio_stream_url(&mut service.clone(), &first.id) {
                        Ok(url) => eprintln!("[youtube] Stream: {}", url),
                        Err(e) => eprintln!("[youtube] Error: {}", e),
                    }
                }
            }
            Err(e) => eprintln!("[youtube] Search error: {}", e),
        }
    }
}
