import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';

import 'models.dart';

// ---------------------------------------------------------------------------
// C-compatible typedefs matching `rust/src/ffi.rs`
// ---------------------------------------------------------------------------

typedef _EngineCreateNative = Pointer<Void> Function();
typedef _EngineCreateDart = Pointer<Void> Function();

typedef _EngineDestroyNative = Void Function(Pointer<Void>);
typedef _EngineDestroyDart = void Function(Pointer<Void>);

typedef _EnginePlayNative = Int32 Function(Pointer<Void>, Pointer<Utf8>, Int64);

typedef _EnginePlayDart = int Function(Pointer<Void>, Pointer<Utf8>, int);

typedef _EnginePlayYoutubeNative = Int32 Function(Pointer<Void>, Pointer<Utf8>, Int64);
typedef _EnginePlayYoutubeDart = int Function(Pointer<Void>, Pointer<Utf8>, int);

typedef _EngineSetCacheDirNative = Int32 Function(Pointer<Utf8>);
typedef _EngineSetCacheDirDart = int Function(Pointer<Utf8>);

typedef _EngineCanSeekNative = Bool Function(Pointer<Void>);
typedef _EngineCanSeekDart = bool Function(Pointer<Void>);

typedef _EngineCanDownloadNative = Bool Function(Pointer<Void>);
typedef _EngineCanDownloadDart = bool Function(Pointer<Void>);

typedef _EnginePauseNative = Void Function(Pointer<Void>);
typedef _EnginePauseDart = void Function(Pointer<Void>);

typedef _EngineResumeNative = Void Function(Pointer<Void>);
typedef _EngineResumeDart = void Function(Pointer<Void>);

typedef _EngineStopNative = Void Function(Pointer<Void>);
typedef _EngineStopDart = void Function(Pointer<Void>);

typedef _EngineSeekNative = Int32 Function(Pointer<Void>, Uint64);
typedef _EngineSeekDart = int Function(Pointer<Void>, int);

typedef _EngineSetVolumeNative = Void Function(Pointer<Void>, Float);
typedef _EngineSetVolumeDart = void Function(Pointer<Void>, double);

typedef _EngineGetVolumeNative = Float Function(Pointer<Void>);
typedef _EngineGetVolumeDart = double Function(Pointer<Void>);

typedef _EngineIsPlayingNative = Bool Function(Pointer<Void>);
typedef _EngineIsPlayingDart = bool Function(Pointer<Void>);

typedef _EngineGetStateNative = Int32 Function(Pointer<Void>);
typedef _EngineGetStateDart = int Function(Pointer<Void>);

final class PlaybackPosition extends Struct {
  @Uint64()
  external int currentMs;

  @Uint64()
  external int totalMs;
}

typedef _EngineGetPositionNative = PlaybackPosition Function(Pointer<Void>);
typedef _EngineGetPositionDart = PlaybackPosition Function(Pointer<Void>);

final class EngineEventStruct extends Struct {
  @Int32()
  external int eventType;

  @Int64()
  external int intParam;
}

typedef _EnginePollEventNative = EngineEventStruct Function(Pointer<Void>);
typedef _EnginePollEventDart = EngineEventStruct Function(Pointer<Void>);

typedef _EngineSetEventCallbackNative = Void Function(Pointer<Void>, Pointer<NativeFunction<Void Function(Int32, Int64)>>);
typedef _EngineSetEventCallbackDart = void Function(Pointer<Void>, Pointer<NativeFunction<Void Function(Int32, Int64)>>);

const int engineEventNone = 0;
const int engineEventStateChanged = 1;
const int engineEventSeekStarted = 2;
const int engineEventSeekCompleted = 3;
const int engineEventEndOfStream = 4;
const int engineEventPositionReset = 5;
const int engineEventError = 6;
const int engineEventSeekQueued = 7;
const int engineEventPositionUpdate = 8;
final class AdaptiveRingBufferStruct extends Struct {
  @Uint64()
  external int capacityMs;

  @Uint64()
  external int readOffsetMs;

  @Uint64()
  external int writeOffsetMs;

  @Uint64()
  external int totalMs;

  @Bool()
  external bool isComplete;
}

