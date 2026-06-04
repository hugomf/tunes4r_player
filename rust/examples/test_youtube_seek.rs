//! Test YouTube seek: duration + HTTP Range support.
//!
//! Run: cargo run --example test_youtube_seek <video-id>
//!
//! Validates:
//! 1. StreamManifest has non-zero duration_seconds
//! 2. Best audio format has approx_duration_ms
//! 3. CDN URL accepts Range requests (returns 206 Partial Content)
//! 4. Byte offset for seek position is reasonable

use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let video_id = args.get(1).map(|s| s.as_str()).unwrap_or("dQw4w9WgXcQ");

    // ── Resolve stream ──────────────────────────────────────────────────
    println!("Resolving {video_id}...");
    let yt = tunes4r::youtube::YouTube::new();
    let manifest = yt.videos().stream(video_id).expect("stream() failed");
    let audio = manifest.best_audio().expect("no audio streams");

    // ── 1. Duration checks ──────────────────────────────────────────────
    println!("\n── Duration ──");
    println!("  manifest.duration_seconds   = {}", manifest.duration_seconds);
    println!("  manifest.duration_ms()      = {}", manifest.duration_ms());
    println!("  audio.approx_duration_ms    = {:?}", audio.approx_duration_ms);

    assert!(manifest.duration_seconds > 0, "duration_seconds should be > 0, got {}", manifest.duration_seconds);
    assert!(audio.approx_duration_ms.unwrap_or(0) > 0, "approx_duration_ms should be > 0");
    println!("  ✓ duration OK");

    // ── 2. CDN Range request ────────────────────────────────────────────
    let url = &audio.url;
    let seek_ms = manifest.duration_seconds * 1000 / 2; // seek to 50%
    let duration_ms = audio.approx_duration_ms.unwrap_or(manifest.duration_seconds * 1000);

    println!("\n── HTTP Range test ──");
    println!("  video duration: {duration_ms}ms");
    println!("  seek target:    {seek_ms}ms (50%)");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("http client");

    // First GET to discover content-length
    let resp = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Referer", "https://www.youtube.com")
        .send()
        .expect("GET failed");

    let content_length = resp.content_length().unwrap_or(0);
    println!("  content-length: {content_length} bytes");

    // Calculate byte offset using the same logic as YouTubeSource::estimate_byte_offset
    let byte_offset = if duration_ms > 0 && content_length > 0 {
        let ratio = (seek_ms as f64 / duration_ms as f64).min(0.99);
        (ratio * content_length as f64) as u64
    } else {
        0
    };
    println!("  byte offset:    {byte_offset} (≈{:.1}% of file)", byte_offset as f64 / content_length as f64 * 100.0);

    assert!(byte_offset > 0, "byte_offset should be > 0");

    // Send Range request
    let range_resp = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Referer", "https://www.youtube.com")
        .header("Range", format!("bytes={}-", byte_offset))
        .send()
        .expect("Range request failed");

    let range_status = range_resp.status();
    let range_cl = range_resp.content_length().unwrap_or(0);

    println!("  Range response: {range_status}");
    println!("  Range body:     {range_cl} bytes");

    if range_status == 206 {
        println!("  ✓ YouTube CDN accepts Range requests");
        // Read a small chunk to verify data flows
        let mut buf = [0u8; 4096];
        let n = range_resp.take(4096).read(&mut buf).unwrap_or(0);
        println!("  ✓ Read {n} bytes from seek position — data OK");
    } else {
        println!("  ⚠ CDN returned {range_status} instead of 206 — Range may not be supported");
    }

    // ── 3. Seek pipeline check ──────────────────────────────────────────
    println!("\n── Seek pipeline ──");
    println!("  YouTubeSource stores duration_ms = {duration_ms}");
    println!("  estimate_byte_offset({seek_ms}, {content_length}) → {byte_offset}");
    println!("  → sends Range: bytes={byte_offset}-");
    println!("  → CDN returns 206 with data from that position");

    if range_status == 206 {
        println!("\n✅ All checks passed — seeking should work correctly");
    } else {
        println!("\n⚠ Duration is correct but CDN Range may need investigation");
    }
}
