import 'dart:async';
import 'dart:ffi';

import 'models.dart';
import 'tunes4r_player_ffi.dart';

/// High-level audio engine that manages a native Rust playback engine.
///
/// ```dart
/// final engine = await AudioEngine.create();
/// engine.stateStream.listen((state) => print(state));
/// engine.play('https://example.com/audio.mp3');
/// ```
class AudioEngine {
  final Tunes4rFFI _ffi;
  Pointer<Void>? _handle;
  bool _disposed = false;

  Timer? _spectrumPoller;
  Timer? _positionPoller;
  Timer? _eventPoller;
  Timer? _bufferPoller;

  final StreamController<PlaybackState> _stateCtrl =
      StreamController<PlaybackState>.broadcast();
  final StreamController<List<double>> _spectrumCtrl =
      StreamController<List<double>>.broadcast();
  final StreamController<PlaybackPosition> _positionCtrl =
      StreamController<PlaybackPosition>.broadcast();
  final StreamController<EngineEvent> _eventCtrl =
      StreamController<EngineEvent>.broadcast();
  final StreamController<AdaptiveRingBuffer> _bufferCtrl =
      StreamController<AdaptiveRingBuffer>.broadcast();

  /// Stream of playback state changes (driven by native events).
  Stream<PlaybackState> get stateStream => _stateCtrl.stream;

  /// Stream of FFT spectrum data (polled every 100ms).
  Stream<List<double>> get spectrumStream => _spectrumCtrl.stream;

  /// Stream of playback position updates.
  ///
  /// Emits a [PlaybackPosition] every 16ms (vsync-aligned) with the current
  /// playhead and total duration. The stream also emits synchronously
  /// inside [seek] (with the seek target) and [play] (with 0), so the UI
  /// gets instant feedback without waiting for the next poll.
  ///
  /// Subscribers should use `distinct()` to skip duplicates if they only
  /// care about changes.
  Stream<PlaybackPosition> get positionStream => _positionCtrl.stream;

  /// Stream of native engine events (state changes, seek lifecycle,
  /// end-of-stream, errors). The previous `stateStream` is still driven
  /// from these events for backward compatibility.
  Stream<EngineEvent> get playbackEventStream => _eventCtrl.stream;

  /// Stream of adaptive ring buffer updates for progressive streams
  /// (HTTP / YouTube). Polled every 200ms — slow enough to be cheap, fast
  /// enough to feel live as the ring fills.
  ///
  /// For local files the ring covers the full duration from the start
  /// (`AdaptiveRingBuffer.isFullyBuffered == true`).
  Stream<AdaptiveRingBuffer> get downloadBufferStream => _bufferCtrl.stream;

  AudioEngine._(this._ffi, this._handle);

  /// Create a new audio engine instance.
  ///
  /// Call [initialize] first on the global [Tunes4rFFI] singleton.
  /// Throws [Tunes4rEngineException] if the native engine cannot be created.
  static AudioEngine create({Tunes4rFFI? ffi}) {
    final engineFfi = ffi ?? tunes4rFFI;
    if (!engineFfi.isInitialized) {
      throw const Tunes4rEngineException(
        'Tunes4rFFI not initialized. Call Tunes4rFFI().initialize() first.',
      );
    }
    final handle = engineFfi.createEngine();
    if (handle == nullptr) {
      throw const Tunes4rEngineException('Failed to create native engine');
    }
    return AudioEngine._(engineFfi, handle);
  }

  /// Same as [create] but also initializes FFI if needed.
  static Future<AudioEngine> createWithInit({
    Tunes4rFFI? ffi,
    EngineConfig config = const EngineConfig(),
  }) async {
    final engineFfi = ffi ?? tunes4rFFI;
    if (!engineFfi.initialize()) {
      throw Tunes4rEngineException(
        'Native initialization failed: ${engineFfi.initError}',
      );
    }
    if (config.spectrumBandCount > 0) {
      engineFfi.setSpectrumBandCountGlobal(config.spectrumBandCount);
    }
    return create(ffi: engineFfi);
  }

  // ---------------------------------------------------------------------------
  // Polling
  // ---------------------------------------------------------------------------

