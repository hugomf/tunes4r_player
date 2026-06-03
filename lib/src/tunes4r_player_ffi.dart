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

typedef _EnginePlayNative = Int32 Function(Pointer<Void>, Pointer<Utf8>);
typedef _EnginePlayDart = int Function(Pointer<Void>, Pointer<Utf8>);

typedef _EngineCanSeekNative = Bool Function(Pointer<Void>);
typedef _EngineCanSeekDart = bool Function(Pointer<Void>);

typedef _EngineCanDownloadNative = Bool Function(Pointer<Void>);
typedef _EngineCanDownloadDart = bool Function(Pointer<Void>);

typedef _EnginePlayStreamFromBytesNative =
    Int32 Function(Pointer<Void>, Pointer<Utf8>);
typedef _EnginePlayStreamFromBytesDart =
    int Function(Pointer<Void>, Pointer<Utf8>);

typedef _EngineFetchAndPipeNative =
    Int32 Function(Pointer<Void>, Pointer<Utf8>);
typedef _EngineFetchAndPipeDart =
    int Function(Pointer<Void>, Pointer<Utf8>);

typedef _EnginePushAudioBytesNative =
    Void Function(Pointer<Void>, Pointer<Uint8>, Int32);
typedef _EnginePushAudioBytesDart =
    void Function(Pointer<Void>, Pointer<Uint8>, int);

typedef _EngineEndAudioStreamNative = Void Function(Pointer<Void>);
typedef _EngineEndAudioStreamDart = void Function(Pointer<Void>);

typedef _EngineSetStreamErrorNative =
    Void Function(Pointer<Void>, Pointer<Utf8>);
typedef _EngineSetStreamErrorDart =
    void Function(Pointer<Void>, Pointer<Utf8>);

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

typedef _EngineGetSpectrumBandCountForEngineNative =
    Int32 Function(Pointer<Void>);
typedef _EngineGetSpectrumBandCountForEngineDart =
    int Function(Pointer<Void>);

typedef _EngineSetSpectrumBandCountNative =
    Void Function(Pointer<Void>, Int32);
typedef _EngineSetSpectrumBandCountDart =
    void Function(Pointer<Void>, int);

typedef _EngineSetSpectrumBandCountGlobalNative = Void Function(Int32);
typedef _EngineSetSpectrumBandCountGlobalDart = void Function(int);

final class PlaybackPosition extends Struct {
  @Uint64()
  external int currentMs;

  @Uint64()
  external int totalMs;
}

typedef _EngineGetPositionNative = PlaybackPosition Function(Pointer<Void>);
typedef _EngineGetPositionDart = PlaybackPosition Function(Pointer<Void>);

typedef _EngineGetSpectrumNative =
    Bool Function(Pointer<Void>, Pointer<Float>, Uint64);
typedef _EngineGetSpectrumDart =
    bool Function(Pointer<Void>, Pointer<Float>, int);

typedef _EngineGetBufferedPositionNative = Uint64 Function(Pointer<Void>);
typedef _EngineGetBufferedPositionDart = int Function(Pointer<Void>);

typedef _EngineGetSampleRateNative = Uint64 Function(Pointer<Void>);
typedef _EngineGetSampleRateDart = int Function(Pointer<Void>);

typedef _EngineGetChannelsNative = Uint64 Function(Pointer<Void>);
typedef _EngineGetChannelsDart = int Function(Pointer<Void>);

typedef _EngineGetLoadErrorNative = Pointer<Utf8> Function(Pointer<Void>);
typedef _EngineGetLoadErrorDart = Pointer<Utf8> Function(Pointer<Void>);

typedef _EngineGetLastErrorNative = Pointer<Utf8> Function(Pointer<Void>);
typedef _EngineGetLastErrorDart = Pointer<Utf8> Function(Pointer<Void>);

typedef _EngineGetPipeSeekOffsetNative = Int64 Function(Pointer<Void>);
typedef _EngineGetPipeSeekOffsetDart = int Function(Pointer<Void>);

typedef _EnginePollPipeSeekByteOffsetNative = Int64 Function(Pointer<Void>);
typedef _EnginePollPipeSeekByteOffsetDart = int Function(Pointer<Void>);

typedef _EngineClearPipeSeekRequestNative = Void Function(Pointer<Void>);
typedef _EngineClearPipeSeekRequestDart = void Function(Pointer<Void>);

typedef _EngineSetPipeTotalBytesNative =
    Void Function(Pointer<Void>, Uint64);
typedef _EngineSetPipeTotalBytesDart =
    void Function(Pointer<Void>, int);

typedef _YoutubeServiceCreateNative = Pointer<Void> Function();
typedef _YoutubeServiceCreateDart = Pointer<Void> Function();

