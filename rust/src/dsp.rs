//! Digital Signal Processing module
//!
//! Provides FFT-based spectrum analysis and equalizer capabilities.

use crate::models::{EqualizerBand, SpectrumData};
use num_complex::Complex;
use rustfft::Fft;
use std::f32::consts::PI;
use std::sync::Arc;

// ── Constants ───────────────────────────────────────────────────────────────

/// Default FFT size; must be a power of two.
const FFT_SIZE: usize = 1024;

/// Default number of Bark-scale output bands.
pub const DEFAULT_SPECTRUM_BANDS: usize = 24;

/// Floor for RMS magnitude before dB conversion (avoids −∞).
const RMS_FLOOR: f32 = 1e-4;

/// dB offset applied after log conversion; calibrates silence level.
const DB_OFFSET: f32 = -51.0;

/// dB range mapped to the normalised output [0, 1].
const DB_FLOOR: f32 = -80.0;
const DB_RANGE: f32 = 80.0;

/// Equalizer gain limits (dB).
const EQ_GAIN_MIN: f32 = -12.0;
const EQ_GAIN_MAX: f32 = 12.0;

// ── Shared Bark-scale helpers ───────────────────────────────────────────────

/// Convert a Bark value to Hz using a piecewise-linear approximation.
#[inline]
fn bark_to_hz(bark: f32) -> f32 {
    if bark < 2.0 {
        bark * 100.0
    } else if bark < 8.0 {
        200.0 + (bark - 2.0) * 200.0
    } else if bark < 15.0 {
        1400.0 + (bark - 8.0) * 400.0
    } else {
        4200.0 + (bark - 15.0) * 800.0
    }
}

/// Return `bands + 1` frequency edges evenly spaced on the Bark scale,
/// each clamped to `max_hz`.
fn bark_scale_edges(bands: usize, max_hz: f32) -> Vec<f32> {
    const MAX_BARK: f32 = 24.0;
    (0..=bands)
        .map(|i| {
            let bark = (i as f32 / bands as f32) * MAX_BARK;
            bark_to_hz(bark).min(max_hz)
        })
        .collect()
}

/// Pre-compute the half-open `[low_bin, high_bin)` range of FFT bins for
/// each Bark band, avoiding per-frame linear scans.
fn precompute_bark_bin_ranges(
    bands: usize,
    sample_rate: u32,
    fft_size: usize,
) -> Vec<(usize, usize)> {
    let bin_hz = sample_rate as f32 / fft_size as f32;
    let num_bins = fft_size / 2;
    let edges = bark_scale_edges(bands, sample_rate as f32 / 2.0);

    (0..bands)
        .map(|band| {
            let low_bin = (edges[band] / bin_hz).floor() as usize;
            let high_bin = (edges[band + 1] / bin_hz).ceil() as usize;
            let low_bin = low_bin.min(num_bins - 1);
            let high_bin = high_bin.clamp(low_bin + 1, num_bins);
            (low_bin, high_bin)
        })
        .collect()
}

/// Build a Hann window of the given size.
fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (size - 1) as f32).cos()))
        .collect()
}

// ── SpectrumConfig ──────────────────────────────────────────────────────────

/// Configuration for spectrum analysis.
#[derive(Clone, Debug, PartialEq)]
pub struct SpectrumConfig {
    /// Number of FFT bins (must be power of 2).
    pub fft_size: usize,
    /// Sample rate of the audio.
    pub sample_rate: u32,
    /// Number of frequency bands to output.
    pub band_count: usize,
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self {
            fft_size: FFT_SIZE,
            sample_rate: 44100,
            band_count: DEFAULT_SPECTRUM_BANDS,
        }
    }
}

// ── SpectrumAnalyzer ────────────────────────────────────────────────────────

/// FFT-based spectrum analyzer (batch / offline usage).
pub struct SpectrumAnalyzer {
    config: SpectrumConfig,
    buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    window: Vec<f32>,
    fft_plan: Arc<dyn Fft<f32>>,
    bark_bin_ranges: Vec<(usize, usize)>,
}

impl SpectrumAnalyzer {
    pub fn new(config: SpectrumConfig) -> Self {
        debug_assert!(
            config.fft_size.is_power_of_two(),
            "fft_size must be a power of two"
        );

        let fft_size = config.fft_size;
        let mut planner = rustfft::FftPlanner::new();
        let fft_plan = planner.plan_fft_forward(fft_size);

        let bark_bin_ranges =
            precompute_bark_bin_ranges(config.band_count, config.sample_rate, fft_size);

        Self {
            config,
            buffer: vec![Complex::default(); fft_size],
            scratch: vec![Complex::default(); fft_plan.get_inplace_scratch_len()],
            window: hann_window(fft_size),
            fft_plan,
            bark_bin_ranges,
        }
    }

    pub fn analyze(&mut self, samples: &[f32]) -> SpectrumData {
        self.analyze_with_config(samples, self.config.clone())
    }