typedef _EngineGetDownloadBufferNative =
    AdaptiveRingBufferStruct Function(Pointer<Void>);
typedef _EngineGetDownloadBufferDart =
    AdaptiveRingBufferStruct Function(Pointer<Void>);

typedef _EngineGetBufferedPositionNative = Uint64 Function(Pointer<Void>);
typedef _EngineGetBufferedPositionDart = int Function(Pointer<Void>);

typedef _EngineGetSampleRateNative = Uint64 Function(Pointer<Void>);
typedef _EngineGetSampleRateDart = int Function(Pointer<Void>);

typedef _EngineGetChannelsNative = Uint64 Function(Pointer<Void>);
typedef _EngineGetChannelsDart = int Function(Pointer<Void>);

typedef _EngineGetLoadErrorNative = Pointer<Utf8> Function(Pointer<Void>);
typedef _EngineGetLoadErrorDart = Pointer<Utf8> Function(Pointer<Void>);

typedef _YoutubeGetStreamUrlNative =
    Pointer<Utf8> Function(Pointer<Utf8>);
typedef _YoutubeGetStreamUrlDart =
    Pointer<Utf8> Function(Pointer<Utf8>);

typedef _YoutubeSearchNative =
    Pointer<Utf8> Function(Pointer<Utf8>, Int32);
typedef _YoutubeSearchDart =
    Pointer<Utf8> Function(Pointer<Utf8>, int);

typedef _YoutubeFreeStringNative = Void Function(Pointer<Utf8>);
typedef _YoutubeFreeStringDart = void Function(Pointer<Utf8>);

typedef _EngineSetEqBandNative = Int32 Function(Pointer<Void>, Uint32, Float);
typedef _EngineSetEqBandDart = int Function(Pointer<Void>, int, double);

typedef _EngineGetEqGainsNative = Int32 Function(Pointer<Void>, Pointer<Float>);
typedef _EngineGetEqGainsDart = int Function(Pointer<Void>, Pointer<Float>);

typedef _EngineSetBassEnhancementNative = Int32 Function(Pointer<Void>, Bool, Float);
typedef _EngineSetBassEnhancementDart = int Function(Pointer<Void>, bool, double);

typedef _EngineGetBassEnhancementNative = Int32 Function(Pointer<Void>, Pointer<Bool>, Pointer<Float>);
typedef _EngineGetBassEnhancementDart = int Function(Pointer<Void>, Pointer<Bool>, Pointer<Float>);

typedef _EngineSetPreGainNative = Int32 Function(Pointer<Void>, Float);
typedef _EngineSetPreGainDart = int Function(Pointer<Void>, double);

typedef _EngineGetPreGainNative = Int32 Function(Pointer<Void>, Pointer<Float>);
typedef _EngineGetPreGainDart = int Function(Pointer<Void>, Pointer<Float>);

typedef _IdentifySongNative = Pointer<Utf8> Function(
  Pointer<Int16>,
  Int32,
  Int32,
);
typedef _IdentifySongDart = Pointer<Utf8> Function(
  Pointer<Int16>,
  int,
  int,
);

typedef _StartRecordingNative = Int32 Function(Int32);
typedef _StartRecordingDart = int Function(int);

typedef _PollRecordingNative = Pointer<Utf8> Function();
typedef _PollRecordingDart = Pointer<Utf8> Function();

typedef _GetFingerprintNative = Pointer<Utf8> Function(Pointer<Void>);
typedef _GetFingerprintDart = Pointer<Utf8> Function(Pointer<Void>);

typedef _FingeridHashesNative = Pointer<Utf8> Function(
  Pointer<Int16>,
  Int32,
  Int32,
);
typedef _FingeridHashesDart = Pointer<Utf8> Function(
  Pointer<Int16>,
  int,
  int,
);

typedef _GetFingeridHashesNative = Pointer<Utf8> Function(Pointer<Void>);
typedef _GetFingeridHashesDart = Pointer<Utf8> Function(Pointer<Void>);

// Telegram FFI -----------------------------------------------
typedef _TgAuthStepNative = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _TgAuthStepDart = Pointer<Utf8> Function(Pointer<Utf8>);

