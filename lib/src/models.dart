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

/// Engine event mirroring Rust's `EngineEvent` FFI struct. Emitted on
/// [AudioEngine.playbackEventStream].
class EngineEvent {
  /// One of the `engineEvent*` constants declared in `tunes4r_player_ffi.dart`.
  final int eventType;
  final int intParam;

  const EngineEvent({required this.eventType, required this.intParam});
}

/// Sliding-window ring buffer state for progressive streams (YouTube, HTTP).
///
/// Architecturally a ring buffer: a fixed-capacity window that slides along
/// the file as playback progresses. The downloader fills ahead of the
/// playhead up to `capacityMs`; older data is discarded.
///
/// All offsets are in milliseconds and are file-relative.
class AdaptiveRingBuffer {
  /// Fixed ring size in ms (e.g. 30 000 = 30 s of audio).
  final int capacityMs;

  /// Playhead position in the file (file-relative, ms).
  final int readOffsetMs;

  /// How far into the file the downloader has reached (file-relative, ms).
  final int writeOffsetMs;

  /// Total file duration (0 until known).
  final int totalMs;

  /// True once the entire file has been downloaded.
  final bool isComplete;

  const AdaptiveRingBuffer({
    required this.capacityMs,
    required this.readOffsetMs,
    required this.writeOffsetMs,
    required this.totalMs,
    required this.isComplete,
  });

  /// Empty ring buffer with default 30 s capacity.
  const AdaptiveRingBuffer.empty()
      : capacityMs = 30000,
        readOffsetMs = 0,
        writeOffsetMs = 0,
        totalMs = 0,
        isComplete = false;

  /// How many ms of audio are currently in the ring buffer, clamped to
  /// `[0, capacityMs]`. Returns the remainder if the file is complete.
  int get availableMs {
    if (isComplete && totalMs > 0) {
      return (totalMs - readOffsetMs).clamp(0, 1 << 31);
    }
    final filled = writeOffsetMs - readOffsetMs;
    if (filled <= 0) return 0;
    return filled > capacityMs ? capacityMs : filled;
  }

  /// File-relative position of the last buffered byte.
  int get endMs => readOffsetMs + availableMs;

  /// UI-safe end position: never reads as less than the playhead.
  int get endMsClamped => endMs > readOffsetMs ? endMs : readOffsetMs;

  /// `true` if the user may seek anywhere within `[0, totalMs]`.
  bool get isFullyBuffered => isComplete || (totalMs > 0 && writeOffsetMs >= totalMs);

  /// `true` if the given position is within the buffered region.
  bool contains(int positionMs) =>
      positionMs >= readOffsetMs && positionMs <= endMs;

  @override
  String toString() =>
      'AdaptiveRingBuffer(playhead=${readOffsetMs}ms, end=${endMs}ms, '
      'total=${totalMs}ms, available=${availableMs}ms, complete=$isComplete)';
}