    pub fn analyze_with_config(&mut self, samples: &[f32], config: SpectrumConfig) -> SpectrumData {
        let fft_size = config.fft_size;

        // Update config if different
        if self.config != config {
            self.config = config.clone();
            let mut planner = rustfft::FftPlanner::new();
            self.fft_plan = planner.plan_fft_forward(fft_size);
            self.buffer = vec![Complex::default(); fft_size];
            self.scratch = vec![Complex::default(); self.fft_plan.get_inplace_scratch_len()];
            self.window = hann_window(fft_size);
            self.bark_bin_ranges =
                precompute_bark_bin_ranges(config.band_count, config.sample_rate, fft_size);
        }

        // Fill reusable buffer with windowed samples (zero-pad if input is short)
        for (i, sample) in self.buffer.iter_mut().enumerate().take(fft_size) {
            let s = samples.get(i).copied().unwrap_or(0.0);
            *sample = Complex::new(s * self.window[i], 0.0);
        }
        self.buffer.resize(fft_size, Complex::default());

        self.fft_plan
            .process_with_scratch(&mut self.buffer, &mut self.scratch);

        let magnitudes: Vec<f32> = self
            .buffer
            .iter()
            .take(fft_size / 2)
            .map(|c| (c.norm() / fft_size as f32).sqrt())
            .collect();

        let frequencies: Vec<f32> = (0..fft_size / 2)
            .map(|i| i as f32 * self.config.sample_rate as f32 / fft_size as f32)
            .collect();

        self.downsample_to_bark_bands(&frequencies, &magnitudes)
    }

    fn downsample_to_bark_bands(&self, frequencies: &[f32], magnitudes: &[f32]) -> SpectrumData {
        let band_count = self.config.band_count;

        if frequencies.is_empty() {
            return SpectrumData {
                frequencies: vec![0.0; band_count],
                magnitudes: vec![0.0; band_count],
            };
        }

        let max_freq = frequencies.last().copied().unwrap_or(22050.0);
        let bark_edges = bark_scale_edges(band_count, max_freq);

        let mut output_freqs = Vec::with_capacity(band_count);
        let mut output_mags = Vec::with_capacity(band_count);

        // Binary-search through the sorted frequency array for each band,
        // carrying the search start forward so each band is O(log n) not O(n).
        let mut search_start = 0usize;
        for band in 0..band_count {
            let low_hz = bark_edges[band];
            let high_hz = bark_edges[band + 1];

            let low_idx =
                frequencies[search_start..].partition_point(|&f| f < low_hz) + search_start;
            let high_idx = frequencies[low_idx..].partition_point(|&f| f < high_hz) + low_idx;

            let count = high_idx - low_idx;
            let avg_mag = if count > 0 {
                magnitudes[low_idx..high_idx].iter().sum::<f32>() / count as f32
            } else {
                0.0
            };
            let center_freq = (low_hz * high_hz).sqrt();

            output_freqs.push(center_freq);
            output_mags.push(avg_mag);
            search_start = low_idx;
        }

        SpectrumData {
            frequencies: output_freqs,
            magnitudes: output_mags,
        }
    }
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new(SpectrumConfig::default())
    }
}

/// FFT-based spectrum analyzer that computes RMS per Bark band,
/// matching the approach used by SpectrumSource::bins_to_bark_bands.
/// Produces normalized [0, 1] values with high-frequency boost.
pub struct RmsSpectrumAnalyzer {
    fft_plan: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    bark_ranges: Vec<(usize, usize)>,
    fft_input: Vec<Complex<f32>>,
    fft_scratch: Vec<Complex<f32>>,
    band_count: usize,
}

impl RmsSpectrumAnalyzer {
    pub fn new(sample_rate: u32, band_count: usize) -> Self {
        let mut planner = rustfft::FftPlanner::new();
        let fft_plan = planner.plan_fft_forward(FFT_SIZE);
        let scratch_len = fft_plan.get_inplace_scratch_len();
        Self {
            fft_plan,
            window: hann_window(FFT_SIZE),
            bark_ranges: precompute_bark_bin_ranges(band_count, sample_rate, FFT_SIZE),
            fft_input: vec![Complex::default(); FFT_SIZE],
            fft_scratch: vec![Complex::default(); scratch_len],
            band_count,
        }
    }

