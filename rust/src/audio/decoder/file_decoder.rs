//! Symphonia-based audio decoder for desktop platforms (non-Android)

use log::info;

use crate::audio::decoder::seek::seek_to_position;
use crate::audio::engine::{get_band_count, update_global_spectrum};
use crate::audio::stream::cpal_source::{build_output_stream, pick_output_config};
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

    let config = match pick_output_config(&device, sample_rate, channels as u16) {
        Some(c) => c,
        None => {
            let err_msg = "No suitable output config".to_string();
            info!("[file] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

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

    let pre_buffer_target = (sample_rate as usize * 7) / 10 * channels;
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
                let mut samples: Vec<f32> = Vec::new();
                audio_buf.copy_to_vec_interleaved(&mut samples);
                audio_queue.lock().extend(samples);
            }
            Err(e) => {
                info!("[file] Decode error: {}", e);
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
    let mut analyzer = RmsSpectrumAnalyzer::new(sample_rate, band_count);
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
                let mut samples: Vec<f32> = Vec::new();
                audio_buf.copy_to_vec_interleaved(&mut samples);

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
