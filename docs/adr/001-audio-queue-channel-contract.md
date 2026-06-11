# ADR 001: Audio Queue Channel Contract

## Status
Accepted (2026-06-11)

## Context
The audio pipeline decodes compressed audio (AAC, MP3, etc.) via Symphonia, resamples to the CPAL device's output sample rate, and pushes interleaved float samples into a shared `VecDeque<f32>` queue. A CPAL output callback drains the queue in its `run_output_callback`.

Symphonia decoders produce the channel count reported by the codec -- e.g., an HE-AAC v2 stream's ADTS header may report `channels=1` (mono) even though the stream is intended for stereo output. The decoder does not apply SBR/PS upmixing (AAC-LC only).

The CPAL output callback processes the queue with `data.chunks_exact_mut(CHANNELS)` where `CHANNELS` is the device's configured channel count (typically 2 for stereo). The callback always consumes `output_sample_rate × output_channels` samples per second.

## Problem
Before this fix, the decode loop pushed mono samples into the queue. The CPAL callback consumed them at double the rate:

- Each AAC frame: 1024 samples at 22050 Hz mono = 46.4ms of audio
- After resample to 48000 Hz: ~2230 mono samples (still 46.4ms of audio)
- Queue consumption rate: 48000 × 2 = 96000 samples/sec
- Playback time per frame from queue: 2230 / 96000 = 23.2ms -- **2× speed**

The queue held the correct number of samples, but the device consumed them at stereo rate while they were only mono-length.

## Decision
**The audio queue MUST always hold samples matching the CPAL device's output channel count.** After resampling to the output sample rate, upmix to the output channel count before pushing to the queue.

Upmixing strategy for mono → stereo: duplicate each sample to produce L/R pairs. This is the simplest approach and preserves waveform shape without introducing artifacts.

## Consequences
- Mono streams now play at correct speed through stereo devices.
- The `upmix_interleaved()` helper is reusable for other channel count ratios.
- The spectrum analyzer must run on pre-upmix samples to avoid processing duplicate data -- this is already the case, as it operates before the upmix call.
- Any code that reads `samples_played` must account for the output channel count in its position calculations -- seek position seeding and drain queue duration estimation both use `output_channels`.

## Invariant
For each decoded frame, the queue must receive:
```
frame_samples × (output_sample_rate / decoded_rate) × (output_channels / decoded_channels)
```
total interleaved float samples. Any mismatch between queue channel layout and device channel layout produces incorrect playback speed.

## Test coverage
- `test_upmix_mono_to_stereo`: verifies 1→2 duplication
- `test_upmix_same_channels_is_noop`: identity for equal channel counts
- `test_upmix_empty_input`: handles empty arrays
- `test_upmix_mono_to_quad`: 1→4 channel expansion
- `test_resample_interleaved_basic_upsample`: resample length sanity check
