import 'dart:async';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';

import 'models.dart';
import 'tunes4r_player_ffi.dart';

// HACK: set to true to log position-update diagnostics
bool _debugPos = true;

// ---------------------------------------------------------------------------
// Native event bridge
// ---------------------------------------------------------------------------

/// Single-packed event written by the Rust monitor thread via
/// [NativeCallable.isolateGroupBound] and drained by the event-dispatch Timer.
/// Packed as (eventType << 56) | (intParam & 0xFF...).
/// A single int64 guarantees atomic read/write on arm64.
int _gLastPackedEvent = 0;

/// Trampoline called from the Rust monitor thread via
/// [NativeCallable.isolateGroupBound]. Writes the packed event to
/// [_gLastPackedEvent] where the event-dispatch Timer picks it up.
/// Must only access top-level static fields (isolateGroupBound restriction).
void _onNativeEvent(int eventType, int intParam) {
  _gLastPackedEvent =
      ((eventType & 0xFF) << 56) | (intParam & 0x00FFFFFFFFFFFFFF);
}

// ---------------------------------------------------------------------------
// SeekClock — interpolates position between native sync pulses
// ---------------------------------------------------------------------------

/// Local clock that interpolates the playback position between periodic
/// sync pulses from the Rust monitor thread (~250ms interval). The UI
/// reads [displayPositionMs] every frame (via Ticker) without crossing
/// the FFI boundary — a pure-Dart computation.
class SeekClock {
  int _lastKnownMs = 0;
  int _durationMs = 0;
  bool _isPlaying = false;
  DateTime _lastSyncAt = DateTime.now();

  /// Feed a position update from the native engine.
  void onPositionUpdate(int positionMs, int totalMs) {
    _lastKnownMs = positionMs;
    if (totalMs > 0) _durationMs = totalMs;
    _lastSyncAt = DateTime.now();
  }

  /// Mark playback as active (interpolation runs) or inactive (returns last
  /// known position unchanged).
  void setPlaying(bool playing) {
    _isPlaying = playing;
    if (playing) {
      _lastSyncAt = DateTime.now();
    }
  }

  /// Interpolated position in milliseconds. Pure Dart — no FFI.
  int get displayPositionMs {
    if (!_isPlaying) return _lastKnownMs;
    final elapsed = DateTime.now().difference(_lastSyncAt).inMilliseconds;
    final interpolated = _lastKnownMs + elapsed;
    return interpolated.clamp(0, _durationMs > 0 ? _durationMs : interpolated);
  }

  int get durationMs => _durationMs;

