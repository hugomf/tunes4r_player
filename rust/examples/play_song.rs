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

    // The Player handles YouTube resolution internally.
    // Just pass the search query URL and Player does the rest.
    let mut player = tunes4r::Player::new();

    eprintln!("[player] Starting YouTube playback for: {}", query);
    match player.start(query) {
        Ok(()) => {
            eprintln!("[player] Playing! Press Ctrl+C to stop.");
            loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if player.state() == tunes4r::PlaybackState::Finished {
                    break;
                }
            }
            eprintln!("[player] Playback finished.");
        }
        Err(e) => {
            eprintln!("[player] Playback failed: {}", e);
        }
    }
}
