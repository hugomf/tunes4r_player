//! CacheDecorator — wraps a StreamSource and caches its bytes to disk.
//!
//! On first `open()`, the inner source is read fully and written to a cache
//! file. Subsequent `open()` calls serve from the cached file directly,
//! avoiding network I/O.

use crate::audio::error::PlaybackError;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::audio::stream::source::{Capability, SourceInfo, StreamSource};

pub struct CacheDecorator {
    inner: Box<dyn StreamSource>,
    cache_path: PathBuf,
    state: Mutex<CacheState>,
}

enum CacheState {
    Uncached,
    Cached,
    Failed(String),
}

impl CacheDecorator {
    pub fn new(inner: Box<dyn StreamSource>, cache_dir: &str) -> Self {
        let dir = PathBuf::from(cache_dir);
        let _ = std::fs::create_dir_all(&dir);

        let mut hasher = DefaultHasher::new();
        inner.info().uri.hash(&mut hasher);
        let filename = format!("{}.cache", hasher.finish());
        let cache_path = dir.join(filename);

        Self {
            inner,
            cache_path,
            state: Mutex::new(CacheState::Uncached),
        }
    }

    fn cache_path(&self) -> &PathBuf {
        &self.cache_path
    }
}

impl StreamSource for CacheDecorator {
    fn info(&self) -> &SourceInfo {
        self.inner.info()
    }

    fn supports(&self, capability: Capability) -> bool {
        // Cache decorator adds caching capability
        if capability == Capability::Cache {
            return true;
        }
        self.inner.supports(capability)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn open(
        &self,
        seek_to: Option<u64>,
    ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
        let mut state = self.state.lock().unwrap();

        match &*state {
            CacheState::Failed(msg) => {
                return Err(PlaybackError::Cache {
                    detail: msg.clone(),
                });
            }
            CacheState::Cached => {
                let file = std::fs::File::open(self.cache_path()).map_err(|e| {
                    PlaybackError::Cache {
                        detail: format!("Failed to open cache file: {}", e),
                    }
                })?;
                return Ok(Box::new(file));
            }
            CacheState::Uncached => {}
        }

        // First open: read from inner, write to cache
        let mut reader = self.inner.open(seek_to)?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|e| {
            *state = CacheState::Failed(format!("Read failed: {}", e));
            PlaybackError::Cache {
                detail: format!("Failed to read inner source for caching: {}", e),
            }
        })?;

        std::fs::write(self.cache_path(), &bytes).map_err(|e| {
            *state = CacheState::Failed(format!("Write failed: {}", e));
            PlaybackError::Cache {
                detail: format!("Failed to write cache file: {}", e),
            }
        })?;

        *state = CacheState::Cached;
        Ok(Box::new(std::io::Cursor::new(bytes)))
    }

    fn total_bytes(&self) -> Option<u64> {
        let state = self.state.lock().unwrap();
        match &*state {
            CacheState::Cached => {
                std::fs::metadata(self.cache_path()).ok().map(|m| m.len())
            }
            _ => self.inner.total_bytes(),
        }
    }

    fn pipe_writer(&self) -> Option<std::sync::Arc<crate::audio::stream::pipe::PipeWriter>> {
        self.inner.pipe_writer()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::stream::source::SourceKind;

    struct TestSource {
        info: SourceInfo,
        data: Vec<u8>,
    }

    impl TestSource {
        fn new(uri: &str, data: Vec<u8>) -> Self {
            Self {
                info: SourceInfo {
                    kind: SourceKind::Radio,
                    stream_type: crate::models::StreamType::Seekable { total_bytes: data.len() as u64 },
                    uri: uri.to_string(),
                    title: None,
                },
                data,
            }
        }
    }

    impl StreamSource for TestSource {
        fn info(&self) -> &SourceInfo {
            &self.info
        }
        fn supports(&self, capability: Capability) -> bool {
            matches!(capability, Capability::Seek | Capability::Download)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn open(
            &self,
            _seek_to: Option<u64>,
        ) -> Result<Box<dyn Read + Send + Sync + 'static>, PlaybackError> {
            Ok(Box::new(std::io::Cursor::new(self.data.clone())))
        }
    }

    #[test]
    fn test_cache_decorator_adds_cache_capability() {
        let inner = TestSource::new("http://example.com/audio.mp3", vec![0, 1, 2]);
        let decorator = CacheDecorator::new(Box::new(inner), "/tmp/test_cache");
        assert!(decorator.supports(Capability::Cache));
        assert!(decorator.supports(Capability::Seek));
        assert!(decorator.supports(Capability::Download));
    }

    #[test]
    fn test_cache_decorator_caches_and_serves() {
        let data: Vec<u8> = (0..100).collect();
        let tmp = std::env::temp_dir().join("test_cache_decorator");
        let _ = std::fs::remove_dir_all(&tmp);

        let inner = TestSource::new("test://unique-uri-1", data.clone());

        let decorator = CacheDecorator::new(Box::new(inner), tmp.to_str().unwrap());

        // First open — should cache
        let mut reader1 = decorator.open(None).unwrap();
        let mut buf1 = Vec::new();
        reader1.read_to_end(&mut buf1).unwrap();
        assert_eq!(buf1, data);

        // Cache file should exist
        assert!(decorator.cache_path().exists());

        // Second open — should read from cache
        let mut reader2 = decorator.open(None).unwrap();
        let mut buf2 = Vec::new();
        reader2.read_to_end(&mut buf2).unwrap();
        assert_eq!(buf2, data);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cache_decorator_unique_uris() {
        let tmp = std::env::temp_dir().join("test_cache_unique");
        let _ = std::fs::remove_dir_all(&tmp);

        let d1 = CacheDecorator::new(
            Box::new(TestSource::new("http://a.com/1", vec![1])),
            tmp.to_str().unwrap(),
        );
        let d2 = CacheDecorator::new(
            Box::new(TestSource::new("http://a.com/2", vec![2])),
            tmp.to_str().unwrap(),
        );

        assert_ne!(d1.cache_path(), d2.cache_path());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cache_decorator_info_delegates() {
        let inner = TestSource::new("http://example.com/audio.mp3", vec![1, 2, 3]);
        let decorator = CacheDecorator::new(Box::new(inner), "/tmp/test_cache");
        assert_eq!(decorator.info().uri, "http://example.com/audio.mp3");
        assert_eq!(decorator.info().kind, SourceKind::Radio);
    }
}
