use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub struct FileCache {
    file: Arc<Mutex<File>>,
    path: PathBuf,
    cached_bytes: Arc<AtomicU64>,
    max_size: u64,
}

impl FileCache {
    pub fn new(
        cache_dir: &std::path::Path,
        max_size_mb: u64,
        stream_id: &str,
    ) -> std::io::Result<Self> {
        let path = cache_dir.join(format!("cache_{}.bin", stream_id));
        let file = File::options()
            .create_new(true)
            .write(true)
            .read(true)
            .open(&path)?;

        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            path,
            cached_bytes: Arc::new(AtomicU64::new(0)),
            max_size: max_size_mb * 1024 * 1024,
        })
    }

    pub fn append(&self, data: &[u8]) -> std::io::Result<usize> {
        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::End(0))?;
        let written = file.write_all(data)?;
        let new_len = self
            .cached_bytes
            .fetch_add(data.len() as u64, Ordering::Relaxed)
            + data.len() as u64;

        if new_len > self.max_size {
            let excess = new_len - self.max_size;
            let keep_bytes = self.max_size;
            drop(file);
            let mut file = self.file.lock().unwrap();
            let old_data = std::fs::read(&self.path)?;
            let start = excess as usize;
            if start < old_data.len() {
                let keep_data = &old_data[start.min(old_data.len())..];
                file.set_contents(&self.path, keep_data)?;
            }
            self.cached_bytes.store(keep_bytes, Ordering::Relaxed);
            Ok(data.len() - excess as usize)
        } else {
            Ok(data.len())
        }
    }

    pub fn len(&self) -> u64 {
        self.cached_bytes.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) -> std::io::Result<()> {
        self.cached_bytes.store(0, Ordering::Relaxed);
        let mut file = self.file.lock().unwrap();
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}
