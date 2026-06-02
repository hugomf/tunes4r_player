/// Playback state enum matching Rust's PlaybackState.
enum PlaybackState {
  stopped(0),
  connecting(1),
  buffering(2),
  decoding(3),
  playing(4),
  paused(5),
  error(6);

  final int value;
  const PlaybackState(this.value);

  static PlaybackState fromValue(int v) =>
      PlaybackState.values.firstWhere(
        (s) => s.value == v,
        orElse: () => error,
      );
}

/// Configuration for creating an [AudioEngine].
class EngineConfig {
  final int spectrumBandCount;

  const EngineConfig({
    this.spectrumBandCount = 16,
  });
}

class Tunes4rInitException implements Exception {
  final String message;
  const Tunes4rInitException(this.message);
  @override
  String toString() => 'Tunes4rInitException: $message';
}

class Tunes4rEngineException implements Exception {
  final String message;
  const Tunes4rEngineException(this.message);
  @override
  String toString() => 'Tunes4rEngineException: $message';
}

class Tunes4rLoadException implements Exception {
  final String message;
  const Tunes4rLoadException(this.message);
  @override
  String toString() => 'Tunes4rLoadException: $message';
}