  void reset() {
    _lastKnownMs = 0;
    _isPlaying = false;
    _lastSyncAt = DateTime.now();
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
  bool _active = false;

  // Native event callback — isolateGroupBound allows invocation from any
  // native thread. The callback can only access top-level static fields.
  late final NativeCallable<Void Function(Int32, Int64)> _eventCallback;

  final StreamController<PlaybackState> stateCtrl =
      StreamController<PlaybackState>.broadcast();
  final StreamController<PlaybackPosition> positionCtrl =
      StreamController<PlaybackPosition>.broadcast();
  final StreamController<EngineEvent> eventCtrl =
      StreamController<EngineEvent>.broadcast();
  final StreamController<AdaptiveRingBuffer> bufferCtrl =
      StreamController<AdaptiveRingBuffer>.broadcast();

  /// Interpolation clock for smooth UI without per-frame FFI.
  final SeekClock seekClock = SeekClock();

  /// Returns the native handle or throws if disposed.
  Pointer<Void> get _h {
    if (_disposed) {
      throw const Tunes4rEngineException('AudioEngine has been disposed');
    }
    return _handle!;
  }

  /// Stream of playback state changes (driven by native events).
  Stream<PlaybackState> get stateStream => stateCtrl.stream;

  /// Stream of playback position updates.
  Stream<PlaybackPosition> get positionStream => positionCtrl.stream;

  /// Stream of native engine events.
  Stream<EngineEvent> get playbackEventStream => eventCtrl.stream;

  /// Stream of adaptive ring buffer updates.
  Stream<AdaptiveRingBuffer> get downloadBufferStream => bufferCtrl.stream;

  /// Interpolated position in milliseconds. Pure Dart — no FFI call.
  int get displayPositionMs => seekClock.displayPositionMs;

  /// Duration in milliseconds from the last sync pulse.
  int get displayDurationMs => seekClock.durationMs;

  AudioEngine._(this._ffi, this._handle) {
    _eventCallback = NativeCallable<Void Function(Int32, Int64)>
        .isolateGroupBound(_onNativeEvent);
  }

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

  /// Event-dispatch Timer (pure Dart, no FFI/Mutex). Fires at ~250ms to
  /// drain the packed event written by the Rust monitor thread (when the
  /// native callback actually works), PLUS polls the cached position/state
  /// directly every tick (always works).
  Timer? _eventTimer;
  int _lastPolledPosMs = 0;
  int _lastPolledDurMs = 0;
  int _lastPolledState = -1;
  bool _engineError = false;

  /// Error message captured from the LOAD_FAILED event. Read by consumers
  /// (e.g. the app layer) to surface stream-resolution failures.
  String? lastLoadError;

  void _startEvents() {
    if (_active) return;
    _active = true;

    _ffi.setEventCallback(_h, _eventCallback.nativeFunction);

    // Pure-Dart dispatch Timer at ~250ms.
    _eventTimer = Timer.periodic(
      const Duration(milliseconds: 250),
      (_) => _processPendingEvents(),
    );

    final currentState = _ffi.getState(_h);
    if (currentState >= 0) {
      final playing = currentState == 2;
      seekClock.reset();
      seekClock.setPlaying(playing);
    }
  }

  void _processPendingEvents() {
    // 1. Drain any packed event from the native callback (best-effort)
    final packed = _gLastPackedEvent;
    if (packed != 0) {
      _gLastPackedEvent = 0;
      final eventType = packed >> 56;
      final intParam = packed & 0x00FFFFFFFFFFFFFF;
      _handleNativeEvent(eventType, intParam);
    }

    // 2. Poll cached state/position directly from Rust every tick.
    final handle = _handle;
    if (handle == null) { debugPrint('[tunes4r] poll: handle is null'); return; }
    final pos = _ffi.getPosition(handle);
    final st = _ffi.getState(handle);
    if (pos.currentMs != _lastPolledPosMs || st != _lastPolledState) {
      debugPrint('[tunes4r] poll: pos=${pos.currentMs}/${pos.totalMs} st=$st — pushing');
      _lastPolledPosMs = pos.currentMs;
      _lastPolledState = st;
      seekClock.onPositionUpdate(pos.currentMs, pos.totalMs);
      seekClock.setPlaying(st == 2);
      positionCtrl.add(pos);
      if (!_engineError) {
        stateCtrl.add(PlaybackState.fromValue(st));
      }
      if (st == 4) {
        debugPrint('[tunes4r] poll: Finished — stopping event timer');
        _active = false;
        _eventTimer?.cancel();
        _eventTimer = null;
      }
    }
  }

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
          if (_debugPos) debugPrint('[tunes4r] event: stateChanged=${intParam}');
          seekClock.setPlaying(intParam == 2);
          stateCtrl.add(PlaybackState.fromValue(intParam));
        case EngineEventType.positionUpdate:
          if (_debugPos) debugPrint('[tunes4r] event: positionUpdate');
          final pos = _ffi.getPosition(handle);
          if (_debugPos) debugPrint('[tunes4r] event: pos currentMs=${pos.currentMs} totalMs=${pos.totalMs}');
          seekClock.onPositionUpdate(pos.currentMs, pos.totalMs);
          positionCtrl.add(pos);
          // Sync state too — STATE_CHANGED is lost when Rust fires
          // both events in the same monitor tick (single packed slot).
          final s = _ffi.getState(handle);
          seekClock.setPlaying(s == 2);
          stateCtrl.add(PlaybackState.fromValue(s));
        case EngineEventType.loadFailed:
          if (_debugPos) debugPrint('[tunes4r] event: loadFailed');
          _engineError = true;
          lastLoadError = loadError;
          seekClock.setPlaying(false);
          stateCtrl.add(PlaybackState.error);
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

  void _stopEvents() {
    // Push state before clearing — the event timer is about to be cancelled
    // so no more STATE_CHANGED events from Rust will be processed.
    stateCtrl.add(PlaybackState.stopped);
    _active = false;
    _engineError = false;
    lastLoadError = null;
    seekClock.reset();
    _gLastPackedEvent = 0;
    _eventTimer?.cancel();
    _eventTimer = null;
    final h = _handle;
    if (h != null) {
      _ffi.setEventCallback(
        h,
        Pointer<NativeFunction<Void Function(Int32, Int64)>>.fromAddress(0),
      );
    }
  }

  void _closeStreams() {
    stateCtrl.close();
    positionCtrl.close();
    eventCtrl.close();
    bufferCtrl.close();
  }

  // ---------------------------------------------------------------------------
  // Playback control
  // ---------------------------------------------------------------------------

  int play(String uri, {int bufferSizeMs = -1}) {
    _engineError = false;
    lastLoadError = null;
    _startEvents();
    final result = _ffi.play(_h, uri, bufferSizeMs: bufferSizeMs);
    _syncPositionAfterPlay();
    return result;
  }

  void _syncPositionAfterPlay() {
    _syncPositionAfterPlayImpl(0);
  }

  void _syncPositionAfterPlayImpl(int depth) {
    if (_disposed || _handle == null) return;
    if (_engineError) return; // stream resolution failed — don't force playing state
    if (depth > 10) return; // 10 * 50ms = 500ms max
    final pos = _ffi.getPosition(_handle!);
    debugPrint('[tunes4r] syncPos depth=$depth currentMs=${pos.currentMs} totalMs=${pos.totalMs}');
    if (pos.totalMs > 0) {
      debugPrint('[tunes4r] syncPos SUCCESS — pushing position');
      seekClock.onPositionUpdate(pos.currentMs, pos.totalMs);
      seekClock.setPlaying(true);
      positionCtrl.add(pos);
    } else {
      Timer(const Duration(milliseconds: 50),
          () => _syncPositionAfterPlayImpl(depth + 1));
    }
  }

  int playYoutube(String videoId, {int bufferSizeMs = -1}) {
    _engineError = false;
    lastLoadError = null;
    _startEvents();
    final result = _ffi.playYoutube(_h, videoId, bufferSizeMs: bufferSizeMs);
    _syncPositionAfterPlay();
    return result;
  }

  @Deprecated('Use play() instead — it auto-detects the source type.')
  int playStream(String url) {
    _engineError = false;
    lastLoadError = null;
    _startEvents();
    final result = _ffi.play(_h, url);
    _syncPositionAfterPlay();
    return result;
  }

  void pause() {
    _ffi.pause(_h);
    seekClock.setPlaying(false);
  }

  void resume() {
    _ffi.resume(_h);
    seekClock.setPlaying(true);
  }

  void stop() {
    _ffi.stop(_h);
    _stopEvents();
  }

  void seek(int positionMs) {
    _ffi.seek(_h, positionMs);
    seekClock.onPositionUpdate(positionMs, seekClock.durationMs);
  }

  void setVolume(double volume) {
    _ffi.setVolume(_h, volume.clamp(0.0, 1.0));
  }

  void setEqBand(int band, double gainDb) {
    _ffi.setEqBand(_h, band, gainDb);
  }

  void setBassEnhancement(bool enabled, double intensity) {
    _ffi.setBassEnhancement(_h, enabled, intensity.clamp(0.0, 1.0));
  }

  void setPreGain(double gainDb) {
    _ffi.setPreGain(_h, gainDb);
  }

  double getPreGain() {
    final p = calloc<Float>();
    try {
      _ffi.getPreGain(_h, p);
      return p.value;
    } finally {
      calloc.free(p);
    }
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

  String? youtubeGetStreamUrl(String videoId) {
    return _ffi.youtubeGetStreamUrl(videoId);
  }

  String? identifySong(Int16List pcm, int sampleRate) {
    final pcmPtr = calloc<Int16>(pcm.length);
    pcmPtr.asTypedList(pcm.length).setAll(0, pcm);
    final result = _ffi.identifySong(pcmPtr, pcm.length, sampleRate);
    calloc.free(pcmPtr);
    return result;
  }

  /// Record audio from microphone and identify the song in one call.
  /// Start recording from microphone in background (non-blocking).
  /// Returns 0 on success. Poll with [pollRecording].
  int startRecording(int durationMs) => _ffi.startRecording(durationMs);

  /// Poll for recording result. Returns null while recording, JSON when done.
  String? pollRecording() => _ffi.pollRecording();

  /// Generate fingerid hashes from raw PCM audio.
  /// Returns a JSON string (array of {hash, timeOffset, peakEnergy}) or null.
  String? fingeridHashes(Int16List pcm, int sampleRate) {
    final pcmPtr = calloc<Int16>(pcm.length);
    pcmPtr.asTypedList(pcm.length).setAll(0, pcm);
    final result = _ffi.fingeridHashes(pcmPtr, pcm.length, sampleRate);
    calloc.free(pcmPtr);
    return result;
  }

  /// Get fingerprint from the current decode pipeline buffer.
  String? getFingerprint() => _ffi.getFingerprint(_handle!);

  /// Get fingerid hashes from the current decode pipeline buffer.
  /// Returns a JSON array of `{hash, timeOffset, peakEnergy}` objects
  /// suitable for querying a catalog-recognizer service.
  /// Returns null if nothing is playing or buffer is empty.
  String? getFingerprintHashes() => _ffi.getFingeridHashes(_handle!);

  // ---------------------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------------------

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _stopEvents();
    _closeStreams();
    if (_handle != null) {
      _ffi.destroyEngine(_handle!);
      _handle = null;
    }
    _eventCallback.close();
  }
}
