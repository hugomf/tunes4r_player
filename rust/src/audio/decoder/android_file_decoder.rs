//! Android audio playback using cpal direct with symphonia for decode.
//!
//! Mirrors the macOS/iOS decode path: a producer (symphonia decode loop)
//! pushes interleaved f32 samples into a shared ring buffer, and a cpal
//! output stream callback drains it to the device. No rodio, no
//! QueueSource, no mixer indirection.

#[cfg(target_os = "android")]
use parking_lot::Mutex;
#[cfg(target_os = "android")]
use std::collections::VecDeque;
#[cfg(target_os = "android")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(target_os = "android")]
use std::sync::Arc;
#[cfg(target_os = "android")]
use std::thread;
#[cfg(target_os = "android")]
use std::time::Duration;

#[cfg(target_os = "android")]
use cpal::traits::{HostTrait, StreamTrait};
#[cfg(target_os = "android")]
use log::{error, info, LevelFilter};
#[cfg(target_os = "android")]
use reqwest::Client as AsyncClient;
#[cfg(target_os = "android")]
use std::io::Read;
#[cfg(target_os = "android")]
use symphonia::core::codecs::audio::AudioDecoderOptions;
#[cfg(target_os = "android")]
use symphonia::core::formats::probe::Hint;
#[cfg(target_os = "android")]
use symphonia::core::formats::FormatOptions;
#[cfg(target_os = "android")]
use symphonia::core::formats::FormatReader;
#[cfg(target_os = "android")]
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
#[cfg(target_os = "android")]
use symphonia::core::meta::MetadataOptions;
#[cfg(target_os = "android")]
use symphonia::core::units::{TimeBase, Timestamp};

#[cfg(target_os = "android")]
use crate::audio::stream::cpal_source::{build_output_stream, pick_output_config};
#[cfg(target_os = "android")]
use crate::dsp::RmsSpectrumAnalyzer;

#[cfg(target_os = "android")]
use crate::audio::decoder::seek::{seek_to_position, SeekMethod};
#[cfg(target_os = "android")]
use crate::audio::stream::cpal_source::AudioBuffer;

#[cfg(not(target_os = "android"))]
use crate::audio::stream::cpal_source::AudioBuffer;

#[cfg(not(target_os = "android"))]
use log::warn;

#[cfg(not(target_os = "android"))]
use std::sync::Arc;

#[cfg(target_os = "android")]
pub fn get_codec_registry() -> symphonia::core::codecs::registry::CodecRegistry {
    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

#[cfg(target_os = "android")]
static JVM_HANDLE: std::sync::OnceLock<jni::JavaVM> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
fn attach_current_thread_to_jvm() -> Option<jni::AttachGuard<'static>> {
    use jni::JavaVM;

    let ctx = ndk_context::android_context();
    let vm_ptr = ctx.vm() as *mut jni::sys::JavaVM;

    if vm_ptr.is_null() {
        error!("[audio] JVM pointer is null");
        return None;
    }

    unsafe {
        let jvm: &'static JavaVM = JVM_HANDLE.get_or_init(|| {
            JavaVM::from_raw(vm_ptr).expect("Failed to create JavaVM from raw pointer")
        });
        match jvm.attach_current_thread() {
            Ok(guard) => {
                info!("[audio] JVM attached to current thread");
                Some(guard)
            }
            Err(e) => {
                error!("[audio] Failed to attach JVM: {:?}", e);
                None
            }
        }
    }
}

#[cfg(target_os = "android")]
fn extract_duration(
    format: &mut dyn symphonia::core::formats::FormatReader,
    time_base: TimeBase,
) -> u64 {
    let mut duration_ms: u64 = 0;

    if let Some(media_duration) = format.media_info().duration {
        let ts = Timestamp::new(media_duration.get() as i64);
        let time = time_base.calc_time(ts).unwrap_or_default();
        duration_ms = time.as_millis() as u64;
    }

    if duration_ms == 0 {
        for track in format.tracks() {
            if let Some(track_duration) = track.duration {
                let ts = Timestamp::new(track_duration.get() as i64);
                let time = time_base.calc_time(ts).unwrap_or_default();
                duration_ms = time.as_millis() as u64;
                break;
            }
        }
    }

    duration_ms
}

