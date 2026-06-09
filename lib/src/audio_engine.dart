import 'dart:async';
import 'dart:ffi';

import 'package:flutter/foundation.dart';

import 'models.dart';
import 'tunes4r_player_ffi.dart';

// ---------------------------------------------------------------------------
// Named polling interval constants
// ---------------------------------------------------------------------------

/// Interval (ms) between spectrum data polls (10 Hz).
const _spectrumPollIntervalMs = 100;

/// Interval (ms) between position polls, vsync-aligned (~60 Hz).
const _positionPollIntervalMs = 16;

/// Interval (ms) between event queue drains (~60 Hz).
const _eventPollIntervalMs = 16;

/// Interval (ms) between ring buffer state polls (5 Hz).
const _bufferPollIntervalMs = 200;

// ---------------------------------------------------------------------------
// Poller engine — encapsulates timers and stream controllers
// ---------------------------------------------------------------------------

/// Encapsulates the periodic polling timers and their associated stream
/// controllers for [AudioEngine]. Owns all five broadcast streams so that
/// [AudioEngine] can focus on playback control and lifecycle.
class _EnginePoller {
  final Tunes4rFFI _ffi;
  Pointer<Void>? _handle;
  bool _active = false;

  Timer? _spectrumPoller;
  Timer? _positionPoller;
  Timer? _eventPoller;
  Timer? _bufferPoller;

  final StreamController<PlaybackState> stateCtrl =
      StreamController<PlaybackState>.broadcast();
  final StreamController<List<double>> spectrumCtrl =
      StreamController<List<double>>.broadcast();
  final StreamController<PlaybackPosition> positionCtrl =
      StreamController<PlaybackPosition>.broadcast();
  final StreamController<EngineEvent> eventCtrl =
      StreamController<EngineEvent>.broadcast();
  final StreamController<AdaptiveRingBuffer> bufferCtrl =
      StreamController<AdaptiveRingBuffer>.broadcast();

  _EnginePoller(this._ffi);

  /// Start all pollers using the given native handle.
  ///
  /// If already active (e.g. from a previous [start] call), the new handle
  /// is silently ignored. This is safe because the handle is stable for the
  /// engine's lifetime — it is only reassigned on a fresh [AudioEngine]
  /// instance.
  void start(Pointer<Void> handle) {
    if (_active) return;
    _active = true;
    _handle = handle;

    _spectrumPoller ??= Timer.periodic(
      const Duration(milliseconds: _spectrumPollIntervalMs),
      (_) {
        if (!_active || _handle == null) return;
        try {
          final s = _ffi.getSpectrum(_handle!);
          if (s.isNotEmpty) spectrumCtrl.add(s);
        } catch (e) {
          debugPrint('[tunes4r] spectrum poll error: $e');
        }
      },
    );

    _positionPoller ??= Timer.periodic(
      const Duration(milliseconds: _positionPollIntervalMs),
      (_) {
        if (!_active || _handle == null) return;
        try {
          positionCtrl.add(_ffi.getPosition(_handle!));
        } catch (e) {
          debugPrint('[tunes4r] position poll error: $e');
        }
      },
    );

    _eventPoller ??= Timer.periodic(
      const Duration(milliseconds: _eventPollIntervalMs),
      (_) {
        if (!_active || _handle == null) return;
        try {
          while (true) {
            final e = _ffi.pollEvent(_handle!);
            if (e.eventType == engineEventNone) break;
            final eventType = EngineEventType.fromValue(e.eventType);
            final event = EngineEvent(
              eventType: eventType,
              intParam: e.intParam,
            );
            eventCtrl.add(event);
            if (eventType == EngineEventType.stateChanged) {
              stateCtrl.add(PlaybackState.fromValue(event.intParam));
            }
          }
        } catch (e) {
          debugPrint('[tunes4r] event poll error: $e');
        }
      },
    );

    _bufferPoller ??= Timer.periodic(
      const Duration(milliseconds: _bufferPollIntervalMs),
      (_) {
        if (!_active || _handle == null) return;
        try {
          final b = _ffi.getDownloadBuffer(_handle!);
          bufferCtrl.add(
            AdaptiveRingBuffer(
              capacityMs: b.capacityMs,
              readOffsetMs: b.readOffsetMs,
              writeOffsetMs: b.writeOffsetMs,
              totalMs: b.totalMs,
              isComplete: b.isComplete,
            ),
          );
        } catch (e) {
          debugPrint('[tunes4r] buffer poll error: $e');
        }
      },
    );
  }

  /// Stop all pollers. Does not close stream controllers.
  void stop() {
    _active = false;
    _handle = null;
    _spectrumPoller?.cancel();
    _spectrumPoller = null;
    _positionPoller?.cancel();
    _positionPoller = null;
    _eventPoller?.cancel();
    _eventPoller = null;
    _bufferPoller?.cancel();
    _bufferPoller = null;
  }

  /// Stop polling and release stream controllers.
  void dispose() {
    stop();
    stateCtrl.close();
    spectrumCtrl.close();
    positionCtrl.close();
    eventCtrl.close();
    bufferCtrl.close();
  }
}

