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

  Timer? _poller;
  Timer? _spectrumPoller;

  final StreamController<PlaybackState> _stateCtrl =
      StreamController<PlaybackState>.broadcast();
  final StreamController<List<double>> _spectrumCtrl =
      StreamController<List<double>>.broadcast();

  /// Stream of playback state changes (polled every 250ms).
  Stream<PlaybackState> get stateStream => _stateCtrl.stream;

  /// Stream of FFT spectrum data (polled every 100ms).
  Stream<List<double>> get spectrumStream => _spectrumCtrl.stream;

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

  /// Start polling for state and spectrum updates.
  /// Called automatically by [play], [resume], etc.
  void startPolling() {
    _poller ??= Timer.periodic(const Duration(milliseconds: 250), (_) {
      if (_disposed || _handle == null) return;
      try {
        _stateCtrl.add(PlaybackState.fromValue(_ffi.getState(_handle!)));
      } catch (_) {}
    });
    _spectrumPoller ??= Timer.periodic(const Duration(milliseconds: 100), (_) {
      if (_disposed || _handle == null) return;
      try {
        final s = _ffi.getSpectrum(_handle!);
        if (s.isNotEmpty) _spectrumCtrl.add(s);
      } catch (_) {}
    });
  }

  void stopPolling() {
    _poller?.cancel();
    _poller = null;
    _spectrumPoller?.cancel();
    _spectrumPoller = null;
  }

  // ---------------------------------------------------------------------------
  // Playback control
  // ---------------------------------------------------------------------------

  /// Play a URI. Auto-detects source type (file, HTTP stream, YouTube).
  /// Returns 0 on success, non-zero on failure.
  int play(String uri) {
    _ensureAlive();
    startPolling();
    return _ffi.play(_handle!, uri);
  }

  /// Play a YouTube URL or video ID.
  int playYoutube(String url, {String? cacheDir}) {
    _ensureAlive();
    startPolling();
    return _ffi.playYoutube(_handle!, url, cacheDir ?? '');
  }

  /// Play an HTTP stream via the progressive downloader.
  int playStream(String url) {
    _ensureAlive();
    startPolling();
    return _ffi.playStreamWithDownloader(_handle!, url);
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
  }
}
