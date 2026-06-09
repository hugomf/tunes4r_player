//! Audio stream processing and decoding logic, primarily for non-Android platforms.

use crate::audio::stream::source::ReadSeek;
#[cfg(not(target_os = "android"))]
use crate::dsp::RmsSpectrumAnalyzer;
#[cfg(not(target_os = "android"))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(not(target_os = "android"))]
use log::{debug, error, info, warn};
#[cfg(not(target_os = "android"))]
use parking_lot::Mutex;
#[cfg(not(target_os = "android"))]
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(not(target_os = "android"))]
use std::sync::Arc;
#[cfg(not(target_os = "android"))]
use std::collections::VecDeque;
#[cfg(not(target_os = "android"))]
use std::thread;
#[cfg(not(target_os = "android"))]
use std::time::Duration;

#[cfg(not(target_os = "android"))]
use symphonia::core::codecs::audio::AudioCodecParameters;
#[cfg(not(target_os = "android"))]
use symphonia::core::codecs::audio::AudioDecoder;
#[cfg(not(target_os = "android"))]
use symphonia::core::formats::probe::Hint;
#[cfg(not(target_os = "android"))]
use symphonia::core::formats::FormatOptions;
#[cfg(not(target_os = "android"))]
use symphonia::core::formats::FormatReader;
#[cfg(not(target_os = "android"))]
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
#[cfg(not(target_os = "android"))]
use symphonia::core::meta::MetadataOptions;
#[cfg(not(target_os = "android"))]
use symphonia::core::units::{TimeBase, Timestamp};
#[cfg(not(target_os = "android"))]
use crate::audio::decoder::seek::seek_to_position;

/// A `Read` wrapper that counts bytes read and updates an `AtomicU64`.
/// Used for Read-based sources (YouTube, progressive HTTP) to feed the
/// buffer poller's `pipe_bytes_sent` counter, so the adaptive ring buffer
/// reflects actual download progress instead of appearing 100% complete.
#[cfg(not(target_os = "android"))]
pub struct ByteCountingRead<R: Read> {
    inner: R,
    counter: Arc<AtomicU64>,
}

#[cfg(not(target_os = "android"))]
impl<R: Read> ByteCountingRead<R> {
    pub fn new(inner: R, counter: Arc<AtomicU64>) -> Self {
        Self { inner, counter }
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Read + Seek> Read for ByteCountingRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.counter.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Read + Seek> Seek for ByteCountingRead<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

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
type AudioBuffer = crate::audio::stream::cpal_source::AudioBuffer;

#[cfg(not(target_os = "android"))]
struct DecodeCtx {
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<Mutex<String>>,
}

#[cfg(not(target_os = "android"))]
fn probe_format(
    reader: Box<dyn ReadSeek + Send + Sync + 'static>,
    ctx: &DecodeCtx,
) -> Option<(Box<dyn FormatReader>, u32, AudioCodecParameters, u32, usize, u64)> {
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
            *ctx.load_error.lock() = err_msg;
            return None;
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
            *ctx.load_error.lock() = err_msg;
            return None;
        }
    };

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
            *ctx.load_error.lock() = err_msg;
            return None;
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
        ctx.total_duration_ms.store(duration_ms, Ordering::Relaxed);
    }

    let track = match probed.first_track(symphonia::core::formats::TrackType::Audio) {
        Some(track) => track,
        None => {
            let err_msg = "No audio track found".to_string();
            debug!("[stream] {}", err_msg);
            *ctx.load_error.lock() = err_msg;
            return None;
        }
    };

    let track_id = track.id;
    let codec_params = match track.codec_params.as_ref() {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            let err_msg = "No audio codec params".to_string();
            debug!("[stream] {}", err_msg);
            *ctx.load_error.lock() = err_msg;
            return None;
        }
    };

    let sample_rate = match codec_params.sample_rate {
        Some(rate) => rate,
        None => {
            let err_msg = "No sample rate".to_string();
            debug!("[stream] {}", err_msg);
            *ctx.load_error.lock() = err_msg;
            return None;
        }
    };

    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);

    debug!("[stream] Stream: {} Hz, {} channels", sample_rate, channels);
    ctx.sample_rate_out.store(sample_rate as u64, Ordering::Relaxed);
    ctx.channels_out.store(channels as u64, Ordering::Relaxed);

    let format: Box<dyn FormatReader> = probed;
    Some((format, track_id, codec_params, sample_rate, channels, duration_ms))
}