typedef _TgSearchNative = Pointer<Utf8> Function(
  Pointer<Utf8>,
  Pointer<Utf8>,
  Pointer<Utf8>,
  Int32,
);
typedef _TgSearchDart = Pointer<Utf8> Function(
  Pointer<Utf8>,
  Pointer<Utf8>,
  Pointer<Utf8>,
  int,
);

typedef _TgExtractNative = Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>);
typedef _TgExtractDart = Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>);

typedef _TgFreeStringNative = Void Function(Pointer<Utf8>);
typedef _TgFreeStringDart = void Function(Pointer<Utf8>);

// ---------------------------------------------------------------------------
// Low-level FFI wrapper
// ---------------------------------------------------------------------------

class Tunes4rFFI {
  DynamicLibrary? _lib;
  bool _isInitialized = false;
  String? _initError;

  DynamicLibrary get _libRef => _lib!;

  // Bound function references ------------------------------------------------
  late _EngineCreateDart _create;
  late _EngineDestroyDart _destroy;
  late _EnginePlayDart _play;
  late _EnginePlayYoutubeDart _playYoutube;
  late _EngineSetCacheDirDart _setCacheDir;
  late _EngineCanSeekDart _canSeek;
  late _EngineCanDownloadDart _canDownload;
  late _EnginePauseDart _pause;
  late _EngineResumeDart _resume;
  late _EngineStopDart _stop;
  late _EngineSeekDart _seek;
  late _EngineSetVolumeDart _setVolume;
  late _EngineGetVolumeDart _getVolume;
  late _EngineIsPlayingDart _isPlaying;
  late _EngineGetStateDart _getState;
  late _EngineGetPositionDart _getPosition;
  late _EnginePollEventDart _pollEvent;
  late _EngineSetEventCallbackDart _setEventCallback;
  late _EngineGetDownloadBufferDart _getDownloadBuffer;
  late _EngineGetBufferedPositionDart _getBufferedPosition;
  late _EngineSetEqBandDart _setEqBand;
  late _EngineGetEqGainsDart _getEqGains;
  late _EngineSetBassEnhancementDart _setBassEnhancement;
  late _EngineGetBassEnhancementDart _getBassEnhancement;
  late _EngineSetPreGainDart _setPreGain;
  late _EngineGetPreGainDart _getPreGain;
  late _EngineGetSampleRateDart _getSampleRate;
  late _EngineGetChannelsDart _getChannels;
  late _EngineGetLoadErrorDart _getLoadError;
  _IdentifySongDart? _identifySong;
  _StartRecordingDart? _startRecording;
  _PollRecordingDart? _pollRecording;
  _GetFingerprintDart? _getFingerprint;
  _FingeridHashesDart? _fingeridHashes;
  _GetFingeridHashesDart? _getFingeridHashes;
  _YoutubeGetStreamUrlDart? _youtubeGetStreamUrl;
  _YoutubeSearchDart? _youtubeSearch;
  _YoutubeFreeStringDart? _youtubeFreeString;
  _TgAuthStepDart? _tgAuthStep;
  _TgSearchDart? _tgSearch;
  _TgExtractDart? _tgExtract;
  _TgFreeStringDart? _tgFreeString;

  String? get initError => _initError;
  bool get isInitialized => _isInitialized;

  /// Opens the native library and verifies it works by creating/destroying
  /// a test engine. Returns `true` on success.
  bool initialize({String? macOSBundlePath}) {
    if (_isInitialized) return true;

    try {
      _lib = _loadLibrary(macOSBundlePath: macOSBundlePath);
      _bindFunctions();
      final engine = _create();
      if (engine == nullptr) {
        throw const Tunes4rInitException('createEngine returned null');
      }
      _destroy(engine);
      _isInitialized = true;
      debugPrint('[tunes4r] Native library loaded successfully');
      return true;
    } catch (e) {
      _initError = e.toString();
      debugPrint('[tunes4r] Initialization failed: $e');
      return false;
    }
  }

