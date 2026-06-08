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

## Logging
- Rust: `tracing` crate throughout; `tracing_subscriber` on non-Android, `android_logger` on Android
- Dart: `debugPrint` in poller catch blocks
