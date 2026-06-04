// ============================================================================
// Error Types
// ============================================================================

pub use crate::audio::buffer::{
    AdaptiveBuffer, BufferConfig, DETECT_HEAD_TIMEOUT_MS, DETECT_MAX_RETRIES,
    DETECT_RANGE_TIMEOUT_MS, DETECT_RETRY_DELAY_MS, LIVE_KEEP_AHEAD_BYTES, LIVE_MAX_LAG_BYTES,
    LIVE_MIN_READ_BYTES, LIVE_RECONNECT_DELAY_MS, NETWORK_QUALITY_EXCELLENT_THRESHOLD,
    NETWORK_QUALITY_GOOD_THRESHOLD, NETWORK_QUALITY_MODERATE_THRESHOLD,
    NETWORK_QUALITY_POOR_THRESHOLD, PREFILL_LIVE_BYTES, PREFILL_LIVE_TIMEOUT_MS,
    PREFILL_SEEKABLE_BYTES, PREFILL_SEEKABLE_TIMEOUT_MS, READ_WAIT_MS,
};

#[derive(Debug, thiserror::Error)]
pub enum PlaybackError {
    #[error("Failed to open file '{path}': {source}")]
    FileOpen {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to initialize audio output device: {detail}")]
    OutputInit { detail: String },

    #[error("Audio device lost: {detail}")]
    DeviceLost { detail: String },

    #[error("No audio device found: {detail}")]
    NoDevice { detail: String },

    #[error("HTTP stream error: {detail}")]
    HttpStream { operation: String, detail: String },

    #[error("HTTP {status_code} for {url}: {detail}")]
    HttpStatus {
        url: String,
        status_code: u16,
        detail: String,
    },

    #[error("Seek not supported for this source: {detail}")]
    SeekNotSupported { detail: String },

    #[error("Seek failed at position {position_ms}ms: {detail}")]
    SeekFailed { position_ms: u64, detail: String },

    #[error("Background thread error: {detail}")]
    ThreadSpawn { operation: String, detail: String },

    #[error("Async runtime error: {detail}")]
    RuntimeError { detail: String },

    #[error("Cache error: {detail}")]
    Cache { detail: String },
}

impl From<std::io::Error> for PlaybackError {
    fn from(err: std::io::Error) -> Self {
        PlaybackError::HttpStream {
            operation: "I/O operation".into(),
            detail: err.to_string(),
        }
    }
}
