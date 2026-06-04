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
        // rodio 0.22 treats `Some(0)` as "source exhausted" (see
        // `Source::is_exhausted` in rodio/src/source/mod.rs). For a dynamic
        // streaming queue we must only report `Some(0)` once the producer is
        // truly done; otherwise a momentarily empty queue would cause rodio's
        // mixer to permanently detach this source mid-playback.
        let len = self.queue.lock().len();
        if len == 0 && self.done.load(Ordering::Acquire) {
            Some(0)
        } else {
            None
        }
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

    // ─────────────────────────────────────────────────────────────────────────
    // Regression tests for the Android "playback stops after ~30s" bug.
    //
    // Root cause: rodio 0.22's `Source::is_exhausted()` is defined as
    // `current_span_len() == Some(0)` (rodio/src/source/mod.rs:187), and
    // `UniformSourceIterator::bootstrap` wraps the source in
    // `Take { n: span_len.map(|x| x.min(32768)) }` (uniform.rs:54-56).
    //
    // If `current_span_len()` returns `Some(0)` while the queue is merely
    // momentarily empty (producer still feeding), rodio's mixer permanently
    // detaches the source mid-stream and closes AAudio.
    //
    // The contract we must uphold for ALL streaming queue sources:
    //   - `Some(0)`  ⇔  producer marked done AND queue empty (truly exhausted)
    //   - `None`     ⇔  more samples may arrive; do not detach
    //   - Anything else: never report `Some(0)` for a "drainable but live" queue.
    // ─────────────────────────────────────────────────────────────────────────

    /// The original 30s bug: an empty queue without `done` MUST NOT report
    /// `Some(0)`, otherwise rodio considers the source exhausted and removes it.
    #[test]
    fn test_current_span_len_empty_queue_not_done_is_none() {
        let (_, source, _) = make_source();
        assert_eq!(
            source.current_span_len(),
            None,
            "empty live queue must report None, never Some(0) — \
             returning Some(0) here is what caused Android playback to die at ~30s"
        );
    }

    /// `Some(0)` is reserved exclusively for the genuinely-exhausted state.
    #[test]
    fn test_current_span_len_empty_queue_done_is_some_zero() {
        let (_, source, _) = make_source();
        source.mark_done();
        assert_eq!(
            source.current_span_len(),
            Some(0),
            "queue empty AND producer done should report Some(0) (truly exhausted)"
        );
    }

    /// A non-empty queue is always "alive" — span length is unknown ahead of time.
    #[test]
    fn test_current_span_len_with_samples_is_none() {
        let (buffer, source, _) = make_source();
        buffer.lock().extend([0.1, 0.2, 0.3, 0.4]);
        assert_eq!(
            source.current_span_len(),
            None,
            "a streaming queue has no predetermined span length"
        );
    }

    /// Even when the producer has marked done, as long as samples remain
    /// queued we are NOT exhausted yet — the consumer must be allowed to drain.
    #[test]
    fn test_current_span_len_done_but_queue_not_empty_is_none() {
        let (buffer, source, _) = make_source();
        buffer.lock().extend([1.0, 2.0]);
        source.mark_done();
        assert_eq!(
            source.current_span_len(),
            None,
            "done + non-empty queue means draining, not exhausted"
        );
    }

    /// Direct check of rodio's `Source::is_exhausted()` (which is what the
    /// mixer/uniform-source-iterator actually consult). This is the precise
    /// behavior that broke Android: a transient empty queue must not be
    /// reported as exhausted.
    #[test]
    fn test_is_exhausted_contract_for_rodio_mixer() {
        let (buffer, source, _) = make_source();

        // 1. Brand new, empty, live → not exhausted.
        assert!(
            !source.is_exhausted(),
            "live empty queue must not be is_exhausted() — would kill source in rodio mixer"
        );

        // 2. Samples arrive → still not exhausted.
        buffer.lock().extend([1.0; 8]);
        assert!(!source.is_exhausted(), "queue with samples is not exhausted");

        // 3. Producer done, samples still buffered → still not exhausted (draining).
        source.mark_done();
        assert!(
            !source.is_exhausted(),
            "done flag while draining must not flip is_exhausted() to true"
        );

        // 4. Consumer drains the buffer → only NOW exhausted.
        buffer.lock().clear();
        assert!(
            source.is_exhausted(),
            "queue empty + done must finally report exhausted"
        );
    }

    /// Simulates the rodio `UniformSourceIterator::bootstrap` pattern:
    ///     `let span_len = input.current_span_len().map(|x| x.min(32768));`
    ///     `Take { iter: input, n: span_len }`
    /// With the old buggy impl returning `Some(0)`, `Take.n` was `Some(0)` and
    /// Take immediately yielded `None`, which made the mixer remove the source.
    /// The fix returns `None`, which makes `Take` unbounded (`n = None`).
    #[test]
    fn test_uniform_source_bootstrap_does_not_get_take_zero() {
        let (_, source, _) = make_source();

        // Mirrors rodio's bootstrap math exactly.
        let span_len = source.current_span_len().map(|x| x.min(32768));

        assert_ne!(
            span_len,
            Some(0),
            "rodio's UniformSourceIterator::bootstrap would build `Take {{ n: Some(0) }}` \
             from this, immediately yielding None and causing the mixer to detach the source. \
             This is the exact mechanism behind the Android 30s playback cutoff."
        );
        assert_eq!(
            span_len, None,
            "for a live streaming queue, bootstrap should see an unbounded Take (n = None)"
        );
    }

    /// End-to-end simulation of a producer that pauses/catches up while the
    /// consumer is draining. The consumer must NEVER see a state where the
    /// source declares itself exhausted (`is_exhausted()` true) while the
    /// producer is still going to push more data.
    ///
    /// This mirrors what happens on Android: the synchronous decoder fills
    /// the queue in bursts; the player drains at real-time rate; occasionally
    /// the queue is empty for a moment between bursts.
    #[test]
    fn test_streaming_burst_pattern_never_reports_exhausted() {
        let (buffer, source, _) = make_source();

        // Burst 1: producer writes, consumer drains fully.
        buffer.lock().extend([1.0, 2.0, 3.0, 4.0]);
        assert!(!source.is_exhausted());
        buffer.lock().clear(); // consumer drained
        assert!(
            !source.is_exhausted(),
            "queue empty between producer bursts must not report exhausted"
        );

        // Burst 2: producer writes more (this is exactly the moment that was
        // broken — rodio had already detached the source by now).
        buffer.lock().extend([5.0, 6.0, 7.0]);
        assert!(!source.is_exhausted());

        // Final burst + done.
        buffer.lock().extend([8.0, 9.0]);
        source.mark_done();
        assert!(!source.is_exhausted(), "still draining after done");

        // Consumer finishes the buffer → only now exhausted.
        buffer.lock().clear();
        assert!(source.is_exhausted(), "done + drained should be exhausted");
    }
}
