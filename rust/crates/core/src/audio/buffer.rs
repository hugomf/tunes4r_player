// ============================================================================
// Constants - Optimized for live streaming
// ============================================================================

pub const DETECT_HEAD_TIMEOUT_MS: u64 = 10_000;
pub const DETECT_RANGE_TIMEOUT_MS: u64 = 10_000;
pub const DETECT_MAX_RETRIES: u32 = 3;
pub const DETECT_RETRY_DELAY_MS: u64 = 500;
pub const PREFILL_SEEKABLE_BYTES: usize = 128 * 1024;
// Reasonable prefill for live streams - enough for decoder to start
pub const PREFILL_LIVE_BYTES: usize = 128 * 1024; // 128KB default
pub const PREFILL_SEEKABLE_TIMEOUT_MS: u128 = 15_000;
pub const PREFILL_LIVE_TIMEOUT_MS: u128 = 10_000; // 10 seconds timeout
pub const READ_WAIT_MS: u64 = 1;
pub const LIVE_MIN_READ_BYTES: usize = 4096;
pub const LIVE_MAX_LAG_BYTES: usize = 64 * 1024;
pub const LIVE_KEEP_AHEAD_BYTES: usize = 512 * 1024;
pub const LIVE_RECONNECT_DELAY_MS: u64 = 100;
  
// Network quality thresholds (bytes per second)
pub const NETWORK_QUALITY_EXCELLENT_THRESHOLD: f64 = 2_000_000.0;
pub const NETWORK_QUALITY_GOOD_THRESHOLD: f64 = 1_000_000.0;
pub const NETWORK_QUALITY_MODERATE_THRESHOLD: f64 = 500_000.0;
pub const NETWORK_QUALITY_POOR_THRESHOLD: f64 = 100_000.0;

// ============================================================================
// Adaptive Buffer Management
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkQuality {
    Excellent,
    Good,
    Moderate,
    Poor,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct BufferConfig {
    pub prefill_bytes: usize,
    pub prefill_timeout_ms: u128,
    pub live_buffer_bytes: usize,
    pub max_buffer_bytes: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            prefill_bytes: PREFILL_SEEKABLE_BYTES,
            prefill_timeout_ms: PREFILL_SEEKABLE_TIMEOUT_MS,
            live_buffer_bytes: 20 * 1024 * 1024,
            max_buffer_bytes: 50 * 1024 * 1024,
        }
    }
}

pub struct AdaptiveBuffer {
    config: BufferConfig,
    samples: Vec<BufferSample>,
    network_quality: NetworkQuality,
}

#[derive(Debug, Clone, Copy)]
struct BufferSample {
    bytes: usize,
    elapsed_ms: u128,
}

impl AdaptiveBuffer {
    pub fn new() -> Self {
        Self {
            config: BufferConfig::default(),
            samples: Vec::with_capacity(10),
            network_quality: NetworkQuality::Unknown,
        }
    }

    pub fn record_sample(&mut self, bytes: usize, elapsed_ms: u128) {
        self.samples.push(BufferSample { bytes, elapsed_ms });
        if self.samples.len() > 10 {
            self.samples.remove(0);
        }
        self.network_quality = self.assess_network_quality();
        self.update_config();
    }

    fn assess_network_quality(&self) -> NetworkQuality {
        if self.samples.is_empty() {
            return NetworkQuality::Unknown;
        }

        let avg_speed = self
            .samples
            .iter()
            .map(|s| s.bytes as f64 / (s.elapsed_ms as f64 / 1000.0).max(1.0))
            .sum::<f64>()
            / self.samples.len() as f64;

        match avg_speed {
            s if s > NETWORK_QUALITY_EXCELLENT_THRESHOLD => NetworkQuality::Excellent,
            s if s > NETWORK_QUALITY_GOOD_THRESHOLD => NetworkQuality::Good,
            s if s > NETWORK_QUALITY_MODERATE_THRESHOLD => NetworkQuality::Moderate,
            s if s > NETWORK_QUALITY_POOR_THRESHOLD => NetworkQuality::Poor,
            _ => NetworkQuality::Unknown,
        }
    }

    fn update_config(&mut self) {
        let (prefill_mult, timeout_mult, buffer_mult) = match self.network_quality {
            NetworkQuality::Excellent => (0.5, 0.5, 0.5),
            NetworkQuality::Good => (0.75, 0.75, 0.75),
            NetworkQuality::Moderate => (1.0, 1.0, 1.0),
            NetworkQuality::Poor => (2.0, 2.0, 2.0),
            NetworkQuality::Unknown => (1.0, 1.0, 1.0),
        };

        self.config.prefill_bytes = (PREFILL_SEEKABLE_BYTES as f64 * prefill_mult) as usize;
        self.config.prefill_timeout_ms =
            (PREFILL_SEEKABLE_TIMEOUT_MS as f64 * timeout_mult) as u128;
        self.config.max_buffer_bytes = ((50 * 1024 * 1024) as f64 * buffer_mult) as usize;
    }

    pub fn config(&self) -> &BufferConfig {
        &self.config
    }

    pub fn network_quality(&self) -> NetworkQuality {
        self.network_quality
    }
}

impl Default for AdaptiveBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adaptive_buffer_default() {
        let buf = AdaptiveBuffer::new();
        assert!(matches!(buf.network_quality(), NetworkQuality::Unknown));
        assert_eq!(buf.config().prefill_bytes, PREFILL_SEEKABLE_BYTES);
    }

    #[test]
    fn test_adaptive_buffer_records_samples() {
        let mut buf = AdaptiveBuffer::new();
        buf.record_sample(1_000_000, 100);
        buf.record_sample(2_000_000, 100);
        assert!(!matches!(buf.network_quality(), NetworkQuality::Unknown));
    }

    #[test]
    fn test_buffer_config_defaults() {
        let config = BufferConfig::default();
        assert_eq!(config.prefill_bytes, PREFILL_SEEKABLE_BYTES);
        assert_eq!(config.prefill_timeout_ms, PREFILL_SEEKABLE_TIMEOUT_MS);
    }
}