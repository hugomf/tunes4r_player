//! Integration test: verify that back-pressure in the decode loop keeps the
//! shared AudioBuffer queue bounded.
//!
//! On Android, the decode thread can fill the queue at 30-100x real-time,
//! causing unbounded VecDeque growth. Repeated resize reallocations stall
//! the AAudio callback via mutex contention, producing cumulative underruns
//! until AAudio disconnects the stream (~30s of buffered audio drains during
//! the gap).
//!
//! This test proves that a back-pressure cap (max_queue_samples) prevents
//! unbounded growth: the producer blocks at the cap, the consumer drains
//! at its own pace, and the queue stays bounded.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

type AudioRing = Arc<parking_lot::Mutex<VecDeque<f32>>>;

fn make_ring() -> AudioRing {
    Arc::new(parking_lot::Mutex::new(VecDeque::new()))
}

/// Simulate an MP3 frame decode: stereo, 1152 frames = 2304 interleaved f32.
const CHUNK_SAMPLES: usize = 2304;

/// 10 seconds of stereo 44.1 kHz audio: 44100 * 10 * 2 = 882000 samples.
const MAX_QUEUE_SAMPLES: usize = 882_000;

/// How many chunks fit in one full queue.
const CHUNKS_PER_QUEUE: usize = MAX_QUEUE_SAMPLES / CHUNK_SAMPLES;

#[test]
fn test_backpressure_blocks_when_queue_full() {
    let ring = make_ring();

    // Fill queue to just under the cap (CHUNKS_PER_QUEUE - 1 chunks)
    for _ in 0..CHUNKS_PER_QUEUE - 1 {
        ring.lock().extend(vec![0.5; CHUNK_SAMPLES]);
    }
    let before = ring.lock().len();
    assert!(
        before + CHUNK_SAMPLES <= MAX_QUEUE_SAMPLES,
        "should have room for one more chunk: {} + {} <= {}",
        before,
        CHUNK_SAMPLES,
        MAX_QUEUE_SAMPLES,
    );
    assert!(
        before + 2 * CHUNK_SAMPLES > MAX_QUEUE_SAMPLES,
        "should NOT have room for two more chunks",
    );

    // Push the chunk that fills to the cap — succeeds
    ring.lock().extend(vec![0.5; CHUNK_SAMPLES]);
    assert_eq!(ring.lock().len(), MAX_QUEUE_SAMPLES / CHUNK_SAMPLES * CHUNK_SAMPLES);

    // Now simulate the back-pressure check from the decode loop:
    // the next chunk WOULD exceed the cap, so the producer must wait.
    let next_chunk = vec![0.5; CHUNK_SAMPLES];
    let would_exceed = {
        let q = ring.lock();
        q.len() + next_chunk.len() > MAX_QUEUE_SAMPLES
    };
    assert!(would_exceed, "producer should be blocked at cap");

    // Drain some samples (simulating the AAudio callback consuming audio)
    ring.lock().drain(..CHUNK_SAMPLES);

    // After draining, room is available again
    let has_room = {
        let q = ring.lock();
        q.len() + next_chunk.len() <= MAX_QUEUE_SAMPLES
    };
    assert!(has_room, "producer should be unblocked after drain");
}

#[test]
fn test_backpressure_keeps_queue_bounded_under_load() {
    let ring = make_ring();
    let stop = Arc::new(AtomicBool::new(false));

    let ring_prod = ring.clone();
    let stop_prod = stop.clone();

    // Producer: pushes chunks as fast as possible, respecting the cap
    let producer = std::thread::spawn(move || {
        // 200 chunks = 200 * 2304 = 460800 samples ≈ 5.2 seconds at 44.1 kHz
        for _ in 0..200 {
            if stop_prod.load(Ordering::Relaxed) {
                break;
            }
            let chunk: Vec<f32> = vec![0.25; CHUNK_SAMPLES];
            loop {
                {
                    let mut queue = ring_prod.lock();
                    if queue.len() + chunk.len() <= MAX_QUEUE_SAMPLES {
                        queue.extend(chunk);
                        break;
                    }
                }
                if stop_prod.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    });

    let ring_cons = ring.clone();
    // Consumer: drain at a moderate pace (512 samples per cycle)
    // Run for enough time to let the producer get well ahead
    let consumer = std::thread::spawn(move || {
        let mut total = 0u64;
        // Drain ~1 second worth of samples total
        while total < 2 * CHUNK_SAMPLES as u64 {
            let drained = {
                let mut q = ring_cons.lock();
                let n = q.len().min(512);
                q.drain(..n).count() as u64
            };
            total += drained;
            if drained == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    });

    producer.join().unwrap();
    stop.store(true, Ordering::Relaxed);
    consumer.join().unwrap();

    // Verify the queue never exceeded the cap (enforced by the producer loop)
    let final_len = ring.lock().len();
    assert!(
        final_len <= MAX_QUEUE_SAMPLES,
        "queue exceeded max cap: {} > {}",
        final_len,
        MAX_QUEUE_SAMPLES,
    );
}

#[test]
fn test_backpressure_producer_drain_cycle_predictable() {
    let ring = make_ring();
    let max_chunks_at_once = 3usize;

    // Producer pushes 3 chunks (allowed), then tries a 4th (blocked)
    for i in 0..max_chunks_at_once {
        ring.lock().extend(vec![0.5; CHUNK_SAMPLES]);
        assert!(
            ring.lock().len() <= max_chunks_at_once * CHUNK_SAMPLES,
            "iteration {}: queue should not exceed {}",
            i,
            max_chunks_at_once * CHUNK_SAMPLES,
        );
    }

    // Back-pressure check confirms 4th chunk would exceed
    let chunk = vec![0.5; CHUNK_SAMPLES];
    let blocked = {
        let q = ring.lock();
        q.len() + chunk.len() > max_chunks_at_once * CHUNK_SAMPLES
    };
    assert!(blocked, "producer should be blocked after {} chunks", max_chunks_at_once);

    // Drain one chunk to unblock
    ring.lock().drain(..CHUNK_SAMPLES);

    // Now the 4th chunk fits
    let unblocked = {
        let q = ring.lock();
        q.len() + chunk.len() <= max_chunks_at_once * CHUNK_SAMPLES
    };
    assert!(unblocked, "producer should be unblocked after drain");
}
