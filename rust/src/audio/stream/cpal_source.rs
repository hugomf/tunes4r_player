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
use cpal::{SampleFormat, Stream, StreamConfig, SupportedStreamConfigRange};
use log::info;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Device-agnostic view of a cpal `SupportedStreamConfigRange`.
///
/// cpal 0.17.3 keeps the fields of `SupportedStreamConfigRange`
/// `pub(crate)`, which means callers can't construct one. This
/// struct exposes the same shape with public fields, so the
/// config-selection logic can be tested without a real device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigRange {
    pub channels: u16,
    pub min_sample_rate: u32,
    pub max_sample_rate: u32,
    pub sample_format: SampleFormat,
}

impl ConfigRange {
    /// Translate from cpal's `SupportedStreamConfigRange`. This is
    /// the only place that needs to know the cpal struct exists;
    /// everything else operates on `ConfigRange`.
    pub fn from_cpal(c: SupportedStreamConfigRange) -> Self {
        Self {
            channels: c.channels(),
            min_sample_rate: c.min_sample_rate(),
            max_sample_rate: c.max_sample_rate(),
            sample_format: c.sample_format(),
        }
    }

    /// True iff `sample_rate` is within this config's supported range.
    pub fn supports_sample_rate(&self, sample_rate: u32) -> bool {
        self.min_sample_rate <= sample_rate && self.max_sample_rate >= sample_rate
    }
}

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
    if let Ok(configs) = device.supported_output_configs() {
        let ranges: Vec<ConfigRange> = configs.map(ConfigRange::from_cpal).collect();
        if let Some(cfg) = pick_output_config_from_ranges(&ranges, sample_rate, channels) {
            return Some(cfg);
        }
    }

    // Fall back to default config.
    device.default_output_config().ok().map(|c| c.config())
}

/// Pure (device-free) version of the config selection logic.
///
/// Three-stage fallback, in order:
///   1. Exact match: `channels` match AND `sample_rate` in the range,
///      preferring F32 sample format (no per-sample quantization in the
///      callback hot path).
///   2. Sample-rate-only match: any config that supports `sample_rate`
///      (accepts device's native channel count; resampling is a follow-up).
///   3. `None` — the caller should fall back to the device's default.
///
/// Takes a slice of `ConfigRange` so it can be tested without a real
/// device (cpal 0.17 keeps `SupportedStreamConfigRange`'s fields
/// `pub(crate)`).
pub fn pick_output_config_from_ranges(
    configs: &[ConfigRange],
    sample_rate: u32,
    channels: u16,
) -> Option<StreamConfig> {
    // First try: exact channel match + sample rate in range.
    if let Some(cfg) = configs
        .iter()
        .find(|c| c.channels == channels && c.supports_sample_rate(sample_rate))
    {
        // Prefer F32 to avoid per-sample quantization in the callback.
        if cfg.sample_format == SampleFormat::F32 {
            return Some(StreamConfig {
                channels: cfg.channels,
                sample_rate,
                buffer_size: cpal::BufferSize::Default,
            });
        }
    }
    // Second try: sample rate matches, channels differ (accept
    // the device's native channel count; resampling is a follow-up).
    if let Some(cfg) = configs
        .iter()
        .find(|c| c.supports_sample_rate(sample_rate))
    {
        return Some(StreamConfig {
            channels: cfg.channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        });
    }
    None
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
                run_output_callback(data, &audio_ring_c, &buffer_ready_c, &samples_played_c);
            },
            |err| info!("[cpal_source] stream error: {}", err),
            None,
        )
        .map_err(|e| format!("Failed to build output stream: {}", e))
}

