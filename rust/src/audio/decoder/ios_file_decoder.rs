//! iOS audio playback using rodio with symphonia for duration

use log::{debug, info, warn};

use crate::audio::decoder::seek::seek_to_position;
use crate::audio::stream::cpal_source::{build_output_stream, pick_output_config};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

#[cfg(not(target_os = "android"))]
use crate::audio::stream::handling::decode_and_play_from_read;
use crate::dsp::RmsSpectrumAnalyzer;
#[cfg(not(target_os = "android"))]
use reqwest::blocking::get as http_get;
#[cfg(not(target_os = "android"))]
use std::io::Read;

pub fn play_file_internal(
    path: String,
    audio_queue: Arc<parking_lot::Mutex<VecDeque<f32>>>,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    _total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
    _seek_target_ms: Arc<AtomicU64>,
) {
    info!("[file] Starting file playback: {}", path);

    let file_data = match std::fs::read(&path) {
        Ok(data) => {
            info!("[file] File read successfully: {} bytes", data.len());
            data
        }
        Err(e) => {
            let err_msg = format!("Failed to read file: {}", e);
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let cursor = std::io::Cursor::new(file_data);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let probed = match symphonia::default::get_probe().probe(
        &Hint::new(),
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(probed) => probed,
        Err(e) => {
            let err_msg = format!("Format detection failed: {}", e);
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track = match probed.first_track(symphonia::core::formats::TrackType::Audio) {
        Some(track) => track,
        None => {
            let err_msg = "No audio track found".to_string();
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track_id = track.id;
    let codec_params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            let err_msg = "No audio codec params found".to_string();
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);

    info!("[file] File: {} Hz, {} channels", sample_rate, channels);
    sample_rate_out.store(sample_rate as u64, Ordering::Relaxed);
    channels_out.store(channels as u64, Ordering::Relaxed);

    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            let err_msg = "No output device".to_string();
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let config = match device
        .supported_output_configs()
        .expect("No configs")
        .find(|c| c.min_sample_rate() <= sample_rate && c.max_sample_rate() >= sample_rate)
    {
        Some(c) => c.with_sample_rate(sample_rate).into(),
        None => match device.default_output_config() {
            Ok(c) => c.into(),
            Err(_) => cpal::StreamConfig {
                channels: 2,
                sample_rate: 44100,
                buffer_size: cpal::BufferSize::Default,
            },
        },
    };

    info!("[file] Audio device configured: {} Hz", config.sample_rate);

    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();

    let mut decoder = match registry.make_audio_decoder(
        &codec_params,
        &symphonia::core::codecs::audio::AudioDecoderOptions::default(),
    ) {
        Ok(d) => d,
        Err(e) => {
            let err_msg = format!("Decoder creation failed: {}", e);
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let target_buffer_secs = 5.0;
    let target_buffer_samples = (sample_rate as f32 * target_buffer_secs) as usize * channels;
    info!("[file] Pre-buffering {} samples...", target_buffer_samples);

    let mut buffered = 0;
    let mut format = probed;

    // --- SEEK LOGIC: must run BEFORE prebuffer so the queue is filled
    // with samples starting at the seek target, not at time 0.
    // Delegate to the unified seek module shared with macOS and Android.
    let seek_pos_ms = _seek_target_ms.load(Ordering::Relaxed);
    if seek_pos_ms > 0 {
        info!("[file] Seek target: {} ms", seek_pos_ms);
        if let Err(e) = seek_to_position(
            &mut format,
            &codec_params,
            track_id,
            seek_pos_ms,
            &should_stop,
        ) {
            info!("[file] Seek failed: {}", e);
        }
        _seek_target_ms.store(0, Ordering::Relaxed);
    }

    while buffered < target_buffer_samples {
        if should_stop.load(Ordering::Relaxed) {
            info!("[file] Stop requested during pre-buffering");
            return;
        }

        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => {
                info!("[file] End of file during pre-buffering");
                break;
            }
            Err(e) => {
                info!("[file] Packet error: {}", e);
                thread::sleep(Duration::from_millis(10));
                continue;
            }
        };

        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let mut samples: Vec<f32> = Vec::new();
                audio_buf.copy_to_vec_interleaved(&mut samples);
                buffered += samples.len();
                audio_queue.lock().extend(samples);
            }
            Err(e) => {
                info!("[file] Decode error: {}", e);
            }
        }
    }

    info!("[file] Pre-buffering complete: {} samples", buffered);

    let queue_clone = audio_queue.clone();
    let buffer_ready_clone = buffer_ready.clone();
    let samples_played_clone = samples_played.clone();

    let stream = match build_output_stream(
        &device,
        &config,
        queue_for_decode.clone(),
        buffer_ready.clone(),
        samples_played.clone(),
    ) {
        Ok(s) => s,
        Err(e) => {
            let err_msg = format!("Failed to build output stream: {}", e);
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    buffer_ready.store(true, Ordering::Relaxed);
    is_playing_flag.store(true, Ordering::Relaxed);
    info!("[file] Output stream started");

    let band_count = crate::audio::engine::get_band_count();
    let mut analyzer = RmsSpectrumAnalyzer::new(sample_rate, band_count);
    let mut spectrum_accum: VecDeque<f32> = VecDeque::with_capacity(4096);
    let mut last_spectrum_update = std::time::Instant::now();

    // ── Decode loop (synchronous, same thread) ──
    loop {
        if should_stop.load(Ordering::Relaxed) {
            info!("[file] Stop requested during decode");
            break;
        }

        // Check for seek request
        if seek_target_ms.load(Ordering::Acquire) > 0 {
            info!("[file] Seek requested during playback, stopping decode");
            break;
        }

        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => {
                info!("[file] End of file reached");
                break;
            }
            Err(e) => {
                info!("[file] Packet error: {}", e);
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };

        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let mut samples: Vec<f32> = Vec::new();
                audio_buf.copy_to_vec_interleaved(&mut samples);

                for &s in &samples {
                    spectrum_accum.push_back(s);
                }

                queue_for_decode.lock().extend(samples);

                if last_spectrum_update.elapsed().as_millis() >= 100
                    && spectrum_accum.len() >= channels
                {
                    let ch = channels as usize;
                    let total = spectrum_accum.len()
                        - (spectrum_accum.len() % ch);
                    let count = total.min(4096);
                    let raw: Vec<f32> =
                        spectrum_accum.drain(..count).collect();
                    let mono_frames = raw.len() / ch;
                    let mut mono = Vec::with_capacity(mono_frames);
                    for frame in 0..mono_frames {
                        let base = frame * ch;
                        let sum: f32 =
                            raw[base..base + ch].iter().sum();
                        mono.push(sum / ch as f32);
                    }
                    let normalized = analyzer.analyze(&mono);
                    crate::audio::engine::update_global_spectrum(normalized);
                    last_spectrum_update = std::time::Instant::now();
                }
            }
            Err(e) => {
                info!("[file] Decode error: {}", e);
            }
        }
    }

    // --- Wait for the cpal output stream to drain the remaining queue
    // (no QueueSource to signal — just empty the buffer or stop).
    info!("[file] Decode complete, waiting for output drain");
    let drain_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !should_stop.load(Ordering::Relaxed) {
        if queue_for_decode.lock().is_empty() {
            info!("[file] Queue empty, done draining");
            break;
        }
        if std::time::Instant::now() > drain_deadline {
            error!("[file] Drain timeout — queue still has {} samples", queue_for_decode.lock().len());
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    info!("[file] Playback complete");
    drop(stream);
    buffer_ready.store(false, Ordering::Relaxed);
    is_playing_flag.store(false, Ordering::Relaxed);
}

pub fn play_stream_internal(
    url: String,
    audio_queue: Arc<Mutex<VecDeque<f32>>>,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    _total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
    seek_target_ms: Arc<AtomicU64>,
    _seek_byte_offset: u64,
) {
    debug!("[stream] Starting stream playback (iOS with CPAL): {}", url);

    let response = match http_get(&url) {
        Ok(resp) => {
            if !resp.status().is_success() {
                let err_msg = format!("HTTP error: {}", resp.status());
                debug!("[stream] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
            resp
        }
        Err(e) => {
            let err_msg = format!("Failed to connect: {}", e);
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let bytes = match response.bytes() {
        Ok(b) => b.to_vec(),
        Err(e) => {
            let err_msg = format!("Failed to read response body: {}", e);
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let cursor = std::io::Cursor::new(bytes);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let probed = match symphonia::default::get_probe().probe(
        &Hint::new(),
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(probed) => probed,
        Err(e) => {
            let err_msg = format!("Format detection failed: {}", e);
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track = match probed.first_track(symphonia::core::formats::TrackType::Audio) {
        Some(track) => track,
        None => {
            let err_msg = "No audio track found".to_string();
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track_id = track.id;
    let codec_params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            let err_msg = "No audio codec params".to_string();
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);

    debug!("[stream] Stream: {} Hz, {} channels", sample_rate, channels);
    sample_rate_out.store(sample_rate as u64, Ordering::Relaxed);
    channels_out.store(channels as u64, Ordering::Relaxed);

    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            let err_msg = "No output device".to_string();
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let config = match device
        .supported_output_configs()
        .expect("No configs")
        .find(|c| c.min_sample_rate() <= sample_rate && c.max_sample_rate() >= sample_rate)
    {
        Some(c) => c.with_sample_rate(sample_rate).into(),
        None => match device.default_output_config() {
            Ok(c) => c.into(),
            Err(_) => cpal::StreamConfig {
                channels: 2,
                sample_rate: 44100,
                buffer_size: cpal::BufferSize::Default,
            },
        },
    };

    debug!(
        "[stream] Audio device configured: {} Hz",
        config.sample_rate
    );

    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();

    let mut decoder = match registry.make_audio_decoder(
        &codec_params,
        &symphonia::core::codecs::audio::AudioDecoderOptions::default(),
    ) {
        Ok(d) => d,
        Err(e) => {
            let err_msg = format!("Decoder creation failed: {}", e);
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let target_buffer_secs = 5.0;
    let target_buffer_samples = (sample_rate as f32 * target_buffer_secs) as usize * channels;
    let mut buffered_samples = 0;
    let mut format = probed;

    // Handle seek for streams — delegate to the unified seek module.
    let seek_pos_ms = seek_target_ms.load(Ordering::Relaxed);
    if seek_pos_ms > 0 {
        debug!("[stream] Seek target: {} ms", seek_pos_ms);
        if let Err(e) = seek_to_position(
            &mut format,
            &codec_params,
            track_id,
            seek_pos_ms,
            &should_stop,
        ) {
            debug!("[stream] Seek failed: {}", e);
        }
        seek_target_ms.store(0, Ordering::Relaxed);
    }

    let queue_clone = audio_queue.clone();
    let buffer_ready_clone = buffer_ready.clone();
    let samples_played_clone = samples_played.clone();

    let stream = match build_output_stream(
        &device,
        &config,
        queue_clone.clone(),
        buffer_ready.clone(),
        samples_played.clone(),
    ) {
        Ok(s) => s,
        Err(e) => {
            let err_msg = format!("Failed to build output stream: {}", e);
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    buffer_ready.store(true, Ordering::Relaxed);
    is_playing_flag.store(true, Ordering::Relaxed);
    debug!("[stream] Playback flags set");

    stream.play().expect("Failed to start stream");
    debug!("[stream] Audio stream started!");

    loop {
        if should_stop.load(Ordering::Relaxed) {
            debug!("[stream] Stop requested");
            break;
        }

        if buffered_samples >= target_buffer_samples {
            match format.next_packet() {
                Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                    Ok(audio_buf) => {
                        let mut samples: Vec<f32> = Vec::new();
                        audio_buf.copy_to_vec_interleaved(&mut samples);
                        buffered_samples = 0;
                        audio_queue.lock().extend(samples);
                    }
                    Err(e) => {
                        debug!("[stream] Decode error: {}", e);
                        thread::sleep(Duration::from_millis(10));
                    }
                },
                Ok(Some(_)) => continue,
                Ok(None) => {
                    debug!("[stream] Stream ended");
                    break;
                }
                Err(e) => {
                    debug!("[stream] Packet error: {}", e);
                    thread::sleep(Duration::from_millis(100));
                }
            }
        } else {
            match format.next_packet() {
                Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                    Ok(audio_buf) => {
                        let mut samples: Vec<f32> = Vec::new();
                        audio_buf.copy_to_vec_interleaved(&mut samples);
                        buffered_samples += samples.len();
                        audio_queue.lock().extend(samples);
                    }
                    Err(e) => {
                        debug!("[stream] Decode error: {}", e);
                        thread::sleep(Duration::from_millis(10));
                    }
                },
                Ok(Some(_)) => continue,
                Ok(None) => {
                    debug!("[stream] Stream ended during pre-buffer");
                    break;
                }
                Err(e) => {
                    debug!("[stream] Packet error: {}", e);
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    debug!("[stream] Playback complete");
    buffer_ready.store(false, Ordering::Relaxed);
    is_playing_flag.store(false, Ordering::Relaxed);
}

pub fn play_stream_from_pipe_internal(
    reader: crate::audio::stream::pipe::PipeReader,
    audio_queue: Arc<parking_lot::Mutex<VecDeque<f32>>>,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
) {
    info!("[ios] Starting pipe-based stream playback");

    crate::audio::stream::handling::decode_and_play_from_read(
        Box::new(reader),
        audio_queue,
        buffer_ready,
        is_playing_flag,
        should_stop,
        samples_played,
        sample_rate_out,
        channels_out,
        total_duration_ms,
        load_error,
        Arc::new(AtomicU64::new(0)),
    );
}

pub fn play_adaptive_buffer_internal(
    pipe_writer: Arc<crate::audio::stream::pipe::PipeWriter>,
    audio_queue: Arc<parking_lot::Mutex<VecDeque<f32>>>,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
    seek_target_ms: Arc<AtomicU64>,
    url: String,
    cache_dir: String,
) {
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::time::SystemTime;

    info!(
        "[adaptive_buffer] Starting adaptive buffer playback for: {}",
        url
    );
    info!("[adaptive_buffer] Using cache directory: {}", cache_dir);

    if !Path::new(&cache_dir).exists() {
        match fs::create_dir_all(&cache_dir) {
            Ok(_) => info!("[adaptive_buffer] Created cache directory: {}", cache_dir),
            Err(e) => warn!(
                "[adaptive_buffer] Failed to create cache directory: {}, error: {}",
                cache_dir, e
            ),
        }
    }

    let cache_file_name = format!(
        "{}.cache",
        url.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_")
    );
    let cache_file_path = Path::new(&cache_dir).join(cache_file_name);

    let mut cache_file: Option<fs::File> = None;
    if cache_file_path.exists() {
        match fs::metadata(&cache_file_path) {
            Ok(metadata) => {
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let now = SystemTime::now();
                let age = now.duration_since(modified).unwrap_or_default();
                if age.as_secs() < 24 * 60 * 60 {
                    info!(
                        "[adaptive_buffer] Using cached file: {:?} (age: {:.2} hours)",
                        cache_file_path,
                        age.as_secs_f64() / 3600.0
                    );
                    match fs::File::open(&cache_file_path) {
                        Ok(file) => cache_file = Some(file),
                        Err(e) => warn!("[adaptive_buffer] Failed to open cache file: {}", e),
                    }
                } else {
                    info!(
                        "[adaptive_buffer] Cache file is too old (>24h), deleting: {:?}",
                        cache_file_path
                    );
                    let _ = fs::remove_file(&cache_file_path);
                }
            }
            Err(e) => warn!(
                "[adaptive_buffer] Failed to read cache file metadata: {}",
                e
            ),
        }
    }

    let (stream_url, http_client): (String, reqwest::blocking::Client) =
        if url.contains("googlevideo.com") {
            info!("[adaptive_buffer] Direct CDN URL detected, streaming directly");
            let client = match reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    let err_msg = format!("Failed to build HTTP client: {}", e);
                    warn!("[adaptive_buffer] {}", err_msg);
                    *load_error.lock() = err_msg;
                    return;
                }
            };
            (url, client)
        } else {
            let video_id = if url.contains("youtube.com/watch?v=") {
                url.split("v=")
                    .nth(1)
                    .unwrap_or("")
                    .split('&')
                    .next()
                    .unwrap_or("")
                    .to_string()
            } else if url.contains("youtu.be/") {
                url.split("youtu.be/")
                    .nth(1)
                    .unwrap_or("")
                    .split('?')
                    .next()
                    .unwrap_or("")
                    .to_string()
            } else if url.len() == 11
                && url
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                url.clone()
            } else {
                info!("[adaptive_buffer] Treating input as search query: {}", url);
                let yt = crate::youtube::YouTube::new();
                let search_client = yt.client().http();
                match crate::youtube::search::search(search_client, &url, 1) {
                    Ok(results) if !results.is_empty() => {
                        info!(
                            "[adaptive_buffer] Search found: {} ({})",
                            results[0].title, results[0].id
                        );
                        results[0].id.clone()
                    }
                    _ => {
                        let err_msg = format!("Could not find video for: {}", url);
                        warn!("[adaptive_buffer] {}", err_msg);
                        *load_error.lock() = err_msg;
                        return;
                    }
                }
            };

            info!("[adaptive_buffer] Resolved video_id: {}", video_id);

            let yt = crate::youtube::YouTube::new();
            let (manifest, client) = match yt.videos().stream_with_client(&video_id) {
                Ok((m, c)) => {
                    info!(
                        "[adaptive_buffer] Got manifest: {} audio, {} video formats",
                        m.audio.len(),
                        m.video.len()
                    );
                    (m, c)
                }
                Err(e) => {
                    let err_msg = format!("Failed to extract YouTube stream: {}", e);
                    warn!("[adaptive_buffer] {}", err_msg);
                    *load_error.lock() = err_msg;
                    return;
                }
            };

            let audio_format = match manifest.best_audio() {
                Some(a) => {
                    info!(
                        "[adaptive_buffer] Selected audio: itag={:?}, mime={:?}, bitrate={:?}",
                        a.itag, a.mime_type, a.bitrate
                    );
                    a
                }
                None => {
                    let err_msg = "No audio stream found".to_string();
                    warn!("[adaptive_buffer] {}", err_msg);
                    *load_error.lock() = err_msg;
                    return;
                }
            };

            let audio_url = audio_format.url.clone();
            if audio_url.is_empty() {
                let err_msg = "Extracted stream URL is empty".to_string();
                warn!("[adaptive_buffer] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
            info!("[adaptive_buffer] Stream URL: {}", audio_url);
            (audio_url, client)
        };

    let response = match http_client.get(&stream_url).send() {
        Ok(resp) => {
            info!("[adaptive_buffer] HTTP response status: {}", resp.status());
            if !resp.status().is_success() {
                let err_msg = format!("HTTP error: {}", resp.status());
                warn!("[adaptive_buffer] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
            resp
        }
        Err(e) => {
            let err_msg = format!("Failed to connect to stream: {}", e);
            warn!("[adaptive_buffer] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let pipe_writer_clone = pipe_writer.clone();
    let should_stop_clone = should_stop.clone();
    let cache_file_path_clone = cache_file_path.to_string_lossy().to_string();
    let cache_exists = cache_file.is_some();
    let fetch_handle = thread::spawn(move || {
        info!("[adaptive_buffer] Fetch thread started");
        let mut response_reader = response;
        let mut buf = [0u8; 8192];
        let mut total_read: u64 = 0;
        let mut cache_file_opt: Option<fs::File> = None;

        if !cache_exists {
            match fs::File::create(&cache_file_path_clone) {
                Ok(file) => cache_file_opt = Some(file),
                Err(e) => warn!("[adaptive_buffer] Failed to create cache file: {}", e),
            }
        }

        while !should_stop_clone.load(Ordering::Relaxed) {
            match response_reader.read(&mut buf) {
                Ok(0) => {
                    info!("[adaptive_buffer] Stream ended");
                    break;
                }
                Ok(n) => {
                    total_read += n as u64;
                    pipe_writer_clone.push(&buf[..n]);
                    if let Some(ref mut cache) = cache_file_opt {
                        let _ = cache.write_all(&buf[..n]);
                    }
                }
                Err(e) => {
                    warn!("[adaptive_buffer] Read error: {}", e);
                    break;
                }
            }
        }

        if let Some(mut cache) = cache_file_opt {
            let _ = cache.flush();
        }

        pipe_writer_clone.end();
        info!(
            "[adaptive_buffer] Fetch finished, total bytes read: {}",
            total_read
        );
    });
    info!("[adaptive_buffer] Fetch thread spawned, starting decode...");

    let pipe_reader = crate::audio::stream::pipe::PipeReader::new(&pipe_writer);
    decode_and_play_from_read(
        Box::new(pipe_reader),
        audio_queue,
        buffer_ready,
        is_playing_flag,
        should_stop,
        samples_played,
        sample_rate_out,
        channels_out,
        total_duration_ms,
        load_error,
        seek_target_ms,
    );

    let _ = fetch_handle.join();
}