// ---------------------------------------------------------------------------
// AudioEngine
// ---------------------------------------------------------------------------

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
  final _EnginePoller _poller;

  /// Returns the native handle or throws if disposed.
  Pointer<Void> get _h {
    if (_disposed) {
      throw const Tunes4rEngineException('AudioEngine has been disposed');
    }
    return _handle!;
  }

  /// Stream of playback state changes (driven by native events).
  Stream<PlaybackState> get stateStream => _poller.stateCtrl.stream;

  /// Stream of FFT spectrum data (polled every 100ms).
  Stream<List<double>> get spectrumStream => _poller.spectrumCtrl.stream;

  /// Stream of playback position updates.
  ///
  /// Emits a [PlaybackPosition] every 16ms (vsync-aligned) with the current
  /// playhead and total duration. The stream also emits synchronously
  /// inside [seek] and [play], so the UI gets instant feedback without
  /// waiting for the next poll.
  Stream<PlaybackPosition> get positionStream => _poller.positionCtrl.stream;

  /// Stream of native engine events (state changes, seek lifecycle,
  /// end-of-stream, errors). The previous `stateStream` is still driven
  /// from these events for backward compatibility.
  Stream<EngineEvent> get playbackEventStream => _poller.eventCtrl.stream;

  /// Stream of adaptive ring buffer updates for progressive streams
  /// (HTTP / YouTube). Polled every 200ms — slow enough to be cheap, fast
  /// enough to feel live as the ring fills.
  ///
  /// For local files the ring covers the full duration from the start
  /// (`AdaptiveRingBuffer.isFullyBuffered == true`).
  Stream<AdaptiveRingBuffer> get downloadBufferStream =>
      _poller.bufferCtrl.stream;

  AudioEngine._(this._ffi, this._handle) : _poller = _EnginePoller(_ffi);

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
  void startPolling() => _poller.start(_h);

  void stopPolling() => _poller.stop();

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
    startPolling();
    final result = _ffi.play(_h, uri, bufferSizeMs: bufferSizeMs);
    _poller.positionCtrl.add(_ffi.getPosition(_h));
    return result;
  }

  /// Play a YouTube URL or video ID.
  int playYoutube(String url) {
    startPolling();
    final result = _ffi.playYoutube(_h, url);
    _poller.positionCtrl.add(_ffi.getPosition(_h));
    return result;
  }

  /// Play an HTTP stream. Deprecated — use [play] instead; it
  /// auto-detects the source type.
  @Deprecated('Use play() instead — it auto-detects the source type.')
  int playStream(String url) {
    startPolling();
    final result = _ffi.play(_h, url);
    _poller.positionCtrl.add(_ffi.getPosition(_h));
    return result;
  }

  /// Play a live internet stream with backward-seek support.
  ///
  /// [cacheMaxMs] controls how many ms of audio are kept in the ring
  /// buffer for seeking backward (default 30 min).
  int playLive(String url, {int cacheMaxMs = 30 * 60 * 1000}) {
    startPolling();
    final result = _ffi.playLive(_h, url, cacheMaxMs);
    _poller.positionCtrl.add(_ffi.getPosition(_h));
    return result;
  }

  /// Push raw audio bytes to the Rust engine (pipe mode).
  void pushAudioBytes(Pointer<Uint8> data, int len) {
    _ffi.pushAudioBytes(_h, data, len);
  }

  /// Signal end of pipe stream.
  void endAudioStream() {
    _ffi.endAudioStream(_h);
  }

  void pause() {
    _ffi.pause(_h);
  }

  void resume() {
    _ffi.resume(_h);
  }

  void stop() {
    _ffi.stop(_h);
    stopPolling();
  }

  void seek(int positionMs) {
    _ffi.seek(_h, positionMs);
    _poller.positionCtrl.add(_ffi.getPosition(_h));
  }

  void setVolume(double volume) {
    _ffi.setVolume(_h, volume.clamp(0.0, 1.0));
  }

  // ---------------------------------------------------------------------------
  // State queries
  // ---------------------------------------------------------------------------

  PlaybackState get state =>
      PlaybackState.fromValue(_ffi.getState(_h));

  bool get isPlaying => _ffi.isPlaying(_h);

  bool get canSeek => _ffi.canSeek(_h);

  bool get canDownload => _ffi.canDownload(_h);

  double get volume => _ffi.getVolume(_h);

  int get positionMs => _ffi.getPosition(_h).currentMs;

  int get durationMs => _ffi.getPosition(_h).totalMs;

  int get bufferedPositionMs => _ffi.getBufferedPosition(_h);

  /// Snapshot of the current ring buffer (one-shot read; prefer
  /// [downloadBufferStream] for live updates).
  AdaptiveRingBuffer get downloadBuffer {
    final b = _ffi.getDownloadBuffer(_h);
    return AdaptiveRingBuffer(
      capacityMs: b.capacityMs,
      readOffsetMs: b.readOffsetMs,
      writeOffsetMs: b.writeOffsetMs,
      totalMs: b.totalMs,
      isComplete: b.isComplete,
    );
  }

  int get sampleRate => _ffi.getSampleRate(_h);

  int get channels => _ffi.getChannels(_h);

  List<double> getSpectrum() => _ffi.getSpectrum(_h);

  String? get loadError => _ffi.getLoadError(_h);

  // ---------------------------------------------------------------------------
  // YouTube service
  // ---------------------------------------------------------------------------

  /// Look up the best audio stream URL for a YouTube video ID.
  String? youtubeGetStreamUrl(String videoId) {
    return _ffi.youtubeGetStreamUrl(videoId);
  }

  // ---------------------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------------------

  /// Release the native engine handle.
  ///
  /// Order is intentional: pollers (timers + stream controllers) are stopped
  /// and closed first, so any final native events during [destroyEngine] have
  /// nowhere to go — they are harmless because the engine is being torn down.
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _poller.dispose();
    if (_handle != null) {
      _ffi.destroyEngine(_handle!);
      _handle = null;
    }
  }
}
