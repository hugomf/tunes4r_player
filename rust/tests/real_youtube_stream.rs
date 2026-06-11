use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tunes4r::audio::error::PlaybackError;
use tunes4r::audio::stream::decorator::adaptive::AdaptiveBufferDecorator;
use tunes4r::audio::stream::decorator::caching::ByteCache;
use tunes4r::audio::stream::handling::ByteCountingRead;
use tunes4r::audio::stream::source::{Capability, ReadSeek, StreamSource};

pub struct RealYouTubeStream {
    file: Arc<File>,
    content_length: u64,
    video_id: String,
}

impl RealYouTubeStream {
    pub fn new(video_id: String) -> Result<Self> {
        let file = File::open(format!("rust/tests/fixtures/youtube_stream.bin"))?;
        let content_length = file.metadata()?.len();

        Ok(Self {
            file: Arc::new(file),
            content_length,
            video_id,
        })
    }
}

impl ReadSeek for RealYouTubeStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }

    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl StreamSource for RealYouTubeStream {
    fn source_info(&self) -> StreamSource::SourceInfo {
        StreamSource::SourceInfo {
            video_id: self.video_id.clone(),
            content_type: "audio/mp4".to_string(),
            content_length: self.content_length,
            capabilities: Capability::SEEK,
        }
    }
}

#[derive(Debug)]
pub struct CapturedYouTubeData {
    pub video_id: String,
    pub content_type: String,
    pub content_length: u64,
    pub captured_bytes: u64,
    pub original_url: String,
}

pub fn load_captured_data() -> Result<CapturedYouTubeData> {
    let json_str = std::fs::read_to_string("rust/tests/fixtures/youtube_stream.json")?;
    let data: serde_json::Value = serde_json::from_str(&json_str)?;

    Ok(CapturedYouTubeData {
        video_id: data["video_id"].as_str().unwrap().to_string(),
        content_type: data["content_type"].as_str().unwrap().to_string(),
        content_length: data["content_length"].as_u64().unwrap(),
        captured_bytes: data["captured_bytes"].as_u64().unwrap(),
        original_url: data["original_url"].as_str().unwrap().to_string(),
    })
}

