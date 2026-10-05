import 'dart:convert';

import 'package:tunes4r_player/src/tunes4r_player_ffi.dart';

/// A Chromaprint fingerprint of an audio file, plus the length of the excerpt
/// that was analysed.
///
/// [duration] is what AcoustID must be told the fingerprint covers. It is the
/// decoded excerpt length, not the file's full length — sending the full track
/// duration against a partial fingerprint is a silent scoring error.
class AudioFingerprint {
  final String fingerprint;
  final int durationSeconds;

  const AudioFingerprint({
    required this.fingerprint,
    required this.durationSeconds,
  });
}

/// Fingerprint the audio file at [path] without instantiating a player.
///
/// This deliberately does not live on [AudioEngine]: identifying a file already
/// on disk needs no decode pipeline, no audio output and no engine handle, and
/// the metadata context that wants this must not take a dependency on the
/// playback context just to hash some bytes.
///
/// Returns null when the native library is unavailable, was built without the
/// `fingerprint` cargo feature, or the file could not be decoded — all of which
/// mean "identification unavailable", not "this song is unknown".
///
/// Initializes the shared library if the player has not already done so. That
/// initializer is idempotent once it succeeds, but it has no re-entrancy guard,
/// so a call arriving while the player is mid-initialization can repeat the
/// bind-and-verify step. The outcome is still correct — only a throwaway engine
/// handle is created and destroyed — which is why this does not coordinate with
/// the player rather than failing. Identification is called from repair jobs,
/// which run after playback has started.
Future<AudioFingerprint?> fingerprintAudioFile(
  String path, {
  int maxSeconds = 0,
}) async {
  final ffi = tunes4rFFI;
  if (!ffi.initialize()) return null;

  final raw = ffi.fingerprintFile(path, maxSeconds: maxSeconds);
  if (raw == null) return null;

  final Map<String, dynamic> decoded;
  try {
    decoded = jsonDecode(raw) as Map<String, dynamic>;
  } on FormatException {
    return null;
  }

  final error = decoded['error'];
  if (error != null) return null;

  final fingerprint = decoded['fingerprint'];
  final duration = decoded['duration'];
  if (fingerprint is! String || fingerprint.isEmpty) return null;
  if (duration is! num || duration <= 0) return null;

  return AudioFingerprint(
    fingerprint: fingerprint,
    durationSeconds: duration.toInt(),
  );
}