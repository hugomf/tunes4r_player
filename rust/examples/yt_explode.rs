//! YouTube Explode example
//!
//! Demonstrates the youtube_explode-like API for YouTube extraction.

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" {
        eprintln!("Usage: cargo run --example yt_explode <video-id|search-query>");
        eprintln!();
        eprintln!("YouTube Explode - A Rust library for YouTube video extraction.");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  cargo run --example yt_explode dQw4w9WgXcQ");
        eprintln!("  cargo run --example yt_explode \"never gonna give you up\"");
        return;
    }

    let query = &args[1];
    let is_video_id = query.len() == 11
        && query
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    let yt = tunes4r::youtube::YouTube::new();

    if is_video_id {
        eprintln!("[youtube] Video ID: {}", query);

        match yt.videos().stream(query) {
            Ok(manifest) => {
                if let Some(audio) = manifest.best_audio() {
                    eprintln!(
                        "[youtube] Audio: {} kbps - {}",
                        audio.bitrate / 1000,
                        audio.mime_type
                    );
                    eprintln!("[youtube] URL: {}", audio.url);
                } else {
                    eprintln!("[youtube] No audio streams found");
                }
            }
            Err(e) => eprintln!("[youtube] Error: {}", e),
        }
    } else {
        eprintln!("[youtube] Search: {}", query);
        eprintln!("[youtube] Search not implemented in this example");
    }
}
