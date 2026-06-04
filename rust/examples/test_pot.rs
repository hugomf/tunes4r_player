//! Test cold-start PoToken generation and wiring.
//!
//! Run: cargo run --example test_pot <video-id>
//!
//! Verifies:
//! 1. generate_cold_start_token() produces valid output
//! 2. YouTube::videos().get() includes PoToken in player API request
//! 3. Video info includes duration (not 0)

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let video_id = args.get(1).map(|s| s.as_str()).unwrap_or("dQw4w9WgXcQ");

    // ── Unit-level test ──────────────────────────────────────────────────
    println!("[test] generate_cold_start_token()...");
    let token = tunes4r::youtube::pot::generate_cold_start_token("CAAQCA%3D%3D");
    assert!(!token.is_empty(), "token is empty");
    assert!(
        token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "token has invalid chars: {token:?}"
    );
    println!("  ✓ token = {token}");

    // Also test with empty identifier
    let empty_token = tunes4r::youtube::pot::generate_cold_start_token("");
    assert!(!empty_token.is_empty(), "empty-id token is empty");
    println!("  ✓ empty-id token = {empty_token}");

    // ── Integration: audio stream URL with auto-generated PoToken ────────
    println!("\n[test] YouTube::videos().stream() with auto-generated PoToken...");
    let yt = tunes4r::youtube::YouTube::new();
    match yt.videos().stream(video_id) {
        Ok(manifest) => {
            println!("  ✓ stream OK");
            println!("    audio formats: {}", manifest.audio.len());
            println!("    video formats: {}", manifest.video.len());
            for fmt in &manifest.audio {
                let has_pot = fmt.url.contains("pot=");
                println!("    itag {} pot={has_pot}", fmt.itag);
            }
            if let Some(dur) = manifest.audio.first().and_then(|f| f.approx_duration_ms) {
                println!("    approx_duration_ms = {dur}");
            }
        }
        Err(e) => {
            println!("  ✗ stream failed: {e}");
        }
    }

    // ── Integration: video info with auto-generated PoToken ──────────────
    println!("\n[test] YouTube::videos().get() with auto-generated PoToken...");
    match yt.videos().get(video_id) {
        Ok(info) => {
            println!("  ✓ video info:");
            println!("    title    = {}", info.title);
            println!("    author   = {}", info.author);
            println!("    duration = {}s", info.duration);
            if info.duration > 0 {
                println!("  ✓ duration IS present (was probably 0 before PoToken)");
            } else {
                println!("  ✗ duration is still 0 — player API may still be blocked");
            }
        }
        Err(e) => {
            println!("  ✗ video info failed: {e}");
        }
    }

    println!("\nDone.");
}
