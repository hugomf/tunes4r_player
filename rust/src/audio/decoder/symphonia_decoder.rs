//! Symphonia-based audio decoder with rodio integration
//!
//! This module provides audio decoding using Symphonia 0.6 directly,
//! with output as f32 samples for use with rodio.

use log::debug;

use crate::audio::stream::reader::SeekableStreamReader;
use crate::models::StreamMetadata;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;

use std::io::Read; // ADD THIS IMPORT
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct SymphoniaDecoder {
    sample_rate: u32,
    channels: u16,
    total_ms: u64,
    receiver: Receiver<f32>,
    is_playing: Arc<AtomicBool>,
}

impl SymphoniaDecoder {
    pub fn new(
        mut reader: SeekableStreamReader,
        metadata: StreamMetadata,
    ) -> Result<(Self, StreamMetadata), String> {
        debug!("[symphonia] Creating decoder...");

        // CRITICAL: We need to pass a Read + Seek type, but ReadOnlySource needs ownership
        // We'll create a temporary buffer to hold pre-loaded data
        let mut pre_buffer = Vec::new();
        let mut temp_buf = [0u8; 8192];

        for _ in 0..50 {
            match reader.read(&mut temp_buf) {
                Ok(n) if n > 0 => {
                    pre_buffer.extend_from_slice(&temp_buf[..n]);
                    if pre_buffer.len() > 131072 {
                        break;
                    }
                }
                _ => break,
            }
        }

        debug!("[symphonia] Pre-buffered {} bytes", pre_buffer.len());

        // Create a cursor from the pre-buffered data
        let cursor = std::io::Cursor::new(pre_buffer);
        let source = ReadOnlySource::new(cursor);
        let mss = MediaSourceStream::new(Box::new(source), Default::default());

        let hint = Hint::new();

        debug!("[symphonia] Probing format...");
        let probed = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| format!("Format detection failed: {}", e))?;

        debug!("[symphonia] Format detected successfully");

        let format = probed;

        let track = format
            .first_track(symphonia::core::formats::TrackType::Audio)
            .ok_or_else(|| "No audio track found".to_string())?;

        let codec_params = match track.codec_params.as_ref() {
            Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params,
            _ => return Err("No audio codec parameters".to_string()),
        };

        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .unwrap_or(2);

        debug!(
            "[symphonia] Stream info: {} Hz, {} channels",
            sample_rate, channels
        );

        let mut registry = CodecRegistry::new();
        registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
        registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
        registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
        registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
        let decoder = registry
            .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
            .map_err(|e| format!("Decoder creation failed: {}", e))?;

        let (sample_tx, sample_rx) = channel::<f32>();

        let track_id = track.id;
        let total_ms = 0;
        let is_playing = Arc::new(AtomicBool::new(true));
        let is_playing_clone = is_playing.clone();

        debug!("[symphonia] Starting decode thread...");
        thread::spawn(move || {
            decode_loop(format, decoder, track_id, sample_tx, is_playing_clone);
        });

        Ok((
            Self {
                sample_rate,
                channels,
                total_ms,
                receiver: sample_rx,
                is_playing,
            },
            metadata,
        ))
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn total_duration(&self) -> Option<Duration> {
        if self.total_ms > 0 {
            Some(Duration::from_millis(self.total_ms))
        } else {
            None
        }
    }

    pub fn stop(&self) {
        self.is_playing.store(false, Ordering::Relaxed);
    }

    pub fn into_source(self) -> SymphoniaSource {
        SymphoniaSource {
            receiver: self.receiver,
            buffer: Vec::new(),
            sample_rate: self.sample_rate,
            channels: self.channels,
        }
    }
}

fn decode_loop(
    mut format: Box<dyn symphonia::core::formats::FormatReader>,
    mut decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    track_id: u32,
    tx: Sender<f32>,
    is_playing: Arc<AtomicBool>,
) {
    let mut packet_count = 0;
    let mut sample_count = 0;

    while is_playing.load(Ordering::Relaxed) {
        match format.next_packet() {
            Ok(Some(packet)) if packet.track_id == track_id => match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let mut samples: Vec<f32> = Vec::new();
                    audio_buf.copy_to_vec_interleaved(&mut samples);
                    sample_count += samples.len();
                    packet_count += 1;

                    if packet_count % 100 == 0 {
                        debug!(
                            "[symphonia] Decoded {} packets, {} samples",
                            packet_count, sample_count
                        );
                    }

                    for sample in samples {
                        if tx.send(sample).is_err() {
                            debug!("[symphonia] Receiver dropped, stopping decode");
                            return;
                        }
                    }
                }
                Err(e) => {
                    debug!("[symphonia] Decode error: {}", e);
                    thread::sleep(Duration::from_millis(10));
                }
            },
            Ok(Some(_)) => continue,
            Ok(None) => {
                debug!("[symphonia] No more packets, stream ended");
                break;
            }
            Err(e) => {
                debug!("[symphonia] Packet error: {}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    debug!(
        "[symphonia] Decode loop ended ({} packets, {} samples)",
        packet_count, sample_count
    );
}

pub struct SymphoniaSource {
    receiver: Receiver<f32>,
    buffer: Vec<f32>,
    sample_rate: u32,
    channels: u16,
}

impl Iterator for SymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // Try to fill buffer if empty
        if self.buffer.is_empty() {
            // Try to receive multiple samples at once
            let mut count = 0;
            while count < 1024 {
                match self.receiver.try_recv() {
                    Ok(s) => {
                        self.buffer.push(s);
                        count += 1;
                    }
                    Err(_) => break,
                }
            }

            // If still empty, block for one sample
            if self.buffer.is_empty() {
                match self.receiver.recv() {
                    Ok(s) => self.buffer.push(s),
                    Err(_) => return None,
                }
            }
        }

        self.buffer.pop()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.buffer.len(), None)
    }
}

impl rodio::Source for SymphoniaSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.buffer.len())
    }

    fn sample_rate(&self) -> std::num::NonZero<u32> {
        std::num::NonZero::new(self.sample_rate)
            .unwrap_or_else(|| std::num::NonZero::new(44100).unwrap())
    }

    fn channels(&self) -> std::num::NonZero<u16> {
        std::num::NonZero::new(self.channels).unwrap_or_else(|| std::num::NonZero::new(2).unwrap())
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
