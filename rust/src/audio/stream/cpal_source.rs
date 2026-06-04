//! Shared cpal output stream + ring buffer pattern used by all three
//! platform decoders (macOS, iOS, Android).
//!
//! This is the simplest defensible audio output shape:
//!   1. A producer (symphonia decode loop) pushes interleaved f32
//!      samples into a shared `VecDeque` ring buffer.
//!   2. A cpal output stream callback pops samples from the ring and
//!      writes them to the device buffer. If the ring is empty, it
//!      writes silence (the consumer waits for the producer).
//!   3. A `buffer_ready` flag gates whether the stream is allowed to
//!      drain the queue (used to prebuffer without glitches).
//!   4. `samples_played` is incremented on every sample that actually
//!      makes it to the device, for position reporting.
//!
//! The same struct/method shape works on macOS (coreaudio), iOS
//! (coreaudio), Android (aaudio), Linux (alsa), and Windows (wasapi).

use cpal::traits::DeviceTrait;
use cpal::{SampleFormat, Stream, StreamConfig};
use log::info;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Type alias for the shared ring buffer between the decode thread
/// (producer) and the cpal callback (consumer).
///
/// This used to live in `queue_source` (as `AudioBuffer`) when rodio
/// drove the output. After the cpal-everywhere migration, the
/// ring-buffer pattern is the only shape used, so it lives here.
pub type AudioBuffer = Arc<parking_lot::Mutex<VecDeque<f32>>>;
/// Backward-compatible alias; `AudioBuffer` is the canonical name now.
pub type AudioRing = AudioBuffer;

/// Pick a cpal output config matching the decoded audio's sample rate
/// and channel count, falling back to the device's default if no exact
/// match is available.
///
/// Returns `None` if no config can be obtained (no outputs, or device
/// reports no configs).
pub fn pick_output_config(
    device: &cpal::Device,
    sample_rate: u32,
    channels: u16,
) -> Option<StreamConfig> {
    if let Ok(mut configs) = device.supported_output_configs() {
        // First try: exact channel match + sample rate in range.
        if let Some(cfg) = configs.find(|c| {
            c.min_sample_rate() <= sample_rate
                && c.max_sample_rate() >= sample_rate
                && c.channels() == channels
        }) {
            let supported = cfg.with_sample_rate(sample_rate);
            // Prefer F32 to avoid per-sample quantization in the callback.
            if supported.sample_format() == SampleFormat::F32 {
                return Some(supported.config());
            }
        }
        // Second try: sample rate matches, channels differ (accept
        // the device's native channel count; resampling is a follow-up).
        if let Some(cfg) = configs.find(|c| {
            c.min_sample_rate() <= sample_rate && c.max_sample_rate() >= sample_rate
        }) {
            return Some(cfg.with_sample_rate(sample_rate).config());
        }
    }

    // Fall back to default config.
    device.default_output_config().ok().map(|c| c.config())
}

/// Build a cpal output stream that drains from `audio_ring`.
///
/// Callback behavior:
///   - If `buffer_ready` is `false`, outputs silence (used during
///     prebuffer so the user never hears pre-seek audio).
///   - Otherwise pops interleaved f32 samples from `audio_ring`,
///     writing silence when the ring is empty.
///   - Increments `samples_played` for every sample the device
///     actually consumes (used for position reporting).
///
/// The stream is **not** playing when returned. The caller must call
/// `.play()` on it after prebuffer is complete.
pub fn build_output_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_ring: AudioRing,
    buffer_ready: Arc<AtomicBool>,
    samples_played: Arc<AtomicU64>,
) -> Result<Stream, String> {
    let audio_ring_c = audio_ring.clone();
    let buffer_ready_c = buffer_ready.clone();
    let samples_played_c = samples_played.clone();

    device
        .build_output_stream(
            config,
            move |data: &mut [f32], _| {
                if !buffer_ready_c.load(Ordering::Relaxed) {
                    data.fill(0.0);
                    return;
                }
                let mut queue = audio_ring_c.lock();
                let mut count: u64 = 0;
                for sample in data.iter_mut() {
                    *sample = queue.pop_front().unwrap_or(0.0);
                    count += 1;
                }
                if count > 0 {
                    samples_played_c.fetch_add(count, Ordering::Relaxed);
                }
            },
            |err| info!("[cpal_source] stream error: {}", err),
            None,
        )
        .map_err(|e| format!("Failed to build output stream: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_ring_clone_shares_state() {
        let ring: AudioRing = Arc::new(parking_lot::Mutex::new(VecDeque::new()));
        let ring2 = ring.clone();
        ring.lock().push_back(1.0);
        ring.lock().push_back(2.0);
        assert_eq!(ring2.lock().pop_front(), Some(1.0));
        assert_eq!(ring2.lock().pop_front(), Some(2.0));
        assert_eq!(ring2.lock().pop_front(), None);
    }

    /// The signature of `build_output_stream` is what the cpal callback
    /// closure must conform to. We don't call it here (no device), but
    /// the type-check confirms the API surface is consistent.
    #[test]
    fn test_build_output_stream_signature_compiles() {
        fn _check(
            device: &cpal::Device,
            config: &StreamConfig,
            ring: AudioRing,
            ready: Arc<AtomicBool>,
            played: Arc<AtomicU64>,
        ) -> Result<Stream, String> {
            build_output_stream(device, config, ring, ready, played)
        }
        // Just ensure the function is callable in this shape.
        let _ = _check;
    }
}
