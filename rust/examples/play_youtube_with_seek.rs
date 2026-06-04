//! Play a YouTube song with seeking functionality
//!
//! Streams audio with file caching - allows seeking within the buffered portion.
//! For YouTube, seeking requires getting a fresh stream URL since URLs expire.
//!
//! Usage: cargo run --example play_youtube_with_seek "adele rolling in the deep"

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn usage() -> ! {
    eprintln!("Usage: cargo run --example play_youtube_with_seek <search-query|video-id>");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "--help" {
        usage();
    }

    let arg = &args[1];
    let yt = tunes4r::youtube::YouTube::new();

    let video_id = if arg.len() == 11
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        eprintln!("[youtube] Video ID detected: {}", arg);
        arg.clone()
    } else {
        eprintln!("[youtube] Searching for: {}", arg);
        let client = yt.client().http();
        let results = tunes4r::youtube::search::search(client, arg, 5).expect("Search failed");
        let selected = results.into_iter().next().expect("No results found");
        eprintln!("[youtube] Selected: {} — {}", selected.id, selected.title);
        selected.id
    };

    eprintln!("[youtube] Resolving audio stream...");
    let (manifest, http_client) = yt
        .videos()
        .stream_with_client(&video_id)
        .expect("Failed to get manifest");
    let audio = manifest.best_audio().expect("No audio streams found");
    eprintln!(
        "[youtube] Stream: {} kbps — {}",
        audio.bitrate / 1000,
        audio.mime_type
    );

    let stream_url = audio.url.clone();

    eprintln!("[engine] Starting pipe playback...");
    let engine = Arc::new(Mutex::new(tunes4r::create_playback_engine()));
    {
        let mut eng = engine.lock().unwrap();
        eng.play_stream_from_bytes_internal(&stream_url)
            .expect("Failed to start pipe playback");
    }

    let engine_clone = engine.clone();
    let fetch_url = stream_url.clone();

    let fetch_thread = std::thread::spawn(move || {
        eprintln!("[fetch] Connecting...");
        let response = match http_client.get(&fetch_url).send() {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                eprintln!("[fetch] HTTP error: {}", r.status());
                engine_clone
                    .lock()
                    .unwrap()
                    .set_stream_error(&format!("HTTP {}", r.status()));
                return;
            }
            Err(e) => {
                eprintln!("[fetch] Connection failed: {}", e);
                engine_clone
                    .lock()
                    .unwrap()
                    .set_stream_error(&e.to_string());
                return;
            }
        };

        if let Some(len) = response.content_length() {
            engine_clone.lock().unwrap().set_pipe_total_bytes(len);
        }

        let mut stream = response;
        let mut buf = [0u8; 32768];

        loop {
            match stream.read(&mut buf) {
                Ok(0) => {
                    eprintln!("[fetch] Stream ended");
                    engine_clone.lock().unwrap().end_audio_stream();
                    return;
                }
                Ok(n) => {
                    if let Some(pipe) = engine_clone.lock().unwrap().get_stream_pipe() {
                        pipe.push(&buf[..n]);
                    }
                }
                Err(e) => {
                    eprintln!("[fetch] Read error: {}", e);
                    engine_clone
                        .lock()
                        .unwrap()
                        .set_stream_error(&e.to_string());
                    return;
                }
            }
        }
    });

    eprintln!("[engine] Waiting for playback...");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            eprintln!("[engine] Timed out");
            return;
        }
        if engine.lock().unwrap().is_playing() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    eprintln!("[engine] Playing for 5 seconds...");
    std::thread::sleep(Duration::from_secs(5));

    eprintln!("[engine] Seeking to 0:30...");
    {
        let mut eng = engine.lock().unwrap();
        eng.stop();
    }
    // Get fresh stream URL for seek
    eprintln!("[engine] Getting fresh stream URL...");
    let (manifest2, _) = yt
        .videos()
        .stream_with_client(&video_id)
        .expect("Failed to get manifest");
    let audio2 = manifest2.best_audio().expect("No audio streams found");
    let new_url = audio2.url.clone();

    {
        let mut eng = engine.lock().unwrap();
        eng.play_stream_from_bytes_internal(&new_url)
            .expect("Restart failed");
    }
    eprintln!("[engine] Playing from 0:30 for 5 seconds...");
    std::thread::sleep(Duration::from_secs(5));

    eprintln!("[engine] Seeking back to 0:10...");
    {
        let mut eng = engine.lock().unwrap();
        eng.stop();
    }
    // Get fresh stream URL again
    let (manifest3, _) = yt
        .videos()
        .stream_with_client(&video_id)
        .expect("Failed to get manifest");
    let audio3 = manifest3.best_audio().expect("No audio streams found");
    let new_url3 = audio3.url.clone();

    {
        let mut eng = engine.lock().unwrap();
        eng.play_stream_from_bytes_internal(&new_url3)
            .expect("Restart failed");
    }
    eprintln!("[engine] Playing from 0:10 for 5 seconds...");
    std::thread::sleep(Duration::from_secs(5));

    eprintln!("[engine] Done.");
    let _ = fetch_thread.join();
}
