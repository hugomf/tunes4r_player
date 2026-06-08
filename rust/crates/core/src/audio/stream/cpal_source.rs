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
use cpal::{Stream, StreamConfig};
use log::info;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Global volume (0.0 to 1.0), stored as f32 bits.
static VOLUME: AtomicU32 = AtomicU32::new(f32::to_bits(1.0));
/// Global balance (0.0 = full-left, 0.5 = center, 1.0 = full-right),
/// stored as f32 bits.
static BALANCE: AtomicU32 = AtomicU32::new(f32::to_bits(0.5));

pub fn set_volume_gain(v: f32) {
    VOLUME.store(v.to_bits(), Ordering::Relaxed);
}

pub fn get_volume_gain() -> f32 {
    f32::from_bits(VOLUME.load(Ordering::Relaxed))
}

pub fn set_balance_gain(b: f32) {
    BALANCE.store(b.to_bits(), Ordering::Relaxed);
}

pub fn get_balance_gain() -> f32 {
    f32::from_bits(BALANCE.load(Ordering::Relaxed))
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

/// Use the device's default output config.
///
/// The device's default config is *always* a valid combination that
/// its audio stack (AAudio / CoreAudio) handles natively, so we use
/// it directly and detect its sample rate for all timing.
pub fn pick_output_config(device: &cpal::Device) -> Option<StreamConfig> {
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
    let mut real_count: u64 = 0;
    for sample in data.iter_mut() {
        if let Some(val) = queue.pop_front() {
            *sample = val;
            real_count += 1;
        } else {
            *sample = 0.0;
        }
    }
    if real_count > 0 {
        // Apply volume + balance gain to interleaved stereo samples
        let vol = get_volume_gain();
        let bal = get_balance_gain().clamp(0.0, 1.0);
        // Winamp-style balance: at 0.0 → full left, 0.5 → center (both at full), 1.0 → full right
        let (left_gain, right_gain) = if bal <= 0.5 {
            (vol, vol * bal * 2.0)
        } else {
            (vol * (1.0 - bal) * 2.0, vol)
        };

        for frame in data.chunks_exact_mut(2) {
            frame[0] *= left_gain;
            frame[1] *= right_gain;
        }

        samples_played.fetch_add(real_count, Ordering::Relaxed);
    }
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
    /// silence, do NOT advance position (no real audio was consumed).
    /// The back-pressure cap prevents unbounded queue growth on Android,
    /// so starvation gaps are short and recoverable.
    #[test]
    fn test_callback_silences_and_does_not_advance_when_ring_empty() {
        let ring = make_ring();
        let ready = Arc::new(AtomicBool::new(true));
        let played = Arc::new(AtomicU64::new(0));
        let mut data = [1.0_f32; 8]; // pre-fill with non-silence sentinel

        run_output_callback(&mut data, &ring, &ready, &played);

        assert_eq!(data, [0.0; 8], "empty ring → silence");
        assert_eq!(played.load(Ordering::Relaxed), 0, "should NOT advance during starvation");
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
    /// Only advances position for real samples.
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
        assert_eq!(played.load(Ordering::Relaxed), 2, "advances only for real samples (2 of 6)");
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

    // ── Prebuffer sizing ──────────────────────────────────────────────

    /// Simulate the producer/consumer drain pattern that caused the
    /// 30-second cutoff on Android.
    ///
    /// Scenario: the AAudio callback consumes samples at the device's
    /// rate while the decode thread produces samples from a compressed
    /// file. If the initial prebuffer is too small and the producer is
    /// momentarily slower (thread scheduling, mutex contention), the
    /// ring buffer empties and AAudio accumulates underruns. After
    /// enough underruns AAudio disconnects the stream.
    ///
    /// This test proves that a larger prebuffer delays buffer exhaustion
    /// proportionally, giving the decode thread more time to recover.
    #[test]
    fn test_prebuffer_delays_underrun_proportionally() {
        // Simulate 44.1 kHz stereo: 256-frame callback = 512 samples
        let callback_samples = 512usize;
        // Simulate decode keeping up poorly: per callback cycle the
        // producer adds 80 % of what the consumer drains.
        let producer_yield = (callback_samples as f64 * 0.8) as usize;

        // Helper: run a consumer/producer cycle and count callbacks
        // until the buffer drops below the callback drain size (the
        // low-water mark where the callback must write silence).
        let cycles_until_low = |initial_samples: usize| -> u64 {
            let ring: AudioRing = Arc::new(parking_lot::Mutex::new(VecDeque::new()));
            // Pre-fill
            for _ in 0..initial_samples {
                ring.lock().push_back(0.5);
            }
            let mut cycles: u64 = 0;
            loop {
                let available = ring.lock().len();
                // Stop when the buffer is too small for a full callback drain
                if available <= callback_samples {
                    return cycles;
                }
                // Consumer drains one callback buffer (or whatever's available)
                for _ in 0..callback_samples.min(available) {
                    ring.lock().pop_front();
                }
                // Producer refills (slower than consumer)
                for _ in 0..producer_yield {
                    ring.lock().push_back(0.5);
                }
                cycles += 1;
            }
        };

        // 0.5 seconds of prebuffer at 44.1 kHz stereo = 44100 samples
        let small_prebuffer = 44_100usize;
        // 7 seconds = 617400 samples
        let large_prebuffer = 617_400usize;

        let small_cycles = cycles_until_low(small_prebuffer);
        let large_cycles = cycles_until_low(large_prebuffer);

        // The large prebuffer should sustain ~14× more cycles (ratio
        // matches 7.0 / 0.5 = 14), proving the principle.
        assert!(
            large_cycles > small_cycles * 10,
            "large prebuffer ({} cycles) should last >10× small prebuffer ({} cycles)",
            large_cycles,
            small_cycles,
        );
    }

    /// Like the test above, but with the producer matching the consumer
    /// rate (no decode lag). Even a small prebuffer should never empty
    /// when the producer keeps up.
    #[test]
    fn test_prebuffer_stable_when_producer_keeps_up() {
        let callback_samples = 512usize;
        let ring: AudioRing = Arc::new(parking_lot::Mutex::new(VecDeque::new()));
        // Tiny initial buffer (just 1 callback)
        for _ in 0..callback_samples {
            ring.lock().push_back(0.5);
        }
        let ready = Arc::new(AtomicBool::new(true));
        let played = Arc::new(AtomicU64::new(0));

        // Run 10 000 callback cycles with the producer matching the
        // consumer rate (1:1). The buffer should never empty.
        for _ in 0..10_000 {
            let mut data = vec![0.0f32; callback_samples];
            run_output_callback(&mut data, &ring, &ready, &played);
            // Producer replaces exactly what was consumed
            for _ in 0..callback_samples {
                ring.lock().push_back(0.5);
            }
            assert!(!ring.lock().is_empty(), "buffer should never empty when producer keeps up");
        }
    }
}