  DynamicLibrary _loadLibrary({String? macOSBundlePath}) {
    if (Platform.isAndroid) {
      // ignore: avoid_print
      print('[tunes4r] Loading libtunes4r.so (Android)');
      return DynamicLibrary.open('libtunes4r.so');
    }
    if (Platform.isIOS) {
      return DynamicLibrary.process();
    }
    if (Platform.isMacOS) {
      if (macOSBundlePath != null) {
        // ignore: avoid_print
        print('[tunes4r] Loading from explicit path: $macOSBundlePath');
        return DynamicLibrary.open(macOSBundlePath);
      }
      final exe = Platform.resolvedExecutable;
      final frameworksDir = '${File(exe).parent.parent.parent.path}/Contents/Frameworks';

      // Flutter's macOS plugin integration (CocoaPods vendored_libraries
      // and Swift Package Manager binary targets) wraps the dylib inside a
      // `libtunes4r.framework` bundle instead of dropping a flat dylib in
      // Contents/Frameworks. Try the framework locations first, then fall
      // back to the flat dylib for direct `flutter run`/CLI workflows.
      final frameworkCandidates = <String>[
        '$frameworksDir/libtunes4r.framework/Versions/A/libtunes4r',
        '$frameworksDir/libtunes4r.framework/Versions/Current/libtunes4r',
        '$frameworksDir/libtunes4r.framework/libtunes4r',
      ];
      for (final p in frameworkCandidates) {
        if (File(p).existsSync()) {
          // ignore: avoid_print
          print('[tunes4r] Loading from framework: $p');
          return DynamicLibrary.open(p);
        }
      }

      final flatDylib = '$frameworksDir/libtunes4r.dylib';
      if (File(flatDylib).existsSync()) {
        // ignore: avoid_print
        print('[tunes4r] Loading flat dylib: $flatDylib');
        return DynamicLibrary.open(flatDylib);
      }

      final dev = '${Directory.current.path}/libtunes4r.dylib';
      if (File(dev).existsSync()) {
        // ignore: avoid_print
        print('[tunes4r] Loading from project root: $dev');
        return DynamicLibrary.open(dev);
      }

      // ignore: avoid_print
      print('[tunes4r] libtunes4r.dylib not found at any expected location');
      throw Tunes4rLoadException(
        'libtunes4r.dylib not found.\n'
        'Tried:\n'
        '  - ${frameworkCandidates.join('\n  - ')}\n'
        '  - $flatDylib\n'
        '  - $dev\n'
        'Build with: cd ~/Projects/tunes4r-core && PACKAGE=tunes4r ./scripts/build_rust.sh macos',
      );
    }
    if (Platform.isLinux) {
      return DynamicLibrary.open('libtunes4r.so');
    }
    if (Platform.isWindows) {
      return DynamicLibrary.open('tunes4r.dll');
    }
    throw UnsupportedError(
      'Unsupported platform: ${Platform.operatingSystem}',
    );
  }

