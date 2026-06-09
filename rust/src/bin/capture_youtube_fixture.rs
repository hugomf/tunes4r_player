//! One-time tool to capture a real YouTube CDN stream for use in integration tests.
//!
//! Usage:
//!   cargo run --bin capture_youtube_fixture -- <youtube-video-id> [output-dir]
//!
//! Downloads audio from the YouTube video, saves the raw bytes to
//! `fixtures/youtube_stream.bin` and metadata to `fixtures/youtube_stream.json`.
//!
//! Subsequent test runs use the fixture instead of hitting YouTube's CDN.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use tunes4r::youtube::YouTube;

const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024; // 2 MB

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <youtube-video-id> [output-dir]", args[0]);
        std::process::exit(1);
    }

    let video_id = &args[1];
    let output_dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures");
        p
    });
    fs::create_dir_all(&output_dir).expect("create fixtures dir");

    eprintln!("[capture] Resolving audio stream for video: {}", video_id);

    let yt = YouTube::new();
    let (manifest, _http_client) = yt
        .videos()
        .stream_with_client(video_id)
        .expect("failed to get YouTube stream manifest");

    let audio = manifest
        .best_audio()
        .expect("no audio streams in manifest");
    let audio_url = &audio.url;

    eprintln!(
        "[capture] Stream: {} kbps — {}",
        audio.bitrate / 1000,
        audio.mime_type
    );
    eprintln!(
        "[capture] CDN URL: {}...",
        &audio_url[..audio_url.len().min(120)]
    );
    eprintln!("[capture] Downloading up to {} bytes...", MAX_DOWNLOAD_BYTES);

    let dl_client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("build download client");

    let resp = dl_client
        .get(audio_url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .expect("CDN GET failed");

    let total = resp.content_length().unwrap_or(0);
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mp4")
        .to_string();

    let mut body = Vec::new();
    let mut stream = resp.take(MAX_DOWNLOAD_BYTES);
    stream
        .read_to_end(&mut body)
        .expect("read CDN response");

    // Write metadata
    let meta = serde_json::json!({
        "video_id": video_id,
        "content_type": content_type,
        "content_length": total,
        "captured_bytes": body.len(),
        "original_url": audio_url,
    });
    let meta_path = output_dir.join("youtube_stream.json");
    fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap())
        .expect("write metadata");
    eprintln!("[capture] Metadata written: {}", meta_path.display());

    // Write raw bytes
    let bin_path = output_dir.join("youtube_stream.bin");
    fs::write(&bin_path, &body).expect("write stream data");
    eprintln!(
        "[capture] Stream data written: {} ({} bytes)",
        bin_path.display(),
        body.len()
    );

    eprintln!("[capture] Done. You can now run the fixture-based tests.");
}
