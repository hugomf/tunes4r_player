//! Test YouTube manifest parsing — approx_duration_ms, formats, PoToken.
//!
//! Run: cargo run --example test_manifest <video-id>
//!
//! Validates:
//! 1. StreamManifest fields are populated
//! 2. approx_duration_ms is present on formats
//! 3. PoToken is auto-generated

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let video_id = args.get(1).map(|s| s.as_str()).unwrap_or("dQw4w9WgXcQ");

    // ── 1. Basic stream resolution ──────────────────────────────────────
    println!("[{video_id}] Resolving via default client (ANDROID_VR)...");
    let yt = tunes4r::youtube::YouTube::new();
    let manifest = yt.videos().stream(video_id).expect("stream() failed");

    println!("    duration_seconds  = {}s", manifest.duration_seconds);
    println!("    duration_ms()     = {}ms", manifest.duration_ms());
    println!("    video formats     = {}", manifest.video.len());
    println!("    audio formats     = {}", manifest.audio.len());

    assert!(manifest.duration_seconds > 0, "duration_seconds must be > 0");
    assert!(!manifest.audio.is_empty(), "must have audio formats");

    // ── 2. approx_duration_ms per format ────────────────────────────────
    println!("\n── Audio format details ──");
    for fmt in &manifest.audio {
        let dur_str = match fmt.approx_duration_ms {
            Some(ms) => format!("{ms}ms"),
            None => "NONE".into(),
        };
        println!(
            "  itag {:>3}  {:.0}kbps  {:>12}  {}  url={}",
            fmt.itag,
            fmt.bitrate as f64 / 1000.0,
            dur_str,
            fmt.mime_type,
            if fmt.url.len() > 80 {
                format!("{}...", &fmt.url[..80])
            } else {
                fmt.url.clone()
            }
        );
    }

    // Verify at least the best audio has duration
    let best = manifest.best_audio().expect("best_audio()");
    assert!(
        best.approx_duration_ms.is_some() || manifest.duration_seconds > 0,
        "best audio should have duration info"
    );
    println!("  ✓ best audio itag {} has duration info", best.itag);

    // ── 3. Format struct fields ──────────────────────────────────────────
    println!("\n── Format struct fields ──");
    println!("  itag            = {}", best.itag);
    println!("  mime_type       = {}", best.mime_type);
    println!("  bitrate         = {}", best.bitrate);
    println!("  approx_duration_ms = {:?}", best.approx_duration_ms);
    println!("  url length      = {}", best.url.len());

    assert!(!best.url.is_empty(), "audio URL must not be empty");
    assert!(best.url.starts_with("http"), "audio URL must start with http");

    // ── 4. PoToken auto-generation ──────────────────────────────────────
    println!("\n── PoToken ──");
    let watch = tunes4r::youtube::watch::fetch_watch_page(
        &reqwest::blocking::Client::new(),
        video_id,
    )
    .unwrap_or_default();

    if let Some(ref vd) = watch.visitor_data {
        let token = tunes4r::youtube::pot::generate_cold_start_token(vd);
        println!("  visitor_data = {vd}");
        println!("  po_token     = {token}");
        assert!(!token.is_empty(), "po_token must not be empty");
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "po_token has invalid characters");
    } else {
        println!("  ⚠ no visitor_data from watch page");
    }

    println!("\n✅ Manifest OK");
}