  void _bindFunctions() {
    final l = _libRef;
    _create = l.lookup<NativeFunction<_EngineCreateNative>>(
      'audio_engine_create',
    ).asFunction();
    _destroy = l.lookup<NativeFunction<_EngineDestroyNative>>(
      'audio_engine_destroy',
    ).asFunction();
    _play = l.lookup<NativeFunction<_EnginePlayNative>>(
      'audio_engine_play',
    ).asFunction();
    _playYoutube = l.lookup<NativeFunction<_EnginePlayYoutubeNative>>(
      'audio_engine_play_youtube',
    ).asFunction();
    _setCacheDir = l.lookup<NativeFunction<_EngineSetCacheDirNative>>(
      'audio_engine_set_cache_dir',
    ).asFunction();
    _canSeek = l.lookup<NativeFunction<_EngineCanSeekNative>>(
      'audio_engine_can_seek',
    ).asFunction();
    _canDownload = l.lookup<NativeFunction<_EngineCanDownloadNative>>(
      'audio_engine_can_download',
    ).asFunction();
    _pause = l.lookup<NativeFunction<_EnginePauseNative>>(
      'audio_engine_pause',
    ).asFunction();
    _resume = l.lookup<NativeFunction<_EngineResumeNative>>(
      'audio_engine_resume',
    ).asFunction();
    _stop = l.lookup<NativeFunction<_EngineStopNative>>(
      'audio_engine_stop',
    ).asFunction();
    _seek = l.lookup<NativeFunction<_EngineSeekNative>>(
      'audio_engine_seek',
    ).asFunction();
    _setVolume = l.lookup<NativeFunction<_EngineSetVolumeNative>>(
      'audio_engine_set_volume',
    ).asFunction();
    _getVolume = l.lookup<NativeFunction<_EngineGetVolumeNative>>(
      'audio_engine_get_volume',
    ).asFunction();
    _isPlaying = l.lookup<NativeFunction<_EngineIsPlayingNative>>(
      'audio_engine_is_playing',
    ).asFunction();
    _getState = l.lookup<NativeFunction<_EngineGetStateNative>>(
      'audio_engine_get_state',
    ).asFunction();
    _getPosition = l.lookup<NativeFunction<_EngineGetPositionNative>>(
      'audio_engine_get_position',
    ).asFunction();
    _pollEvent = l.lookup<NativeFunction<_EnginePollEventNative>>(
      'audio_engine_poll_event',
    ).asFunction();
    _setEventCallback =
        l.lookup<NativeFunction<_EngineSetEventCallbackNative>>(
          'audio_engine_set_event_callback',
        ).asFunction();
    _getDownloadBuffer =
        l.lookup<NativeFunction<_EngineGetDownloadBufferNative>>(
          'audio_engine_get_download_buffer',
        ).asFunction();
    _getBufferedPosition =
        l.lookup<NativeFunction<_EngineGetBufferedPositionNative>>(
          'audio_engine_get_buffered_position',
        ).asFunction();
    _setEqBand = l.lookup<NativeFunction<_EngineSetEqBandNative>>(
      'audio_engine_set_eq_band',
    ).asFunction();
    _getEqGains = l.lookup<NativeFunction<_EngineGetEqGainsNative>>(
      'audio_engine_get_eq_gains',
    ).asFunction();
    _setBassEnhancement = l.lookup<NativeFunction<_EngineSetBassEnhancementNative>>(
      'audio_engine_set_bass_enhancement',
    ).asFunction();
    _getBassEnhancement = l.lookup<NativeFunction<_EngineGetBassEnhancementNative>>(
      'audio_engine_get_bass_enhancement',
    ).asFunction();
    _setPreGain = l.lookup<NativeFunction<_EngineSetPreGainNative>>(
      'audio_engine_set_pre_gain',
    ).asFunction();
    _getPreGain = l.lookup<NativeFunction<_EngineGetPreGainNative>>(
      'audio_engine_get_pre_gain',
    ).asFunction();
    _getSampleRate = l.lookup<NativeFunction<_EngineGetSampleRateNative>>(
      'audio_engine_get_sample_rate',
    ).asFunction();
    _getChannels = l.lookup<NativeFunction<_EngineGetChannelsNative>>(
      'audio_engine_get_channels',
    ).asFunction();
    _getLoadError = l.lookup<NativeFunction<_EngineGetLoadErrorNative>>(
      'audio_engine_get_load_error',
    ).asFunction();
    _bindYoutubeSymbols(l);
    _bindIdentifySong(l);
    _bindRecordSongId(l);
    _bindFingeridHashes(l);
    _bindTelegramSymbols(l);
  }

  void _bindYoutubeSymbols(DynamicLibrary l) {
    try {
      _youtubeGetStreamUrl =
          l.lookup<NativeFunction<_YoutubeGetStreamUrlNative>>(
            'youtube_get_stream_url',
          ).asFunction();
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: youtube_get_stream_url');
    }
    try {
      _youtubeSearch = l.lookup<NativeFunction<_YoutubeSearchNative>>(
        'youtube_search',
      ).asFunction();
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: youtube_search');
    }
    try {
      _youtubeFreeString = l.lookup<NativeFunction<_YoutubeFreeStringNative>>(
        'youtube_free_string',
      ).asFunction();
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: youtube_free_string');
    }
  }

  void _bindIdentifySong(DynamicLibrary l) {
    try {
      _identifySong = l.lookup<NativeFunction<_IdentifySongNative>>(
        'audio_engine_identify_song',
      ).asFunction();
      debugPrint('[tunes4r] identifySong bound successfully');
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: audio_engine_identify_song');
      _identifySong = null;
    }
  }