typedef _YoutubeServiceDestroyNative = Void Function(Pointer<Void>);
typedef _YoutubeServiceDestroyDart = void Function(Pointer<Void>);

typedef _YoutubeGetStreamUrlNative =
    Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>);
typedef _YoutubeGetStreamUrlDart =
    Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>);

typedef _YoutubeSearchNative =
    Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>, Int32);
typedef _YoutubeSearchDart =
    Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>, int);

typedef _YoutubeGetVideoInfoNative =
    Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>);
typedef _YoutubeGetVideoInfoDart =
    Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>);

typedef _EnginePlayYoutubeNative =
    Int32 Function(Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>);
typedef _EnginePlayYoutubeDart =
    int Function(Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>);

typedef _EnginePlayStreamWithDownloaderNative =
    Int32 Function(Pointer<Void>, Pointer<Utf8>);
typedef _EnginePlayStreamWithDownloaderDart =
    int Function(Pointer<Void>, Pointer<Utf8>);

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
  late _EngineCanSeekDart _canSeek;
  late _EngineCanDownloadDart _canDownload;
  late _EnginePlayStreamFromBytesDart _playStreamFromBytes;
  late _EngineFetchAndPipeDart _fetchAndPipe;
  late _EnginePushAudioBytesDart _pushAudioBytes;
  late _EngineEndAudioStreamDart _endAudioStream;
  late _EngineSetStreamErrorDart _setStreamError;
  late _EnginePauseDart _pause;
  late _EngineResumeDart _resume;
  late _EngineStopDart _stop;
  late _EngineSeekDart _seek;
  late _EngineSetVolumeDart _setVolume;
  late _EngineGetVolumeDart _getVolume;
  late _EngineIsPlayingDart _isPlaying;
  late _EngineGetStateDart _getState;
  late _EngineGetPositionDart _getPosition;
  late _EngineGetSpectrumDart _getSpectrum;
  late _EngineGetSpectrumBandCountForEngineDart
      _getSpectrumBandCountForEngine;
  late _EngineSetSpectrumBandCountDart _setSpectrumBandCount;
  late _EngineSetSpectrumBandCountGlobalDart _setSpectrumBandCountGlobal;
  late _EngineGetBufferedPositionDart _getBufferedPosition;
  late _EngineGetSampleRateDart _getSampleRate;
  late _EngineGetChannelsDart _getChannels;
  late _EngineGetLoadErrorDart _getLoadError;
  late _EngineGetLastErrorDart _getLastError;
  late _EngineGetPipeSeekOffsetDart _getPipeSeekOffset;
  late _EnginePollPipeSeekByteOffsetDart _pollPipeSeekByteOffset;
  late _EngineClearPipeSeekRequestDart _clearPipeSeekRequest;
  late _EngineSetPipeTotalBytesDart _setPipeTotalBytes;
  late _YoutubeServiceCreateDart _youtubeServiceCreate;
  late _YoutubeServiceDestroyDart _youtubeServiceDestroy;
  late _YoutubeGetStreamUrlDart _youtubeGetStreamUrl;
  late _YoutubeSearchDart _youtubeSearch;
  late _YoutubeGetVideoInfoDart _youtubeGetVideoInfo;
  late _EnginePlayYoutubeDart _playYoutube;
  late _EnginePlayStreamWithDownloaderDart _playStreamWithDownloader;

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
      return DynamicLibrary.open('libtunes4r.so');
    }
    if (Platform.isIOS) {
      return DynamicLibrary.process();
    }
    if (Platform.isMacOS) {
      if (macOSBundlePath != null) {
        debugPrint('[tunes4r] Loading from explicit path: $macOSBundlePath');
        return DynamicLibrary.open(macOSBundlePath);
      }
      final exe = Platform.resolvedExecutable;
      final bundle = '${File(exe).parent.parent.parent.path}/Contents/Frameworks/libtunes4r.dylib';
      debugPrint('[tunes4r] Trying bundle: $bundle');
      if (File(bundle).existsSync()) {
        debugPrint('[tunes4r] Loading from bundle');
        return DynamicLibrary.open(bundle);
      }
      final dev = '${Directory.current.path}/libtunes4r.dylib';
      debugPrint('[tunes4r] Trying project root: $dev');
      if (File(dev).existsSync()) {
        debugPrint('[tunes4r] Loading from project root');
        return DynamicLibrary.open(dev);
      }
      throw Tunes4rLoadException(
        'libtunes4r.dylib not found.\n'
        'Tried:\n'
        '  - $bundle\n'
        '  - $dev\n'
        'Build with: make build-macos',
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
    _canSeek = l.lookup<NativeFunction<_EngineCanSeekNative>>(
      'audio_engine_can_seek',
    ).asFunction();
    _canDownload = l.lookup<NativeFunction<_EngineCanDownloadNative>>(
      'audio_engine_can_download',
    ).asFunction();
    _playStreamFromBytes =
        l.lookup<NativeFunction<_EnginePlayStreamFromBytesNative>>(
          'audio_engine_play_stream_from_bytes',
        ).asFunction();
    _fetchAndPipe = l.lookup<NativeFunction<_EngineFetchAndPipeNative>>(
      'audio_engine_fetch_and_pipe',
    ).asFunction();
    _pushAudioBytes = l.lookup<NativeFunction<_EnginePushAudioBytesNative>>(
      'audio_engine_push_audio_bytes',
    ).asFunction();
    _endAudioStream = l.lookup<NativeFunction<_EngineEndAudioStreamNative>>(
      'audio_engine_end_audio_stream',
    ).asFunction();
    _setStreamError = l.lookup<NativeFunction<_EngineSetStreamErrorNative>>(
      'audio_engine_set_stream_error',
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
    _getSpectrum = l.lookup<NativeFunction<_EngineGetSpectrumNative>>(
      'audio_engine_get_spectrum',
    ).asFunction();
    _getSpectrumBandCountForEngine =
        l.lookup<NativeFunction<_EngineGetSpectrumBandCountForEngineNative>>(
          'audio_engine_get_spectrum_band_count_for_engine',
        ).asFunction();
    _setSpectrumBandCount =
        l.lookup<NativeFunction<_EngineSetSpectrumBandCountNative>>(
          'audio_engine_set_spectrum_band_count',
        ).asFunction();
    _setSpectrumBandCountGlobal =
        l.lookup<NativeFunction<_EngineSetSpectrumBandCountGlobalNative>>(
          'audio_engine_set_spectrum_band_count_global',
        ).asFunction();
    _getBufferedPosition =
        l.lookup<NativeFunction<_EngineGetBufferedPositionNative>>(
          'audio_engine_get_buffered_position',
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
    _getLastError = l.lookup<NativeFunction<_EngineGetLastErrorNative>>(
      'audio_engine_get_last_error',
    ).asFunction();
    _getPipeSeekOffset =
        l.lookup<NativeFunction<_EngineGetPipeSeekOffsetNative>>(
          'audio_engine_get_pipe_seek_offset',
        ).asFunction();
    _pollPipeSeekByteOffset =
        l.lookup<NativeFunction<_EnginePollPipeSeekByteOffsetNative>>(
          'audio_engine_poll_pipe_seek_byte_offset',
        ).asFunction();
    _clearPipeSeekRequest =
        l.lookup<NativeFunction<_EngineClearPipeSeekRequestNative>>(
          'audio_engine_clear_pipe_seek_request',
        ).asFunction();
    _setPipeTotalBytes =
        l.lookup<NativeFunction<_EngineSetPipeTotalBytesNative>>(
          'audio_engine_set_pipe_total_bytes',
        ).asFunction();
    _youtubeServiceCreate = l.lookup<NativeFunction<_YoutubeServiceCreateNative>>(
      'youtube_service_create',
    ).asFunction();
    _youtubeServiceDestroy =
        l.lookup<NativeFunction<_YoutubeServiceDestroyNative>>(
          'youtube_service_destroy',
        ).asFunction();
    _youtubeGetStreamUrl =
        l.lookup<NativeFunction<_YoutubeGetStreamUrlNative>>(
          'youtube_get_stream_url',
        ).asFunction();
    _youtubeSearch = l.lookup<NativeFunction<_YoutubeSearchNative>>(
      'youtube_search',
    ).asFunction();
    _youtubeGetVideoInfo =
        l.lookup<NativeFunction<_YoutubeGetVideoInfoNative>>(
          'youtube_get_video_info',
        ).asFunction();
    _playYoutube = l.lookup<NativeFunction<_EnginePlayYoutubeNative>>(
      'audio_engine_play_youtube',
    ).asFunction();
    _playStreamWithDownloader =
        l.lookup<NativeFunction<_EnginePlayStreamWithDownloaderNative>>(
          'audio_engine_play_stream_with_downloader',
        ).asFunction();
  }

  // ---------------------------------------------------------------------------
  // Public API — low-level (Pointer<AduioEngineHandle>)
  // ---------------------------------------------------------------------------

  Pointer<Void> createEngine() => _create();
  void destroyEngine(Pointer<Void> h) => _destroy(h);

  int play(Pointer<Void> h, String uri) {
    final ptr = uri.toNativeUtf8();
    try {
      return _play(h, ptr);
    } finally {
      calloc.free(ptr);
    }
  }

  bool canSeek(Pointer<Void> h) => _canSeek(h);
  bool canDownload(Pointer<Void> h) => _canDownload(h);

  int playStreamFromBytes(Pointer<Void> h, String url) {
    final ptr = url.toNativeUtf8();
    try {
      return _playStreamFromBytes(h, ptr);
    } finally {
      calloc.free(ptr);
    }
  }

  int fetchAndPipe(Pointer<Void> h, String url) {
    final ptr = url.toNativeUtf8();
    try {
      return _fetchAndPipe(h, ptr);
    } finally {
      calloc.free(ptr);
    }
  }

  void pushAudioBytes(Pointer<Void> h, Pointer<Uint8> data, int len) =>
      _pushAudioBytes(h, data, len);

  void endAudioStream(Pointer<Void> h) => _endAudioStream(h);

  void setStreamError(Pointer<Void> h, String message) {
    final ptr = message.toNativeUtf8();
    try {
      _setStreamError(h, ptr);
    } finally {
      calloc.free(ptr);
    }
  }

  void pause(Pointer<Void> h) => _pause(h);
  void resume(Pointer<Void> h) => _resume(h);
  void stop(Pointer<Void> h) => _stop(h);
  int seek(Pointer<Void> h, int positionMs) => _seek(h, positionMs);
  void setVolume(Pointer<Void> h, double volume) => _setVolume(h, volume);
  double getVolume(Pointer<Void> h) => _getVolume(h);
  bool isPlaying(Pointer<Void> h) => _isPlaying(h);
  int getState(Pointer<Void> h) => _getState(h);
  PlaybackPosition getPosition(Pointer<Void> h) => _getPosition(h);

  List<double> getSpectrum(Pointer<Void> h) {
    final count = _getSpectrumBandCountForEngine(h);
    if (count <= 0) return [];
    final buf = calloc<Float>(count);
    try {
      if (_getSpectrum(h, buf, count)) {
        return List.generate(count, (i) => buf[i]);
      }
      return [];
    } finally {
      calloc.free(buf);
    }
  }

  void setSpectrumBandCount(Pointer<Void> h, int c) =>
      _setSpectrumBandCount(h, c);
  void setSpectrumBandCountGlobal(int c) => _setSpectrumBandCountGlobal(c);
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

  String? getLastError(Pointer<Void> h) {
    final ptr = _getLastError(h);
    if (ptr == nullptr) return null;
    final s = ptr.toDartString();
    calloc.free(ptr);
    return s.isEmpty ? null : s;
  }

  int getPipeSeekOffset(Pointer<Void> h) => _getPipeSeekOffset(h);
  int pollPipeSeekByteOffset(Pointer<Void> h) => _pollPipeSeekByteOffset(h);
  void clearPipeSeekRequest(Pointer<Void> h) => _clearPipeSeekRequest(h);
  void setPipeTotalBytes(Pointer<Void> h, int b) => _setPipeTotalBytes(h, b);

  Pointer<Void> youtubeServiceCreate() => _youtubeServiceCreate();
  void youtubeServiceDestroy(Pointer<Void> h) => _youtubeServiceDestroy(h);

  String? youtubeGetStreamUrl(Pointer<Void> h, String videoId) {
    final ptr = videoId.toNativeUtf8();
    try {
      final resultPtr = _youtubeGetStreamUrl(h, ptr);
      if (resultPtr == nullptr) return null;
      final s = resultPtr.toDartString();
      calloc.free(resultPtr);
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(ptr);
    }
  }

  String? youtubeSearch(Pointer<Void> h, String query, {int limit = 20}) {
    final queryPtr = query.toNativeUtf8();
    try {
      final resultPtr = _youtubeSearch(h, queryPtr, limit);
      if (resultPtr == nullptr) return null;
      final s = resultPtr.toDartString();
      calloc.free(resultPtr);
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(queryPtr);
    }
  }

  String? youtubeGetVideoInfo(Pointer<Void> h, String videoId) {
    final ptr = videoId.toNativeUtf8();
    try {
      final resultPtr = _youtubeGetVideoInfo(h, ptr);
      if (resultPtr == nullptr) return null;
      final s = resultPtr.toDartString();
      calloc.free(resultPtr);
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(ptr);
    }
  }

  int playYoutube(Pointer<Void> h, String url, String cacheDir) {
    final urlPtr = url.toNativeUtf8();
    final cachePtr = cacheDir.toNativeUtf8();
    try {
      return _playYoutube(h, urlPtr, cachePtr);
    } finally {
      calloc.free(urlPtr);
      calloc.free(cachePtr);
    }
  }

  int playStreamWithDownloader(Pointer<Void> h, String url) {
    final ptr = url.toNativeUtf8();
    try {
      return _playStreamWithDownloader(h, ptr);
    } finally {
      calloc.free(ptr);
    }
  }
}

/// Convenience singleton.
final Tunes4rFFI tunes4rFFI = Tunes4rFFI();
