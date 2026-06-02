# tunes4r_player

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
import 'package:tunes4r_player/tunes4r_player.dart';

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

## Installation

Add to `pubspec.yaml`:

```yaml
dependencies:
  tunes4r_player: ^0.1.0
```

Or use a Git dependency for the latest:

```yaml
dependencies:
  tunes4r_player:
    git:
      url: https://github.com/hugomf/tunes4r_player.git
      ref: v0.1.0
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

## Release workflow

### 1. Update version

Edit `pubspec.yaml` and `CHANGELOG.md` with the new version number.

### 2. Tag and push

```bash
# Commit changes
git add -A && git commit -m "Prepare v0.2.0"

# Tag
git tag v0.2.0

# Push everything
git push && git push origin v0.2.0
```

Pushing a tag triggers the CI workflow (`.github/workflows/build_tunes4r_player.yml`):
1. Cross-compiles the Rust library for all platforms (macOS dylib, iOS .a, Android .so)
2. Uploads the binaries as build artifacts
3. Creates a **GitHub Release** with a tarball of all native libs

### 3. Publish to pub.dev (optional)

```bash
cd tunes4r_player
flutter pub publish
```

### CI build without a tag

To manually trigger a build from the GitHub UI:
- Go to Actions → **Build tunes4r_player native libs** → Run workflow
- Artifacts are available for 30 days

## Project structure

```
tunes4r_player/
├── lib/
│   ├── tunes4r_player.dart        # Barrel export
│   └── src/
│       ├── tunes4r_player_ffi.dart # Raw FFI bindings
│       ├── audio_engine.dart      # High-level API
│       └── models.dart            # Data classes
├── ios/
│   └── tunes4r_player.podspec     # iOS CocoaPod config
├── macos/
│   └── tunes4r_player.podspec     # macOS CocoaPod config
├── android/
│   └── build.gradle               # Android Gradle config
├── scripts/
│   ├── build_rust.sh              # Cross-compilation script
│   └── prepare_publish.sh         # Pre-publish verification
├── rust/                          # Rust source (tunes4r crate)
│   ├── Cargo.toml
│   └── src/
├── .github/workflows/
│   └── build_tunes4r_player.yml   # CI: build + release
└── example/                       # Flutter example app
```

## License

MIT