/// The cpal output stream callback, extracted as a free function so
/// it can be tested without a real audio device.
///
/// Behavior:
///   - If `buffer_ready` is `false`, fills `data` with silence and
///     returns. This is how the prebuffer phase silences the device
///     while the decode thread fills the ring buffer.
///   - Otherwise, pops interleaved f32 samples from `audio_ring` and
///     writes them to `data`. If the ring is empty, writes silence
///     (the consumer waits for the producer to catch up).
///   - Increments `samples_played` by the number of samples the device
///     requested, even when the ring is empty — this keeps the
///     position counter monotonically advancing during starvation
///     gaps, so the UI doesn't appear frozen.
pub fn run_output_callback(
    data: &mut [f32],
    audio_ring: &AudioRing,
    buffer_ready: &Arc<AtomicBool>,
    samples_played: &Arc<AtomicU64>,
) {
    if !buffer_ready.load(Ordering::Relaxed) {
        data.fill(0.0);
        return;
    }
    let mut queue = audio_ring.lock();
    let mut count: u64 = 0;
    for sample in data.iter_mut() {
        *sample = queue.pop_front().unwrap_or(0.0);
        count += 1;
    }
    if count > 0 {
        samples_played.fetch_add(count, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a single F32 ConfigRange covering a sample rate.
    fn f32_range(channels: u16, min: u32, max: u32) -> ConfigRange {
        ConfigRange {
            channels,
            min_sample_rate: min,
            max_sample_rate: max,
            sample_format: SampleFormat::F32,
        }
    }

    /// Helper: build a single I16 ConfigRange covering a sample rate.
    fn i16_range(channels: u16, min: u32, max: u32) -> ConfigRange {
        ConfigRange {
            channels,
            min_sample_rate: min,
            max_sample_rate: max,
            sample_format: SampleFormat::I16,
        }
    }

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

    // ── pick_output_config_from_ranges ─────────────────────────────────

    /// Exact match wins: F32, 2 ch, 44.1 kHz → returned with the
    /// requested sample rate.
    #[test]
    fn test_pick_exact_match_f32_preferred() {
        let configs = vec![f32_range(2, 44100, 44100)];
        let cfg = pick_output_config_from_ranges(&configs, 44100, 2).expect("should match");
        assert_eq!(cfg.channels, 2);
        assert_eq!(cfg.sample_rate, 44100);
    }

    /// When the exact match exists but is I16 (not F32), the function
    /// falls through to the second-stage sample-rate-only match, which
    /// finds the same config and returns it (I16, not F32 — that's the
    /// "prefer F32" soft preference, not a hard requirement).
    #[test]
    fn test_pick_exact_match_non_f32_falls_through_to_same_config() {
        // Only one config available: I16 / 2 ch.
        let configs = vec![i16_range(2, 44100, 44100)];
        let cfg = pick_output_config_from_ranges(&configs, 44100, 2).expect("should match");
        assert_eq!(cfg.channels, 2);
        assert_eq!(cfg.sample_rate, 44100);
    }

    /// When both F32 (wrong channels) and F32 (right channels) exist,
    /// the F32 right-channels one wins.
    #[test]
    fn test_pick_f32_right_channels_wins_over_f32_wrong_channels() {
        let configs = vec![f32_range(1, 44100, 44100), f32_range(2, 44100, 44100)];
        let cfg = pick_output_config_from_ranges(&configs, 44100, 2).expect("should match");
        assert_eq!(cfg.channels, 2);
    }

    /// Second-stage fallback: no exact channel match, but a config
    /// supports the sample rate → use it (device's native channel count).
    #[test]
    fn test_pick_sample_rate_only_fallback_accepts_different_channels() {
        // Device only offers 1-channel output, decoded audio is stereo.
        let configs = vec![f32_range(1, 44100, 44100)];
        let cfg = pick_output_config_from_ranges(&configs, 44100, 2).expect("should match");
        assert_eq!(cfg.channels, 1); // accepted device's native count
        assert_eq!(cfg.sample_rate, 44100);
    }

    /// No config supports the requested sample rate → return None.
    /// The caller is expected to fall back to `default_output_config()`.
    #[test]
    fn test_pick_no_support_returns_none() {
        let configs = vec![f32_range(2, 48000, 96000)];
        assert!(pick_output_config_from_ranges(&configs, 44100, 2).is_none());
    }

    /// Sample rate is on the edge of the supported range — should still match.
    #[test]
    fn test_pick_sample_rate_at_range_boundary() {
        let configs = vec![f32_range(2, 8000, 44100)];
        let cfg = pick_output_config_from_ranges(&configs, 44100, 2).expect("boundary match");
        assert_eq!(cfg.sample_rate, 44100);
        // Below the range — no match.
        assert!(pick_output_config_from_ranges(&configs, 7999, 2).is_none());
    }

    /// `ConfigRange::supports_sample_rate` is the boundary check
    /// (`min <= sr <= max`) used by the selection logic. Pin it down.
    #[test]
    fn test_config_range_supports_sample_rate_boundaries() {
        let r = f32_range(2, 8000, 48000);
        assert!(r.supports_sample_rate(8000));
        assert!(r.supports_sample_rate(44100));
        assert!(r.supports_sample_rate(48000));
        assert!(!r.supports_sample_rate(7999));
        assert!(!r.supports_sample_rate(48001));
    }

    /// Empty config list → None.
    #[test]
    fn test_pick_empty_configs_returns_none() {
        let configs: Vec<ConfigRange> = vec![];
        assert!(pick_output_config_from_ranges(&configs, 44100, 2).is_none());
    }

    // ── run_output_callback ───────────────────────────────────────────

    fn make_ring() -> AudioRing {
        Arc::new(parking_lot::Mutex::new(VecDeque::new()))
    }

    /// Prebuffer gate: when `buffer_ready` is false, the callback
    /// fills the device buffer with silence and does NOT touch
    /// `samples_played` (we're not playing anything yet).
    #[test]
    fn test_callback_silences_when_buffer_not_ready() {
        let ring = make_ring();
        ring.lock().push_back(0.5); // pretend the producer is done
        let ready = Arc::new(AtomicBool::new(false));
        let played = Arc::new(AtomicU64::new(0));
        let mut data = [0.0_f32; 4];

        run_output_callback(&mut data, &ring, &ready, &played);

        assert_eq!(data, [0.0; 4], "should be silence, not 0.5");
        assert_eq!(played.load(Ordering::Relaxed), 0, "should not advance position");
        // Ring is untouched — sample is still there for when ready=true.
        assert_eq!(ring.lock().len(), 1);
    }

    /// Starvation: ring is empty but `buffer_ready` is true → output
    /// silence, advance position so the UI keeps moving. The producer
    /// is expected to catch up before any audible glitch.
    #[test]
    fn test_callback_silences_and_advances_when_ring_empty() {
        let ring = make_ring();
        let ready = Arc::new(AtomicBool::new(true));
        let played = Arc::new(AtomicU64::new(0));
        let mut data = [1.0_f32; 8]; // pre-fill with non-silence sentinel

        run_output_callback(&mut data, &ring, &ready, &played);

        assert_eq!(data, [0.0; 8], "empty ring → silence");
        assert_eq!(played.load(Ordering::Relaxed), 8, "still advances during starvation");
    }

    /// Happy path: ring has enough samples to fill the device buffer.
    #[test]
    fn test_callback_drains_ring_into_data() {
        let ring = make_ring();
        for v in [0.1, 0.2, 0.3, 0.4] {
            ring.lock().push_back(v);
        }
        let ready = Arc::new(AtomicBool::new(true));
        let played = Arc::new(AtomicU64::new(100));
        let mut data = [0.0_f32; 4];

        run_output_callback(&mut data, &ring, &ready, &played);

        assert_eq!(data, [0.1, 0.2, 0.3, 0.4]);
        assert!(ring.lock().is_empty());
        assert_eq!(played.load(Ordering::Relaxed), 104);
    }

    /// Partial drain: ring has fewer samples than the device wants.
    /// Drains what's there, fills the rest with silence.
    #[test]
    fn test_callback_partial_drain_fills_remainder_with_silence() {
        let ring = make_ring();
        ring.lock().push_back(0.7);
        ring.lock().push_back(0.8);
        let ready = Arc::new(AtomicBool::new(true));
        let played = Arc::new(AtomicU64::new(0));
        let mut data = [0.0_f32; 6];

        run_output_callback(&mut data, &ring, &ready, &played);

        assert_eq!(data, [0.7, 0.8, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(played.load(Ordering::Relaxed), 6, "advances by full buffer size");
    }

    /// Backpressure: ring has more samples than the device wants.
    /// The callback leaves the excess in the ring for the next call.
    #[test]
    fn test_callback_leaves_excess_in_ring() {
        let ring = make_ring();
        for v in [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0] {
            ring.lock().push_back(v);
        }
        let ready = Arc::new(AtomicBool::new(true));
        let played = Arc::new(AtomicU64::new(0));
        let mut data = [0.0_f32; 4];

        run_output_callback(&mut data, &ring, &ready, &played);

        assert_eq!(data, [0.1, 0.2, 0.3, 0.4]);
        // The remaining 6 samples are still in the ring.
        let rest: Vec<f32> = ring.lock().iter().copied().collect();
        assert_eq!(rest, vec![0.5, 0.6, 0.7, 0.8, 0.9, 1.0]);
    }

    /// `data` of length 0 → no-op, no position advance.
    #[test]
    fn test_callback_zero_length_data_is_noop() {
        let ring = make_ring();
        ring.lock().push_back(0.42);
        let ready = Arc::new(AtomicBool::new(true));
        let played = Arc::new(AtomicU64::new(0));
        let mut data: [f32; 0] = [];

        run_output_callback(&mut data, &ring, &ready, &played);

        assert_eq!(played.load(Ordering::Relaxed), 0);
        assert_eq!(ring.lock().len(), 1, "ring untouched on empty data");
    }
}
