use std::collections::VecDeque;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::Source;

pub type AudioBuffer = Arc<parking_lot::Mutex<VecDeque<f32>>>;

pub struct QueueSource {
    pub(crate) queue: AudioBuffer,
    pub(crate) channels: u16,
    pub(crate) sample_rate: u32,
    pub(crate) starve_counter: Arc<AtomicU64>,
    pub(crate) done: Arc<AtomicBool>,
    pub(crate) samples_played: Arc<AtomicU64>,
}

impl Source for QueueSource {
    fn current_span_len(&self) -> Option<usize> {
        self.queue.lock().len().into()
    }

    fn total_duration(&self) -> Option<Duration> {
        // Total duration is unknown as it's a dynamic queue
        None
    }

    fn channels(&self) -> NonZero<u16> {
        NonZero::new(self.channels).expect("Number of channels must be non-zero")
    }

    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(self.sample_rate).expect("Sample rate must be non-zero")
    }
}

impl QueueSource {
    pub fn new(
        queue: AudioBuffer,
        channels: u16,
        sample_rate: u32,
        starve_counter: Arc<AtomicU64>,
        samples_played: Arc<AtomicU64>,
    ) -> Self {
        Self {
            queue,
            channels,
            sample_rate,
            starve_counter,
            done: Arc::new(AtomicBool::new(false)),
            samples_played,
        }
    }

    pub fn mark_done(&self) {
        self.done.store(true, Ordering::Release);
    }

    pub fn set_done(&mut self, done: Arc<AtomicBool>) {
        self.done = done;
    }

    pub fn starve_count(&self) -> u64 {
        self.starve_counter.load(Ordering::Relaxed)
    }

    pub fn has_pending(&self) -> bool {
        self.queue.lock().len() > 0
    }
}

pub const QUEUE_BATCH_SIZE: usize = 4096;

impl Iterator for QueueSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let mut queue = self.queue.lock();
        match queue.pop_front() {
            Some(s) => {
                self.samples_played.fetch_add(1, Ordering::Relaxed);
                Some(s)
            }
            None => {
                if self.done.load(Ordering::Acquire) {
                    None
                } else {
                    self.starve_counter.fetch_add(1, Ordering::Relaxed);
                    Some(0.0)
                }
            }
        }
    }
}

pub fn create_buffer(_capacity: usize) -> AudioBuffer {
    Arc::new(parking_lot::Mutex::new(VecDeque::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn make_source() -> (AudioBuffer, QueueSource, Arc<AtomicU64>) {
        let buffer = create_buffer(QUEUE_BATCH_SIZE * 2);
        let starve = Arc::new(AtomicU64::new(0));
        let samples_played = Arc::new(AtomicU64::new(0));
        let source = QueueSource::new(
            buffer.clone(),
            2,
            44100,
            starve.clone(),
            samples_played.clone(),
        );
        (buffer, source, starve)
    }

    #[test]
    fn test_metadata() {
        let (_, source, _) = make_source();
        assert_eq!(source.channels().get(), 2);
        assert_eq!(source.sample_rate().get(), 44100);
    }

    #[test]
    fn test_returns_zero_when_empty() {
        let (_, mut source, starve) = make_source();
        let sample = source.next().unwrap();
        assert_eq!(sample, 0.0);
        assert_eq!(starve.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_returns_none_when_done_and_empty() {
        let (_, mut source, starve) = make_source();
        source.mark_done();
        assert!(source.next().is_none(), "done source should return None");
        assert_eq!(starve.load(Ordering::Relaxed), 0, "no starve when done");
    }

    #[test]
    fn test_done_during_consumption() {
        let (buffer, mut source, starve) = make_source();

        buffer.lock().extend([1.0, 2.0, 3.0]);

        assert_eq!(source.next().unwrap(), 1.0);
        assert_eq!(source.next().unwrap(), 2.0);

        source.mark_done();

        assert_eq!(source.next().unwrap(), 3.0);
        assert!(
            source.next().is_none(),
            "should return None after done flag and queue empty"
        );
        assert_eq!(
            starve.load(Ordering::Relaxed),
            0,
            "should not starve when done"
        );
    }

    #[test]
    fn test_starve_counter_increments_on_empty() {
        let (_, mut source, starve) = make_source();
        assert_eq!(starve.load(Ordering::Relaxed), 0);

        source.next();
        assert_eq!(starve.load(Ordering::Relaxed), 1);

        source.next();
        assert_eq!(starve.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_reads_single_sample() {
        let (buffer, mut source, _) = make_source();
        buffer.lock().push_back(0.5);
        assert_eq!(source.next().unwrap(), 0.5);
    }

    #[test]
    fn test_drains_in_batches() {
        let (buffer, mut source, _) = make_source();
        let data: Vec<f32> = (0..QUEUE_BATCH_SIZE + 100).map(|i| i as f32).collect();
        buffer.lock().extend(data);
        for i in 0..QUEUE_BATCH_SIZE {
            assert_eq!(source.next().unwrap() as usize, i);
        }
    }

    #[test]
    fn test_starves_after_draining() {
        let (buffer, mut source, starve) = make_source();
        buffer.lock().push_back(1.0);
        assert_eq!(source.next().unwrap(), 1.0);
        assert_eq!(starve.load(Ordering::Relaxed), 0);
        assert_eq!(source.next().unwrap(), 0.0);
        assert_eq!(starve.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_has_pending() {
        let (_, source, _) = make_source();
        assert!(
            !source.has_pending(),
            "empty source should not have pending"
        );
    }

    #[test]
    fn test_concurrent_producer_consumer() {
        let (buffer, mut source, _) = make_source();
        let mut received: Vec<f32> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);

        buffer.lock().extend((0..5000).map(|i| i as f32 + 0.001));

        loop {
            if Instant::now() > deadline {
                break;
            }
            let s = source.next().unwrap();
            if s.abs() > 1e-6 {
                received.push(s);
            }
            if received.len() >= 5000 {
                break;
            }
        }

        assert_eq!(received.len(), 5000);
        for (i, &val) in received.iter().enumerate() {
            let expected = i as f32 + 0.001;
            assert!(
                (val - expected).abs() < 0.001,
                "sample mismatch at index {}",
                i
            );
        }
    }
}
