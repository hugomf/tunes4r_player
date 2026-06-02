use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{atomic::AtomicBool, mpsc, Arc};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::TimeBase;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" {
        eprintln!("Usage: cargo run --example play_youtube <video-id|audio-url>");
        eprintln!();
        eprintln!("Pass a YouTube video ID (11 chars) or a direct audio URL.");
        eprintln!("For video IDs, the example will attempt to construct a YouTube audio URL.");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  cargo run --example play_youtube dQw4w9WgXcQ");
        eprintln!("  cargo run --example play_youtube https://rr2.sn...googlevideo.com");
        return;
    }

    let arg = &args[1];

    let is_video_id = arg.len() == 11
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    if is_video_id {
        eprintln!("[youtube] Video ID detected: {}", arg);
        eprintln!("[youtube] Fetching URL via yt-dlp...");

        let yt_dlp_url = format!("https://www.youtube.com/watch?v={}", arg);

        let output = std::process::Command::new("yt-dlp")
            .args(["-f", "bestaudio", "--get-url", &yt_dlp_url])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stream_url = String::from_utf8_lossy(&out.stdout).trim().to_string();
                eprintln!("[youtube] Got URL: {}", stream_url);
                play_stream(&stream_url);
                return;
            }
            Ok(out) => {
                eprintln!("[youtube] yt-dlp failed: status {}", out.status);
                eprintln!("[youtube] stderr: {}", String::from_utf8_lossy(&out.stderr));
            }
            Err(e) => {
                eprintln!("[youtube] Failed to run yt-dlp: {}", e);
            }
        }

        eprintln!("[youtube] Cannot play directly. Use:");
        eprintln!(
            "  yt-dlp -f bestaudio -o - \"https://youtube.com/watch?v={}\" | ffplay -",
            arg
        );
        return;
    }

    let url = arg.clone();
    eprintln!("[youtube] URL: {}", url);

    if url.contains("mime=audio%2Fwebm")
        || url.contains("itag=251")
        || url.contains("itag=250")
        || url.contains("itag=249")
    {
        eprintln!("[youtube] Detected: Opus in WebM");
    } else if url.contains("mime=audio%2Fmp4") || url.contains("itag=140") {
        eprintln!("[youtube] Detected: AAC in M4A");
    }

    eprintln!("[youtube] Starting HTTP download...");
    let start = Instant::now();

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

    let response = match client.get(&url).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[youtube] HTTP request failed: {}", e);
            return;
        }
    };

    if !response.status().is_success() {
        eprintln!("[youtube] HTTP error: {}", response.status());
        return;
    }

    let download_time = start.elapsed();
    eprintln!("[youtube] HTTP connected in {:?}", download_time);

    let source = ReadOnlySource::new(response);
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    eprintln!("[symphonia] Probing format...");
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
            eprintln!("[symphonia] No audio track found");
            return;
        }
    };

    let track_id = track.id;
    let codec_params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            eprintln!("[symphonia] No audio codec parameters");
            return;
        }
    };

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);
    let time_base = track
        .time_base
        .unwrap_or_else(|| TimeBase::try_from_recip(sample_rate).unwrap_or_default());

    let codec_name = {
        use symphonia::core::codecs::audio::well_known::{
            CODEC_ID_AAC, CODEC_ID_FLAC, CODEC_ID_MP1, CODEC_ID_MP2, CODEC_ID_MP3, CODEC_ID_OPUS,
            CODEC_ID_VORBIS,
        };
        let id = codec_params.codec;
        if id == CODEC_ID_OPUS {
            "Opus"
        } else if id == CODEC_ID_AAC {
            "AAC"
        } else if id == CODEC_ID_MP1 || id == CODEC_ID_MP2 || id == CODEC_ID_MP3 {
            "MP3"
        } else if id == CODEC_ID_FLAC {
            "FLAC"
        } else if id == CODEC_ID_VORBIS {
            "Vorbis"
        } else {
            "Unknown"
        }
    };

    eprintln!(
        "[symphonia] Codec: {}, {} Hz, {} channels, time_base: {:?}",
        codec_name, sample_rate, channels, time_base
    );

    if let Some(media_duration) = probed.media_info().duration {
        let ts = symphonia::core::units::Timestamp::new(media_duration.get() as i64);
        let time = time_base.calc_time(ts).unwrap_or_default();
        eprintln!(
            "[symphonia] Duration: {} ms ({:.2} s)",
            time.as_millis(),
            time.as_secs_f64()
        );
    }

    eprintln!("[symphonia] Creating decoder...");
    let mut registry = CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    let mut decoder =
        match registry.make_audio_decoder(&codec_params, &AudioDecoderOptions::default()) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[symphonia] Decoder creation failed: {}", e);
                return;
            }
        };

    eprintln!("[symphonia] Decoder created. Starting decode...");

    let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(64);
    let decode_done = Arc::new(AtomicBool::new(false));
    let d_done = decode_done.clone();

    std::thread::spawn(move || {
        let mut total_packets = 0u64;
        let mut total_samples = 0u64;
        let mut decode_errors = 0u32;
        let max_errors = 50;
        let decode_start = Instant::now();
        let mut last_log = Instant::now();

        loop {
            match probed.next_packet() {
                Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                    Ok(audio_buf) => {
                        decode_errors = 0;
                        let buf_spec = audio_buf.spec();
                        let actual_channels = buf_spec.channels().count();
                        let actual_rate = buf_spec.rate();
                        if actual_channels != channels || actual_rate != sample_rate {
                            if total_packets <= 5 || total_packets % 100 == 0 {
                                eprintln!(
                                    "[decode] ⚠️ Packet {}: expected {}ch/{}Hz, \
                                     actual {}ch/{}Hz",
                                    total_packets,
                                    channels,
                                    sample_rate,
                                    actual_channels,
                                    actual_rate
                                );
                            }
                        }
                        let mut samples: Vec<f32> = Vec::new();
                        audio_buf.copy_to_vec_interleaved(&mut samples);
                        let count = samples.len();
                        total_samples += count as u64;
                        total_packets += 1;

                        if total_packets == 1 {
                            eprintln!(
                                "[decode] First packet: {}ch/{}Hz, {} samples",
                                actual_channels, actual_rate, count
                            );
                        }

                        if tx.send(samples).is_err() {
                            break;
                        }

                        let elapsed = decode_start.elapsed();
                        let audio_time =
                            total_samples as f64 / (actual_rate as f64 * actual_channels as f64);
                        if last_log.elapsed().as_secs() >= 2 {
                            let speed = audio_time / elapsed.as_secs_f64();
                            eprintln!(
                                "[decode] {} packets, {} samples ({:.1}s audio @ {}ch/{}Hz), \
                                 wall: {:.1}s, speed: {:.2}x{}",
                                total_packets,
                                total_samples,
                                audio_time,
                                actual_channels,
                                actual_rate,
                                elapsed.as_secs_f64(),
                                speed,
                                if speed < 0.5 {
                                    " ⚠️ SLOW!"
                                } else if speed < 0.9 {
                                    " ⚡ slight lag"
                                } else {
                                    ""
                                }
                            );
                            last_log = Instant::now();
                        }
                    }
                    Err(e) => {
                        decode_errors += 1;
                        if decode_errors >= max_errors {
                            eprintln!(
                                "[decode] Too many errors ({}/{}), stopping",
                                decode_errors, max_errors
                            );
                            break;
                        }
                        if decode_errors <= 3 || decode_errors % 10 == 0 {
                            eprintln!(
                                "[decode] Packet decode error: {} (error #{})",
                                e, decode_errors
                            );
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                },
                Ok(Some(_)) => continue,
                Ok(None) => {
                    eprintln!(
                        "[decode] Stream ended (packets: {}, samples: {})",
                        total_packets, total_samples
                    );
                    break;
                }
                Err(e) => {
                    decode_errors += 1;
                    if decode_errors >= max_errors {
                        eprintln!(
                            "[decode] Too many packet errors ({}), stopping",
                            decode_errors
                        );
                        break;
                    }
                    if decode_errors <= 5 || decode_errors % 10 == 0 {
                        eprintln!("[decode] Packet error: {} (error #{})", e, decode_errors);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }

        let elapsed = decode_start.elapsed();
        // Use channels from the capture in the loop — this has been updated at compile time
        // but we just approximate here since it's the final message
        let audio_time = total_samples as f64 / (sample_rate as f64 * channels as f64);
        eprintln!(
            "[decode] DONE: {} packets, {} samples ({:.1}s audio) \
             in {:.1}s wall = {:.2}x",
            total_packets,
            total_samples,
            audio_time,
            elapsed.as_secs_f64(),
            if elapsed.as_secs_f64() > 0.0 {
                audio_time / elapsed.as_secs_f64()
            } else {
                0.0
            }
        );
        d_done.store(true, Ordering::SeqCst);
    });

    let host = cpal::default_host();
    let device = host.default_output_device().expect("No output device");
    let supported_config = device.default_output_config().expect("No default config");
    let stream_config: cpal::StreamConfig = supported_config.clone().into();
    eprintln!(
        "[cpal] Device config: supported={:?} -> stream={:?}",
        supported_config, stream_config
    );
    eprintln!(
        "[cpal] Output sr={}, ch={}, decoded sr={}, ch={}",
        stream_config.sample_rate, stream_config.channels, sample_rate, channels
    );

    // Warn if sample rates don't match (would cause speed issues)
    if stream_config.sample_rate != sample_rate {
        eprintln!(
            "[cpal] ⚠️ SAMPLE RATE MISMATCH: device wants {} Hz, decoded audio is {} Hz!",
            stream_config.sample_rate, sample_rate
        );
    }

    let err_fn = move |err| {
        eprintln!("[cpal] Output error: {}", err);
    };

    // Use a lock-free queue to avoid dropping leftover samples
    let decoded_queue = Arc::new(std::sync::Mutex::new(VecDeque::<f32>::new()));
    let queue_for_playback = decoded_queue.clone();

    // Thread to receive from decode channel and buffer into queue
    let play_should_stop = Arc::new(AtomicBool::new(false));
    let p_stop = play_should_stop.clone();
    let queue_for_thread = decoded_queue.clone();
    std::thread::spawn(move || {
        while !p_stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(samples) => {
                    queue_for_thread.lock().unwrap().extend(samples);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    let stream = device
        .build_output_stream(
            &stream_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut q = queue_for_playback.lock().unwrap();
                let available = q.len();
                let needed = data.len();
                if available >= needed {
                    for sample in data.iter_mut() {
                        *sample = q.pop_front().unwrap();
                    }
                } else {
                    // Underflow — fill partial
                    let to_copy = available.min(needed);
                    for sample in data[..to_copy].iter_mut() {
                        *sample = q.pop_front().unwrap();
                    }
                    for sample in data[to_copy..].iter_mut() {
                        *sample = 0.0;
                    }
                }
            },
            err_fn,
            None,
        )
        .expect("Failed to build output stream");

    stream.play().expect("Failed to start playback");

    let mut last_status = Instant::now();
    let play_start = Instant::now();
    let max_play_secs = 120u64;

    while !decode_done.load(Ordering::Relaxed) && play_start.elapsed().as_secs() < max_play_secs {
        if last_status.elapsed().as_secs() >= 5 {
            eprintln!(
                "[main] Still playing... (elapsed: {}s)",
                play_start.elapsed().as_secs()
            );
            last_status = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let final_elapsed = play_start.elapsed();
    eprintln!(
        "[main] Playback finished after {:.1}s. Cleaning up...",
        final_elapsed.as_secs_f64()
    );
    stream.pause().ok();
    drop(stream);
    eprintln!("[main] Done!");
}

fn play_stream(url: &str) {
    eprintln!("[player] Playing stream: {}", url);

    let client = reqwest::blocking::Client::new();

    let response = match client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
        )
        .header("Referer", "https://www.youtube.com")
        .header("Accept", "*/*")
        .header("Accept-Encoding", "identity")
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[player] HTTP request failed: {}", e);
            return;
        }
    };

    if !response.status().is_success() {
        eprintln!("[player] HTTP error: {}", response.status());
        return;
    }

    eprintln!("[player] Connected, starting playback...");

    let source = ReadOnlySource::new(response);
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    let mut probed = match symphonia::default::get_probe().probe(
        &Hint::new(),
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[player] Format probe failed: {}", e);
            return;
        }
    };

    let track = match probed.first_track(symphonia::core::formats::TrackType::Audio) {
        Some(t) => t,
        None => {
            eprintln!("[player] No audio track");
            return;
        }
    };

    let track_id = track.id;
    let codec_params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            eprintln!("[player] No audio codec params");
            return;
        }
    };

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);

    eprintln!("[player] Codec: {} Hz, {} channels", sample_rate, channels);

    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            eprintln!("[player] No output device");
            return;
        }
    };

    let config = device
        .supported_output_configs()
        .expect("No configs")
        .find(|c| c.min_sample_rate() <= sample_rate && c.max_sample_rate() >= sample_rate)
        .map(|c| c.with_sample_rate(sample_rate).into())
        .unwrap_or_else(|| {
            device
                .default_output_config()
                .expect("No default config")
                .into()
        });

    let mut registry = CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();

    let mut decoder =
        match registry.make_audio_decoder(&codec_params, &AudioDecoderOptions::default()) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[player] Decoder creation failed: {}", e);
                return;
            }
        };

    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(64);

    std::thread::spawn(move || {
        while let Ok(Some(packet)) = probed.next_packet() {
            if packet.track_id != track_id {
                continue;
            }
            match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let mut samples: Vec<f32> = Vec::new();
                    audio_buf.copy_to_vec_interleaved(&mut samples);
                    if tx.send(samples).is_err() {
                        break;
                    }
                }
                Err(_) => continue,
            }
        }
    });

    let err_fn = |err| eprintln!("[player] Stream error: {}", err);

    let stream = match device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &_| {
            for sample in data.iter_mut() {
                *sample = rx
                    .recv_timeout(std::time::Duration::from_millis(10))
                    .map(|s| s[0])
                    .unwrap_or(0.0);
            }
        },
        err_fn,
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[player] Stream build failed: {}", e);
            return;
        }
    };

    stream.play().expect("Failed to start stream");
    eprintln!("[player] Playback started!");

    let start = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if start.elapsed().as_secs() > 300 {
            eprintln!("[player] Timeout after 5 minutes");
            break;
        }
    }

    stream.pause().ok();
}
