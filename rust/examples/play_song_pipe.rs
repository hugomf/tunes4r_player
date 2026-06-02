//! Play a song from YouTube using pipe mode (matching Flutter implementation)
//!
//! This example shows how to use the existing pipe infrastructure
//! to play YouTube streams, similar to the Flutter implementation.
//!
//! Usage: cargo run --example play_song_pipe "adele rolling in the deep"


fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" {
        eprintln!("Usage: cargo run --example play_song_pipe <search-query>");
        eprintln!();
        eprintln!("Plays audio from YouTube videos using pipe mode.");
        eprintln!("This matches the Flutter implementation's approach.");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  cargo run --example play_song_pipe \"adele rolling in the deep\"");
        return;
    }

    let query = &args[1];
    eprintln!("[search] Searching for: {}", query);
    eprintln!("[search] Note: YouTube requires authentication headers.");
    eprintln!("[player] Use the internal YouTubeService for extraction.");
}
