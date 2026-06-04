use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VideoQuality {
    Quality144,
    Quality240,
    Quality360,
    Quality480,
    Quality720,
    Quality1080,
    Quality1440,
    Quality2160,
}

impl VideoQuality {
    pub fn from_itag(itag: i64) -> Option<Self> {
        match itag {
            17 => Some(Self::Quality144),
            18 => Some(Self::Quality360),
            22 => Some(Self::Quality720),
            37 => Some(Self::Quality1080),
            43 => Some(Self::Quality360),
            44 => Some(Self::Quality360),
            45 => Some(Self::Quality480),
            46 => Some(Self::Quality480),
            52 => Some(Self::Quality240),
            313 => Some(Self::Quality2160),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AudioQuality {
    QualityDefault,
    Quality48k,
    Quality64k,
    Quality96k,
    Quality128k,
    Quality192k,
    Quality256k,
    Quality312k,
}

impl AudioQuality {
    pub fn from_bitrate(bitrate: i64) -> Self {
        match bitrate {
            b if b >= 312_000 => Self::Quality312k,
            b if b >= 256_000 => Self::Quality256k,
            b if b >= 192_000 => Self::Quality192k,
            b if b >= 128_000 => Self::Quality128k,
            b if b >= 96_000 => Self::Quality96k,
            b if b >= 64_000 => Self::Quality64k,
            b if b >= 48_000 => Self::Quality48k,
            _ => Self::QualityDefault,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamFormat {
    pub itag: i64,
    pub mime_type: String,
    pub bitrate: i64,
    pub quality: VideoQuality,
    pub audio_quality: AudioQuality,
    pub url: String,
    /// Duration in milliseconds, from approxDurationMs in streaming data
    pub approx_duration_ms: Option<u64>,
}

impl StreamFormat {
    pub fn is_audio(&self) -> bool {
        self.mime_type.starts_with("audio/")
    }

    pub fn is_video(&self) -> bool {
        self.mime_type.starts_with("video/")
    }
}