    /// Analyze mono samples and return normalized [0, 1] Bark-band values.
    pub fn analyze(&mut self, mono_samples: &[f32]) -> Vec<f32> {
        // Apply window and zero-pad
        for (i, sample) in self.fft_input.iter_mut().enumerate().take(FFT_SIZE) {
            let s = mono_samples.get(i).copied().unwrap_or(0.0);
            *sample = Complex::new(s * self.window[i], 0.0);
        }

        self.fft_plan
            .process_with_scratch(&mut self.fft_input, &mut self.fft_scratch);

        (0..self.band_count)
            .map(|band| {
                let (low, high) = self.bark_ranges[band];
                let bin_count = (high - low) as f32;
                let sum_sq: f32 = self.fft_input[low..high]
                    .iter()
                    .map(|c| c.re * c.re + c.im * c.im)
                    .sum();
                let rms = (sum_sq / bin_count).sqrt();

                // Same dB/normalize/boost/clamp as SpectrumSource::bins_to_bark_bands
                let db = 20.0 * rms.max(RMS_FLOOR).log10() + DB_OFFSET;
                let normalized = (db - DB_FLOOR) / DB_RANGE;
                let t = band as f32 / (self.band_count - 1).max(1) as f32;
                let boost = 1.0 + t * t;
                (normalized * boost).clamp(0.0, 1.0)
            })
            .collect()
    }
}

// ── Equalizer ───────────────────────────────────────────────────────────────

/// Simple parametric equalizer (placeholder — biquad filters not yet wired).
pub struct Equalizer {
    bands: Vec<EqualizerBand>,
    sample_rate: u32,
}

impl Equalizer {
    pub fn new(sample_rate: u32) -> Self {
        let bands = vec![
            EqualizerBand::new(32.0, 0.0),
            EqualizerBand::new(64.0, 0.0),
            EqualizerBand::new(125.0, 0.0),
            EqualizerBand::new(250.0, 0.0),
            EqualizerBand::new(500.0, 0.0),
            EqualizerBand::new(1000.0, 0.0),
            EqualizerBand::new(2000.0, 0.0),
            EqualizerBand::new(4000.0, 0.0),
            EqualizerBand::new(8000.0, 0.0),
            EqualizerBand::new(16000.0, 0.0),
        ];
        Self { bands, sample_rate }
    }

    pub fn set_band_gain(&mut self, index: usize, gain_db: f32) {
        if let Some(band) = self.bands.get_mut(index) {
            band.gain_db = gain_db.clamp(EQ_GAIN_MIN, EQ_GAIN_MAX);
        }
    }

    pub fn get_bands(&self) -> &[EqualizerBand] {
        &self.bands
    }

    pub fn process(&self, samples: &mut [f32]) {
        // Placeholder — a full implementation would use per-band biquad filters
        let _ = (samples, self.sample_rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spectrum_analyzer_creation() {
        let analyzer = SpectrumAnalyzer::default();
        assert_eq!(analyzer.config.fft_size, 1024);
    }

    #[test]
    fn test_spectrum_analysis() {
        let mut analyzer = SpectrumAnalyzer::default();
        let samples: Vec<f32> = (0..1024)
            .map(|i| (2.0 * PI * 440.0 * i as f32 / 44100.0).sin())
            .collect();
        let result = analyzer.analyze(&samples);
        assert!(!result.frequencies.is_empty());
        assert!(!result.magnitudes.is_empty());
        assert_eq!(result.frequencies.len(), result.magnitudes.len());
    }

    #[test]
    fn test_equalizer_creation() {
        let eq = Equalizer::new(44100);
        assert_eq!(eq.get_bands().len(), 10);
    }

    #[test]
    fn test_equalizer_gain_clamping() {
        let mut eq = Equalizer::new(44100);
        eq.set_band_gain(0, 20.0);
        assert_eq!(eq.get_bands()[0].gain_db, 12.0);
        eq.set_band_gain(0, -20.0);
        assert_eq!(eq.get_bands()[0].gain_db, -12.0);
    }

    #[test]
    fn test_bark_to_hz_monotonic() {
        let mut prev = bark_to_hz(0.0);
        for i in 1..=24 {
            let curr = bark_to_hz(i as f32);
            assert!(
                curr > prev,
                "bark_to_hz should be strictly monotonic: bark={i}, prev={prev}, curr={curr}"
            );
            prev = curr;
        }
    }

    #[test]
    fn test_bark_scale_edges_count() {
        let edges = bark_scale_edges(16, 22050.0);
        assert_eq!(edges.len(), 17); // bands + 1
        assert_eq!(edges[0], 0.0);
        assert!(edges[16] <= 22050.0);
    }

    #[test]
    fn test_precomputed_bin_ranges_valid() {
        let ranges = precompute_bark_bin_ranges(16, 44100, 1024);
        assert_eq!(ranges.len(), 16);
        for (low, high) in &ranges {
            assert!(low < high, "bin range must be non-empty: {low}..{high}");
            assert!(*high <= 512, "high bin must not exceed fft_size/2");
        }
    }

    #[test]
    fn test_hann_window_bounds() {
        let w = hann_window(1024);
        assert_eq!(w.len(), 1024);
        assert!((w[0] - 0.0).abs() < 1e-6);
        let peak = w.iter().copied().fold(0.0f32, f32::max);
        assert!((peak - 1.0).abs() < 0.01, "peak was {}", peak);
    }
}
