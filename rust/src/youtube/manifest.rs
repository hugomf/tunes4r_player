use crate::youtube::formats::StreamFormat;

#[derive(Debug, Clone)]
pub struct StreamManifest {
    pub audio: Vec<StreamFormat>,
    pub video: Vec<StreamFormat>,
}

impl StreamManifest {
    pub fn audio_only(&self) -> Vec<&StreamFormat> {
        self.audio.iter().collect()
    }

    pub fn with_video(&self) -> Vec<&StreamFormat> {
        self.video.iter().collect()
    }

    pub fn best_audio(&self) -> Option<&StreamFormat> {
        self.audio.iter().max_by_key(|f| f.bitrate)
    }

    pub fn best_video(&self) -> Option<&StreamFormat> {
        self.video.iter().max_by_key(|f| f.quality)
    }
}
