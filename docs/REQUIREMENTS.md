# tunes4r — Requirements

## Functional
- Play audio from local files, HTTP streams, and YouTube URLs
- Seek, pause, resume, volume control
- Real-time FFT spectrum visualization
- Live stream playback with backward seek support
- YouTube video search and audio stream extraction

## Technical
- Rust logging uses `tracing`, not `log` or `eprintln!`
- Build must compile clean with no warnings
- Dart analysis must pass with no issues in `lib/`
- Dead code should be removed, not left commented out
