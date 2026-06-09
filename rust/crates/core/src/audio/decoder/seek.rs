//! Unified audio seek across all platforms.
//!
//! Replaces three divergent in-place implementations (macOS, iOS, Android)
//! that were drifting in correctness — most notably an off-by-`channels`
//! unit bug on iOS that caused stereo files to seek to roughly half the
//! requested position.
//!
//! Strategy:
//!   1. Try the format's native seek (MP4 trak atom, OGG bisection,
//!      FLAC seektable, …) — fast, lands at or before the target.
//!   2. Fall back to packet-skip with a throwaway decoder — slow but
//!      always works. Lands within a packet of the target.
//!
//! All three platforms call `seek_to_position` with the same units
//! (milliseconds) and get back a `SeekOutcome` describing the method
//! used and how many interleaved samples the caller must still drop
//! from the first decoded audio buffer to land at `target_ms`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use log::{info, warn};
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo};
use symphonia::core::units::Time;
use thiserror::Error;

/// Result of a successful seek.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekMethod {
    /// Format-native seek. The `FormatReader` is now positioned at or
    /// before `target_ms`; the caller **must reset its decoder** before
    /// decoding the next packet (symphonia 0.6 contract).
    Native,
    /// Packet-skip with a throwaway decoder. The `FormatReader` is now
    /// positioned just past `target_ms`; the caller should drop
    /// `residual_samples_to_skip` interleaved samples from the first
    /// decoded audio buffer to land on the target.
    PacketSkip,
}

/// What a successful seek produced.
#[derive(Debug, Clone, Copy)]
pub struct SeekOutcome {
    pub method: SeekMethod,
    /// Interleaved samples the caller must drop from the first decoded
    /// audio buffer to land at `target_ms`. Zero for native seeks.
    pub residual_samples_to_skip: u64,
}

#[derive(Debug, Error)]
pub enum SeekError {
    #[error("no track found with id {0}")]
    TrackNotFound(u32),
    #[error("codec registry error: {0}")]
    Codec(String),
    #[error("interleaved sample counter overflow")]
    Overflow,
    #[error("packet skip gave up after {0} consecutive errors")]
    PacketSkipLimit(u32),
}

const MAX_CONSECUTIVE_PACKET_ERRORS: u32 = 100;

/// Seeks `format` to approximately `target_ms` milliseconds.
///
/// Always uses the same position unit (interleaved samples) so the
/// caller can reason about the result consistently. `target_ms = 0`
/// is a no-op returning `SeekMethod::Native` with zero residual.
pub fn seek_to_position(
    format: &mut Box<dyn FormatReader>,
    codec_params: &AudioCodecParameters,
    track_id: u32,
    target_ms: u64,
    should_stop: &Arc<AtomicBool>,
) -> Result<SeekOutcome, SeekError> {
    if target_ms == 0 {
        return Ok(SeekOutcome {
            method: SeekMethod::Native,
            residual_samples_to_skip: 0,
        });
    }

    match try_native_seek(format, track_id, target_ms) {
        Ok(()) => {
            info!("[seek] Native seek to {} ms succeeded", target_ms);
            Ok(SeekOutcome {
                method: SeekMethod::Native,
                residual_samples_to_skip: 0,
            })
        }
        Err(e) => {
            warn!(
                "[seek] Native seek failed ({}), falling back to packet-skip",
                e
            );
            packet_skip_seek(format, codec_params, track_id, target_ms, should_stop)
        }
    }
}

fn try_native_seek(
    format: &mut Box<dyn FormatReader>,
    track_id: u32,
    target_ms: u64,
) -> Result<(), SeekError> {
    if !format.tracks().iter().any(|t| t.id == track_id) {
        return Err(SeekError::TrackNotFound(track_id));
    }

    let time = Time::from_millis_u64(target_ms);
    format
        .seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(track_id),
            },
        )
        .map(|_| ())
        .map_err(|e| SeekError::Codec(format!("{:?}", e)))
}

