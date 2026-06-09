# tunes4r — Project Context

## Architecture
- Rust native library (`rust/`) with C FFI, wrapped by Dart FFI bindings (`lib/src/tunes4r_player_ffi.dart`)
- High-level Dart API: `AudioEngine` class (`lib/src/audio_engine.dart`)
- YouTube stream extraction: `rust/crates/youtube/`

## Key Design Patterns
- `AudioEngine._h` getter replaces `_ensureAlive()` + `_handle!` boilerplate (single chokepoint for disposed checks)
- FFI bindings in `Tunes4rFFI` class, low-level methods take `Pointer<Void>`
- Timers poll for state, spectrum, position, events, and buffer progress
- Native engine holds `Arc<RwLock<PlaybackEngine>>`

## Seek Architecture (Stream/YouTube sources)
- ALL seeks within the buffered region use **cache-reopen** (`commands.rs`): the old decode thread is detached (not joined), a new `CachedReader` is opened at the seek position via `CachingDecorator::open(Some(position))`, and a new decode thread is spawned.
- `OUTPUT_GEN` global atomic (`cpal_source.rs`) is incremented on each seek — old CPAL callbacks write silence when their captured gen doesn't match.
- `ByteCache` has a permanent `header` buffer (512 KB) for the first bytes of the stream, ensuring format re-probes always work even after the ring buffer wraps.
- In-thread seek (via `seek_request` + `seek_to_position`) is NOT used for stream/YouTube sources because `ReadOnlySource.byte_len()` returns `None`, causing the Matroska demuxer's native seeking to fail, and packet-skip fallback is too slow for forward seeks.

## Logging
- Rust: `tracing` crate throughout; `tracing_subscriber` on non-Android, `android_logger` on Android
- Dart: `debugPrint` in poller catch blocks