  /// Start polling for state, spectrum, position, and event updates.
  /// Called automatically by [play], [resume], etc.
  void startPolling() {
    _spectrumPoller ??= Timer.periodic(const Duration(milliseconds: 100), (_) {
      if (_disposed || _handle == null) return;
      try {
        final s = _ffi.getSpectrum(_handle!);
        if (s.isNotEmpty) _spectrumCtrl.add(s);
      } catch (_) {}
    });
    // Vsync-aligned position polling (~60Hz). Dedup is done by callers via
    // Stream.distinct(), or in their setState logic.
    _positionPoller ??= Timer.periodic(const Duration(milliseconds: 16), (_) {
      if (_disposed || _handle == null) return;
      try {
        _positionCtrl.add(_ffi.getPosition(_handle!));
      } catch (_) {}
    });
    // Event drain. We poll at 60Hz so latency-sensitive events (seek
    // completed, end-of-stream) are delivered promptly without busy-spinning.
    _eventPoller ??= Timer.periodic(const Duration(milliseconds: 16), (_) {
      if (_disposed || _handle == null) return;
      try {
        // Drain the whole queue per tick to avoid backpressure.
        while (true) {
          final e = _ffi.pollEvent(_handle!);
          if (e.eventType == engineEventNone) break;
          final event = EngineEvent(
            eventType: e.eventType,
            intParam: e.intParam,
          );
          _eventCtrl.add(event);
          if (event.eventType == engineEventStateChanged) {
            _stateCtrl.add(PlaybackState.fromValue(event.intParam));
          }
        }
      } catch (_) {}
    });
    // Ring buffer poller (5Hz). Cheap to read, changes slowly.
    _bufferPoller ??= Timer.periodic(const Duration(milliseconds: 200), (_) {
      if (_disposed || _handle == null) return;
      try {
        final b = _ffi.getDownloadBuffer(_handle!);
        _bufferCtrl.add(
          AdaptiveRingBuffer(
            capacityMs: b.capacityMs,
            readOffsetMs: b.readOffsetMs,
            writeOffsetMs: b.writeOffsetMs,
            totalMs: b.totalMs,
            isComplete: b.isComplete,
          ),
        );
      } catch (_) {}
    });
  }

  void stopPolling() {
    _spectrumPoller?.cancel();
    _spectrumPoller = null;
    _positionPoller?.cancel();
    _positionPoller = null;
    _eventPoller?.cancel();
    _eventPoller = null;
    _bufferPoller?.cancel();
    _bufferPoller = null;
  }

  // ---------------------------------------------------------------------------
  // Playback control
  // ---------------------------------------------------------------------------

  /// Play a URI. Auto-detects source type (file, HTTP stream, YouTube).
  /// Returns 0 on success, non-zero on failure.
  ///
  /// [bufferSizeMs] — optional fixed ring buffer capacity in ms.
  /// When unset (or <= 0), the buffer is adaptively sized based on
  /// connection speed. Larger values allow wider seek range for
  /// progressive streams but use more memory.
  int play(String uri, {int bufferSizeMs = -1}) {
    _ensureAlive();
    startPolling();
    final result = _ffi.play(_handle!, uri, bufferSizeMs: bufferSizeMs);
    // Synchronous position reset so the UI snaps to 0 immediately,
    // before the first 16ms position poll fires.
    _positionCtrl.add(_ffi.getPosition(_handle!));
    return result;
  }

  /// Play a YouTube URL or video ID.
  int playYoutube(String url, {String? cacheDir}) {
    _ensureAlive();
    startPolling();
    final result = _ffi.playYoutube(_handle!, url, cacheDir ?? '');
    _positionCtrl.add(_ffi.getPosition(_handle!));
    return result;
  }

  /// Play an HTTP stream via the progressive downloader.
  int playStream(String url) {
    _ensureAlive();
    startPolling();
    final result = _ffi.playStreamWithDownloader(_handle!, url);
    _positionCtrl.add(_ffi.getPosition(_handle!));
    return result;
  }

  /// Play a live internet stream with backward-seek support.
  ///
  /// [cacheMaxMs] controls how many ms of audio are kept in the ring
  /// buffer for seeking backward (default 30 min = 1_800_000 ms).
  int playLive(String url, {int cacheMaxMs = 30 * 60 * 1000}) {
    _ensureAlive();
    startPolling();
    final result = _ffi.playLive(_handle!, url, cacheMaxMs);
    _positionCtrl.add(_ffi.getPosition(_handle!));
    return result;
  }