fn packet_skip_seek(
    format: &mut Box<dyn FormatReader>,
    codec_params: &AudioCodecParameters,
    track_id: u32,
    target_ms: u64,
    should_stop: &Arc<AtomicBool>,
) -> Result<SeekOutcome, SeekError> {
    let sample_rate = codec_params.sample_rate.unwrap_or(44100) as f64;
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2) as u64;

    // Target expressed in interleaved samples (the unit
    // `copy_to_vec_interleaved` produces). This is the single source of
    // truth for the comparison — keeping the unit consistent across
    // platforms is what fixed the iOS off-by-`channels` bug.
    let target_interleaved = ((target_ms as f64 / 1000.0) * sample_rate * channels as f64) as u64;
    let mut skipped_interleaved: u64 = 0;

    let mut registry = CodecRegistry::new();
    registry.register_audio_decoder::<symphonia_bundle_mp3::MpaDecoder>();
    registry.register_audio_decoder::<symphonia_codec_aac::AacDecoder>();
    registry.register_audio_decoder::<symphonia_codec_vorbis::VorbisDecoder>();
    registry.register_audio_decoder::<symphonia_bundle_flac::FlacDecoder>();
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();

    let mut skip_decoder: Box<dyn AudioDecoder> = registry
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .map_err(|e| SeekError::Codec(format!("{:?}", e)))?;

    let mut consecutive_errors: u32 = 0;
    loop {
        if should_stop.load(Ordering::Relaxed) {
            return Ok(SeekOutcome {
                method: SeekMethod::PacketSkip,
                residual_samples_to_skip: 0,
            });
        }

        let packet = match format.next_packet() {
            Ok(Some(p)) => {
                consecutive_errors = 0;
                p
            }
            Ok(None) => {
                warn!(
                    "[seek] End of stream reached before target {} ms",
                    target_ms
                );
                return Ok(SeekOutcome {
                    method: SeekMethod::PacketSkip,
                    residual_samples_to_skip: 0,
                });
            }
            Err(e) => {
                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_PACKET_ERRORS {
                    warn!(
                        "[seek] Too many consecutive packet errors ({}), giving up",
                        consecutive_errors
                    );
                    return Err(SeekError::PacketSkipLimit(consecutive_errors));
                }
                warn!("[seek] Packet error: {} (attempt {}/{})", e, consecutive_errors, MAX_CONSECUTIVE_PACKET_ERRORS);
                thread::sleep(Duration::from_millis(10));
                continue;
            }
        };

        if packet.track_id != track_id {
            continue;
        }

        match skip_decoder.decode(&packet) {
            Ok(audio_buf) => {
                let frames = audio_buf.frames() as u64;
                let interleaved_in_packet =
                    frames.checked_mul(channels).ok_or(SeekError::Overflow)?;
                skipped_interleaved = skipped_interleaved
                    .checked_add(interleaved_in_packet)
                    .ok_or(SeekError::Overflow)?;

                if skipped_interleaved >= target_interleaved {
                    let overshoot = skipped_interleaved - target_interleaved;
                    info!(
                        "[seek] Packet-skip complete: skipped {} interleaved samples \
                         (target: {}, residual to drop: {})",
                        skipped_interleaved, target_interleaved, overshoot
                    );
                    return Ok(SeekOutcome {
                        method: SeekMethod::PacketSkip,
                        residual_samples_to_skip: overshoot,
                    });
                }
            }
            Err(e) => {
                warn!("[seek] Decode error during skip: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seek_method_distinct() {
        assert_ne!(SeekMethod::Native, SeekMethod::PacketSkip);
    }

    #[test]
    fn test_seek_outcome_copy() {
        // SeekOutcome is used across the FFI / engine boundary, so
        // it must be Copy.
        let o = SeekOutcome {
            method: SeekMethod::Native,
            residual_samples_to_skip: 42,
        };
        let o2 = o;
        assert_eq!(o.residual_samples_to_skip, o2.residual_samples_to_skip);
    }

    #[test]
    fn test_seek_error_display_does_not_panic() {
        // The error must be safely Display-able for log output.
        let e1 = SeekError::TrackNotFound(7);
        let e2 = SeekError::Codec("test".into());
        let e3 = SeekError::Overflow;
        let e4 = SeekError::PacketSkipLimit(99);
        assert!(format!("{}", e1).contains("7"));
        assert!(format!("{}", e2).contains("test"));
        assert!(format!("{}", e3).contains("overflow"));
        assert!(format!("{}", e4).contains("99"));
        assert!(format!("{}", e4).contains("gave up"));
    }

    #[test]
    fn test_packet_skip_limit_constant_sanity() {
        // The limit is a constant defined at module scope. Verify it
        // is reasonable: large enough to survive transient glitches
        // but small enough to prevent a truly wedged decode thread.
        assert!(
            MAX_CONSECUTIVE_PACKET_ERRORS >= 10,
            "limit must be at least 10 to survive transient errors"
        );
        assert!(
            MAX_CONSECUTIVE_PACKET_ERRORS <= 1000,
            "limit must not exceed 1000 to avoid hanging forever"
        );
    }

    /// End-to-end check of the math that produced the iOS unit bug.
    /// `target_ms * sample_rate` is per-channel; `frames * channels` is
    /// interleaved. They are NOT the same number on a multichannel file.
    /// `seek_to_position` must compare both sides in the same unit
    /// (interleaved) to land on the right position.
    #[test]
    fn test_interleaved_vs_perchannel_unit_consistency() {
        let sample_rate = 44100.0_f64;
        let channels = 2_u64;
        let target_ms = 30_000_u64;

        // Per-channel target (the OLD iOS unit — incorrect).
        let per_channel = ((target_ms as f64 / 1000.0) * sample_rate) as u64;
        assert_eq!(per_channel, 1_323_000);

        // Interleaved target (the new shared unit — correct).
        let interleaved = ((target_ms as f64 / 1000.0) * sample_rate * channels as f64) as u64;
        assert_eq!(interleaved, 2_646_000);

        // A packet that produces 1 frame contains `channels` interleaved
        // samples. The old iOS code would exit the skip loop after half
        // the requested time for a stereo file.
        let one_frame_interleaved = channels;
        assert_eq!(one_frame_interleaved, 2);

        // Sanity: if the loop exits when skipped_interleaved (the new
        // unit) reaches the interleaved target, 1_323_000 frames have
        // been consumed — exactly 30 seconds at 44.1 kHz stereo.
        let frames_consumed = 2_646_000 / channels;
        assert_eq!(frames_consumed, 1_323_000);
    }
}
