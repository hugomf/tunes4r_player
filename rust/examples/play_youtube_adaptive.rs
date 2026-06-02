use tunes4r::PlaybackEngine;

fn main() {
    let mut engine = PlaybackEngine::new().expect("Failed to create playback engine");
    
    let url = "https://www.youtube.com/watch?v=hLQl3WQQoQ0"; // Adele - Someone Like You
    let cache_dir = "./audio_cache";
    
    println!("Playing YouTube video with adaptive buffering...");
    println!("URL: {}", url);
    println!("Cache directory: {}", cache_dir);
    
    match engine.play_adaptive_buffer(url, cache_dir) {
        Ok(_) => {
            println!("Playback started successfully!");
            
            // Wait for user input to stop playback
            println!("Press Enter to stop playback...");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).unwrap();
            
            engine.stop();
            println!("Playback stopped.");
        }
        Err(e) => {
            println!("Error starting playback: {}", e);
        }
    }
}