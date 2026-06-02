//! Test the exact rodio + QueueSource approach from ios_file_decoder.rs.
//!
//! Usage:
//!   cargo run --example audio_test <audio-url>     # stream
//!   cargo run --example audio_test /path/to/file   # local file via RodioDecoder
//!   cargo run --example audio_test --queue <url>   # stream via QueueSource

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rodio::{Decoder as RodioDecoder, DeviceSinkBuilder, Player, Source};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;

fn get_codec_registry() -> CodecRegistry {
    let mut registry = CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "--help" {
        eprintln!("Usage:");
        eprintln!("  cargo run --example audio_test <audio-url>       stream via QueueSource");
        eprintln!("  cargo run --example audio_test /path/to/file     local file via RodioDecoder");
        eprintln!("  cargo run --example audio_test --raw <url>       stream via CPAL (old path)");
        return;
    }

    let use_queue = !args.contains(&"--raw".to_string());
    let target = if args[1] == "--raw" || args[1] == "--queue" {
        args[2].clone()
    } else {
        args[1].clone()
    };

    if target.starts_with("http://") || target.starts_with("https://") {
        if use_queue {
            test_stream_queue(&target);
        } else {
            eprintln!("[main] --raw mode: use play_youtube.rs example instead");
        }
    } else {
        test_file(&target);
    }
}

/// Test local file playback using RodioDecoder directly.
fn test_file(path: &str) {
    eprintln!("[file] Opening: {}", path);
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[file] Error opening file: {}", e);
            return;
        }
    };

    let source = match RodioDecoder::new(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[file] RodioDecoder error: {}", e);
            return;
        }
    };

    let sr = source.sample_rate().get();
    let ch = source.channels().get();
    eprintln!("[file] Decoder: {} Hz, {} ch", sr, ch);

    let sink = DeviceSinkBuilder::open_default_sink()
        .expect("[file] Failed to open sink");
    let player = Player::connect_new(sink.mixer());
    player.append(source);
    player.play();

    eprintln!("[file] Playing... Press Ctrl+C to stop");
    std::thread::sleep(Duration::from_secs(30));
    eprintln!("[file] Done");
}

