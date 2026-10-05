import 'package:flutter_test/flutter_test.dart';
import 'package:tunes4r_player/src/audio_engine.dart';

/// Regression: `AudioEngine.seek()` used to seed [SeekClock] with the requested
/// position *before* the engine attempted the seek, so the interpolated playhead
/// — the position the UI renders — jumped to the target even when the engine
/// dropped the seek. With a dropped seek the slider stayed at the target while
/// the audio played on from the old spot.
///
/// The clock is now driven only by real engine polls, which is what keeps the
/// rendered position equal to where the audio actually is.
void main() {
  group('SeekClock', () {
    test('advances only from real sync pulses, never from a seek request', () {
      final clock = SeekClock();
      clock.onPositionUpdate(30_000, 180_000);
      clock.setPlaying(true);

      // A seek request no longer feeds the clock, so a target far from the
      // playhead cannot move the reported position.
      expect(clock.displayPositionMs, 30_000);
    });

    test('clamps the reported position into [0, duration]', () {
      final clock = SeekClock();
      clock.onPositionUpdate(30_000, 180_000);
      clock.setPlaying(true);

      // Whatever the engine reports, the rendered playhead stays inside the
      // track so the remaining-time label can never render a negative value.
      expect(clock.displayPositionMs, inInclusiveRange(0, 180_000));

      clock.onPositionUpdate(179_000, 180_000);
      expect(clock.displayPositionMs, inInclusiveRange(0, 180_000));
    });

    test('reports the last real position when paused', () {
      final clock = SeekClock();
      clock.onPositionUpdate(12_000, 180_000);
      clock.setPlaying(false);
      expect(clock.displayPositionMs, 12_000);
    });
  });

  group('isPositionAcceptableAfterSeek', () {
    test('accepts position when no seek target is set', () {
      expect(isPositionAcceptableAfterSeek(1000, null), isTrue);
      expect(isPositionAcceptableAfterSeek(0, null), isTrue);
    });

    test('accepts position within tolerance of seek target', () {
      expect(isPositionAcceptableAfterSeek(1000, 1000), isTrue);
      expect(isPositionAcceptableAfterSeek(1001, 1000), isTrue);
      expect(isPositionAcceptableAfterSeek(500, 1000), isTrue);
      expect(isPositionAcceptableAfterSeek(1499, 1000), isTrue);
    });

    test('rejects position far from seek target (forward)', () {
      expect(isPositionAcceptableAfterSeek(2000, 1000), isFalse);
      expect(isPositionAcceptableAfterSeek(3000, 1000), isFalse);
    });

    test('rejects position far from seek target (backward)', () {
      expect(isPositionAcceptableAfterSeek(0, 1000), isFalse);
      expect(isPositionAcceptableAfterSeek(500, 2000), isFalse);
    });

    test('boundary: exactly at tolerance', () {
      expect(isPositionAcceptableAfterSeek(500, 1000), isTrue);
      expect(isPositionAcceptableAfterSeek(1500, 1000), isTrue);
      expect(isPositionAcceptableAfterSeek(501, 1000), isTrue);
      expect(isPositionAcceptableAfterSeek(1499, 1000), isTrue);
      expect(isPositionAcceptableAfterSeek(499, 1000), isFalse);
      expect(isPositionAcceptableAfterSeek(1501, 1000), isFalse);
    });
  });
}
