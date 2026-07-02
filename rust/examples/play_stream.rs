use std::time::Duration;
use tunes4r::Player;

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

    println!("Creating player...");
    let mut player = Player::new();

    let arg = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://mangoradio.stream.laut.fm/mangoradio".to_string());

    let prefixed = if arg.starts_with('/') || std::path::Path::new(&arg).exists() {
        format!("file://{}", arg)
    } else {
        arg.clone()
    };

    println!("Starting playback: {}", arg);
    player
        .start(&prefixed)
        .expect("Failed to start playback");
    println!("Listening for 30 seconds... (Ctrl+C to stop)");

    for i in 1..=30 {
        std::thread::sleep(Duration::from_secs(1));
        let state = player.state();
        let pos = player.position_ms();
        println!("{}s: state: {:?}, pos: {}ms", i, state, pos);
    }

    player.stop();
    println!("Done!");
}