  void _bindRecordSongId(DynamicLibrary l) {
    try {
      _startRecording = l.lookup<NativeFunction<_StartRecordingNative>>(
        'audio_engine_start_recording',
      ).asFunction();
      debugPrint('[tunes4r] startRecording bound');
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: audio_engine_start_recording');
      _startRecording = null;
    }
    try {
      _pollRecording = l.lookup<NativeFunction<_PollRecordingNative>>(
        'audio_engine_poll_recording',
      ).asFunction();
      debugPrint('[tunes4r] pollRecording bound');
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: audio_engine_poll_recording');
      _pollRecording = null;
    }
    try {
      _getFingerprint = l.lookup<NativeFunction<_GetFingerprintNative>>(
        'audio_engine_get_fingerprint',
      ).asFunction();
      debugPrint('[tunes4r] getFingerprint bound');
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: audio_engine_get_fingerprint');
      _getFingerprint = null;
    }
  }

  void _bindFingeridHashes(DynamicLibrary l) {
    try {
      _fingeridHashes = l.lookup<NativeFunction<_FingeridHashesNative>>(
        'audio_engine_fingerid_hashes',
      ).asFunction();
      debugPrint('[tunes4r] fingeridHashes bound');
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: audio_engine_fingerid_hashes');
      _fingeridHashes = null;
    }
    try {
      _getFingeridHashes = l.lookup<NativeFunction<_GetFingeridHashesNative>>(
        'audio_engine_get_fingerid_hashes',
      ).asFunction();
      debugPrint('[tunes4r] getFingeridHashes bound');
    } catch (_) {
      debugPrint('[tunes4r] Optional symbol not found: audio_engine_get_fingerid_hashes');
      _getFingeridHashes = null;
    }
  }

  void _bindTelegramSymbols(DynamicLibrary l) {
    try {
      _tgAuthStep = l.lookup<NativeFunction<_TgAuthStepNative>>(
        'tg_auth_step',
      ).asFunction();
      debugPrint('[tunes4r] tg_auth_step bound');
    } catch (e) {
      // ignore: avoid_print
      print('[tunes4r] MISSING telegram symbol: tg_auth_step — $e');
      // ignore: avoid_print
      print('[tunes4r] Rebuild with: cd ~/Projects/tunes4r-core && PACKAGE=tunes4r ./scripts/build_rust.sh macos');
    }
    try {
      _tgSearch = l.lookup<NativeFunction<_TgSearchNative>>(
        'tg_search',
      ).asFunction();
      debugPrint('[tunes4r] tg_search bound');
    } catch (e) {
      // ignore: avoid_print
      print('[tunes4r] MISSING telegram symbol: tg_search — $e');
    }
    try {
      _tgExtract = l.lookup<NativeFunction<_TgExtractNative>>(
        'tg_extract',
      ).asFunction();
      debugPrint('[tunes4r] tg_extract bound');
    } catch (e) {
      // ignore: avoid_print
      print('[tunes4r] MISSING telegram symbol: tg_extract — $e');
    }
    try {
      _tgFreeString = l.lookup<NativeFunction<_TgFreeStringNative>>(
        'tg_free_string',
      ).asFunction();
      debugPrint('[tunes4r] tg_free_string bound');
    } catch (e) {
      // ignore: avoid_print
      print('[tunes4r] MISSING telegram symbol: tg_free_string — $e');
    }
  }

  // ---------------------------------------------------------------------------
  // Public API — low-level (Pointer<AudioEngineHandle>)
  // ---------------------------------------------------------------------------

  Pointer<Void> createEngine() => _create();
  void destroyEngine(Pointer<Void> h) => _destroy(h);

  int play(Pointer<Void> h, String uri, {int bufferSizeMs = -1}) {
    final ptr = uri.toNativeUtf8();
    try {
      return _play(h, ptr, bufferSizeMs);
    } finally {
      calloc.free(ptr);
    }
  }

  int playYoutube(Pointer<Void> h, String videoId, {int bufferSizeMs = -1}) {
    final ptr = videoId.toNativeUtf8();
    try {
      return _playYoutube(h, ptr, bufferSizeMs);
    } finally {
      calloc.free(ptr);
    }
  }

