//! Internal state management and helper methods for the PlaybackEngine.

use super::types::{PlaybackEngine, PlaybackType};
use crate::models::{
    DownloadBuffer, EngineEvent, PlaybackPosition, PlaybackState, ENGINE_EVENT_NONE,
    ENGINE_EVENT_POSITION_RESET, ENGINE_EVENT_SEEK_COMPLETED, ENGINE_EVENT_SEEK_STARTED,
    ENGINE_EVENT_STATE_CHANGED,
};
use log::{debug, warn};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

impl PlaybackEngine {
    pub(super) fn join_with_timeout(handle: thread::JoinHandle<()>, label: &str) {
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if handle.is_finished() {
                let _ = handle.join();
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        warn!(
            "[engine] Thread '{}' failed to join within timeout, detaching",
            label
        );
    }

    pub fn calculate_byte_offset_for_seek(&self, position_ms: u64) -> u64 {
        let total_ms = self.total_duration_ms.load(Ordering::Relaxed);
        let total_bytes = self.pipe_total_bytes.load(Ordering::Relaxed);
        if total_ms > 0 && total_bytes > 0 {
            (position_ms * total_bytes) / total_ms
        } else {
            0
        }
    }

    pub fn get_pipe_seek_info(&self) -> Option<(String, u64, u64)> {
        if let Some(PlaybackType::Pipe { url, .. }) = &self.playback_type {
            let seek_target = self.seek_target_ms.load(Ordering::Relaxed);
            if seek_target > 0 {
                let byte_offset = self.calculate_byte_offset_for_seek(seek_target);
                Some((url.clone(), seek_target, byte_offset))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn get_state(&self) -> PlaybackState {
        self.state.clone()
    }

    /// Set the playback state and emit a `STATE_CHANGED` event.
    /// Use this anywhere `self.state = ...` is set from a public-facing
    /// command so the Dart side is notified without polling.
    pub fn set_state(&mut self, new_state: PlaybackState) {
        if self.state != new_state {
            self.state = new_state.clone();
            self.push_event(EngineEvent {
                event_type: ENGINE_EVENT_STATE_CHANGED,
                int_param: new_state.to_i32() as i64,
            });
        }
    }

    pub fn get_position(&self) -> PlaybackPosition {
        let raw_samples = self.samples_played.load(Ordering::Relaxed);
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1);
        let ch = self.channels.load(Ordering::Relaxed).max(1);
        let total_ms = self.total_duration_ms.load(Ordering::Relaxed);
        let current_ms = (raw_samples * 1000) / (rate * ch);
        PlaybackPosition {
            current_ms,
            total_ms,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing_flag.load(Ordering::Relaxed)
    }

    pub fn get_buffered_position(&self) -> u64 {
        let raw_samples = self.samples_played.load(Ordering::Relaxed);
        let queue_len = self.audio_queue.lock().len() as u64;
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1);
        let ch = self.channels.load(Ordering::Relaxed).max(1);
        let current_ms = (raw_samples * 1000) / (rate * ch);
        let queue_frames = queue_len / ch.max(1);
        let buffered_ms = (queue_frames * 1000) / rate;
        if rate > 0 {
            current_ms + buffered_ms
        } else {
            current_ms
        }
    }

    pub fn get_load_error(&self) -> Option<String> {
        let err = self.load_error.lock().clone();
        if err.is_empty() {
            None
        } else {
            Some(err)
        }
    }

    pub fn get_pipe_seek_request(&self) -> Option<(String, u64)> {
        if let Some(PlaybackType::Pipe { url, .. }) = &self.playback_type {
            if let Some(seek_offset) = self
                .stream_pipe
                .as_ref()
                .and_then(|pipe| pipe.take_seek_request())
            {
                let total_bytes = self.pipe_total_bytes.load(Ordering::Relaxed);
                let total_ms = self.total_duration_ms.load(Ordering::Relaxed);
                let seek_target_ms = if total_bytes > 0 && total_ms > 0 {
                    (seek_offset * total_ms) / total_bytes
                } else {
                    (seek_offset * 1000) / 44100 / 2
                };
                debug!(
                    "[engine] get_pipe_seek_request: {} bytes -> {} ms",
                    seek_offset, seek_target_ms
                );
                Some((url.clone(), seek_target_ms))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn clear_pipe_seek_request(&mut self) {
        self.seek_target_ms.store(0, Ordering::Relaxed);
    }

    pub fn get_pipe_total_bytes(&self) -> u64 {
        self.pipe_total_bytes.load(Ordering::Relaxed)
    }

    pub fn set_pipe_total_bytes(&self, value: u64) {
        self.pipe_total_bytes.store(value, Ordering::Relaxed)
    }

    pub fn push_audio_bytes(&self, data: &[u8]) {
        if let Some(pipe) = &self.stream_pipe {
            pipe.push(data);
        }
    }

    pub fn end_audio_stream(&self) {
        if let Some(pipe) = &self.stream_pipe {
            pipe.end();
        }
    }

    pub fn set_stream_error(&self, message: &str) {
        if let Some(pipe) = &self.stream_pipe {
            pipe.set_error(message.to_string());
        }
    }

    pub fn get_pipe_url(&self) -> Option<String> {
        if let Some(PlaybackType::Pipe { url, .. }) = &self.playback_type {
            Some(url.clone())
        } else {
            None
        }
    }

    pub fn get_stream_pipe(&self) -> Option<Arc<crate::audio::stream::pipe::PipeWriter>> {
        self.stream_pipe.clone()
    }

    pub fn restart_pipe(&self) {
        if let Some(pipe) = &self.stream_pipe {
            pipe.restart();
        }
    }

    pub fn set_spectrum_band_count(&mut self, count: usize) {
        self.band_count = count;
        super::types::set_band_count(count);
    }

    pub fn get_spectrum_internal(&self, max_bands: usize) -> Option<Vec<f32>> {
        let spectrum = super::types::GLOBAL_SPECTRUM.read().unwrap();
        let count = spectrum.len().min(max_bands).min(32);
        Some(spectrum[..count].to_vec())
    }

    pub fn get_seek_target_ms(&self) -> u64 {
        self.seek_target_ms.load(Ordering::Relaxed)
    }

    pub fn clear_seek_target(&mut self) {
        self.seek_target_ms.store(0, Ordering::Relaxed);
    }

    /// Push an event to the queue. Drops the oldest event if the queue
    /// grows past a reasonable cap so a slow consumer can't OOM the engine.
    pub fn push_event(&self, event: EngineEvent) {
        let mut q = self.event_queue.lock();
        if q.len() >= 256 {
            q.pop_front();
        }
        q.push_back(event);
    }

    /// Push a state-changed event with the new state encoded as i32.
    pub fn push_state_event(&self, state: &PlaybackState) {
        self.push_event(EngineEvent {
            event_type: ENGINE_EVENT_STATE_CHANGED,
            int_param: state.to_i32() as i64,
        });
    }

    pub fn push_seek_started(&self, position_ms: u64) {
        self.push_event(EngineEvent {
            event_type: ENGINE_EVENT_SEEK_STARTED,
            int_param: position_ms as i64,
        });
    }

    pub fn push_seek_completed(&self, position_ms: u64) {
        self.push_event(EngineEvent {
            event_type: ENGINE_EVENT_SEEK_COMPLETED,
            int_param: position_ms as i64,
        });
    }

    pub fn push_position_reset(&self) {
        self.push_event(EngineEvent {
            event_type: ENGINE_EVENT_POSITION_RESET,
            int_param: 0,
        });
    }

    /// Pop the next event from the queue. Returns `EngineEvent::default()`
    /// (event_type = NONE) when the queue is empty.
    pub fn poll_event(&self) -> EngineEvent {
        let mut q = self.event_queue.lock();
        q.pop_front().unwrap_or(EngineEvent {
            event_type: ENGINE_EVENT_NONE,
            int_param: 0,
        })
    }

    /// Read the current download buffer state.
    pub fn get_download_buffer(&self) -> DownloadBuffer {
        *self.download_buffer.lock()
    }

    /// Update the download buffer state. Called by the streaming code as
    /// bytes arrive. No-op if the new state is identical to the current.
    pub fn set_download_buffer(&self, new: DownloadBuffer) {
        let mut current = self.download_buffer.lock();
        if *current != new {
            *current = new;
        }
    }

    /// Reset the download buffer to "empty, unknown". Called when starting
    /// a new playback so stale state from a previous track doesn't leak.
    pub fn reset_download_buffer(&self) {
        *self.download_buffer.lock() = DownloadBuffer::default();
    }
}
