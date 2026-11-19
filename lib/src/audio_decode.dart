import 'dart:convert';
import 'dart:io';

import 'package:path_provider/path_provider.dart';
import 'package:tunes4r_player/src/tunes4r_player_ffi.dart';

/// Decoded audio ready for a speech recogniser: 16 kHz mono 16-bit PCM WAV.
class AsrAudio {
  final String wavPath;
  final Duration duration;

  const AsrAudio({required this.wavPath, required this.duration});
}

/// Thrown when the file cannot be decoded, with the native reason when there is
/// one. A caller that only gets `null` cannot tell "this codec is unsupported"
/// from "the engine is not installed", and ends up showing the wrong advice.
class AsrAudioException implements Exception {
  final String message;

  const AsrAudioException(this.message);

  @override
  String toString() => message;
}

/// Decode [path] to the 16 kHz mono WAV an on-device recogniser expects.
///
/// Reuses the player's own decoder (symphonia + the pure-Rust AAC path) instead
/// of shipping a second one in Dart, so every container the app can play can
/// also be transcribed.
///
/// [maxSeconds] caps the excerpt; 0 means the whole file. [outputPath] lets the
/// caller choose where the WAV lands; otherwise a temporary file is used. The
/// caller owns the returned file and is responsible for deleting it.
Future<AsrAudio> extractAsrAudio(
  String path, {
  int maxSeconds = 0,
  String? outputPath,
}) async {
  final ffi = tunes4rFFI;
  if (!ffi.initialize()) {
    throw const AsrAudioException(
      'Native audio engine unavailable; cannot decode audio.',
    );
  }

  final output =
      outputPath ??
      '${(await getTemporaryDirectory()).path}/'
          'tunes4r_asr_${DateTime.now().microsecondsSinceEpoch}.wav';

  final raw = ffi.decodeAsrWav(path, output, maxSeconds: maxSeconds);
  if (raw == null) {
    throw const AsrAudioException(
      'This build of the audio engine cannot export PCM '
      '(audio_engine_decode_asr_wav missing).',
    );
  }

  final Map<String, dynamic> decoded;
  try {
    decoded = jsonDecode(raw) as Map<String, dynamic>;
  } on FormatException {
    throw AsrAudioException('Unreadable decoder response: $raw');
  }

  final error = decoded['error'];
  if (error != null) {
    throw AsrAudioException('$error');
  }

  final durationMs = decoded['duration_ms'];
  if (durationMs is! num) {
    throw AsrAudioException('Decoder returned no duration: $raw');
  }

  return AsrAudio(
    wavPath: output,
    duration: Duration(milliseconds: durationMs.toInt()),
  );
}

/// Best-effort cleanup for the temporary WAV produced by [extractAsrAudio].
Future<void> deleteAsrAudio(AsrAudio audio) async {
  try {
    final file = File(audio.wavPath);
    if (await file.exists()) await file.delete();
  } catch (_) {
    // A leaked temp file is not worth failing a transcription that succeeded.
  }
}
