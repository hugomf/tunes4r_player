//! Domain models for the audio engine
//!
//! These are pure data structures with no business logic.

use serde::{Deserialize, Serialize};

/// Domain model for a song
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub file_path: String,
}

impl Song {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        file_path: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            file_path: file_path.into(),
        }
    }

    pub fn with_artist(mut self, artist: impl Into<String>) -> Self {
        self.artist = artist.into();
        self
    }

    pub fn with_album(mut self, album: impl Into<String>) -> Self {
        self.album = album.into();
        self
    }

    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }
}

/// Audio playback state
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    /// Resolving stream type via HEAD/Range probe
    Connecting,
    /// Download started, waiting for enough bytes to decode
    Buffering {
        buffered_bytes: u64,
        total_bytes: Option<u64>,
    },
    /// Decoder is being initialized (parsing headers/metadata)
    Decoding,
    Playing,
    Paused,
    /// Unrecoverable error — message is human-readable
    Error(String),
}

/// Stream metadata including content-type information
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StreamMetadata {
    pub total_bytes: Option<u64>,
    pub is_seekable: bool,
    pub stream_type: StreamType,
    pub content_type: Option<String>,
}

/// Stream type enumeration
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum StreamType {
    /// HTTP file with Content-Length + Accept-Ranges — full seeking supported
    Seekable { total_bytes: u64 },
    /// Live Icecast/Shoutcast — no duration, rolling buffer only
    Live { buffer_window_bytes: usize },
}

impl StreamType {
    pub fn total_bytes(&self) -> Option<u64> {
        match self {
            StreamType::Seekable { total_bytes } => Some(*total_bytes),
            StreamType::Live { .. } => None,
        }
    }
}

impl Default for StreamType {
    fn default() -> Self {
        Self::Live {
            buffer_window_bytes: 20 * 1024 * 1024,
        }
    }
}

impl PlaybackState {
    pub fn is_playing(&self) -> bool {
        matches!(self, Self::Playing)
    }

    pub fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped)
    }

    /// Convert to integer for FFI (0=Stopped, 1=Connecting, 2=Buffering, 3=Decoding, 4=Playing, 5=Paused, 6=Error)
    pub fn to_i32(&self) -> i32 {
        match self {
            Self::Stopped => 0,
            Self::Connecting => 1,
            Self::Buffering { .. } => 2,
            Self::Decoding => 3,
            Self::Playing => 4,
            Self::Paused => 5,
            Self::Error(_) => 6,
        }
    }
}

/// Equalizer band configuration
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct EqualizerBand {
    pub frequency: f32,
    pub gain_db: f32,
}

impl EqualizerBand {
    pub fn new(frequency: f32, gain_db: f32) -> Self {
        Self { frequency, gain_db }
    }
}

/// Spectrum analysis result
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct SpectrumData {
    pub frequencies: Vec<f32>,
    pub magnitudes: Vec<f32>,
}

/// Playback position information
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct PlaybackPosition {
    pub current_ms: u64,
    pub total_ms: u64,
}

impl PlaybackPosition {
    pub fn progress_ratio(&self) -> f32 {
        if self.total_ms == 0 {
            0.0
        } else {
            self.current_ms as f32 / self.total_ms as f32
        }
    }
}
