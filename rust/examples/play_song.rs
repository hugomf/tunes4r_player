//! Play a song from YouTube
//!
//! Searches for a song, extracts the stream URL, and plays it.
//!
//! Usage: cargo run --example play_song "adele rolling in the deep"

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" {
        eprintln!("Usage: cargo run --example play_song <search-query>");
        eprintln!();
        eprintln!("Plays audio from YouTube videos.");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  cargo run --example play_song \"adele rolling in the deep\"");
        return;
    }

    let query = &args[1];
    let yt = tunes4r::youtube::YouTube::new();

    eprintln!("[search] Searching for: {}", query);

    let client = yt.client().http();
    let results = match tunes4r::youtube::search::search(client, query, 5) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[search] Error: {}", e);
            return;
        }
    };

    if results.is_empty() {
        eprintln!("[search] No results found");
        return;
    }

    eprintln!("[search] Found {} results", results.len());
    for (i, result) in results.iter().enumerate() {
        eprintln!("  [{}] {} - {}", i + 1, result.id, result.title);
    }

    let selected = &results[0];
    eprintln!();
    eprintln!("[youtube] Selected: {} - {}", selected.id, selected.title);

    // Get the stream URL via youtube_explode
    let (manifest, _http_client) = match yt.videos().stream_with_client(&selected.id) {
        Ok((m, c)) => (m, c),
        Err(e) => {
            eprintln!("[youtube] Error getting stream: {}", e);
            return;
        }
    };

    let audio = match manifest.best_audio() {
        Some(a) => a,
        None => {
            eprintln!("[youtube] No audio streams found");
            return;
        }
    };

    eprintln!(
        "[youtube] Audio: {} kbps - {}",
        audio.bitrate / 1000,
        audio.mime_type
    );

    // Play directly from the URL (streaming)
    eprintln!("[player] Starting streaming playback...");
    let mut engine = tunes4r::create_playback_engine();

    match tunes4r::play_stream(&mut engine, audio.url.clone()) {
        Ok(()) => {
            eprintln!("[player] Playing! Press Ctrl+C to stop.");
            loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if !tunes4r::is_playing(&mut engine) {
                    break;
                }
            }
            eprintln!("[player] Playback finished.");
        }
        Err(e) => {
            eprintln!("[player] Stream playback failed: {}", e);
            eprintln!("[player] Falling back to download mode...");

            // Download to temp file and play
            let temp_path = std::env::temp_dir().join("tunes4r_play_song.webm");
            let temp_path_str = temp_path.to_string_lossy().to_string();

            eprintln!("[player] Downloading audio...");
            let download_client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("Failed to build download client");

            let response = match download_client
                .get(&audio.url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
                .header("Referer", "https://www.youtube.com/")
                .header("Origin", "https://www.youtube.com")
                .send()
            {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[player] Download error: {}", e);
                    return;
                }
            };

            if !response.status().is_success() {
                eprintln!("[player] HTTP error: {}", response.status());
                return;
            }

            use std::io::{Read, Write};
            let mut stream = response;
            let mut file = match std::fs::File::create(&temp_path) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("[player] Create file error: {}", e);
                    return;
                }
            };

            let mut total: usize = 0;
            let mut buf = [0u8; 65536];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        let _ = file.write_all(&buf[..n]);
                        if total % (256 * 1024) < 65536 {
                            eprintln!("[player] Downloaded: {} KB", total / 1024);
                        }
                    }
                    Err(e) => {
                        eprintln!("[player] Read error: {}", e);
                        return;
                    }
                }
            }
            drop(file);
            eprintln!("[player] Downloaded {} bytes", total);

            let mut engine2 = tunes4r::create_playback_engine();
            match tunes4r::play_file(&mut engine2, temp_path_str.clone()) {
                Ok(()) => {
                    eprintln!("[player] Playing! Press Ctrl+C to stop.");
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        if !tunes4r::is_playing(&mut engine2) {
                            break;
                        }
                    }
                    eprintln!("[player] Playback finished.");
                }
                Err(e) => {
                    eprintln!("[player] File playback error: {}", e);
                }
            }

            let _ = std::fs::remove_file(&temp_path);
        }
    }
}