#[cfg(target_os = "android")]
fn probe_audio_duration(bytes: &[u8], _len: usize, _sample_rate: u64) -> Option<u64> {
    let cursor = std::io::Cursor::new(bytes);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .ok()?;

    let track = format.default_track(symphonia::core::formats::TrackType::Audio)?;

    let codec_params = track.codec_params.as_ref()?;
    if !matches!(
        codec_params,
        symphonia::core::codecs::CodecParameters::Audio(_)
    ) {
        return None;
    }

    let sample_rate = match codec_params {
        symphonia::core::codecs::CodecParameters::Audio(params) => {
            params.sample_rate.unwrap_or(44100)
        }
        _ => 44100,
    };
    let time_base = track
        .time_base
        .unwrap_or_else(|| TimeBase::try_from_recip(sample_rate).unwrap_or_default());

    Some(extract_duration(&mut *format, time_base))
}

#[cfg(target_os = "android")]
pub fn play_file_internal(
    path: String,
    audio_queue: AudioBuffer,
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
    info!("[file] play_file_internal: {}", path);
    android_logger::init_once(android_logger::Config::default().with_max_level(LevelFilter::Info));

    let _jvm = attach_current_thread_to_jvm();

    let file_data = match std::fs::read(&path) {
        Ok(data) => {
            info!("[file] File read successfully: {} bytes", data.len());
            data
        }
        Err(e) => {
            let err_msg = format!("Failed to read file: {}", e);
            error!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let duration_ms = probe_audio_duration(&file_data, file_data.len(), 44100).unwrap_or(0);
    total_duration_ms.store(duration_ms, Ordering::Relaxed);

    // Reusable helpers to create format reader + decoder from the in-memory bytes.
    let build_pipeline = || -> Result<
        (
            Box<dyn symphonia::core::formats::FormatReader>,
            Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
            u32,
            u16,
            u32,
            symphonia::core::codecs::audio::AudioCodecParameters,
        ),
        String,
    > {
        let cursor = std::io::Cursor::new(file_data.clone());
        let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

        let format: Box<dyn FormatReader> = symphonia::default::get_probe()
            .probe(
                &Hint::new(),
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| format!("Format detection failed: {}", e))?;

        let track = format
            .default_track(symphonia::core::formats::TrackType::Audio)
            .ok_or_else(|| "No audio track found".to_string())?;

        let track_id = track.id;
        let codec_params = match &track.codec_params {
            Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
            _ => return Err("No audio codec params".to_string()),
        };

        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let ch = codec_params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .unwrap_or(2);

        let decoder = get_codec_registry()
            .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())
            .map_err(|e| format!("Decoder creation failed: {}", e))?;

        Ok((format, decoder, track_id, ch, sample_rate, codec_params))
    };

    // ── Outer loop: restarts playback on seek ──────────────────────────
    'outer: loop {
        let (mut format, mut decoder, track_id, channels, sample_rate, codec_params) =
            match build_pipeline() {
                Ok(p) => p,
                Err(e) => {
                    *load_error.lock() = e;
                    return;
                }
            };

        sample_rate_out.store(sample_rate as u64, Ordering::Relaxed);
        channels_out.store(channels as u64, Ordering::Relaxed);

        // --- SEEK LOGIC: delegate to the unified seek module shared with
        // macOS and iOS. This runs BEFORE the prebuffer so the queue is
        // filled with samples starting at the seek target, not at time 0.
        let seek_pos_ms = seek_target_ms.load(Ordering::Relaxed);
        if seek_pos_ms > 0 {
            seek_target_ms.store(0, Ordering::Relaxed);
            info!("[file] Seek target: {} ms", seek_pos_ms);
            match seek_to_position(
                &mut format,
                &codec_params,
                track_id,
                seek_pos_ms,
                &should_stop,
            ) {
                Ok(outcome) => {
                    if outcome.method == SeekMethod::Native {
                        decoder = match get_codec_registry()
                            .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())
                        {
                            Ok(d) => d,
                            Err(e) => {
                                *load_error.lock() = format!("Decoder reset failed: {}", e);
                                return;
                            }
                        };
                        info!("[file] Decoder reset after native seek");
                    }
                }
                Err(e) => {
                    error!("[file] Seek failed: {}", e);
                }
            }
        }

        let prebuffer_secs = 7u64;
        let target_buffer_samples = (sample_rate as u64 * prebuffer_secs) as usize * channels as usize;
        info!(
            "[file] Pre-buffering {} samples ({} seconds)...",
            target_buffer_samples, prebuffer_secs
        );

        let queue_for_decode = audio_queue.clone();
        queue_for_decode.lock().clear();

        let mut prebuffer_error_count = 0;
    const MAX_PACKET_ERRORS: u32 = 300;
        let mut buffered_samples = 0;

        while buffered_samples < target_buffer_samples {
            if should_stop.load(Ordering::Relaxed) {
                return;
            }

            match format.next_packet() {
                Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet)
                {
                    Ok(audio_buf) => {
                        prebuffer_error_count = 0;
                        let mut samples: Vec<f32> = Vec::new();
                        audio_buf.copy_to_vec_interleaved(&mut samples);
                        buffered_samples += samples.len();
                        queue_for_decode.lock().extend(samples);
                    }
                    Err(e) => {
                        error!("[file] Decode error: {}", e);
                        prebuffer_error_count += 1;
                        if prebuffer_error_count >= MAX_PACKET_ERRORS {
                            let err_msg =
                                format!("Too many decode errors ({}): {}", prebuffer_error_count, e);
                            error!("[file] {}", err_msg);
                            *load_error.lock() = err_msg;
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                },
                Ok(Some(_)) => continue,
                Ok(None) => {
                    info!("[file] Stream exhausted during pre-buffering");
                    break;
                }
                Err(e) => {
                    error!("[file] Packet error: {}", e);
                    prebuffer_error_count += 1;
                    if prebuffer_error_count >= MAX_PACKET_ERRORS {
                        let err_msg =
                            format!("Too many packet errors ({}): {}", prebuffer_error_count, e);
                        error!("[file] {}", err_msg);
                        *load_error.lock() = err_msg;
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }

        info!("[file] Pre-buffer complete: {} samples", buffered_samples);

        // --- Build cpal output stream (replaces rodio Player + QueueSource).
        // The callback drains the shared ring buffer; no mixer indirection,
        // no rodio Player lifecycle to manage.
        let host = cpal::default_host();
        let device = match host.default_output_device() {
            Some(d) => d,
            None => {
                let err_msg = "No output device".to_string();
                error!("[file] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };
    let config = match pick_output_config(&device) {
            Some(c) => c,
            None => {
                let err_msg = "No suitable output config".to_string();
                error!("[file] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };

        // Use the device's actual output sample rate for all timing.
        let device_sample_rate = config.sample_rate;
        sample_rate_out.store(device_sample_rate as u64, Ordering::Relaxed);
        channels_out.store(config.channels as u64, Ordering::Relaxed);
        info!("[file] Device output: {} Hz, {} ch", device_sample_rate, config.channels);

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
                error!("[file] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };

        if let Err(e) = stream.play() {
            let err_msg = format!("Failed to start output stream: {}", e);
            error!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
        buffer_ready.store(true, Ordering::Relaxed);
        is_playing_flag.store(true, Ordering::Relaxed);
        info!("[file] Output stream started");

        let band_count = crate::audio::engine::get_band_count();
        let mut analyzer = RmsSpectrumAnalyzer::new(device_sample_rate, band_count);
        crate::audio::engine::update_global_spectrum(vec![0.1f32; band_count]);

        // ── Decode loop (synchronous, same thread) ──
        // Cap the decode queue at 10 seconds of device-rate audio to
        // prevent unbounded growth.
        let max_queue_samples = (device_sample_rate as usize) * 10 * (config.channels as usize);
        let mut packet_error_count = 0;
        let mut spectrum_accum: VecDeque<f32> = VecDeque::with_capacity(4096);
        let mut last_spectrum_update = std::time::Instant::now();

        loop {
            if should_stop.load(Ordering::Relaxed) {
                info!("[file] Stop requested during decode");
                break;
            }

            if seek_target_ms.load(Ordering::Acquire) > 0 {
                info!("[file] Seek requested during playback, stopping decode");
                break;
            }

            match format.next_packet() {
                Ok(Some(packet)) if packet.track_id == track_id => {
                    packet_error_count = 0;
                    match decoder.decode(&packet) {
                        Ok(audio_buf) => {
                            let mut samples: Vec<f32> = Vec::new();
                            audio_buf.copy_to_vec_interleaved(&mut samples);

                            for &s in &samples {
                                spectrum_accum.push_back(s);
                            }

                            // Back-pressure: wait until queue has room.
                            // Also breaks on seek/stop so we never block a seek.
                            loop {
                                let full = queue_for_decode.lock().len() + samples.len() > max_queue_samples;
                                if !full { break; }
                                if should_stop.load(Ordering::Relaxed) { break; }
                                if seek_target_ms.load(Ordering::Acquire) > 0 { break; }
                                thread::sleep(Duration::from_millis(5));
                            }
                            if !should_stop.load(Ordering::Relaxed)
                                && seek_target_ms.load(Ordering::Relaxed) == 0
                            {
                                queue_for_decode.lock().extend(samples);
                            }

                            if last_spectrum_update.elapsed().as_millis() >= 100
                                && spectrum_accum.len() >= channels as usize
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
                            error!("[file] Decode error: {}", e);
                            packet_error_count += 1;
                            if packet_error_count >= MAX_PACKET_ERRORS {
                                let err_msg = format!(
                                    "Too many decode errors ({}): {}",
                                    packet_error_count, e
                                );
                                error!("[file] {}", err_msg);
                                *load_error.lock() = err_msg;
                                break;
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
                Ok(Some(_)) => continue,
                Ok(None) => {
                    info!("[file] End of file reached");
                    break;
                }
                Err(e) => {
                    packet_error_count += 1;
                    error!("[file] Packet error ({}): {}", packet_error_count, e);
                    if packet_error_count >= MAX_PACKET_ERRORS {
                        let err_msg = format!(
                            "Too many packet errors ({}): {}",
                            packet_error_count, e
                        );
                        error!("[file] {}", err_msg);
                        *load_error.lock() = err_msg;
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
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
        break 'outer;
    }
}

#[cfg(target_os = "android")]
pub fn decode_and_play_from_read(
    reader: Box<dyn Read + Send + Sync + 'static>,
    audio_queue: AudioBuffer,
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
    info!("[stream] Connected! Creating streaming reader...");

    let source = ReadOnlySource::new(reader);
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    info!("[stream] Probing format...");
    let probed = match symphonia::default::get_probe().probe(
        &Hint::new(),
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(e) => {
            let err_msg = format!("Format detection failed: {}", e);
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track = match probed.first_track(symphonia::core::formats::TrackType::Audio) {
        Some(t) => t,
        None => {
            let err_msg = "No audio track found".to_string();
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track_id = track.id;
    let codec_params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            let err_msg = "No audio codec params".to_string();
            error!("[stream] {}", err_msg);
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
    info!("[stream] Stream: {} Hz, {} channels", sample_rate, channels);
    sample_rate_out.store(sample_rate as u64, Ordering::Relaxed);
    channels_out.store(channels as u64, Ordering::Relaxed);

    // Probe actual duration from format metadata
    let duration_ms = probed
        .first_track(symphonia::core::formats::TrackType::Audio)
        .and_then(|t| {
            let time_base = t.time_base?;
            let duration = probed.media_info().duration?;
            let ts = symphonia::core::units::Timestamp::new(duration.get() as i64);
            let time = time_base.calc_time(ts)?;
            Some(time.as_millis() as u64)
        })
        .unwrap_or_else(|| {
            let estimated = (sample_rate as u64 * 3600) / 1000;
            info!(
                "[stream] No duration metadata, estimated: {} ms",
                estimated
            );
            estimated
        });
    total_duration_ms.store(duration_ms, Ordering::Relaxed);

    let target_buffer_secs = 7.0;
    let target_buffer_samples = (sample_rate as f32 * target_buffer_secs) as usize * channels;

    // Seek — merged into prebuffer to avoid double-decoding.
    // Uses the same shared seek module as the file path, so seek
    // behavior is consistent across all three platforms (and the iOS
    // off-by-channels unit bug is structurally impossible here).
    let target_ms = seek_target_ms.load(Ordering::Relaxed);

    let mut buffered_samples = 0;
    let mut prebuffer_error_count = 0;
    let mut last_logged_pct = 0u32;
    const MAX_PACKET_ERRORS: u32 = 30;
    let mut format_reader = probed;
    let mut decoder = match get_codec_registry().make_audio_decoder(
        &codec_params,
        &symphonia::core::codecs::audio::AudioDecoderOptions::default(),
    ) {
        Ok(d) => d,
        Err(e) => {
            let err_msg = format!("Decoder creation failed: {}", e);
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let residual_samples_to_skip = if target_ms > 0 {
        seek_target_ms.store(0, Ordering::Relaxed);
        info!("[stream] Seek target: {} ms", target_ms);
        match seek_to_position(
            &mut format_reader,
            &codec_params,
            track_id,
            target_ms,
            &should_stop,
        ) {
            Ok(outcome) => {
                if outcome.method == SeekMethod::Native {
                    decoder = match get_codec_registry()
                        .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())
                    {
                        Ok(d) => d,
                        Err(e) => {
                            *load_error.lock() = format!("Decoder reset failed: {}", e);
                            return;
                        }
                    };
                    info!("[stream] Decoder reset after native seek");
                }
                info!(
                    "[stream] Seek complete: {:?}, residual {} samples",
                    outcome.method, outcome.residual_samples_to_skip
                );
                outcome.residual_samples_to_skip
            }
            Err(e) => {
                let err_msg = format!("Seek failed: {}", e);
                error!("[stream] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        }
    } else {
        0
    };
    let mut samples_to_skip = residual_samples_to_skip;

    info!(
        "[stream] Pre-buffering {} samples ({} seconds)...",
        target_buffer_samples, target_buffer_secs
    );

    let queue_for_decode = audio_queue.clone();
    // cpal output stream (replaces rodio Player + QueueSource + sink).
    // The callback drains the shared ring buffer; no mixer indirection.
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            let err_msg = "No output device".to_string();
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };
    let config = match pick_output_config(&device) {
        Some(c) => c,
        None => {
            let err_msg = "No suitable output config".to_string();
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };
    let device_sample_rate = config.sample_rate;
    let device_channels = config.channels;
    sample_rate_out.store(device_sample_rate as u64, Ordering::Relaxed);
    channels_out.store(device_channels as u64, Ordering::Relaxed);
    info!(
        "[stream] Device output: {} Hz, {} ch (codec: {} Hz, {} ch)",
        device_sample_rate, device_channels, sample_rate, channels
    );

    // Recalculate prebuffer target using device rate.
    let target_buffer_samples = (device_sample_rate as f32 * target_buffer_secs) as usize * device_channels as usize;

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
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };
    if let Err(e) = stream.play() {
        let err_msg = format!("Failed to start output stream: {}", e);
        error!("[stream] {}", err_msg);
        *load_error.lock() = err_msg;
        return;
    }
    info!(
        "[stream] cpal output stream started ({} Hz, {} ch)",
        config.sample_rate, config.channels
    );

    while buffered_samples < target_buffer_samples {
        if should_stop.load(Ordering::Relaxed) {
            info!("[stream] Stop requested during pre-buffering");
            return;
        }

        match format_reader.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    prebuffer_error_count = 0;
                    let mut samples: Vec<f32> = Vec::new();
                    audio_buf.copy_to_vec_interleaved(&mut samples);

                    // Fast-forward through seek samples
                    if samples_to_skip > 0 {
                        let n = samples.len() as u64;
                        if samples_to_skip >= n {
                            samples_to_skip -= n;
                            continue;
                        }
                        samples.drain(0..samples_to_skip as usize);
                        samples_to_skip = 0;
                    }

                    buffered_samples += samples.len();
                    queue_for_decode.lock().extend(samples);
                }
                Err(e) => {
                    error!("[stream] Decode error: {}", e);
                    prebuffer_error_count += 1;
                    if prebuffer_error_count >= MAX_PACKET_ERRORS {
                        let err_msg =
                            format!("Too many decode errors ({}): {}", prebuffer_error_count, e);
                        error!("[stream] {}", err_msg);
                        *load_error.lock() = err_msg;
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            },
            Ok(Some(_)) => continue,
            Ok(None) => {
                // Stream is shorter than the prebuffer window — that's fine,
                // just proceed with what we have.
                info!(
                    "[stream] Stream exhausted during pre-buffering after {} samples, proceeding",
                    buffered_samples
                );
                break;
            }
            Err(e) => {
                error!("[stream] Packet error: {}", e);
                prebuffer_error_count += 1;
                if prebuffer_error_count >= MAX_PACKET_ERRORS {
                    let err_msg =
                        format!("Too many packet errors ({}): {}", prebuffer_error_count, e);
                    error!("[stream] {}", err_msg);
                    *load_error.lock() = err_msg;
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }

        let pct = (buffered_samples as f64 / target_buffer_samples as f64 * 100.0) as u32;
        let threshold = pct - (pct % 25);
        if threshold > last_logged_pct {
            info!(
                "[stream] Pre-buffered {}%: {} samples",
                threshold, buffered_samples
            );
            last_logged_pct = threshold;
        }
    }

    info!("[stream] Pre-buffer complete: {} samples", buffered_samples);

    buffer_ready.store(true, Ordering::Relaxed);
    is_playing_flag.store(true, Ordering::Relaxed);
    info!("[stream] buffer_ready and is_playing set to true");

    info!("[stream] Audio stream started!");

    crate::audio::engine::update_global_spectrum(vec![
        0.1f32;
        crate::audio::engine::get_band_count()
    ]);

    // ── Decode loop (synchronous, same thread) ──
    // Cap the decode queue at 10 seconds of device-rate audio.
    let max_queue_samples = (device_sample_rate as usize) * 10 * (device_channels as usize);
    let mut packet_error_count = 0;
    let band_count = crate::audio::engine::get_band_count();
    let mut analyzer = RmsSpectrumAnalyzer::new(device_sample_rate, band_count);
    let mut spectrum_accum: VecDeque<f32> = VecDeque::with_capacity(4096);
    let mut last_spectrum_update = std::time::Instant::now();
    let mut decode_count: u64 = 0;

    loop {
        if should_stop.load(Ordering::Relaxed) {
            info!("[stream] Stop requested during decode");
            break;
        }

        if seek_target_ms.load(Ordering::Acquire) > 0 {
            info!("[stream] Seek requested during playback, stopping decode");
            break;
        }

        match format_reader.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => {
                packet_error_count = 0;
                match decoder.decode(&packet) {
                    Ok(audio_buf) => {
                        let mut samples: Vec<f32> = Vec::new();
                        audio_buf.copy_to_vec_interleaved(&mut samples);

                        for &s in &samples {
                            spectrum_accum.push_back(s);
                        }

                        // Back-pressure: wait until queue has room.
                        // Also breaks on seek/stop so we never block a seek.
                        loop {
                            let full = queue_for_decode.lock().len() + samples.len() > max_queue_samples;
                            if !full { break; }
                            if should_stop.load(Ordering::Relaxed) { break; }
                            if seek_target_ms.load(Ordering::Acquire) > 0 { break; }
                            thread::sleep(Duration::from_millis(5));
                        }
                        if !should_stop.load(Ordering::Relaxed)
                            && seek_target_ms.load(Ordering::Relaxed) == 0
                        {
                            queue_for_decode.lock().extend(samples);
                        }

                        if last_spectrum_update.elapsed().as_millis() >= 100
                            && spectrum_accum.len() >= channels
                        {
                            let channels_usize = channels;
                            let total =
                                spectrum_accum.len() - (spectrum_accum.len() % channels_usize);
                            let count = total.min(4096);
                            let raw: Vec<f32> = spectrum_accum.drain(..count).collect();
                            let mono_frames = raw.len() / channels_usize;
                            let mut mono = Vec::with_capacity(mono_frames);
                            for ch in 0..mono_frames {
                                let base = ch * channels_usize;
                                let sum: f32 = raw[base..base + channels_usize].iter().sum();
                                mono.push(sum / channels_usize as f32);
                            }
                            let normalized = analyzer.analyze(&mono);
                            crate::audio::engine::update_global_spectrum(normalized);
                            last_spectrum_update = std::time::Instant::now();
                        }

                        decode_count += 1;
                    }
                    Err(e) => {
                        error!("[stream] Decode error: {}", e);
                        packet_error_count += 1;
                        if packet_error_count >= MAX_PACKET_ERRORS {
                            let err_msg = format!(
                                "Too many decode errors ({}): {}",
                                packet_error_count, e
                            );
                            error!("[stream] {}", err_msg);
                            *load_error.lock() = err_msg;
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => {
                info!("[stream] Stream ended after {} packets", decode_count);
                break;
            }
            Err(e) => {
                packet_error_count += 1;
                error!("[stream] Packet error ({}): {}", packet_error_count, e);
                if packet_error_count >= MAX_PACKET_ERRORS {
                    let err_msg =
                        format!("Too many packet errors ({}): {}", packet_error_count, e);
                    error!("[stream] {}", err_msg);
                    *load_error.lock() = err_msg;
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    // ── Wait for the cpal output stream to drain the remaining queue
    // (no QueueSource to signal — just empty the buffer or stop).
    info!("[stream] Decode complete, waiting for output drain");
    let drain_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !should_stop.load(Ordering::Relaxed) {
        if queue_for_decode.lock().is_empty() {
            info!("[stream] Queue empty, done draining");
            break;
        }
        if std::time::Instant::now() > drain_deadline {
            error!("[stream] Drain timeout — queue still has {} samples", queue_for_decode.lock().len());
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    info!("[stream] Playback complete");
    drop(stream);
    buffer_ready.store(false, Ordering::Relaxed);
    is_playing_flag.store(false, Ordering::Relaxed);
}

#[cfg(target_os = "android")]
pub fn play_stream_internal(
    url: String,
    _client: Arc<AsyncClient>,
    audio_queue: AudioBuffer,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
    seek_target_ms: Arc<AtomicU64>,
    seek_byte_offset: u64,
) {
    android_logger::init_once(android_logger::Config::default().with_max_level(LevelFilter::Info));

    let _jvm = attach_current_thread_to_jvm();

    info!(
        "[stream] Starting stream playback (Android with async HTTP streaming): {}",
        url
    );

    let (pipe_writer, pipe_reader) = crate::audio::stream::pipe::new_pipe();
    let pipe_writer = Arc::new(pipe_writer);

    let http_url = url.clone();
    let http_seek_offset = seek_byte_offset;
    let pipe_writer_clone = pipe_writer.clone();
    thread::spawn(move || {
        let _jvm = attach_current_thread_to_jvm();
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async move {
            let client = AsyncClient::new();
            let mut request = client.get(&http_url);
            if http_seek_offset > 0 {
                info!(
                    "[stream] Seek: requesting Range: bytes={}-",
                    http_seek_offset
                );
                request = request.header("Range", format!("bytes={}-", http_seek_offset));
            }

            let mut resp = match request.send().await {
                Ok(r) => r,
                Err(e) => {
                    pipe_writer_clone.set_error(e.to_string());
                    return;
                }
            };

            if !resp.status().is_success() {
                pipe_writer_clone.set_error(format!("HTTP error: {}", resp.status()));
                return;
            }

            info!("[stream] Streaming response, starting chunked read...");
            loop {
                match resp.chunk().await {
                    Ok(Some(data)) => {
                        pipe_writer_clone.push(&data);
                    }
                    Ok(None) => {
                        info!("[stream] Stream ended");
                        pipe_writer_clone.end();
                        return;
                    }
                    Err(e) => {
                        pipe_writer_clone.set_error(e.to_string());
                        return;
                    }
                }
            }
        });
    });

    decode_and_play_from_read(
        Box::new(pipe_reader),
        audio_queue,
        buffer_ready,
        is_playing_flag,
        should_stop,
        samples_played,
        sample_rate_out,
        channels,
        total_duration_ms,
        load_error,
        seek_target_ms,
    );
}

#[cfg(target_os = "android")]
pub fn play_stream_from_pipe_internal(
    reader: crate::audio::stream::pipe::PipeReader,
    audio_queue: AudioBuffer,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
) {
    android_logger::init_once(android_logger::Config::default().with_max_level(LevelFilter::Info));

    let _jvm = attach_current_thread_to_jvm();

    info!("[stream] Starting pipe-based stream playback (Android with rodio)");

    decode_and_play_from_read(
        Box::new(reader),
        audio_queue,
        buffer_ready,
        is_playing_flag,
        should_stop,
        samples_played,
        sample_rate_out,
        channels,
        total_duration_ms,
        load_error,
        Arc::new(AtomicU64::new(0)),
    );
}

#[cfg(target_os = "android")]
pub fn play_stream_with_downloader_internal(
    url: String,
    audio_queue: AudioBuffer,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
    seek_target_ms: Arc<AtomicU64>,
    _seek_byte_offset: u64,
) {
    android_logger::init_once(android_logger::Config::default().with_max_level(LevelFilter::Info));

    let _jvm = attach_current_thread_to_jvm();

    info!("[stream] Starting stream playback with stream_download: {}", url);

    let pipe_reader = match crate::audio::stream::stream_download::StreamDownloader::fetch_stream(&url, 0) {
        Ok((reader, _len)) => reader,
        Err(e) => {
            let err_msg = format!("Failed to start stream download: {}", e);
            error!("[stream] {}", err_msg);
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
        channels,
        total_duration_ms,
        load_error,
        seek_target_ms,
    );
}

#[cfg(target_os = "android")]
pub fn play_adaptive_buffer_internal(
    _pipe_writer: Arc<crate::audio::stream::pipe::PipeWriter>,
    audio_queue: AudioBuffer,
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
    _cache_dir: String,
) {
    android_logger::init_once(android_logger::Config::default().with_max_level(LevelFilter::Info));

    let _jvm = attach_current_thread_to_jvm();

    info!(
        "[adaptive_buffer] Starting adaptive buffer playback (Android): {}",
        url
    );

    let stream_url = if url.contains("googlevideo.com") {
        info!("[adaptive_buffer] Direct CDN URL detected");
        url.clone()
    } else {
        let video_id = if url.contains("youtube.com/watch?v=") {
            url.split("v=").nth(1).unwrap_or("").split('&').next().unwrap_or("").to_string()
        } else if url.contains("youtu.be/") {
            url.split("youtu.be/").nth(1).unwrap_or("").split('?').next().unwrap_or("").to_string()
        } else if url.len() == 11 && url.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            url.clone()
        } else {
            info!("[adaptive_buffer] Cannot resolve URL on Android: {}", url);
            *load_error.lock() = "URL resolution not supported on Android for non-YouTube URLs".to_string();
            return;
        };

        info!("[adaptive_buffer] Resolved video_id: {}", video_id);
        let yt = crate::youtube::YouTube::new();
        let (manifest, _client) = match yt.videos().stream_with_client(&video_id) {
            Ok((m, c)) => (m, c),
            Err(e) => {
                let err_msg = format!("Failed to extract YouTube stream: {}", e);
                error!("[adaptive_buffer] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };
        let audio_format = match manifest.best_audio() {
            Some(a) => a,
            None => {
                *load_error.lock() = "No audio stream found".to_string();
                return;
            }
        };
        audio_format.url.clone()
    };

    info!("[adaptive_buffer] Stream URL: {}", stream_url);

    let pipe_reader = match crate::audio::stream::stream_download::StreamDownloader::fetch_stream(&stream_url, 0) {
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

#[cfg(not(target_os = "android"))]
pub fn play_adaptive_buffer_internal(
    _pipe_writer: Arc<crate::audio::stream::pipe::PipeWriter>,
    _audio_queue: AudioBuffer,
    _buffer_ready: Arc<std::sync::atomic::AtomicBool>,
    _is_playing_flag: Arc<std::sync::atomic::AtomicBool>,
    _should_stop: Arc<std::sync::atomic::AtomicBool>,
    _samples_played: Arc<std::sync::atomic::AtomicU64>,
    _sample_rate_out: Arc<std::sync::atomic::AtomicU64>,
    _channels_out: Arc<std::sync::atomic::AtomicU64>,
    _total_duration_ms: Arc<std::sync::atomic::AtomicU64>,
    load_error: Arc<parking_lot::Mutex<String>>,
    _seek_target_ms: Arc<std::sync::atomic::AtomicU64>,
    _url: String,
    _cache_dir: String,
) {
    warn!("[adaptive_buffer] Non-Android adaptive buffer not implemented in android_file_decoder");
    *load_error.lock() = "Non-Android adaptive buffer called on Android module".to_string();
}

#[cfg(not(target_os = "android"))]
pub fn play_file_internal(
    _path: String,
    _audio_queue: AudioBuffer,
    _buffer_ready: Arc<std::sync::atomic::AtomicBool>,
    _is_playing_flag: Arc<std::sync::atomic::AtomicBool>,
    _should_stop: Arc<std::sync::atomic::AtomicBool>,
    _samples_played: Arc<std::sync::atomic::AtomicU64>,
    _sample_rate_out: Arc<std::sync::atomic::AtomicU64>,
    _channels_out: Arc<std::sync::atomic::AtomicU64>,
    _total_duration_ms: Arc<std::sync::atomic::AtomicU64>,
    load_error: Arc<parking_lot::Mutex<String>>,
    _seek_target_ms: Arc<std::sync::atomic::AtomicU64>,
    _seek_byte_offset: u64,
) {
    warn!("[file] Non-Android playback not implemented in android_file_decoder");
    *load_error.lock() = "Non-Android playback called on Android module".to_string();
}

#[cfg(not(target_os = "android"))]
pub fn play_stream_internal(
    _url: String,
    _client: Arc<reqwest::blocking::Client>,
    _audio_queue: AudioBuffer,
    _buffer_ready: Arc<std::sync::atomic::AtomicBool>,
    _is_playing_flag: Arc<std::sync::atomic::AtomicBool>,
    _should_stop: Arc<std::sync::atomic::AtomicBool>,
    _samples_played: Arc<std::sync::atomic::AtomicU64>,
    _sample_rate_out: Arc<std::sync::atomic::AtomicU64>,
    _channels_out: Arc<std::sync::atomic::AtomicU64>,
    _total_duration_ms: Arc<std::sync::atomic::AtomicU64>,
    load_error: Arc<parking_lot::Mutex<String>>,
    _seek_target_ms: Arc<std::sync::atomic::AtomicU64>,
    _seek_byte_offset: u64,
) {
    warn!("[stream] Non-Android stream playback not implemented in android_file_decoder");
    *load_error.lock() = "Non-Android stream playback called on Android module".to_string();
}