/// Test stream playback using the exact ios_file_decoder approach:
/// symphonia decode → shared VecDeque → rodio QueueSource.
fn test_stream_queue(url: &str) {
    eprintln!("[stream] URL: {}", url);

    let client = reqwest::blocking::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/120.0.0.0 Safari/537.36",
        )
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("Referer", "https://www.youtube.com".parse().unwrap());
            h.insert("Accept", "*/*".parse().unwrap());
            h.insert("Accept-Encoding", "identity".parse().unwrap());
            h
        })
        .build()
        .expect("Failed to build HTTP client");

    eprintln!("[stream] Connecting...");
    let response = match client.get(url).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[stream] HTTP error: {}", e);
            return;
        }
    };
    if !response.status().is_success() {
        eprintln!("[stream] HTTP status: {}", response.status());
        return;
    }
    eprintln!("[stream] Connected!");

    let source = ReadOnlySource::new(response);
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    eprintln!("[symphonia] Probing...");
    let mut probed = match symphonia::default::get_probe().probe(
        &Hint::new(),
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[symphonia] Probe failed: {}", e);
            return;
        }
    };

    let track = match probed.first_track(symphonia::core::formats::TrackType::Audio) {
        Some(t) => t,
        None => {
            eprintln!("[symphonia] No audio track");
            return;
        }
    };
    let track_id = track.id;

    let codec_params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            eprintln!("[symphonia] No codec params");
            return;
        }
    };

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);
    eprintln!(
        "[symphonia] {} Hz, {} ch (from codec_params)",
        sample_rate, channels
    );

    let mut decoder = match get_codec_registry().make_audio_decoder(
        &codec_params,
        &AudioDecoderOptions::default(),
    ) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[symphonia] Decoder error: {}", e);
            return;
        }
    };

    let audio_queue = Arc::new(parking_lot::Mutex::new(VecDeque::<f32>::new()));
    let queue_for_player = audio_queue.clone();

    // ---------- Pre-buffer ----------
    let target_buffer_secs = 3.0;
    let target_buffer_samples =
        (sample_rate as f32 * target_buffer_secs) as usize * channels;
    let mut buffered = 0;
    let mut actual_channels = channels;
    let mut actual_sample_rate = sample_rate;

    eprintln!(
        "[stream] Pre-buffering {} samples ({}s)...",
        target_buffer_samples, target_buffer_secs
    );
    let prebuf_start = Instant::now();

    while buffered < target_buffer_samples {
        match probed.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => {
                match decoder.decode(&packet) {
                    Ok(buf) => {
                        let spec = buf.spec();
                        actual_channels = spec.channels().count();
                        actual_sample_rate = spec.rate();
                        let mut samples: Vec<f32> = Vec::new();
                        buf.copy_to_vec_interleaved(&mut samples);
                        buffered += samples.len();
                        audio_queue.lock().extend(samples);
                    }
                    Err(e) => {
                        eprintln!("[stream] Prebuf decode error: {}", e);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => {
                eprintln!("[stream] Stream ended during prebuf");
                return;
            }
            Err(e) => {
                eprintln!("[stream] Prebuf packet error: {}", e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    eprintln!(
        "[stream] Pre-buffer done: {} samples in {:?}",
        buffered,
        prebuf_start.elapsed()
    );
    eprintln!(
        "[stream] Using actual spec: {} Hz, {} ch",
        actual_sample_rate, actual_channels
    );

    let channels = actual_channels;
    let sample_rate = actual_sample_rate;

    // ---------- QueueSource ----------
    let starve_counter = Arc::new(AtomicU64::new(0));

    struct QueueSource {
        shared: Arc<parking_lot::Mutex<VecDeque<f32>>>,
        local: VecDeque<f32>,
        channels: u16,
        sample_rate: u32,
        starve_counter: Arc<AtomicU64>,
        total_samples: u64,
    }

    impl QueueSource {
        fn new(
            shared: Arc<parking_lot::Mutex<VecDeque<f32>>>,
            channels: u16,
            sample_rate: u32,
            starve_counter: Arc<AtomicU64>,
        ) -> Self {
            Self {
                shared,
                local: VecDeque::new(),
                channels,
                sample_rate,
                starve_counter,
                total_samples: 0,
            }
        }
    }

    impl Iterator for QueueSource {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            if let Some(s) = self.local.pop_front() {
                self.total_samples += 1;
                return Some(s);
            }
            let mut guard = self.shared.lock();
            let batch = 4096;
            let count = guard.len().min(batch);
            self.local.extend(guard.drain(..count));
            drop(guard);
            if let Some(s) = self.local.pop_front() {
                self.total_samples += 1;
                Some(s)
            } else {
                self.starve_counter.fetch_add(1, Ordering::Relaxed);
                self.total_samples += 1;
                Some(0.0)
            }
        }
    }

    impl Source for QueueSource {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> std::num::NonZero<u16> {
            std::num::NonZero::new(self.channels).unwrap()
        }
        fn sample_rate(&self) -> std::num::NonZero<u32> {
            std::num::NonZero::new(self.sample_rate).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    // ---------- Rodio output ----------
    let sink = DeviceSinkBuilder::open_default_sink()
        .expect("[stream] Failed to open sink");
    let player = Player::connect_new(sink.mixer());
    let qs = QueueSource::new(
        queue_for_player,
        channels as u16,
        sample_rate as u32,
        starve_counter.clone(),
    );
    player.append(qs);
    player.play();

    let start = Instant::now();
    let mut total_packets: u64 = 0;
    let mut last_queue_check = Instant::now();

    // ---------- Decode loop ----------
    loop {
        match probed.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => {
                match decoder.decode(&packet) {
                    Ok(buf) => {
                        let mut samples: Vec<f32> = Vec::new();
                        buf.copy_to_vec_interleaved(&mut samples);
                        audio_queue.lock().extend(samples);
                        total_packets += 1;
                    }
                    Err(e) => {
                        eprintln!("[decode] Error: {}", e);
                    }
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => {
                eprintln!("[decode] Stream ended");
                break;
            }
            Err(e) => {
                eprintln!("[decode] Packet error: {}", e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        if last_queue_check.elapsed().as_secs() >= 3 {
            let qlen = audio_queue.lock().len();
            let starve = starve_counter.load(Ordering::Relaxed);
            let elapsed = start.elapsed();
            eprintln!(
                "[status] {:>4}s  queue={:<6}  starve={:<4}  packets={:<6}",
                elapsed.as_secs(),
                qlen,
                starve,
                total_packets,
            );
            last_queue_check = Instant::now();
        }

        if start.elapsed().as_secs() >= 120 {
            eprintln!("[main] 120s timeout, stopping");
            break;
        }
    }

    eprintln!("[main] Playback finished ({:.1}s)", start.elapsed().as_secs_f64());
    drop(player);
    drop(sink);
}