  bool canSeek(Pointer<Void> h) => _canSeek(h);
  bool canDownload(Pointer<Void> h) => _canDownload(h);

  void pause(Pointer<Void> h) => _pause(h);
  void resume(Pointer<Void> h) => _resume(h);
  void stop(Pointer<Void> h) => _stop(h);
  int seek(Pointer<Void> h, int positionMs) => _seek(h, positionMs);
  void setVolume(Pointer<Void> h, double volume) => _setVolume(h, volume);
  double getVolume(Pointer<Void> h) => _getVolume(h);
  bool isPlaying(Pointer<Void> h) => _isPlaying(h);
  int getState(Pointer<Void> h) => _getState(h);
  PlaybackPosition getPosition(Pointer<Void> h) => _getPosition(h);
  EngineEventStruct pollEvent(Pointer<Void> h) => _pollEvent(h);
  void setEventCallback(
    Pointer<Void> h,
    Pointer<NativeFunction<Void Function(Int32, Int64)>> cb,
  ) =>
      _setEventCallback(h, cb);
  AdaptiveRingBufferStruct getDownloadBuffer(Pointer<Void> h) =>
      _getDownloadBuffer(h);

  int getBufferedPosition(Pointer<Void> h) => _getBufferedPosition(h);
  int getSampleRate(Pointer<Void> h) => _getSampleRate(h);
  int getChannels(Pointer<Void> h) => _getChannels(h);

  String? getLoadError(Pointer<Void> h) {
    final ptr = _getLoadError(h);
    if (ptr == nullptr) return null;
    final s = ptr.toDartString();
    calloc.free(ptr);
    return s.isEmpty ? null : s;
  }

  String? youtubeGetStreamUrl(String videoId) {
    final fn = _youtubeGetStreamUrl;
    if (fn == null) return null;
    final ptr = videoId.toNativeUtf8();
    try {
      final resultPtr = fn(ptr);
      if (resultPtr == nullptr) return null;
      final s = resultPtr.toDartString();
      calloc.free(resultPtr);
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(ptr);
    }
  }

  String? youtubeSearch(String query, int limit) {
    final fn = _youtubeSearch;
    final free = _youtubeFreeString;
    if (fn == null) return null;
    final queryPtr = query.toNativeUtf8();
    try {
      final resultPtr = fn(queryPtr, limit);
      if (resultPtr == nullptr) return null;
      final s = resultPtr.toDartString();
      free?.call(resultPtr);
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(queryPtr);
    }
  }

  /// Set the cache directory for YouTube streams.
  /// Call before any play_youtube call. On Android, pass the result of
  /// `getApplicationCacheDirectory()` from path_provider.
  int setCacheDir(String path) {
    final pathPtr = path.toNativeUtf8();
    try {
      return _setCacheDir(pathPtr);
    } finally {
      calloc.free(pathPtr);
    }
  }

  int setEqBand(Pointer<Void> h, int band, double gainDb) =>
      _setEqBand(h, band, gainDb);

  int getEqGains(Pointer<Void> h, Pointer<Float> gains) =>
      _getEqGains(h, gains);

  int setBassEnhancement(Pointer<Void> h, bool enabled, double intensity) =>
      _setBassEnhancement(h, enabled, intensity.clamp(0.0, 1.0));

  int getBassEnhancement(
    Pointer<Void> h,
    Pointer<Bool> enabled,
    Pointer<Float> intensity,
  ) =>
      _getBassEnhancement(h, enabled, intensity);

  int setPreGain(Pointer<Void> h, double gainDb) =>
      _setPreGain(h, gainDb.clamp(-20.0, 20.0));

  int getPreGain(Pointer<Void> h, Pointer<Float> gainDb) =>
      _getPreGain(h, gainDb);

  String? identifySong(Pointer<Int16> pcm, int len, int sampleRate) {
    final fn = _identifySong;
    if (fn == null) return null;
    final resultPtr = fn(pcm, len, sampleRate);
    if (resultPtr == nullptr) return null;
    final s = resultPtr.toDartString();
    calloc.free(resultPtr);
    return s.isEmpty ? null : s;
  }

