//! Audio stream processing and decoding logic, primarily for non-Android platforms.

#[cfg(not(target_os = "android"))]
use crate::dsp::RmsSpectrumAnalyzer;
#[cfg(not(target_os = "android"))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(not(target_os = "android"))]
use log::{debug, error, info, warn};
#[cfg(not(target_os = "android"))]
use parking_lot::Mutex;
#[cfg(not(target_os = "android"))]
use std::io::Read;
#[cfg(not(target_os = "android"))]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(not(target_os = "android"))]
use std::sync::Arc;
#[cfg(not(target_os = "android"))]
use std::thread;
#[cfg(not(target_os = "android"))]
use std::time::Duration;

#[cfg(not(target_os = "android"))]
use symphonia::core::formats::probe::Hint;
#[cfg(not(target_os = "android"))]
use symphonia::core::formats::FormatOptions;
#[cfg(not(target_os = "android"))]
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
#[cfg(not(target_os = "android"))]
use symphonia::core::meta::MetadataOptions;
#[cfg(not(target_os = "android"))]
use symphonia::core::units::{TimeBase, Timestamp};

#[cfg(not(target_os = "android"))]
pub fn fast_forward_stream_seek(
    format: &mut Box<dyn symphonia::core::formats::FormatReader>,
    codec_params: &symphonia::core::codecs::audio::AudioCodecParameters,
    track_id: u32,
    target_ms: u64,
    should_stop: &Arc<AtomicBool>,
) -> u64 {
    let sample_rate = codec_params.sample_rate.unwrap_or(44100) as f64;
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);

    let target_samples = (target_ms as f64 / 1000.0 * sample_rate) as u64;
    let mut skipped_samples: u64 = 0;

    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    let mut seek_decoder = match registry.make_audio_decoder(
        codec_params,
        &symphonia::core::codecs::audio::AudioDecoderOptions::default(),
    ) {
        Ok(d) => d,
        Err(_) => return 0,
    };

    loop {
        if should_stop.load(Ordering::Relaxed) {
            return skipped_samples;
        }

        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => {
                warn!("[stream] Packet error during seek fast-forward: {}", e);
                thread::sleep(Duration::from_millis(10));
                continue;
            }
        };

        if packet.track_id != track_id {
            continue;
        }

        match seek_decoder.decode(&packet) {
            Ok(audio_buf) => {
                skipped_samples += audio_buf.frames() as u64;
                if skipped_samples >= target_samples {
                    info!(
                        "[stream] Seek fast-forward complete: skipped {} samples (target: {})",
                        skipped_samples, target_samples
                    );
                    break;
                }
            }
            Err(e) => {
                warn!("[stream] Decode error during seek fast-forward: {}", e);
            }
        }
    }

    if skipped_samples > 0 && channels > 0 {
        let estimated_ms = (skipped_samples * 1000) / (channels as u64 * sample_rate as u64);
        debug!("[stream] Estimated seek position: ~{} ms", estimated_ms);
    }

    skipped_samples
}