  /// Push raw audio bytes to the Rust engine (pipe mode).
  void pushAudioBytes(Pointer<Uint8> data, int len) {
    _ensureAlive();
    _ffi.pushAudioBytes(_handle!, data, len);
  }

  /// Signal end of pipe stream.
  void endAudioStream() {
    _ensureAlive();
    _ffi.endAudioStream(_handle!);
  }

  void pause() {
    _ensureAlive();
    _ffi.pause(_handle!);
  }

  void resume() {
    _ensureAlive();
    _ffi.resume(_handle!);
  }

  void stop() {
    _ensureAlive();
    _ffi.stop(_handle!);
    stopPolling();
  }

  void seek(int positionMs) {
    _ensureAlive();
    _ffi.seek(_handle!, positionMs);
    // Synchronous position emission: the native side will seed the
    // position clock at this target (ExoPlayer-style), so reading it
    // back here gives the UI an instant, accurate update.
    try {
      _positionCtrl.add(_ffi.getPosition(_handle!));
    } catch (_) {}
  }

  void setVolume(double volume) {
    _ensureAlive();
    _ffi.setVolume(_handle!, volume.clamp(0.0, 1.0));
  }

  // ---------------------------------------------------------------------------
  // State queries
  // ---------------------------------------------------------------------------

  PlaybackState get state {
    _ensureAlive();
    return PlaybackState.fromValue(_ffi.getState(_handle!));
  }

  bool get isPlaying {
    _ensureAlive();
    return _ffi.isPlaying(_handle!);
  }

  bool get canSeek {
    _ensureAlive();
    return _ffi.canSeek(_handle!);
  }

  bool get canDownload {
    _ensureAlive();
    return _ffi.canDownload(_handle!);
  }

  double get volume {
    _ensureAlive();
    return _ffi.getVolume(_handle!);
  }

  int get positionMs {
    _ensureAlive();
    return _ffi.getPosition(_handle!).currentMs;
  }

  int get durationMs {
    _ensureAlive();
    return _ffi.getPosition(_handle!).totalMs;
  }

  int get bufferedPositionMs {
    _ensureAlive();
    return _ffi.getBufferedPosition(_handle!);
  }

  /// Snapshot of the current ring buffer (one-shot read; prefer
  /// [downloadBufferStream] for live updates).
  AdaptiveRingBuffer get downloadBuffer {
    _ensureAlive();
    final b = _ffi.getDownloadBuffer(_handle!);
    return AdaptiveRingBuffer(
      capacityMs: b.capacityMs,
      readOffsetMs: b.readOffsetMs,
      writeOffsetMs: b.writeOffsetMs,
      totalMs: b.totalMs,
      isComplete: b.isComplete,
    );
  }

  int get sampleRate {
    _ensureAlive();
    return _ffi.getSampleRate(_handle!);
  }

  int get channels {
    _ensureAlive();
    return _ffi.getChannels(_handle!);
  }

  List<double> getSpectrum() {
    _ensureAlive();
    return _ffi.getSpectrum(_handle!);
  }

  String? get loadError {
    _ensureAlive();
    return _ffi.getLoadError(_handle!);
  }

  String? get lastError {
    _ensureAlive();
    return _ffi.getLastError(_handle!);
  }

  // ---------------------------------------------------------------------------
  // YouTube service
  // ---------------------------------------------------------------------------

  /// Look up the best audio stream URL for a YouTube video ID.
  String? youtubeGetStreamUrl(String videoId) {
    _ensureAlive();
    final svc = _ffi.youtubeServiceCreate();
    try {
      return _ffi.youtubeGetStreamUrl(svc, videoId);
    } finally {
      _ffi.youtubeServiceDestroy(svc);
    }
  }

  // ---------------------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------------------

  void _ensureAlive() {
    if (_disposed) {
      throw const Tunes4rEngineException('AudioEngine has been disposed');
    }
  }

  /// Release the native engine handle.
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    stopPolling();
    if (_handle != null) {
      _ffi.destroyEngine(_handle!);
      _handle = null;
    }
    _stateCtrl.close();
    _spectrumCtrl.close();
    _positionCtrl.close();
    _eventCtrl.close();
    _bufferCtrl.close();
  }
}