  /// Start recording microphone audio in a background thread.
  /// Returns immediately (non-blocking).
  /// Returns 0 on success, -1 on invalid duration.
  int startRecording(int durationMs) {
    final fn = _startRecording;
    if (fn == null) return -1;
    return fn(durationMs);
  }

  /// Poll for recording result.
  /// Returns null while still recording, JSON string when done.
  String? pollRecording() {
    final fn = _pollRecording;
    if (fn == null) return null;
    final resultPtr = fn();
    if (resultPtr == nullptr) return null;
    final s = resultPtr.toDartString();
    calloc.free(resultPtr);
    return s.isEmpty ? null : s;
  }

  /// Generate fingerid hashes from raw PCM audio.
  /// Returns a JSON string (array of {hash, timeOffset, peakEnergy}) or null.
  /// Caller must free the returned string via `freeString`.
  String? fingeridHashes(Pointer<Int16> pcm, int len, int sampleRate) {
    final fn = _fingeridHashes;
    if (fn == null) return null;
    final resultPtr = fn(pcm, len, sampleRate);
    if (resultPtr == nullptr) return null;
    final s = resultPtr.toDartString();
    calloc.free(resultPtr);
    return s.isEmpty ? null : s;
  }

  /// Get fingerprint from the current decode pipeline buffer.
  /// Returns null if nothing is playing or buffer is empty.
  /// Caller must free the returned string via `freeString`.
  String? getFingerprint(Pointer<Void> engine) {
    final fn = _getFingerprint;
    if (fn == null) return null;
    final resultPtr = fn(engine);
    if (resultPtr == nullptr) return null;
    final s = resultPtr.toDartString();
    calloc.free(resultPtr);
    return s.isEmpty ? null : s;
  }

  /// Get fingerid hashes from the current decode pipeline buffer.
  /// Reads the internal PCM buffer (up to ~30s of audio from the current
  /// playback) and returns fingerid hashes as a JSON array.
  /// Returns null if nothing is playing or buffer is empty.
  String? getFingeridHashes(Pointer<Void> engine) {
    final fn = _getFingeridHashes;
    if (fn == null) return null;
    final resultPtr = fn(engine);
    if (resultPtr == nullptr) return null;
    final s = resultPtr.toDartString();
    calloc.free(resultPtr);
    return s.isEmpty ? null : s;
  }

  String? tgAuthStep(String sessionJson) {
    final fn = _tgAuthStep;
    if (fn == null) return null;
    final sessionPtr = sessionJson.toNativeUtf8();
    try {
      final resultPtr = fn(sessionPtr);
      if (resultPtr == nullptr) return null;
      final s = resultPtr.toDartString();
      _tgFreeString?.call(resultPtr);
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(sessionPtr);
    }
  }

  String? tgSearch(String sessionJson, String channel, String query, int limit) {
    final fn = _tgSearch;
    if (fn == null) return null;
    final sessionPtr = sessionJson.toNativeUtf8();
    final channelPtr = channel.toNativeUtf8();
    final queryPtr = query.toNativeUtf8();
    try {
      final resultPtr = fn(sessionPtr, channelPtr, queryPtr, limit);
      if (resultPtr == nullptr) return null;
      final s = resultPtr.toDartString();
      _tgFreeString?.call(resultPtr);
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(sessionPtr);
      calloc.free(channelPtr);
      calloc.free(queryPtr);
    }
  }

  String? tgExtract(String sessionJson, String url) {
    final fn = _tgExtract;
    if (fn == null) return null;
    final sessionPtr = sessionJson.toNativeUtf8();
    final urlPtr = url.toNativeUtf8();
    try {
      final resultPtr = fn(sessionPtr, urlPtr);
      if (resultPtr == nullptr) return null;
      final s = resultPtr.toDartString();
      _tgFreeString?.call(resultPtr);
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(sessionPtr);
      calloc.free(urlPtr);
    }
  }
}

/// Convenience singleton.
///
/// Consider using dependency injection via [AudioEngine.create](ffi:)
/// instead, which accepts a custom [Tunes4rFFI] instance.
@Deprecated(
  'Prefer dependency injection via AudioEngine.create(ffi: myFFI). '
  'This global will be removed in a future release.',
)
final Tunes4rFFI tunes4rFFI = Tunes4rFFI();
