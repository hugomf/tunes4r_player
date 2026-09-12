# tunes4r — Project Context

## Architecture
- Backend is **Rust** (workspace root `tunes4r-core`: `crates/ffi`, `crates/player`, `crates/youtube`; `crates/ytex` module pull via FFI). Frontend is **Dart/Java** and contains no backend logic.
- Rust native library (`libtunes4r.so` / `libtunes4r.dylib` / `libtunes4r.a`), wrapped by Dart FFI bindings (`lib/src/tunes4r_player_ffi.dart`)
- High-level Dart API: `AudioEngine` class (`lib/src/audio_engine.dart`)
- YouTube stream extraction / po_token minting: pure Rust in `tunes4r-core`/`ytex` (`ytex::botguard::mint_po_token` via vendored rustypipe-botguard/deno_core-V8) — **same code path on macOS, iOS, and Android**; no Java/WebView bridge
- Android hooks the native lib via Dart `DynamicLibrary.open('libtunes4r.so')` (pubspec `android: ffiPlugin: true`, no pluginClass)
- Android native lib + libc++_shared.so live in `android/src/main/jniLibs/{arm64-v8a,x86_64}`; built by `scripts/build_rust.sh android` (see SESSION_LOG for the platform-26/OpenSSL/builtins recipe)
- 32-bit ABIs (armeabi-v7a, x86) are NOT built: V8 cannot cross-compile them from macOS; Play requires 64-bit

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
