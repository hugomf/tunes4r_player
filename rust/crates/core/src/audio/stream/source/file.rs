//! FileSource — local audio files with full seek support.

use crate::audio::error::PlaybackError;
use crate::models::StreamType;

use super::{Capability, ReadSeek, SourceInfo, SourceKind, StreamSource};
use std::fs::File;
use std::path::Path;

use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::meta::StandardTag;
use symphonia::default::get_probe;

pub struct FileSource {
    info: SourceInfo,
}

impl FileSource {
    pub fn new(path: &str) -> Self {
        let total = File::open(path).ok().and_then(|f| f.metadata().ok()).map(|m| m.len());

        let (title, artist, album) = extract_file_metadata(path);

        Self {
            info: SourceInfo {
                kind: SourceKind::File,
                stream_type: StreamType::Seekable {
                    total_bytes: total.unwrap_or(0),
                },
                uri: path.to_string(),
                title: title.or_else(|| {
                    Path::new(path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                }),
                artist,
                album,
            },
        }
    }
}

fn extract_file_metadata(path: &str) -> (Option<String>, Option<String>, Option<String>) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, None, None),
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = match get_probe().probe(
        &Hint::new(), mss,
        FormatOptions::default(), MetadataOptions::default(),
    ) {
        Ok(f) => f,
        Err(_) => return (None, None, None),
    };
    let mut title = None;
    let mut artist = None;
    let mut album = None;
    if let Some(rev) = format.metadata().current() {
        for tag in &rev.media.tags {
            if let Some(std_tag) = &tag.std {
                match std_tag {
                    StandardTag::TrackTitle(t) => title = Some(t.to_string()),
                    StandardTag::Artist(a) => artist = Some(a.to_string()),
                    StandardTag::Album(a) => album = Some(a.to_string()),
                    _ => {}
                }
            }
        }
    }
    (title, artist, album)
}

impl StreamSource for FileSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn supports(&self, capability: Capability) -> bool {
        matches!(capability, Capability::Seek | Capability::Download)
    }

    fn as_any(&self) -> &dyn std::any::Any { self }

    fn open(
        &self,
        _seek_to: Option<u64>,
    ) -> Result<Box<dyn ReadSeek + Send + Sync + 'static>, PlaybackError> {
        let file = File::open(&self.info.uri).map_err(|e| PlaybackError::FileOpen {
            path: self.info.uri.clone(),
            source: e,
        })?;

        // Seek is handled by the decode loop via fast_forward_stream_seek;
        // we just provide the raw file. The decoder will fast-forward packets.
        Ok(Box::new(file))
    }
}
