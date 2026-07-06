import 'dart:async';
import 'dart:ffi';

import 'package:flutter/foundation.dart';

import 'models.dart';
import 'tunes4r_player_ffi.dart';

// ---------------------------------------------------------------------------
// Native event callback bridge
// ---------------------------------------------------------------------------

/// Global reference used by the native callback trampoline.
/// Set by [AudioEngine._startEvents] and cleared by [AudioEngine._stopEvents].
AudioEngine? _eventCallbackTarget;

/// Trampoline invoked directly from the Rust monitor thread when events fire.
/// Drives [stateCtrl] and [eventCtrl] instantly instead of waiting for the
/// next Dart event poller tick.
//
// Must be a top-level function (no closures) so it can be passed to
// [NativeCallable.isolateLocal].
void _onNativeEvent(int eventType, int intParam) {
  final target = _eventCallbackTarget;
  if (target == null) return;
  target._handleNativeEvent(eventType, intParam);
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
  bool _active = false;

  // Native event callback — created once, valid for isolate lifetime.
  late final NativeCallable<Void Function(Int32, Int64)> _eventCallback =
      NativeCallable<Void Function(Int32, Int64)>.isolateLocal(_onNativeEvent);

  final StreamController<PlaybackState> stateCtrl =
      StreamController<PlaybackState>.broadcast();
  final StreamController<PlaybackPosition> positionCtrl =
      StreamController<PlaybackPosition>.broadcast();
  final StreamController<EngineEvent> eventCtrl =
      StreamController<EngineEvent>.broadcast();
  final StreamController<AdaptiveRingBuffer> bufferCtrl =
      StreamController<AdaptiveRingBuffer>.broadcast();

  /// Returns the native handle or throws if disposed.
  Pointer<Void> get _h {
    if (_disposed) {
      throw const Tunes4rEngineException('AudioEngine has been disposed');
    }
    return _handle!;
  }

  /// Stream of playback state changes (driven by native events).
  Stream<PlaybackState> get stateStream => stateCtrl.stream;

  /// Stream of playback position updates driven by native events from the
  /// Rust monitor thread (~50ms resolution).
  Stream<PlaybackPosition> get positionStream => positionCtrl.stream;

  /// Stream of native engine events (state changes, seek lifecycle,
  /// end-of-stream, errors).
  Stream<EngineEvent> get playbackEventStream => eventCtrl.stream;

  /// Stream of adaptive ring buffer updates for progressive streams
  /// (HTTP / YouTube). No longer polled — to be driven by native events.
  Stream<AdaptiveRingBuffer> get downloadBufferStream => bufferCtrl.stream;

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
    return create(ffi: engineFfi);
  }

  // ---------------------------------------------------------------------------
  // Native event callback lifecycle
  // ---------------------------------------------------------------------------

  /// Register the native event callback and catch up on any state that
  /// may have been missed during the setup window.
  void _startEvents() {
    if (_active) return;
    _active = true;
    _eventCallbackTarget = this;
    _ffi.setEventCallback(_h, _eventCallback.nativeFunction);

    // Catch up on current state — the monitor thread may have already
    // transitioned to Playing before the callback was registered.
    final currentState = _ffi.getState(_h);
    if (currentState >= 0) {
      stateCtrl.add(PlaybackState.fromValue(currentState));
    }
  }

  /// Handle a native event pushed from the Rust monitor thread.
  /// Called via the trampoline [_onNativeEvent].
  void _handleNativeEvent(int eventType, int intParam) {
    if (!_active) return;
    final handle = _handle;
    if (handle == null) return;
    try {
      final e = EngineEvent(
        eventType: EngineEventType.fromValue(eventType),
        intParam: intParam,
      );
      eventCtrl.add(e);
      switch (e.eventType) {
        case EngineEventType.stateChanged:
          stateCtrl.add(PlaybackState.fromValue(intParam));
        case EngineEventType.positionUpdate:
          positionCtrl.add(_ffi.getPosition(handle));
        case EngineEventType.seekStarted:
        case EngineEventType.seekCompleted:
        case EngineEventType.endOfStream:
        case EngineEventType.none:
        case EngineEventType.positionReset:
        case EngineEventType.error:
        case EngineEventType.seekQueued:
          break;
      }
    } catch (e) {
      debugPrint('[tunes4r] native event handler error: $e');
    }
  }

  /// Stop event callback.
  void _stopEvents() {
    _active = false;
    if (_eventCallbackTarget == this) {
      _eventCallbackTarget = null;
    }
  }

  /// Release stream controllers.
  void _closeStreams() {
    stateCtrl.close();
    positionCtrl.close();
    eventCtrl.close();
    bufferCtrl.close();
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
    _startEvents();
    final result = _ffi.play(_h, uri, bufferSizeMs: bufferSizeMs);
    positionCtrl.add(_ffi.getPosition(_h));
    return result;
  }

  /// Play a YouTube video by video ID. Resolves the CDN URL on a
  /// background thread — no auto-detection needed.
  int playYoutube(String videoId, {int bufferSizeMs = -1}) {
    _startEvents();
    final result = _ffi.playYoutube(_h, videoId, bufferSizeMs: bufferSizeMs);
    positionCtrl.add(_ffi.getPosition(_h));
    return result;
  }

  /// Play an HTTP stream. Deprecated — use [play] instead; it
  /// auto-detects the source type.
  @Deprecated('Use play() instead — it auto-detects the source type.')
  int playStream(String url) {
    _startEvents();
    final result = _ffi.play(_h, url);
    positionCtrl.add(_ffi.getPosition(_h));
    return result;
  }

  void pause() {
    _ffi.pause(_h);
  }

  void resume() {
    _ffi.resume(_h);
  }

  void stop() {
    _ffi.stop(_h);
    _stopEvents();
  }

  void seek(int positionMs) {
    _ffi.seek(_h, positionMs);
    positionCtrl.add(_ffi.getPosition(_h));
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
  /// Order is intentional: event callback and stream controllers are stopped
  /// and closed first, so any final native events during [destroyEngine] have
  /// nowhere to go — they are harmless because the engine is being torn down.
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _stopEvents();
    _closeStreams();
    _eventCallback.close();
    if (_handle != null) {
      _ffi.destroyEngine(_handle!);
      _handle = null;
    }
  }
}