#[cfg(not(target_os = "android"))]
fn playback_loop(
    format: &mut Box<dyn FormatReader>,
    decoder: &mut Box<dyn AudioDecoder>,
    track_id: u32,
    audio_queue: &AudioBuffer,
    output_sample_rate: u32,
    channels: usize,
    should_stop: &Arc<AtomicBool>,
    samples_played: &AtomicU64,
    seek_request: &AtomicU64,
    codec_params: &AudioCodecParameters,
) -> u64 {
    let band_count = crate::audio::engine::get_band_count();
    crate::audio::engine::types::update_global_spectrum(vec![0.1f32; band_count]);
    let mut analyzer = RmsSpectrumAnalyzer::new(output_sample_rate, band_count);
    let mut spectrum_accum: VecDeque<f32> = VecDeque::with_capacity(4096);
    let mut last_spectrum_time = std::time::Instant::now();

    let mut decoded: u64 = 0;
    let mut consecutive_packet_errors: u32 = 0;
    loop {
        if should_stop.load(Ordering::Relaxed) {
            debug!("[stream] Stop signal received, exiting loop");
            break;
        }

        // Check for in-thread seek request from the engine.
        // A non-zero value means the user has dragged the slider and we should
        // reposition the format reader + decoder without re-opening the source.
        let seek_target = seek_request.swap(0, Ordering::AcqRel);
        if seek_target > 0 {
            info!("[stream] In-thread seek to {} ms", seek_target);
            audio_queue.lock().clear();
            match seek_to_position(format, codec_params, track_id, seek_target, should_stop) {
                Ok(outcome) => {
                    info!(
                        "[stream] In-thread seek complete: {:?}, residual {} samples",
                        outcome.method, outcome.residual_samples_to_skip
                    );
                }
                Err(e) => {
                    warn!("[stream] In-thread seek failed: {}", e);
                }
            }
            // Reset decoder — Symphonia 0.6 requires this after a seek
            match make_decoder(codec_params) {
                Ok(d) => *decoder = d,
                Err(e) => {
                    warn!("[stream] Failed to recreate decoder after seek: {}", e);
                    break;
                }
            }
            let ch_out = channels as u64;
            let rate_out = output_sample_rate as u64;
            let initial_samples = (seek_target * rate_out * ch_out) / 1000;
            samples_played.store(initial_samples, Ordering::Relaxed);
            info!(
                "[stream] Re-seeded position: {} ms ({} samples at {} Hz, {} ch)",
                seek_target, initial_samples, rate_out, ch_out
            );
            continue;
        }

        match format.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let mut samples: Vec<f32> = Vec::new();
                    audio_buf.copy_to_vec_interleaved(&mut samples);
                    let decoded_rate = audio_buf.spec().rate();
                    if decoded_rate != output_sample_rate {
                        samples = resample_interleaved(
                            &samples, decoded_rate, output_sample_rate, channels,
                        );
                    }

                    // Spectrum accumulation in the decode loop (matches play_file_internal pattern).
                    for &s in &samples {
                        spectrum_accum.push_back(s);
                    }
                    let now = std::time::Instant::now();
                    if now.duration_since(last_spectrum_time).as_millis() >= 100 {
                        let raw: Vec<f32> = spectrum_accum.drain(..).collect();
                        if raw.len() >= channels {
                            let total = raw.len() - (raw.len() % channels);
                            let mono_frames = total / channels;
                            let mut mono = Vec::with_capacity(mono_frames);
                            for i in 0..mono_frames {
                                let base = i * channels;
                                let sum: f32 = raw[base..base + channels].iter().sum();
                                mono.push(sum / channels as f32);
                            }
                            let normalized = analyzer.analyze(&mono);
                            crate::audio::engine::types::update_global_spectrum(normalized);
                        }
                        last_spectrum_time = now;
                    }

                    audio_queue.lock().extend(samples);
                    decoded += 1;
                    if decoded.is_multiple_of(100) {
                        info!(
                            "[stream] Decoded {} packets, queue: {} samples, played: {}",
                            decoded,
                            audio_queue.lock().len(),
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
                info!("[stream] Stream ended after {} packets", decoded);
                break;
            }
            Err(e) => {
                consecutive_packet_errors += 1;
                debug!("[stream] Packet error ({}): {}", consecutive_packet_errors, e);
                if consecutive_packet_errors >= 50 {
                    info!("[stream] Too many consecutive packet errors ({})", consecutive_packet_errors);
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    decoded
}

#[cfg(not(target_os = "android"))]
fn drain_queue(
    audio_queue: &AudioBuffer,
    should_stop: &AtomicBool,
    samples_played: &AtomicU64,
    total_duration_ms: &AtomicU64,
    sample_rate: u32,
    channels: usize,
) {
    let total_samples =
        samples_played.load(Ordering::Relaxed) + audio_queue.lock().len() as u64;
    if total_duration_ms.load(Ordering::Relaxed) == 0 && total_samples > 0 {
        let estimated = (total_samples * 1000) / (sample_rate as u64 * channels as u64);
        if estimated > 0 {
            info!("[stream] Estimated duration: {} ms", estimated);
            total_duration_ms.store(estimated, Ordering::Relaxed);
        }
    }

    info!(
        "[stream] Decode complete, draining queue ({} samples queued, {} played)",
        audio_queue.lock().len(),
        samples_played.load(Ordering::Relaxed),
    );

    loop {
        if should_stop.load(Ordering::Relaxed) {
            break;
        }
        if audio_queue.lock().is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(target_os = "android"))]
fn build_and_play_stream(
    config: &cpal::StreamConfig,
    device: &cpal::Device,
    audio_queue: &AudioBuffer,
    buffer_ready: &Arc<AtomicBool>,
    samples_played: &Arc<AtomicU64>,
    _load_error: &Mutex<String>,
) -> Result<cpal::Stream, String> {
    let pq = audio_queue.clone();
    let br = buffer_ready.clone();
    let sp = samples_played.clone();
    let my_gen = crate::audio::stream::cpal_source::OUTPUT_GEN.load(Ordering::Relaxed);
    device
        .build_output_stream(
            config,
            move |data: &mut [f32], _| {
                crate::audio::stream::cpal_source::run_output_callback(
                    data, &pq, &br, &sp, my_gen,
                );
            },
            |err| error!("[stream] Audio output error: {}", err),
            None,
        )
        .map_err(|e| format!("{}", e))
}

#[cfg(not(target_os = "android"))]
fn get_output_stream(
    config: &cpal::StreamConfig,
    device: &cpal::Device,
    audio_queue: &AudioBuffer,
    buffer_ready: &Arc<AtomicBool>,
    samples_played: &Arc<AtomicU64>,
    load_error: &Mutex<String>,
) -> Result<cpal::Stream, String> {
    match build_and_play_stream(config, device, audio_queue, buffer_ready, samples_played, load_error) {
        Ok(s) => {
            info!("[stream] Output stream built with primary config");
            Ok(s)
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
                    build_and_play_stream(&fallback, device, audio_queue, buffer_ready, samples_played, load_error)
                }
                Err(e2) => {
                    Err(format!(
                        "Failed to build audio stream ({}) and no default config available: {}",
                        e, e2
                    ))
                }
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
fn make_decoder(
    codec_params: &AudioCodecParameters,
) -> Result<Box<dyn AudioDecoder>, String> {
    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
        .make_audio_decoder(
            codec_params,
            &symphonia::core::codecs::audio::AudioDecoderOptions::default(),
        )
        .map_err(|e| format!("{}", e))
}

#[cfg(not(target_os = "android"))]
pub fn decode_and_play_from_read(
    reader: Box<dyn ReadSeek + Send + Sync + 'static>,
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
    seek_request: Arc<AtomicU64>,
) {
    let ctx = DecodeCtx {
        sample_rate_out: sample_rate_out.clone(),
        channels_out: channels_out.clone(),
        total_duration_ms: total_duration_ms.clone(),
        load_error: load_error.clone(),
    };

    // ── Phase 1: Probe format ──
    let (mut format, track_id, codec_params, sample_rate, channels, _duration_ms) =
        match probe_format(reader, &ctx) {
            Some(r) => r,
            None => return,
        };

    let seek_pos_ms = seek_target_ms.swap(0, Ordering::AcqRel);
    if seek_pos_ms > 0 {
        info!("[stream] Seek target: {} ms", seek_pos_ms);
    }

    // ── Phase 2: Init output device ──
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
                sample_rate: 44100_u32,
                buffer_size: cpal::BufferSize::Default,
            }
        }
    };

    info!(
        "[stream] Audio device configured: {:?} Hz, {} ch",
        config.sample_rate, config.channels
    );
    let output_sample_rate = config.sample_rate;
    let target_buffer_samples =
        (output_sample_rate as f32 * 7.0) as usize * channels;

    // ── Phase 3: Seek (if requested) ──
    let mut samples_to_skip: u64 = 0;

    if seek_pos_ms > 0 {
        info!("[stream] Seeking to {} ms via seek_to_position", seek_pos_ms);
        let seek_result = seek_to_position(
            &mut format,
            &codec_params,
            track_id,
            seek_pos_ms,
            &should_stop,
        );
        match &seek_result {
            Ok(outcome) => {
                info!(
                    "[stream] Seek complete: {:?}, residual {} samples",
                    outcome.method, outcome.residual_samples_to_skip
                );
                samples_to_skip = outcome.residual_samples_to_skip;
            }
            Err(e) => {
                warn!("[stream] Seek failed: {}", e);
            }
        }

        let ch_out = config.channels as u64;
        let rate_out = output_sample_rate as u64;
        let initial_samples = (seek_pos_ms * rate_out * ch_out) / 1000;
        samples_played.store(initial_samples, Ordering::Relaxed);
        info!(
            "[stream] Seeded position: {} ms ({} samples at {} Hz, {} ch)",
            seek_pos_ms, initial_samples, rate_out, ch_out
        );
    }

    // ── Phase 4: Create decoder ──
    let mut decoder = match make_decoder(&codec_params) {
        Ok(d) => d,
        Err(e) => {
            let err_msg = format!("Decoder creation failed: {}", e);
            debug!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    // ── Phase 5: Prebuffer (with residual skip after packet-skip seek) ──
    debug!("[stream] Pre-buffering {} samples (7 seconds)...", target_buffer_samples);
    let mut buffered = 0;
    let mut prebuffer_packet_errors = 0u32;
    while buffered < target_buffer_samples {
        if should_stop.load(Ordering::Relaxed) {
            break;
        }
        match format.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let decoded_rate = audio_buf.spec().rate();
                    let ch = audio_buf.spec().channels().count() as u16;
                    let mut samples: Vec<f32> = Vec::new();
                    audio_buf.copy_to_vec_interleaved(&mut samples);
                    if decoded_rate != output_sample_rate {
                        samples = resample_interleaved(
                            &samples, decoded_rate, output_sample_rate, ch as usize,
                        );
                    }

                    if samples_to_skip > 0 {
                        let n = samples.len() as u64;
                        if samples_to_skip >= n {
                            samples_to_skip -= n;
                            continue;
                        }
                        samples.drain(0..samples_to_skip as usize);
                        samples_to_skip = 0;
                    }

                    buffered += samples.len();
                    audio_queue.lock().extend(samples);
                }
                Err(e) => {
                    debug!("[stream] Prebuffer decode error: {}", e);
                    thread::sleep(Duration::from_millis(10));
                }
            },
            Ok(Some(_)) => continue,
            Ok(None) => {
                info!("[stream] Stream ended during buffering (buffered {} samples)", buffered);
                break;
            }
            Err(e) => {
                prebuffer_packet_errors += 1;
                debug!("[stream] Prebuffer packet error ({}): {}", prebuffer_packet_errors, e);
                if prebuffer_packet_errors >= 50 {
                    warn!("[stream] Too many prebuffer packet errors ({})", prebuffer_packet_errors);
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    info!("[stream] Pre-buffer complete: {} samples", buffered);

    // ── Phase 6: Build & play output stream ──
    info!("[stream] Building output stream...");

    let stream = match get_output_stream(
        &config,
        &device,
        &audio_queue,
        &buffer_ready,
        &samples_played,
        &load_error,
    ) {
        Ok(s) => s,
        Err(e) => {
            let err_msg = format!("Failed to build audio stream: {}", e);
            error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
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

    // ── Phase 7: Playback loop ──
    let decode_count = playback_loop(
        &mut format,
        &mut decoder,
        track_id,
        &audio_queue,
        output_sample_rate,
        channels,
        &should_stop,
        &samples_played,
        &seek_request,
        &codec_params,
    );

    // ── Phase 8: Drain ──
    drain_queue(
        &audio_queue,
        &should_stop,
        &samples_played,
        &total_duration_ms,
        sample_rate,
        channels,
    );

    info!(
        "[stream] Playback thread ending ({} packets decoded, {} samples played)",
        decode_count,
        samples_played.load(Ordering::Relaxed),
    );
    buffer_ready.store(false, Ordering::Relaxed);
    is_playing_flag.store(false, Ordering::Relaxed);
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
        Arc::new(AtomicU64::new(0)),
    );
}

#[cfg(not(target_os = "android"))]
/// Play a live internet stream with backward-seek support via ring buffer.
///
/// On the first call (no cached data), downloads fresh from the URL and caches
/// bytes into the shared ring buffer. On subsequent calls (seek), reads from
/// the ring buffer at the calculated byte position for the seek target.
pub fn play_live_internal(
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
    _pipe_bytes_sent: Arc<AtomicU64>,
    _pipe_total_bytes: Arc<AtomicU64>,
    cache_max_ms: u64,
    ring: std::sync::Arc<std::sync::Mutex<crate::models::LiveByteRing>>,
    cache_head_ms: u64,
) {
    info!("[live] play_live_internal: {}", url);

    let seek_pos = seek_target_ms.load(Ordering::Relaxed);

    // Determine if we should read from cached ring (seek) or download fresh.
    let reader: Box<dyn ReadSeek + Send + Sync + 'static> = if seek_pos > 0 {
        // Seek: map seek_pos (absolute ms from start) to an absolute byte
        // offset in the ring buffer stream.
        // cache_head_ms is the actual elapsed wall-clock time (capped at
        // cache_max_ms), computed in commands.rs from live_start_time.
        // This gives us the true bytes-per-ms ratio regardless of the
        // stream's actual bitrate — unlike the old fill_ratio approach
        // which assumed 128kbps and gave wrong results for lower-bitrate
        // streams like BBC World Service (~38kbps).
        let r = ring.lock().unwrap();
        let total = r.total_written();
        let head_ms = cache_head_ms.max(1);
        let bytes_per_ms = (total as f64) / (head_ms as f64);
        let target_byte = (seek_pos as f64 * bytes_per_ms) as u64;
        let clamped = target_byte.min(total.saturating_sub(1));
        info!("[live] Seek: pos={}ms, cache_head={}ms, total_written={}B, bytes_per_ms={:.1}, target_byte={}, clamped={}",
            seek_pos, cache_head_ms, total, bytes_per_ms, target_byte, clamped);
        drop(r);
        Box::new(crate::models::LiveByteReader::new(ring, clamped))
    } else {
        // Initial play: download fresh and cache into ring buffer.
        info!("[live] Downloading fresh stream");
        let resp = match client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
            .header("Icy-MetaData", "0")
            .send()
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let err = format!("HTTP {}", r.status());
                *load_error.lock() = err.clone();
                error!("[live] {}", err);
                return;
            }
            Err(e) => {
                let err = format!("Connection failed: {}", e);
                *load_error.lock() = err.clone();
                error!("[live] {}", err);
                return;
            }
        };
        Box::new(crate::models::LiveByteCacheReader::new(resp, ring))
    };

    total_duration_ms.store(cache_max_ms, Ordering::Relaxed);
    decode_and_play_from_read(
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
        Arc::new(AtomicU64::new(0)),
    );
}


// ═══════════════════════════════════════════════════════════════════════
// Android-specific implementations (share the same module paths as
// desktop, so callers in commands.rs use a single module namespace).
// ═══════════════════════════════════════════════════════════════════════
// Android-specific implementations (share the same module paths as
// desktop, so callers in commands.rs use a single module namespace).
// ═══════════════════════════════════════════════════════════════════════

#[cfg(target_os = "android")]
use cpal::traits::HostTrait;
#[cfg(target_os = "android")]
use cpal::traits::StreamTrait;
#[cfg(target_os = "android")]
use jni::JavaVM;
#[cfg(target_os = "android")]
use log::LevelFilter;
#[cfg(target_os = "android")]
use std::sync::Arc;
#[cfg(target_os = "android")]
use std::thread;
#[cfg(target_os = "android")]
use std::time::Duration;
#[cfg(target_os = "android")]
use symphonia::core::codecs::audio::AudioDecoderOptions;
#[cfg(target_os = "android")]
use symphonia::core::codecs::registry::CodecRegistry;
#[cfg(target_os = "android")]
use symphonia::core::formats::probe::Hint;
#[cfg(target_os = "android")]
use symphonia::core::formats::FormatOptions;
#[cfg(target_os = "android")]
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
#[cfg(target_os = "android")]
use symphonia::core::meta::MetadataOptions;
#[cfg(target_os = "android")]
use symphonia::core::units::TimeBase;
#[cfg(target_os = "android")]
use crate::audio::decoder::seek::{seek_to_position, SeekMethod};
#[cfg(target_os = "android")]
use crate::audio::stream::cpal_source::{build_output_stream, pick_output_config};
#[cfg(target_os = "android")]
use crate::audio::stream::cpal_source::AudioBuffer;
#[cfg(target_os = "android")]
use crate::dsp::RmsSpectrumAnalyzer;

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
        let jvm: &'static JavaVM = JVM_HANDLE.get_or_init(|| {
            JavaVM::from_raw(vm_ptr).expect("Failed to create JavaVM from raw pointer")
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
fn get_codec_registry() -> CodecRegistry {
    let mut registry = CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

#[cfg(target_os = "android")]
#[allow(dead_code)]
fn probe_audio_duration(bytes: &[u8], _len: usize, _sample_rate: u64) -> Option<u64> {
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
    Some(crate::audio::decoder::file_decoder::extract_duration(&mut *format, time_base))
}

#[cfg(target_os = "android")]
pub fn decode_and_play_from_read(
    reader: Box<dyn ReadSeek + Send + Sync + 'static>,
    audio_queue: AudioBuffer,
    buffer_ready: Arc<AtomicBool>,
    is_playing_flag: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
    sample_rate_out: Arc<AtomicU64>,
    channels_out: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
    load_error: Arc<parking_lot::Mutex<String>>,
    seek_target_ms: Arc<AtomicU64>,
) {
    log::info!("[stream] Connected! Creating streaming reader...");

    let source = ReadOnlySource::new(reader);
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    log::info!("[stream] Probing format...");
    let probed = match symphonia::default::get_probe().probe(
        &Hint::new(),
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(e) => {
            let err_msg = format!("Format detection failed: {}", e);
            log::error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track = match probed.first_track(symphonia::core::formats::TrackType::Audio) {
        Some(t) => t,
        None => {
            let err_msg = "No audio track found".to_string();
            log::error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let track_id = track.id;
    let codec_params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => {
            let err_msg = "No audio codec params".to_string();
            log::error!("[stream] {}", err_msg);
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
    log::info!("[stream] Stream: {} Hz, {} channels", sample_rate, channels);
    sample_rate_out.store(sample_rate as u64, Ordering::Relaxed);
    channels_out.store(channels as u64, Ordering::Relaxed);

    let duration_ms = probed
        .first_track(symphonia::core::formats::TrackType::Audio)
        .and_then(|t| {
            let time_base = t.time_base?;
            let duration = probed.media_info().duration?;
            let ts = symphonia::core::units::Timestamp::new(duration.get() as i64);
            let time_computed = time_base.calc_time(ts)?;
            Some(time_computed.as_millis() as u64)
        })
        .unwrap_or_else(|| {
            let estimated = (sample_rate as u64 * 3600) / 1000;
            log::info!("[stream] No duration metadata, estimated: {} ms", estimated);
            estimated
        });
    total_duration_ms.store(duration_ms, Ordering::Relaxed);

    let target_buffer_secs = 7.0;
    let target_buffer_samples = (sample_rate as f32 * target_buffer_secs) as usize * channels;

    let target_ms = seek_target_ms.swap(0, Ordering::AcqRel);

    let mut buffered_samples = 0;
    let mut prebuffer_error_count = 0;
    let mut last_logged_pct = 0u32;
    const MAX_PACKET_ERRORS: u32 = 30;
    let mut format_reader = probed;
    let mut decoder = match get_codec_registry().make_audio_decoder(
        &codec_params,
        &AudioDecoderOptions::default(),
    ) {
        Ok(d) => d,
        Err(e) => {
            let err_msg = format!("Decoder creation failed: {}", e);
            log::error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };

    let residual_samples_to_skip = if target_ms > 0 {
        log::info!("[stream] Seek target: {} ms", target_ms);
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
                    log::info!("[stream] Decoder reset after native seek");
                }
                log::info!(
                    "[stream] Seek complete: {:?}, residual {} samples",
                    outcome.method, outcome.residual_samples_to_skip
                );
                outcome.residual_samples_to_skip
            }
            Err(e) => {
                let err_msg = format!("Seek failed: {}", e);
                log::error!("[stream] {}", err_msg);
                *load_error.lock() = err_msg;
                return;
            }
        }
    } else {
        0
    };
    let mut samples_to_skip = residual_samples_to_skip;

    log::info!(
        "[stream] Pre-buffering {} samples ({} seconds)...",
        target_buffer_samples, target_buffer_secs
    );

    let queue_for_decode = audio_queue.clone();
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            let err_msg = "No output device".to_string();
            log::error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };
    let config = match pick_output_config(&device) {
        Some(c) => c,
        None => {
            let err_msg = "No suitable output config".to_string();
            log::error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };
    let device_sample_rate = config.sample_rate;
    let device_channels = config.channels;
    sample_rate_out.store(device_sample_rate as u64, Ordering::Relaxed);
    channels_out.store(device_channels as u64, Ordering::Relaxed);
    log::info!(
        "[stream] Device output: {} Hz, {} ch (codec: {} Hz, {} ch)",
        device_sample_rate, device_channels, sample_rate, channels
    );

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
            log::error!("[stream] {}", err_msg);
            *load_error.lock() = err_msg;
            return;
        }
    };
    if let Err(e) = stream.play() {
        let err_msg = format!("Failed to start output stream: {}", e);
        log::error!("[stream] {}", err_msg);
        *load_error.lock() = err_msg;
        return;
    }
    log::info!(
        "[stream] cpal output stream started ({} Hz, {} ch)",
        config.sample_rate, config.channels
    );

    while buffered_samples < target_buffer_samples {
        if should_stop.load(Ordering::Relaxed) {
            log::info!("[stream] Stop requested during pre-buffering");
            return;
        }

        match format_reader.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    prebuffer_error_count = 0;
                    let decoded_rate = audio_buf.spec().rate();
                    let mut samples: Vec<f32> = Vec::new();
                    audio_buf.copy_to_vec_interleaved(&mut samples);

                    if decoded_rate != device_sample_rate {
                        samples = resample_interleaved(
                            &samples, decoded_rate, device_sample_rate, config.channels as usize,
                        );
                    }

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
                    log::error!("[stream] Decode error: {}", e);
                    prebuffer_error_count += 1;
                    if prebuffer_error_count >= MAX_PACKET_ERRORS {
                        let err_msg = format!("Too many decode errors ({}): {}", prebuffer_error_count, e);
                        log::error!("[stream] {}", err_msg);
                        *load_error.lock() = err_msg;
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            },
            Ok(Some(_)) => continue,
            Ok(None) => {
                log::info!("[stream] Stream exhausted during pre-buffering after {} samples, proceeding", buffered_samples);
                break;
            }
            Err(e) => {
                prebuffer_error_count += 1;
                log::warn!("[stream] Packet format error during pre-buffer ({}/{}): {}", prebuffer_error_count, MAX_PACKET_ERRORS, e);
                if prebuffer_error_count >= MAX_PACKET_ERRORS {
                    let err_msg = format!("Too many prebuffer packet errors ({}): {}", prebuffer_error_count, e);
                    log::error!("[stream] {}", err_msg);
                    *load_error.lock() = err_msg;
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        let pct = (buffered_samples as f64 / target_buffer_samples as f64 * 100.0) as u32;
        let threshold = pct - (pct % 25);
        if threshold > last_logged_pct {
            log::info!("[stream] Pre-buffered {}%: {} samples", threshold, buffered_samples);
            last_logged_pct = threshold;
        }
    }

    log::info!("[stream] Pre-buffer complete: {} samples", buffered_samples);

    buffer_ready.store(true, Ordering::Relaxed);
    is_playing_flag.store(true, Ordering::Relaxed);
    log::info!("[stream] buffer_ready and is_playing set to true");
    log::info!("[stream] Audio stream started!");

    crate::audio::engine::update_global_spectrum(vec![
        0.1f32;
        crate::audio::engine::get_band_count()
    ]);

    let max_queue_samples = (device_sample_rate as usize) * 10 * (device_channels as usize);
    let mut _packet_error_count = 0;
    let band_count = crate::audio::engine::get_band_count();
    let mut analyzer = RmsSpectrumAnalyzer::new(device_sample_rate, band_count);
    let mut spectrum_accum: std::collections::VecDeque<f32> = std::collections::VecDeque::with_capacity(4096);
    let mut last_spectrum_update = std::time::Instant::now();
    let mut decode_count: u64 = 0;

    loop {
        if should_stop.load(Ordering::Relaxed) {
            log::info!("[stream] Stop requested during decode");
            break;
        }

        if seek_target_ms.load(Ordering::Acquire) > 0 {
            log::info!("[stream] Seek requested during playback, stopping decode");
            break;
        }

        match format_reader.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => {
                _packet_error_count = 0;
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
                            && spectrum_accum.len() >= channels
                        {
                            let channels_usize = channels;
                            let total = spectrum_accum.len() - (spectrum_accum.len() % channels_usize);
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
                        log::error!("[stream] Decode error: {}", e);
                        _packet_error_count += 1;
                        if _packet_error_count >= MAX_PACKET_ERRORS {
                            let err_msg = format!("Too many decode errors ({}): {}", _packet_error_count, e);
                            log::error!("[stream] {}", err_msg);
                            *load_error.lock() = err_msg;
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => {
                log::info!("[stream] Stream ended after {} packets", decode_count);
                break;
            }
            Err(e) => {
                _packet_error_count += 1;
                log::warn!("[stream] Packet format error ({}/{}): {}", _packet_error_count, MAX_PACKET_ERRORS, e);
                if _packet_error_count >= MAX_PACKET_ERRORS {
                    let err_msg = format!("Too many packet format errors ({}): {}", _packet_error_count, e);
                    log::error!("[stream] {}", err_msg);
                    *load_error.lock() = err_msg;
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    log::info!("[stream] Decode complete, waiting for output drain");
    let drain_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !should_stop.load(Ordering::Relaxed) {
        if queue_for_decode.lock().is_empty() {
            log::info!("[stream] Queue empty, done draining");
            break;
        }
        if std::time::Instant::now() > drain_deadline {
            log::error!("[stream] Drain timeout — queue still has {} samples", queue_for_decode.lock().len());
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    log::info!("[stream] Playback complete");
    drop(stream);
    buffer_ready.store(false, Ordering::Relaxed);
    is_playing_flag.store(false, Ordering::Relaxed);
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
    load_error: Arc<parking_lot::Mutex<String>>,
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

    let reader: Box<dyn ReadSeek + Send + Sync + 'static> = if seek_pos > 0 {
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
                let mut resp = match client
                    .get(&fetch_url)
                    .header("User-Agent", "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36")
                    .header("Accept", "audio/mpeg, audio/*;q=0.9, */*;q=0.8")
                    .header("Icy-MetaData", "0")
                    .send()
                    .await
                {
                    Ok(r) if r.status().is_success() => r,
                    Ok(r) => {
                        let err = format!("HTTP {}", r.status());
                        pw.set_error(err);
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

        Box::new(pipe_reader) as Box<dyn ReadSeek + Send + Sync + 'static>
    };

    total_duration_ms.store(cache_max_ms, Ordering::Relaxed);
    decode_and_play_from_read(
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