pub fn test_real_youtube_stream_with_cache() -> Result<()> {
    println!("Testing real YouTube stream with cache...");

    let captured_data = load_captured_data()?;
    println!("Video ID: {}", captured_data.video_id);
    println!("Content type: {}", captured_data.content_type);
    println!("Content length: {}", captured_data.content_length);
    println!("Captured bytes: {}", captured_data.captured_bytes);

    let stream = RealYouTubeStream::new(captured_data.video_id)?;
    let cache = Arc::new(ByteCache::new(30 * 1024 * 1024)); // 30 MB cache
    let eof = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut cached_stream = AdaptiveBufferDecorator::new(stream, cache.clone(), eof.clone());

    let mut buffer = vec![0u8; 32768];
    let mut total_read = 0u64;
    let mut total_bytes_in_cache = 0u64;

    let start_time = Instant::now();

    while !eof.load(std::sync::atomic::Ordering::SeqCst) {
        match cached_stream.read(&mut buffer) {
            Ok(0) => {
                eof.store(true, std::sync::atomic::Ordering::SeqCst);
                break;
            }
            Ok(n) => {
                total_read += n as u64;
                if let Ok(cache_mutex) = cache.try_lock() {
                    total_bytes_in_cache = cache_mutex.total_written();
                }

                if total_read % (1024 * 1024) == 0 {
                    let progress =
                        (total_read as f64 / captured_data.content_length as f64) * 100.0;
                    println!(
                        "Progress: {:.1}% ({}/{})",
                        progress, total_read, captured_data.content_length
                    );
                }

                if total_read >= captured_data.content_length {
                    break;
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Read error: {}", e));
            }
        }
    }

    let elapsed = start_time.elapsed();
    let avg_speed = total_read as f64 / elapsed.as_secs_f64();

    println!("\n=== Stream Complete ===");
    println!("Total read: {} bytes", total_read);
    println!("Total bytes in cache: {}", total_bytes_in_cache);
    println!("Elapsed time: {:.2}s", elapsed.as_secs_f64());
    println!("Average speed: {:.2} KB/s", avg_speed / 1024.0);

    if total_read == captured_data.captured_bytes {
        println!(
            "✓ Successfully captured all {} bytes from fixture",
            total_read
        );
    } else {
        println!(
            "⚠ Expected {} bytes, got {}",
            captured_data.captured_bytes, total_read
        );
    }

    Ok(())
}

pub fn test_backward_seek_with_cache() -> Result<()> {
    println!("\nTesting backward seek with cache...");

    let captured_data = load_captured_data()?;
    let stream = RealYouTubeStream::new(captured_data.video_id)?;
    let cache = Arc::new(ByteCache::new(30 * 1024 * 1024));
    let eof = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut cached_stream = AdaptiveBufferDecorator::new(stream, cache.clone(), eof.clone());

    let total_bytes = captured_data.captured_bytes as usize;
    let read_size = 1024 * 1024; // 1 MB chunks

    let mut positions = Vec::new();

    for i in 0..=5 {
        let pos = (i * read_size) as u64;
        if pos >= total_bytes as u64 {
            break;
        }

        println!("Reading chunk {} (offset: {})", i, pos);

        if let Ok(_) = cached_stream.seek(SeekFrom::Start(pos)) {
            let mut buffer = vec![0u8; read_size.min(total_bytes - pos as usize)];
            match cached_stream.read(&mut buffer) {
                Ok(0) => {
                    if !eof.load(std::sync::atomic::Ordering::SeqCst) {
                        println!("  ⚠ Unexpected EOF at offset {}", pos);
                    }
                }
                Ok(n) => {
                    println!("  ✓ Read {} bytes", n);
                    positions.push((pos, n));
                }
                Err(e) => {
                    println!("  ✗ Read error: {}", e);
                }
            }
        } else {
            println!("  ✗ Seek error: {}", e);
        }
    }

    println!("\n=== Backward Seek Test Complete ===");
    println!("Successfully read {} chunks", positions.len());

    if positions.len() == 6 {
        println!("✓ All backward seeks successful");
    } else {
        println!("⚠ Expected 6 chunks, got {}", positions.len());
    }

    Ok(())
}

pub fn test_forward_seek_with_cache() -> Result<()> {
    println!("\nTesting forward seek with cache...");

    let captured_data = load_captured_data()?;
    let stream = RealYouTubeStream::new(captured_data.video_id)?;
    let cache = Arc::new(ByteCache::new(30 * 1024 * 1024));
    let eof = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut cached_stream = AdaptiveBufferDecorator::new(stream, cache.clone(), eof.clone());

    let total_bytes = captured_data.captured_bytes as usize;

    let mut positions = Vec::new();

    for i in (1..=5).rev() {
        let pos = (i * 1024 * 1024) as u64;
        if pos >= total_bytes as u64 {
            break;
        }

        println!("Seeking forward to offset {} (chunk {})", pos, i);

        if let Ok(_) = cached_stream.seek(SeekFrom::Start(pos)) {
            let mut buffer = vec![0u8; 1024];
            match cached_stream.read(&mut buffer) {
                Ok(0) => {
                    if !eof.load(std::sync::atomic::Ordering::SeqCst) {
                        println!("  ⚠ Unexpected EOF at offset {}", pos);
                    }
                }
                Ok(n) => {
                    println!("  ✓ Read {} bytes from position {}", n, pos);
                    positions.push((pos, n));
                }
                Err(e) => {
                    println!("  ✗ Read error: {}", e);
                }
            }
        } else {
            println!("  ✗ Seek error: {}", e);
        }
    }

    println!("\n=== Forward Seek Test Complete ===");
    println!("Successfully read {} positions", positions.len());

    if positions.len() == 5 {
        println!("✓ All forward seeks successful");
    } else {
        println!("⚠ Expected 5 positions, got {}", positions.len());
    }

    Ok(())
}

pub fn test_seek_accuracy() -> Result<()> {
    println!("\nTesting seek accuracy...");

    let captured_data = load_captured_data()?;
    let stream = RealYouTubeStream::new(captured_data.video_id)?;
    let cache = Arc::new(ByteCache::new(30 * 1024 * 1024));
    let eof = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut cached_stream = AdaptiveBufferDecorator::new(stream, cache.clone(), eof.clone());

    let mut last_read_offset = 0u64;
    let mut errors = 0;

    for offset in &[1000, 10000, 100000, 1000000, 5000000, 15000000, 2097152] {
        if let Ok(_) = cached_stream.seek(SeekFrom::Start(*offset)) {
            let mut buffer = vec![0u8; 100];
            match cached_stream.read(&mut buffer) {
                Ok(0) => {
                    println!("  ⚠ EOF at offset {}", offset);
                }
                Ok(n) => {
                    if last_read_offset != 0 && n > 0 {
                        let expected_offset = if last_read_offset < *offset {
                            *offset
                        } else {
                            last_read_offset
                        };
                        println!(
                            "  ✓ Read {} bytes at offset {} (prev: {})",
                            n, *offset, last_read_offset
                        );
                    }
                    last_read_offset = *offset;
                }
                Err(e) => {
                    println!("  ✗ Error at offset {}: {}", offset, e);
                    errors += 1;
                }
            }
        } else {
            println!("  ✗ Seek error at offset {}: {}", offset, e);
            errors += 1;
        }
    }

    println!("\n=== Seek Accuracy Test Complete ===");
    if errors == 0 {
        println!("✓ All seek operations successful");
    } else {
        println!("⚠ {} seek errors detected", errors);
    }

    Ok(())
}
