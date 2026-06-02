use std::time::Duration;
use tunes4r::audio::engine::PlaybackEngine;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && args[1] == "--help" {
        println!("Usage: play_stream [url|file]");
        println!("  url: Stream URL (default: https://mangoradio.stream.laut.fm/mangoradio)");
        println!("  file: Path to local audio file (mp3, wav, flac, etc.)");
        println!("\nExamples:");
        println!("  cargo run --example play_stream");
        println!("  cargo run --example play_stream https://listen.reyfm.de/original_192kbps.mp3");
        println!("  cargo run --example play_stream /path/to/audio.mp3");
        return;
    }

    println!("Creating playback engine...");
    let mut engine = PlaybackEngine::new().expect("Failed to create engine");

    let arg = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://mangoradio.stream.laut.fm/mangoradio".to_string());

    let is_file = arg.starts_with('/') || arg.contains('\\') || std::path::Path::new(&arg).exists();

    if is_file {
        println!("Testing file playback: {}", arg);
        engine
            .play_file(&arg)
            .expect("Failed to start file playback");
        println!("Listening for 30 seconds... (Ctrl+C to stop)");

        for i in 1..=30 {
            std::thread::sleep(Duration::from_secs(1));
            let state = engine.get_state();
            println!("{}s: state: {:?}", i, state);
        }
    } else {
        println!("Testing stream playback: {}", arg);
        engine
            .play_stream(&arg)
            .expect("Failed to start stream playback");
        println!("Listening for 30 seconds... (Ctrl+C to stop)");

        for i in 1..=30 {
            std::thread::sleep(Duration::from_secs(1));
            let buffered = engine.get_buffered_position();
            let state = engine.get_state();
            println!("{}s: {}ms buffered, state: {:?}", i, buffered, state);
        }
    }

    engine.stop();
    println!("Done!");
}
