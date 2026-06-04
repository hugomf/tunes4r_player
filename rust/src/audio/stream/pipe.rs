use log::debug;

use std::collections::VecDeque;
use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

struct SharedPipeState {
    buffer: VecDeque<u8>,
    done: bool,
    error: Option<String>,
}

#[derive(Clone)]
pub struct PipeWriter {
    state: Arc<(Mutex<SharedPipeState>, Condvar)>,
    seek_offset: Arc<AtomicI64>,
    total_bytes: Arc<AtomicU64>,
}

impl PipeWriter {
    pub fn push(&self, data: &[u8]) {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.buffer.extend(data);
        cvar.notify_one();
    }

    pub fn end(&self) {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.done = true;
        cvar.notify_all();
    }

    pub fn restart(&self) {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.done = false;
        state.buffer.clear();
        state.error = None;
        cvar.notify_all();
    }

    pub fn set_error(&self, msg: String) {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.error = Some(msg);
        cvar.notify_all();
    }

    pub fn set_total_bytes(&self, n: u64) {
        self.total_bytes.store(n, Ordering::Relaxed);
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes.load(Ordering::Relaxed)
    }

    pub fn take_seek_request(&self) -> Option<u64> {
        let v = self.seek_offset.swap(-1, Ordering::AcqRel);
        if v >= 0 {
            // Differentiate from probing seeks. If it's a small offset, it's likely
            // Symphonia probing, ignore it for Dart's re-fetch.
            // Real seeks (from user or Symphonia after initial probe) will be larger.
            const PROBE_SEEK_THRESHOLD_BYTES: u64 = 10;
            if v as u64 > PROBE_SEEK_THRESHOLD_BYTES {
                Some(v as u64)
            } else {
                debug!("[pipe] Ignoring small probe seek to {} bytes", v);
                None
            }
        } else {
            None
        }
    }

    pub fn set_seek_offset(&self, offset: u64) {
        self.seek_offset.store(offset as i64, Ordering::Release);
    }
}

pub struct PipeReader {
    state: Arc<(Mutex<SharedPipeState>, Condvar)>,
    seek_offset: Arc<AtomicI64>,
    position: u64,
}

impl PipeReader {
    pub fn new(pipe_writer: &PipeWriter) -> Self {
        Self {
            state: pipe_writer.state.clone(),
            seek_offset: pipe_writer.seek_offset.clone(),
            position: 0,
        }
    }
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();

        while state.buffer.is_empty() && !state.done && state.error.is_none() {
            state = cvar.wait(state).unwrap();
        }

        if let Some(ref err) = state.error {
            return Err(std::io::Error::other(err.clone()));
        }

        if state.buffer.is_empty() && state.done {
            return Ok(0);
        }

        let to_read = state.buffer.len().min(buf.len());
        for (i, b) in state.buffer.drain(..to_read).enumerate() {
            buf[i] = b;
        }
        self.position += to_read as u64;
        Ok(to_read)
    }
}

impl Seek for PipeReader {
    fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> {
        let target = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(delta) => {
                let new_pos = self.position as i64 + delta;
                if new_pos < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek before start of pipe stream",
                    ));
                }
                new_pos as u64
            }
            SeekFrom::End(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "SeekFrom::End not supported in pipe mode",
                ));
            }
        };

        debug!(
            "[pipe] seek requested: {} → {} bytes",
            self.position, target
        );

        self.seek_offset.store(target as i64, Ordering::Release);

        {
            let (lock, cvar) = &*self.state;
            let mut state = lock.lock().unwrap();
            state.buffer.clear();
            state.done = false;
            state.error = None;
            cvar.notify_all();
        }

        self.position = target;
        Ok(target)
    }
}

pub fn new_pipe() -> (PipeWriter, PipeReader) {
    let state = Arc::new((
        Mutex::new(SharedPipeState {
            buffer: VecDeque::new(),
            done: false,
            error: None,
        }),
        Condvar::new(),
    ));

    let seek_offset = Arc::new(AtomicI64::new(-1));
    let total_bytes = Arc::new(AtomicU64::new(0));

    (
        PipeWriter {
            state: state.clone(),
            seek_offset: seek_offset.clone(),
            total_bytes: total_bytes.clone(),
        },
        PipeReader {
            state,
            seek_offset,
            position: 0,
        },
    )
}
