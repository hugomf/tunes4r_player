//! Symphonia-based audio decoder for desktop platforms (non-Android)

use log::info;

use crate::audio::decoder::seek::seek_to_position;
use crate::audio::engine::{get_band_count, update_global_spectrum};
use crate::audio::stream::cpal_source::{build_output_stream, pick_output_config};
use crate::audio::stream::handling::resample_interleaved;
use crate::dsp::RmsSpectrumAnalyzer;
use cpal::traits::{HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::FormatReader;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{TimeBase, Timestamp};

pub fn extract_duration(
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

#[cfg(not(target_os = "android"))]
pub fn play_file_internal(
    path: String,
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

    let mut format: Box<dyn FormatReader> = match symphonia::default::get_probe().probe(
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

    let track = match format.default_track(symphonia::core::formats::TrackType::Audio) {
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

    let time_base = track
        .time_base
        .unwrap_or_else(|| TimeBase::try_from_recip(sample_rate).unwrap_or_default());
    let duration_ms = extract_duration(&mut *format, time_base);
    info!(
        "[file] File: {} Hz, {} channels, duration: {} ms",
        sample_rate, channels, duration_ms
    );
    sample_rate_out.store(sample_rate as u64, Ordering::Relaxed);
    channels_out.store(channels as u64, Ordering::Relaxed);
    total_duration_ms.store(duration_ms, Ordering::Relaxed);

    // --- SEEK LOGIC: delegate to unified seek module ---
    let seek_pos_ms = seek_target_ms.load(Ordering::Relaxed);
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
    }

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

    let config = match pick_output_config(&device) {
        Some(c) => c,
        None => {
            let err_msg = "No suitable output config".to_string();
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    // Use the device's actual output sample rate for all timing.
    let device_sample_rate = config.sample_rate;
    sample_rate_out.store(device_sample_rate as u64, Ordering::Relaxed);
    channels_out.store(config.channels as u64, Ordering::Relaxed);
    info!(
        "[file] Device output: {} Hz, {} ch (codec: {} Hz, {} ch)",
        device_sample_rate, config.channels, sample_rate, channels
    );

    // ── Seek application: init the position clock to the seek target ──
    // ExoPlayer-style: the position clock is `samples_played / rate / ch`.
    // When a seek is applied, we seed the clock at the seek target so the
    // position getter naturally returns the new value and advances from there
    // — no 0-snap, no client-side latch needed.
    if seek_pos_ms > 0 {
        let ch_out = config.channels as u64;
        let rate_out = device_sample_rate as u64;
        let initial_samples = (seek_pos_ms * rate_out * ch_out) / 1000;
        samples_played.store(initial_samples, Ordering::Relaxed);
    }

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
                let err_msg = format!("Decoder creation failed: {}", e);
                info!("[file] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };

    let pre_buffer_target = (device_sample_rate as usize * 7) / 10 * config.channels as usize;
    info!("[file] Pre-buffering {} samples...", pre_buffer_target);

    while audio_queue.lock().len() < pre_buffer_target {
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
                let decoded_rate = audio_buf.spec().rate();
                let mut samples: Vec<f32> = Vec::new();
                audio_buf.copy_to_vec_interleaved(&mut samples);
                if decoded_rate != device_sample_rate {
                    samples = resample_interleaved(
                        &samples, decoded_rate, device_sample_rate, config.channels as usize,
                    );
                }
                audio_queue.lock().extend(samples);
            }
            Err(e) => {
                info!("[file] Prebuffer decode error: {}", e);
            }
        }
    }

    info!(
        "[file] Pre-buffering complete. Queue size: {}",
        audio_queue.lock().len()
    );

    let stream = match build_output_stream(&device, &config, audio_queue.clone(), buffer_ready.clone(), samples_played.clone()) {
        Ok(s) => s,
        Err(e) => {
            let err_msg = format!("Failed to build stream: {}", e);
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    buffer_ready.store(true, Ordering::Relaxed);
    is_playing_flag.store(true, Ordering::Relaxed);
    info!("[file] Playback flags set before stream start");

    stream.play().expect("Failed to start stream");
    info!("[file] Audio stream started!");

    let band_count = get_band_count();
    let mut last_spectrum_update = std::time::Instant::now();

    // Real FFT spectrum analyzer (RMS per Bark band, matching SpectrumSource)
    let mut analyzer = RmsSpectrumAnalyzer::new(device_sample_rate, band_count);
    let mut spectrum_accum: VecDeque<f32> = VecDeque::with_capacity(4096);

    let initial_spectrum = vec![0.1f32; 16];
    update_global_spectrum(initial_spectrum);

    let max_queue_size = pre_buffer_target * 2;
    let queue_for_decode = audio_queue.clone();

    loop {
        if should_stop.load(Ordering::Relaxed) {
            info!("[file] Stop requested");
            break;
        }

        if queue_for_decode.lock().len() > max_queue_size {
            thread::sleep(Duration::from_millis(10));
            continue;
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
                let decoded_rate = audio_buf.spec().rate();
                let mut samples: Vec<f32> = Vec::new();
                audio_buf.copy_to_vec_interleaved(&mut samples);

                if decoded_rate != device_sample_rate {
                    samples = resample_interleaved(
                        &samples, decoded_rate, device_sample_rate, config.channels as usize,
                    );
                }

                // Accumulate interleaved samples for FFT analysis
                for &s in &samples {
                    spectrum_accum.push_back(s);
                }

                queue_for_decode.lock().extend(samples);

                if last_spectrum_update.elapsed().as_millis() >= 100 {
                    if spectrum_accum.len() >= channels {
                        // Take accumulated samples and mix to mono
                        let total = spectrum_accum.len() - (spectrum_accum.len() % channels);
                        let count = total.min(4096);
                        let raw: Vec<f32> = spectrum_accum.drain(..count).collect();
                        let mono_frames = raw.len() / channels;
                        let mut mono = Vec::with_capacity(mono_frames);
                        for ch in 0..mono_frames {
                            let base = ch * channels;
                            let sum: f32 = raw[base..base + channels].iter().sum();
                            mono.push(sum / channels as f32);
                        }
                        let normalized = analyzer.analyze(&mono);
                        update_global_spectrum(normalized);
                    }
                    last_spectrum_update = std::time::Instant::now();
                }
            }
            Err(e) => {
                info!("[file] Decode error: {}", e);
            }
        }
    }

    info!("[file] Decode loop finished. Waiting for playback to finish...");
    while !should_stop.load(Ordering::Relaxed) && !queue_for_decode.lock().is_empty() {
        thread::sleep(Duration::from_millis(100));
    }

    stream.pause().ok();
    info!("[file] Playback complete");
    buffer_ready.store(false, Ordering::Relaxed);
    is_playing_flag.store(false, Ordering::Relaxed);
}

#[cfg(not(target_os = "android"))]
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
    info!("[file] Starting pipe-based stream playback");

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

// ═══════════════════════════════════════════════════════════════════════
// Android-specific implementations
// ═══════════════════════════════════════════════════════════════════════

#[cfg(target_os = "android")]
use cpal::traits::HostTrait as CpalHostTrait;
#[cfg(target_os = "android")]
use cpal::traits::StreamTrait as CpalStreamTrait;

#[cfg(target_os = "android")]
use std::io::Read;

#[cfg(target_os = "android")]
static JVM_HANDLE: std::sync::OnceLock<jni::JavaVM> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
fn attach_current_thread_to_jvm() -> Option<jni::AttachGuard<'static>> {
    let ctx = ndk_context::android_context();
    let vm_ptr = ctx.vm() as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() {
        log::error!("[audio] JVM pointer is null");
        return None;
    }
    unsafe {
        let jvm: &'static jni::JavaVM = JVM_HANDLE.get_or_init(|| {
            jni::JavaVM::from_raw(vm_ptr).expect("Failed to create JavaVM from raw pointer")
        });
        match jvm.attach_current_thread() {
            Ok(guard) => {
                log::info!("[audio] JVM attached to current thread");
                Some(guard)
            }
            Err(e) => {
                log::error!("[audio] Failed to attach JVM: {:?}", e);
                None
            }
        }
    }
}

#[cfg(target_os = "android")]
fn get_codec_registry() -> symphonia::core::codecs::registry::CodecRegistry {
    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

#[cfg(target_os = "android")]
fn probe_audio_duration(bytes: &[u8], _len: usize, _sample_rate: u64) -> Option<u64> {
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::units::{TimeBase, Timestamp};
    let cursor = std::io::Cursor::new(bytes);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(&Hint::new(), mss, FormatOptions::default(), MetadataOptions::default())
        .ok()?;
    let track = format.default_track(symphonia::core::formats::TrackType::Audio)?;
    let codec_params = track.codec_params.as_ref()?;
    if !matches!(codec_params, symphonia::core::codecs::CodecParameters::Audio(_)) {
        return None;
    }
    let sample_rate = match codec_params {
        symphonia::core::codecs::CodecParameters::Audio(params) => params.sample_rate.unwrap_or(44100),
        _ => 44100,
    };
    let time_base = track.time_base
        .unwrap_or_else(|| TimeBase::try_from_recip(sample_rate).unwrap_or_default());
    Some(extract_duration(&mut *format, time_base))
}

#[cfg(target_os = "android")]
pub fn play_file_internal(
    path: String,
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
    log::info!("[file] play_file_internal: {}", path);
    android_logger::init_once(android_logger::Config::default().with_max_level(LevelFilter::Info));

    let _jvm = attach_current_thread_to_jvm();

    let file_data = match std::fs::read(&path) {
        Ok(data) => {
            log::info!("[file] File read successfully: {} bytes", data.len());
            data
        }
        Err(e) => {
            let err_msg = format!("Failed to read file: {}", e);
            log::error!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let duration_ms = probe_audio_duration(&file_data, file_data.len(), 44100).unwrap_or(0);
    total_duration_ms.store(duration_ms, Ordering::Relaxed);

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
            .probe(&Hint::new(), mss, FormatOptions::default(), MetadataOptions::default())
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

        let seek_pos_ms = seek_target_ms.swap(0, Ordering::AcqRel);
        if seek_pos_ms > 0 {
            log::info!("[file] Seek target: {} ms", seek_pos_ms);
            match crate::audio::decoder::seek::seek_to_position(
                &mut format,
                &codec_params,
                track_id,
                seek_pos_ms,
                &should_stop,
            ) {
                Ok(outcome) => {
                    if outcome.method == crate::audio::decoder::seek::SeekMethod::Native {
                        decoder = match get_codec_registry()
                            .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())
                        {
                            Ok(d) => d,
                            Err(e) => {
                                *load_error.lock() = format!("Decoder reset failed: {}", e);
                                return;
                            }
                        };
                        log::info!("[file] Decoder reset after native seek");
                    }
                }
                Err(e) => {
                    log::error!("[file] Seek failed: {}", e);
                }
            }
        }

        let prebuffer_secs = 7u64;
        let target_buffer_samples = (sample_rate as u64 * prebuffer_secs) as usize * channels as usize;
        log::info!("[file] Pre-buffering {} samples ({} seconds)...", target_buffer_samples, prebuffer_secs);

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
                Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                    Ok(audio_buf) => {
                        prebuffer_error_count = 0;
                        let decoded_rate = audio_buf.spec().rate();
                        let mut samples: Vec<f32> = Vec::new();
                        audio_buf.copy_to_vec_interleaved(&mut samples);
                        if decoded_rate != device_sample_rate {
                            samples = crate::audio::stream::handling::resample_interleaved(
                                &samples, decoded_rate, device_sample_rate, config.channels as usize,
                            );
                        }
                        buffered_samples += samples.len();
                        queue_for_decode.lock().extend(samples);
                    }
                    Err(e) => {
                        log::error!("[file] Decode error: {}", e);
                        prebuffer_error_count += 1;
                        if prebuffer_error_count >= MAX_PACKET_ERRORS {
                            let err_msg = format!("Too many decode errors ({}): {}", prebuffer_error_count, e);
                            log::error!("[file] {}", err_msg);
                            *load_error.lock() = err_msg;
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                },
                Ok(Some(_)) => continue,
                Ok(None) => {
                    log::info!("[file] Stream exhausted during pre-buffering");
                    break;
                }
                Err(e) => {
                    log::error!("[file] Packet error: {}", e);
                    prebuffer_error_count += 1;
                    if prebuffer_error_count >= MAX_PACKET_ERRORS {
                        let err_msg = format!("Too many packet errors ({}): {}", prebuffer_error_count, e);
                        log::error!("[file] {}", err_msg);
                        *load_error.lock() = err_msg;
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }

        log::info!("[file] Pre-buffer complete: {} samples", buffered_samples);

        let host = cpal::default_host();
        let device = match host.default_output_device() {
            Some(d) => d,
            None => {
                let err_msg = "No output device".to_string();
                log::error!("[file] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };
        let config = match crate::audio::stream::cpal_source::pick_output_config(&device) {
            Some(c) => c,
            None => {
                let err_msg = "No suitable output config".to_string();
                log::error!("[file] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };

        let device_sample_rate = config.sample_rate;
        sample_rate_out.store(device_sample_rate as u64, Ordering::Relaxed);
        channels_out.store(config.channels as u64, Ordering::Relaxed);
        log::info!("[file] Device output: {} Hz, {} ch", device_sample_rate, config.channels);

        if seek_pos_ms > 0 {
            let ch_out = config.channels as u64;
            let rate_out = device_sample_rate as u64;
            let initial_samples = (seek_pos_ms * rate_out * ch_out) / 1000;
            samples_played.store(initial_samples, Ordering::Relaxed);
        }

        let stream = match crate::audio::stream::cpal_source::build_output_stream(
            &device,
            &config,
            queue_for_decode.clone(),
            buffer_ready.clone(),
            samples_played.clone(),
        ) {
            Ok(s) => s,
            Err(e) => {
                let err_msg = format!("Failed to build output stream: {}", e);
                log::error!("[file] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        };

        if let Err(e) = stream.play() {
            let err_msg = format!("Failed to start output stream: {}", e);
            log::error!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
        buffer_ready.store(true, Ordering::Relaxed);
        is_playing_flag.store(true, Ordering::Relaxed);
        log::info!("[file] Output stream started");

        let band_count = crate::audio::engine::get_band_count();
        let mut analyzer = crate::dsp::RmsSpectrumAnalyzer::new(device_sample_rate, band_count);
        crate::audio::engine::update_global_spectrum(vec![0.1f32; band_count]);

        let max_queue_samples = (device_sample_rate as usize) * 10 * (config.channels as usize);
        let mut packet_error_count = 0;
        let mut spectrum_accum: VecDeque<f32> = VecDeque::with_capacity(4096);
        let mut last_spectrum_update = std::time::Instant::now();

        loop {
            if should_stop.load(Ordering::Relaxed) {
                log::info!("[file] Stop requested during decode");
                break;
            }

            if seek_target_ms.load(Ordering::Acquire) > 0 {
                log::info!("[file] Seek requested during playback, stopping decode");
                break;
            }

            match format.next_packet() {
                Ok(Some(packet)) if packet.track_id == track_id => {
                    packet_error_count = 0;
                    match decoder.decode(&packet) {
                        Ok(audio_buf) => {
                            let decoded_rate = audio_buf.spec().rate();
                            let mut samples: Vec<f32> = Vec::new();
                            audio_buf.copy_to_vec_interleaved(&mut samples);

                            if decoded_rate != device_sample_rate {
                                samples = crate::audio::stream::handling::resample_interleaved(
                                    &samples, decoded_rate, device_sample_rate, config.channels as usize,
                                );
                            }

                            for &s in &samples {
                                spectrum_accum.push_back(s);
                            }

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
                                let total = spectrum_accum.len() - (spectrum_accum.len() % ch);
                                let count = total.min(4096);
                                let raw: Vec<f32> = spectrum_accum.drain(..count).collect();
                                let mono_frames = raw.len() / ch;
                                let mut mono = Vec::with_capacity(mono_frames);
                                for frame in 0..mono_frames {
                                    let base = frame * ch;
                                    let sum: f32 = raw[base..base + ch].iter().sum();
                                    mono.push(sum / ch as f32);
                                }
                                let normalized = analyzer.analyze(&mono);
                                crate::audio::engine::update_global_spectrum(normalized);
                                last_spectrum_update = std::time::Instant::now();
                            }
                        }
                        Err(e) => {
                            log::error!("[file] Decode error: {}", e);
                            packet_error_count += 1;
                            if packet_error_count >= MAX_PACKET_ERRORS {
                                let err_msg = format!("Too many decode errors ({}): {}", packet_error_count, e);
                                log::error!("[file] {}", err_msg);
                                *load_error.lock() = err_msg;
                                break;
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
                Ok(Some(_)) => continue,
                Ok(None) => {
                    log::info!("[file] End of file reached");
                    break;
                }
                Err(e) => {
                    packet_error_count += 1;
                    log::error!("[file] Packet error ({}): {}", packet_error_count, e);
                    if packet_error_count >= MAX_PACKET_ERRORS {
                        let err_msg = format!("Too many packet errors ({}): {}", packet_error_count, e);
                        log::error!("[file] {}", err_msg);
                        *load_error.lock() = err_msg;
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }

        log::info!("[file] Decode complete, waiting for output drain");
        let drain_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !should_stop.load(Ordering::Relaxed) {
            if queue_for_decode.lock().is_empty() {
                log::info!("[file] Queue empty, done draining");
                break;
            }
            if std::time::Instant::now() > drain_deadline {
                log::error!("[file] Drain timeout — queue still has {} samples", queue_for_decode.lock().len());
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        log::info!("[file] Playback complete");
        drop(stream);
        buffer_ready.store(false, Ordering::Relaxed);
        is_playing_flag.store(false, Ordering::Relaxed);
        break 'outer;
    }
}

#[cfg(target_os = "android")]
pub fn play_stream_from_pipe_internal(
    reader: crate::audio::stream::pipe::PipeReader,
    audio_queue: crate::audio::stream::cpal_source::AudioBuffer,
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
    log::info!("[stream] Starting pipe-based stream playback (Android)");

    crate::audio::stream::handling::decode_and_play_from_read(
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
pub fn play_live_internal(
    url: String,
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
    _pipe_bytes_sent: Arc<AtomicU64>,
    _pipe_total_bytes: Arc<AtomicU64>,
    cache_max_ms: u64,
    ring: std::sync::Arc<std::sync::Mutex<crate::models::LiveByteRing>>,
    cache_head_ms: u64,
) {
    android_logger::init_once(android_logger::Config::default().with_max_level(LevelFilter::Info));
    let _jvm = attach_current_thread_to_jvm();
    log::info!("[live] play_live_internal (Android): {}", url);

    let seek_pos = seek_target_ms.load(Ordering::Relaxed);
    let reader: Box<dyn Read + Send + Sync + 'static> = if seek_pos > 0 {
        let r = ring.lock().unwrap();
        let total = r.total_written();
        let head_ms = cache_head_ms.max(1);
        let bytes_per_ms = (total as f64) / (head_ms as f64);
        let target_byte = (seek_pos as f64 * bytes_per_ms) as u64;
        let clamped = target_byte.min(total.saturating_sub(1));
        log::info!("[live] Seek: pos={}ms, cache_head={}ms, total_written={}B, bytes_per_ms={:.1}, target_byte={}, clamped={}",
            seek_pos, cache_head_ms, total, bytes_per_ms, target_byte, clamped);
        drop(r);
        Box::new(crate::models::LiveByteReader::new(ring, clamped))
    } else {
        let (pipe_writer, pipe_reader) = crate::audio::stream::pipe::new_pipe();
        let pipe_writer = Arc::new(pipe_writer);
        let pw = pipe_writer.clone();
        let ring_clone = ring.clone();
        let fetch_url = url.clone();
        thread::spawn(move || {
            let _jvm = attach_current_thread_to_jvm();
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            rt.block_on(async move {
                let client = reqwest::Client::new();
                let resp = match client
                    .get(&fetch_url)
                    .header("User-Agent", "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36")
                    .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
                    .header("Icy-MetaData", "0")
                    .send()
                    .await
                {
                    Ok(r) if r.status().is_success() => r,
                    Ok(r) => {
                        pw.set_error(format!("HTTP {}", r.status()));
                        return;
                    }
                    Err(e) => {
                        pw.set_error(format!("Connection failed: {}", e));
                        return;
                    }
                };

                loop {
                    match resp.chunk().await {
                        Ok(Some(data)) => {
                            ring_clone.lock().unwrap().push(&data);
                            pw.push(&data);
                        }
                        Ok(None) => {
                            pw.end();
                            return;
                        }
                        Err(e) => {
                            pw.set_error(format!("Stream error: {}", e));
                            return;
                        }
                    }
                }
            });
        });

        Box::new(pipe_reader) as Box<dyn Read + Send + Sync + 'static>
    };

    total_duration_ms.store(cache_max_ms, Ordering::Relaxed);
    crate::audio::stream::handling::decode_and_play_from_read(
        reader,
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