#[cfg(not(target_os = "android"))]
pub fn resample_interleaved(
    input: &[f32],
    from_rate: u32,
    to_rate: u32,
    channels: usize,
) -> Vec<f32> {
    if from_rate == to_rate {
        return input.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let input_frames = input.len() / channels;
    let output_frames = (input_frames as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_frames * channels);

    for out_frame in 0..output_frames {
        let in_pos_f = out_frame as f64 / ratio;
        let in_frame = in_pos_f.floor() as usize;
        let frac = in_pos_f - in_frame as f64;

        for ch in 0..channels {
            let idx_a = in_frame * channels + ch;
            let val_a = input[idx_a];
            let val_b = if in_frame + 1 < input_frames {
                input[(in_frame + 1) * channels + ch]
            } else {
                val_a
            };
            output.push((val_a as f64 + (val_b as f64 - val_a as f64) * frac) as f32);
        }
    }
    output
}

#[cfg(not(target_os = "android"))]
pub fn decode_and_play_from_read(
    reader: Box<dyn Read + Send + Sync + 'static>,
    audio_queue: crate::audio::stream::cpal_source::AudioBuffer,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
    seek_target_ms: Arc<AtomicU64>,
) {
    use crate::audio::engine::types::get_band_count;

    info!("[stream] Connected! Detecting format...");
    info!("[stream] Connected! Detecting format...");

    let mss = MediaSourceStream::new(
        Box::new(ReadOnlySource::new(Box::new(reader))),
        Default::default(),
    );

    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = match symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(probed) => {
            info!("[stream] Format detected successfully");
            probed
        }
        Err(e) => {
            let err_msg = format!("Format detection failed: {}", e);
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track = match probed.first_track(symphonia::core::formats::TrackType::Audio) {
        Some(track) => {
            info!("[stream] Found audio track: id={}", track.id);
            track
        }
        None => {
            let err_msg = "No audio track found".to_string();
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let _track_id = track.id;
    let _codec_params = match track.codec_params.as_ref() {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => {
            info!(
                "[stream] Codec params: sample_rate={:?}, channels={:?}",
                params.sample_rate, params.channels
            );
            params.clone()
        }
        _ => {
            let err_msg = "No audio codec params".to_string();
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let duration_ms = {
        let time_base = probed
            .first_track(symphonia::core::formats::TrackType::Audio)
            .and_then(|t| t.time_base)
            .unwrap_or_else(|| TimeBase::try_from_recip(44100).unwrap_or_default());
        if let Some(media_duration) = probed.media_info().duration {
            let ts = Timestamp::new(media_duration.get() as i64);
            let time = time_base.calc_time(ts).unwrap_or_default();
            time.as_millis() as u64
        } else {
            0
        }
    };
    if duration_ms > 0 {
        info!("[stream] Stream duration: {} ms", duration_ms);
        total_duration_ms.store(duration_ms, Ordering::Relaxed);
    }

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
    let codec_params = match track.codec_params.as_ref() {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            let err_msg = "No audio codec params".to_string();
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let sample_rate = match codec_params.sample_rate {
        Some(rate) => rate,
        None => {
            let err_msg = "No sample rate".to_string();
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);

    debug!("[stream] Stream: {} Hz, {} channels", sample_rate, channels);
    sample_rate_out.store(sample_rate as u64, Ordering::Relaxed);
    channels_out.store(channels as u64, Ordering::Relaxed);

    let seek_pos_ms = seek_target_ms.load(Ordering::Relaxed);

    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(device) => device,
        None => {
            let err_msg = "No audio output device".to_string();
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let config = match device.default_output_config() {
        Ok(c) => {
            let output_rate = c.sample_rate();
            info!(
                "[stream] Using device default: {} Hz, {} ch (stream is {} Hz) - will resample if needed",
                output_rate, c.channels(), sample_rate
            );
            c.into()
        }
        Err(_) => {
            info!("[stream] No default config, using fallback: 44100 Hz, 2 ch");
            cpal::StreamConfig {
                channels: 2,
                sample_rate: 44100_u32.into(),
                buffer_size: cpal::BufferSize::Default,
            }
        }
    };

    info!(
        "[stream] Audio device configured: {:?} Hz, {} ch",
        config.sample_rate, config.channels
    );

    let output_sample_rate = config.sample_rate;

    let target_buffer_secs = 7.0;
    let target_buffer_samples =
        (output_sample_rate as f32 * target_buffer_secs) as usize * channels;
    let mut buffered_samples = 0;
    let mut format = probed;

    if seek_pos_ms > 0 {
        debug!("[stream] Seek target: {} ms", seek_pos_ms);
        debug!("[stream] Fast-forwarding through packets to seek...");
        fast_forward_stream_seek(
            &mut format,
            &codec_params,
            track_id,
            seek_pos_ms,
            &should_stop,
        );
        seek_target_ms.store(0, Ordering::Relaxed);
    }

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
        Ok(decoder) => decoder,
        Err(e) => {
            let err_msg = format!("Decoder creation failed: {}", e);
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    debug!(
        "[stream] Pre-buffering {} samples ({} seconds)...",
        target_buffer_samples, target_buffer_secs
    );

    #[allow(unused_assignments)]
    let mut actual_channels = channels;
    #[allow(unused_assignments)]
    let mut actual_sample_rate = sample_rate;

    while buffered_samples < target_buffer_samples {
        debug!(
            "[stream] Pre-buffering: {}/{} samples",
            buffered_samples, target_buffer_samples
        );
        match format.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let buf_spec = audio_buf.spec();
                    actual_channels = buf_spec.channels().count();
                    actual_sample_rate = buf_spec.rate();
                    if actual_channels != channels || actual_sample_rate != sample_rate {
                        debug!(
                            "[stream] Decoded buffer: {}ch/{}Hz, expected {}ch/{}Hz",
                            actual_channels, actual_sample_rate, channels, sample_rate
                        );
                    }
                    let mut samples: Vec<f32> = Vec::new();
                    audio_buf.copy_to_vec_interleaved(&mut samples);
                    if actual_sample_rate != output_sample_rate {
                        samples = resample_interleaved(
                            &samples,
                            actual_sample_rate,
                            output_sample_rate,
                            actual_channels,
                        );
                        debug!(
                            "[stream] Resampled packet: {}Hz -> {}Hz, {} samples",
                            actual_sample_rate,
                            output_sample_rate,
                            samples.len()
                        );
                    }
                    let sample_count = samples.len();
                    buffered_samples += sample_count;
                    audio_queue.lock().extend(samples);
                    debug!(
                        "[stream] Pre-buffered {} samples, total: {}",
                        sample_count, buffered_samples
                    );
                }
                Err(e) => {
                    debug!("[stream] Decode error: {}", e);
                    thread::sleep(Duration::from_millis(10));
                }
            },
            Ok(Some(_)) => continue,
            Ok(None) => {
                info!(
                    "[stream] Stream ended during buffering (buffered {} samples)",
                    buffered_samples
                );
                break;
            }
            Err(e) => {
                debug!("[stream] Packet error: {}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    info!("[stream] Pre-buffer complete: {} samples", buffered_samples);

    let spectrum_sample_buffer: Arc<Mutex<Vec<f32>>> =
        Arc::new(Mutex::new(Vec::with_capacity(4096)));
    let callback_spectrum_buf = spectrum_sample_buffer.clone();

    info!("[stream] Building output stream...");

    // Try building with the matched config first; fall back to device default on failure.
    let build_stream = |cfg: &cpal::StreamConfig| -> Result<cpal::Stream, String> {
        let pq = audio_queue.clone();
        let br = buffer_ready.clone();
        let sp = samples_played.clone();
        let cbs = callback_spectrum_buf.clone();
        device
            .build_output_stream(
                cfg,
                move |data: &mut [f32], _| {
                    if !br.load(Ordering::Relaxed) {
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                        return;
                    }
                    let mut queue = pq.lock();
                    let mut count = 0;
                    for sample in data.iter_mut() {
                        *sample = queue.pop_front().unwrap_or(0.0);
                        count += 1;
                    }
                    if count > 0 {
                        sp.fetch_add(count as u64, Ordering::Relaxed);
                    }
                    if let Some(mut buf) = cbs.try_lock() {
                        let len = data.len().min(4096);
                        buf.clear();
                        buf.extend_from_slice(&data[..len]);
                    }
                },
                |err| error!("[stream] Audio output error: {}", err),
                None,
            )
            .map_err(|e| format!("{}", e))
    };

    let stream = match build_stream(&config) {
        Ok(s) => {
            info!("[stream] Output stream built with primary config");
            s
        }
        Err(e) => {
            warn!(
                "[stream] Failed to build stream with primary config ({} Hz, {} ch): {}. Retrying with device default...",
                config.sample_rate, config.channels, e
            );
            match device.default_output_config() {
                Ok(default_cfg) => {
                    let fallback: cpal::StreamConfig = default_cfg.into();
                    info!(
                        "[stream] Retrying with fallback config: {:?} Hz, {} ch",
                        fallback.sample_rate, fallback.channels
                    );
                    match build_stream(&fallback) {
                        Ok(s) => {
                            info!("[stream] Output stream built with fallback config");
                            s
                        }
                        Err(e2) => {
                            let err_msg = format!(
                                "Failed to build audio stream with both configs: primary={}, fallback={}",
                                e, e2
                            );
                            error!("[stream] {}", err_msg);
                            *load_error.lock() = err_msg;
                            return;
                        }
                    }
                }
                Err(e2) => {
                    let err_msg = format!(
                        "Failed to build audio stream ({}) and no default config available: {}",
                        e, e2
                    );
                    error!("[stream] {}", err_msg);
                    *load_error.lock() = err_msg;
                    return;
                }
            }
        }
    };

    buffer_ready.store(true, Ordering::Relaxed);
    is_playing_flag.store(true, Ordering::Relaxed);

    match stream.play() {
        Ok(()) => {
            info!(
                "[stream] Audio stream started! Queue: {} samples",
                audio_queue.lock().len()
            );
        }
        Err(e) => {
            let err_msg = format!("Failed to start audio stream: {}", e);
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    }

    let band_count = get_band_count();
    crate::audio::engine::types::update_global_spectrum(vec![0.1f32; band_count]);

    let spectrum_stop = should_stop.clone();
    let spectrum_reader = spectrum_sample_buffer.clone();
    let spectrum_channels = channels;

    thread::Builder::new()
        .name("stream-spectrum".to_string())
        .spawn(move || {
            let mut analyzer = RmsSpectrumAnalyzer::new(output_sample_rate, band_count);
            loop {
                if spectrum_stop.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
                let raw = {
                    let buf = spectrum_reader.lock();
                    buf.clone()
                };
                if raw.len() >= spectrum_channels {
                    let total = raw.len() - (raw.len() % spectrum_channels);
                    let mono_frames = total / spectrum_channels;
                    let mut mono = Vec::with_capacity(mono_frames);
                    for i in 0..mono_frames {
                        let base = i * spectrum_channels;
                        let sum: f32 = raw[base..base + spectrum_channels].iter().sum();
                        mono.push(sum / spectrum_channels as f32);
                    }
                    let normalized = analyzer.analyze(&mono);
                    crate::audio::engine::types::update_global_spectrum(normalized);
                }
            }
        })
        .unwrap();

    let queue_for_decode = audio_queue.clone();
    let mut decode_count: u64 = 0;

    loop {
        if should_stop.load(Ordering::Relaxed) {
            debug!("[stream] Stop signal received, exiting loop");
            break;
        }

        match format.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let mut samples: Vec<f32> = Vec::new();
                    audio_buf.copy_to_vec_interleaved(&mut samples);

                    let decoded_rate = audio_buf.spec().rate();
                    if decoded_rate != output_sample_rate {
                        samples = resample_interleaved(
                            &samples,
                            decoded_rate,
                            output_sample_rate,
                            channels,
                        );
                    }

                    queue_for_decode.lock().extend(samples);
                    decode_count += 1;
                    if decode_count % 100 == 0 {
                        info!(
                            "[stream] Decoded {} packets, queue: {} samples, played: {}",
                            decode_count,
                            queue_for_decode.lock().len(),
                            samples_played.load(Ordering::Relaxed),
                        );
                    }
                }
                Err(e) => {
                    debug!("[stream] Decode error: {}", e);
                    thread::sleep(Duration::from_millis(10));
                }
            },
            Ok(Some(_)) => continue,
            Ok(None) => {
                info!("[stream] Stream ended after {} packets", decode_count);
                break;
            }
            Err(e) => {
                debug!("[stream] Packet error: {}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    let total_samples =
        samples_played.load(Ordering::Relaxed) + queue_for_decode.lock().len() as u64;
    if total_duration_ms.load(Ordering::Relaxed) == 0 && total_samples > 0 {
        let estimated = (total_samples * 1000) / (sample_rate as u64 * channels as u64);
        if estimated > 0 {
            info!("[stream] Estimated duration: {} ms", estimated);
            total_duration_ms.store(estimated, Ordering::Relaxed);
        }
    }

    info!(
        "[stream] Decode complete, draining queue ({} samples queued, {} played)",
        queue_for_decode.lock().len(),
        samples_played.load(Ordering::Relaxed),
    );

    // Keep the CPAL stream alive until the queue drains or stop is signaled.
    loop {
        if should_stop.load(Ordering::Relaxed) {
            break;
        }
        let remaining = queue_for_decode.lock().len();
        if remaining == 0 {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    info!(
        "[stream] Playback thread ending ({} packets decoded, {} samples played)",
        decode_count,
        samples_played.load(Ordering::Relaxed),
    );
    buffer_ready.store(false, Ordering::Relaxed);
    is_playing_flag.store(false, Ordering::Relaxed);
}

#[cfg(not(target_os = "android"))]
pub fn play_stream_internal(
    url: String,
    client: Arc<reqwest::blocking::Client>,
    audio_queue: crate::audio::stream::cpal_source::AudioBuffer,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
    seek_target_ms: Arc<AtomicU64>,
    seek_byte_offset: u64,
) {
    debug!("[stream] Starting stream playback: {}", url);

    // Check if it's a YouTube URL
    let video_id = if url.contains("youtube.com/watch?v=") {
        Some(
            url.split("v=")
                .nth(1)
                .unwrap_or("")
                .split('&')
                .next()
                .unwrap_or(""),
        )
    } else if url.contains("youtu.be/") {
        Some(
            url.split("youtu.be/")
                .nth(1)
                .unwrap_or("")
                .split('?')
                .next()
                .unwrap_or(""),
        )
    } else {
        None
    };

    if let Some(id) = video_id {
        debug!("[stream] YouTube video ID detected: {}", id);
        let mut yt_service = crate::youtube::YouTubeService::new();
        match crate::youtube::get_audio_stream_url(&mut yt_service, id) {
            Ok(stream_url) => {
                debug!("[stream] YouTube stream URL: {}", stream_url);
                let yt_client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(15))
                    .http1_only()
                    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/96.0.4664.18 Safari/537.36")
                    .default_headers({
                        let mut headers = reqwest::header::HeaderMap::new();
                        headers.insert(reqwest::header::REFERER, "https://www.youtube.com".parse().unwrap());
                        headers.insert("Origin", "https://www.youtube.com".parse().unwrap());
                        headers.insert("Sec-Fetch-Mode", "navigate".parse().unwrap());
                        headers.insert("Accept", "audio/*, text/plain, application/octet-stream".parse().unwrap());
                        headers
                    })
                    .build()
                    .unwrap_or_else(|_| client.as_ref().clone());

                let mut request = yt_client.get(&stream_url);
                if seek_byte_offset > 0 {
                    debug!(
                        "[stream] Seek: requesting Range: bytes={}-",
                        seek_byte_offset
                    );
                    request = request.header("Range", format!("bytes={}-", seek_byte_offset));
                    seek_target_ms.store(0, Ordering::Relaxed);
                }
                let response = match request.send() {
                    Ok(resp) => {
                        let status = resp.status();
                        let headers = resp.headers();
                        debug!("[stream] Response status: {}", status);
                        debug!(
                            "[stream] Content-Type: {:?}",
                            headers.get(reqwest::header::CONTENT_TYPE)
                        );
                        debug!(
                            "[stream] Content-Length: {:?}",
                            headers.get(reqwest::header::CONTENT_LENGTH)
                        );
                        resp
                    }
                    Err(e) => {
                        let err_msg = format!("Failed to connect to YouTube stream: {}", e);
                        debug!("[stream] {}", err_msg);
                        let mut source: &dyn std::error::Error = &e;
                        while let Some(cause) = source.source() {
                            debug!("[stream]   cause: {}", cause);
                            source = cause;
                        }
                        *load_error.lock() = err_msg;
                        return;
                    }
                };

                if !response.status().is_success() {
                    let err_msg = format!("HTTP error: {}", response.status());
                    debug!("[stream] {}", err_msg);
                    *load_error.lock() = err_msg;
                    return;
                }

                decode_and_play_from_read(
                    Box::new(response),
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
            }
            Err(e) => {
                let err_msg = format!("Failed to get YouTube stream URL: {}", e);
                debug!("[stream] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        }
    } else {
        // Not a YouTube URL, play as a regular stream
        let yt_client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .http1_only()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/96.0.4664.18 Safari/537.36")
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(reqwest::header::REFERER, "https://www.youtube.com".parse().unwrap());
                headers.insert("Origin", "https://www.youtube.com".parse().unwrap());
                headers.insert("Sec-Fetch-Mode", "navigate".parse().unwrap());
                headers.insert("Accept", "audio/*, text/plain, application/octet-stream".parse().unwrap());
                headers
            })
            .build()
            .unwrap_or_else(|_| client.as_ref().clone());

        let mut request = yt_client.get(&url);
        if seek_byte_offset > 0 {
            debug!(
                "[stream] Seek: requesting Range: bytes={}-",
                seek_byte_offset
            );
            request = request.header("Range", format!("bytes={}-", seek_byte_offset));
            seek_target_ms.store(0, Ordering::Relaxed);
        }
        let response = match request.send() {
            Ok(resp) => {
                let status = resp.status();
                let headers = resp.headers();
                debug!("[stream] Response status: {}", status);
                debug!(
                    "[stream] Content-Type: {:?}",
                    headers.get(reqwest::header::CONTENT_TYPE)
                );
                debug!(
                    "[stream] Content-Length: {:?}",
                    headers.get(reqwest::header::CONTENT_LENGTH)
                );
                resp
            }
            Err(e) => {
                let err_msg = format!("Failed to connect: {}", e);
                debug!("[stream] {}", err_msg);
                let mut source: &dyn std::error::Error = &e;
                while let Some(cause) = source.source() {
                    debug!("[stream]   cause: {}", cause);
                    source = cause;
                }
                *load_error.lock() = err_msg;
                return;
            }
        };

        if !response.status().is_success() {
            let err_msg = format!("HTTP error: {}", response.status());
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }

        decode_and_play_from_read(
            Box::new(response),
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
    }
}

#[cfg(not(target_os = "android"))]
pub fn play_stream_from_pipe_internal(
    reader: crate::audio::stream::pipe::PipeReader,
    audio_queue: crate::audio::stream::cpal_source::AudioBuffer,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
) {
    decode_and_play_from_read(
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

#[cfg(not(target_os = "android"))]
pub fn play_adaptive_buffer_internal(
    _pipe_writer: Arc<crate::audio::stream::pipe::PipeWriter>,
    audio_queue: crate::audio::stream::cpal_source::AudioBuffer,
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
    use std::path::Path;

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

    let _cache_file_name = format!(
        "{}.cache",
        url.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_")
    );

    let (stream_url, _client): (String, reqwest::blocking::Client) =
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

            for (i, a) in manifest.audio.iter().enumerate() {
                info!(
                    "[adaptive_buffer] Audio[{}]: itag={}, mime={}, bitrate={}, url_len={}",
                    i,
                    a.itag,
                    a.mime_type,
                    a.bitrate,
                    a.url.len()
                );
            }

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

    let pipe_reader =
        match crate::audio::stream::stream_download::StreamDownloader::fetch_stream(&stream_url, 0)
        {
            Ok((reader, _len)) => reader,
            Err(e) => {
                let err_msg = format!("Failed to start stream download: {}", e);
                error!("[adaptive_buffer] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };

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
}

#[cfg(target_os = "android")]
pub fn play_adaptive_buffer_internal(/* ... */) {
    panic!("play_adaptive_buffer_internal is Android-specific and should be in android_file_decoder.rs");
}

#[cfg(target_os = "android")]
pub fn play_stream_internal(/* ... */) {
    panic!("play_stream_internal is Android-specific and should be in android_file_decoder.rs");
}

#[cfg(target_os = "android")]
pub fn play_stream_from_pipe_internal(/* ... */) {
    panic!("play_stream_from_pipe_internal is Android-specific and should be in android_file_decoder.rs");
}
