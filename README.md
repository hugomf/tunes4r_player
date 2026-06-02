# tunes4r

Rust-powered audio playback engine for Flutter.

**Platforms:** iOS, Android, macOS — Linux & Windows coming soon.

**Features:**
- Local file playback (MP3, FLAC, AAC, Ogg Vorbis, Opus, WAV)
- HTTP/HTTPS streaming (Icecast, Shoutcast)
- YouTube audio extraction (URL, video ID, or search query)
- Real-time FFT spectrum analysis
- Seek, volume, pause/resume
- Pipe mode — Dart feeds raw audio bytes to Rust

## Usage

```dart
import 'package:tunes4r/tunes4r.dart';

final engine = await AudioEngine.createWithInit();

// Play a local file
engine.play('/path/to/file.mp3');

// Play an HTTP stream
engine.play('https://example.com/stream.mp3');

// Play YouTube audio
engine.play('https://www.youtube.com/watch?v=dQw4w9WgXcQ');

// Spectrum
engine.spectrumStream.listen((bands) {
  print('Spectrum: $bands');
});

// State
engine.stateStream.listen((state) {
  print('State: $state');
});

engine.dispose();
```

## Development

### Prerequisites

- Flutter 3.22+
- Rust toolchain (`rustup`)
- Xcode (for iOS/macOS)
- Android NDK 27+ (for Android)

### Build native libraries

```bash
# Install Rust cross-compilation targets
make install

# Build for all platforms
make prepare

# Or individually
make build-macos
make build-ios
make build-android
```

Artifacts are copied into:
- `ios/Frameworks/libtunes4r.a`
- `macos/Frameworks/libtunes4r.dylib`
- `android/src/main/jniLibs/<abi>/libtunes4r.so`

### Run example

```bash
cd example
flutter run
```

## Project structure

```
tunes4r/
├── lib/
│   ├── tunes4r.dart              # Barrel export
│   └── src/
│       ├── tunes4r_ffi.dart      # Raw FFI bindings
│       ├── audio_engine.dart     # High-level API
│       └── models.dart           # Data classes
├── ios/
│   └── tunes4r.podspec           # iOS CocoaPod config
├── macos/
│   └── tunes4r.podspec           # macOS CocoaPod config
├── android/
│   └── build.gradle              # Android Gradle config
├── scripts/
│   └── build_rust.sh             # Cross-compilation script
├── rust/                         # Rust source (tunes4r crate)
│   ├── Cargo.toml
│   └── src/
└── example/                      # Flutter example app
```

## License

MIT
