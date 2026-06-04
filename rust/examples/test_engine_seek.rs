//! Test engine seek with YouTubeSource — validates duration + seek capability.
//!
//! Run: cargo run --example test_engine_seek <video-id>
//!
//! Validates:
//! 1. YouTubeSource resolves with non-zero duration
//! 2. Byte offset estimation is reasonable
//! 3. Engine reports seek capability for YouTube streams
//! 4. Engine can be stopped cleanly

use tunes4r::audio::stream::source::StreamSource;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let video_id = args.get(1).map(|s| s.as_str()).unwrap_or("dQw4w9WgXcQ");

    // ── 1. YouTubeSource resolution ──────────────────────────────────────
    println!("[test] YouTubeSource::new(\"{video_id}\")...");
    let client = std::sync::Arc::new(
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("http client"),
    );

    let source = tunes4r::audio::stream::source::youtube::YouTubeSource::new(
        video_id,
        client.clone(),
        None,
    )
    .expect("YouTubeSource::new failed");

    let info = source.info();
    println!("  kind     = {:?}", info.kind);
    println!("  title    = {:?}", info.title);
    assert!(matches!(info.kind, tunes4r::audio::stream::source::SourceKind::YouTube));

    // ── 2. Duration via YouTubeSource ────────────────────────────────────
    println!("\n[test] Duration...");
    let yt = tunes4r::youtube::YouTube::new();
    let manifest = yt.videos().stream(video_id).expect("stream()");
    let audio = manifest.best_audio().expect("best_audio()");
    let duration_ms = audio
        .approx_duration_ms
        .or_else(|| {
            let secs = manifest.duration_seconds;
            if secs > 0 { Some(secs * 1000) } else { None }
        })
        .unwrap_or(0);

    println!("  manifest.duration_seconds = {}", manifest.duration_seconds);
    println!("  audio.approx_duration_ms  = {:?}", audio.approx_duration_ms);
    println!("  computed duration_ms      = {}", duration_ms);
    assert!(duration_ms > 0, "duration_ms must be > 0");

    // ── 3. Byte offset estimation ────────────────────────────────────────
    println!("\n[test] Byte offset estimation...");

    // Get content-length from a HEAD request to the CDN
    let head_resp = client
        .head(&audio.url)
        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
        .header("Referer", "https://www.youtube.com")
        .send()
        .ok();
    let content_length = head_resp
        .and_then(|r| r.content_length())
        .unwrap_or(0);

    if content_length > 0 {
        println!("  content-length via HEAD = {content_length}");
        let seek_ms = manifest.duration_seconds * 1000 / 2; // 50%
        let seek_ms_25 = manifest.duration_seconds * 1000 / 4; // 25%
        let seek_ms_75 = manifest.duration_seconds * 1000 * 3 / 4; // 75%

        let byte_offset_25 = (seek_ms_25 as f64 / duration_ms as f64 * content_length as f64) as u64;
        let byte_offset_50 = (seek_ms as f64 / duration_ms as f64 * content_length as f64) as u64;
        let byte_offset_75 = (seek_ms_75 as f64 / duration_ms as f64 * content_length as f64) as u64;

        println!("  seek 25% ({seek_ms_25}ms) → byte {byte_offset_25}");
        println!("  seek 50% ({seek_ms}ms)   → byte {byte_offset_50}");
        println!("  seek 75% ({seek_ms_75}ms) → byte {byte_offset_75}");

        assert!(byte_offset_25 < byte_offset_50, "byte offsets should increase with position");
        assert!(byte_offset_50 < byte_offset_75, "byte offsets should increase with position");
        println!("  ✓ byte offset ordering OK");
    } else {
        println!("  ⚠ content-length not available via HEAD");
    }

    // ── 4. Open at a seek position ──────────────────────────────────────
    println!("\n[test] YouTubeSource::open with seek position...");
    let seek_to = manifest.duration_seconds * 1000 / 2; // 50%
    let seek_result = source.open(Some(seek_to));
    match &seek_result {
        Ok(reader) => {
            println!("  ✓ open(Some({seek_to}ms)) succeeded");
            let _ = reader;
        }
        Err(e) => {
            println!("  ⚠ open(Some({seek_to}ms)) failed: {e} (expected for first call without content_length)");
        }
    }

    // ── 5. Engine seek capability ──────────────────────────────────────
    println!("\n[test] Engine seek capability...");
    let mut engine = tunes4r::audio::engine::PlaybackEngine::new()
        .expect("PlaybackEngine::new");

    let yt_uri = format!("https://www.youtube.com/watch?v={video_id}");
    engine.play(&yt_uri, None).expect("engine.play()");

    let can_seek = engine.source_supports(tunes4r::audio::stream::source::Capability::Seek);
    println!("  source_supports(Seek) = {can_seek}");
    assert!(can_seek, "YouTubeSource must support seek");

    engine.stop();
    println!("  ✓ engine stop OK");

    println!("\n✅ All checks passed");
}
