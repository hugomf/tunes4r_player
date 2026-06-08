import 'package:flutter_test/flutter_test.dart';
import 'package:tunes4r_player/tunes4r_player.dart';

void main() {
  // ===========================================================================
  // PlaybackState
  // ===========================================================================

  group('PlaybackState', () {
    group('fromValue', () {
      test('returns correct enum for each known value', () {
        expect(PlaybackState.fromValue(0), PlaybackState.stopped);
        expect(PlaybackState.fromValue(1), PlaybackState.connecting);
        expect(PlaybackState.fromValue(2), PlaybackState.buffering);
        expect(PlaybackState.fromValue(3), PlaybackState.decoding);
        expect(PlaybackState.fromValue(4), PlaybackState.playing);
        expect(PlaybackState.fromValue(5), PlaybackState.paused);
        expect(PlaybackState.fromValue(6), PlaybackState.error);
      });

      test('returns error for unknown value', () {
        expect(PlaybackState.fromValue(-1), PlaybackState.error);
        expect(PlaybackState.fromValue(99), PlaybackState.error);
      });

      test('returns error for null-equivalent edge cases', () {
        expect(PlaybackState.fromValue(7), PlaybackState.error);
        expect(PlaybackState.fromValue(-100), PlaybackState.error);
      });
    });
  });

  // ===========================================================================
  // EngineConfig
  // ===========================================================================

  group('EngineConfig', () {
    test('has default spectrumBandCount of 16', () {
      const config = EngineConfig();
      expect(config.spectrumBandCount, 16);
    });

    test('accepts custom spectrumBandCount', () {
      const config = EngineConfig(spectrumBandCount: 32);
      expect(config.spectrumBandCount, 32);
    });
  });

  // ===========================================================================
  // AdaptiveRingBuffer
  // ===========================================================================

  group('AdaptiveRingBuffer', () {
    group('empty()', () {
      test('creates an empty ring buffer with default capacity', () {
        const buf = AdaptiveRingBuffer.empty();
        expect(buf.capacityMs, 30000);
        expect(buf.readOffsetMs, 0);
        expect(buf.writeOffsetMs, 0);
        expect(buf.totalMs, 0);
        expect(buf.isComplete, false);
      });
    });

    group('availableMs', () {
      test('returns 0 for empty buffer', () {
        const buf = AdaptiveRingBuffer.empty();
        expect(buf.availableMs, 0);
      });

      test('returns 0 when filled is 0', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 5000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.availableMs, 0);
      });

      test('returns 0 when filled is negative (write behind read)', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 10000,
          writeOffsetMs: 5000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.availableMs, 0);
      });

      test('returns the filled amount when under capacity', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 0,
          writeOffsetMs: 15000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.availableMs, 15000);
      });

      test('returns capacity when filled exceeds capacity', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 0,
          writeOffsetMs: 60000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.availableMs, 30000);
      });

      test('returns remainder when file is complete and totalMs > 0', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 10000,
          writeOffsetMs: 120000,
          totalMs: 120000,
          isComplete: true,
        );
        // totalMs - readOffsetMs = 120000 - 10000 = 110000
        expect(buf.availableMs, 110000);
      });

      test('clamps remainder to 0 when readOffset exceeds totalMs (complete)',
          () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 200000,
          writeOffsetMs: 120000,
          totalMs: 120000,
          isComplete: true,
        );
        expect(buf.availableMs, 0);
      });

      test('clamps remainder to i32 max (safety guard)', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 0,
          writeOffsetMs: 120000,
          totalMs: 120000,
          isComplete: true,
        );
        // totalMs - readOffsetMs = 120000, clamp(0, i32::MAX) = 120000
        // Avoid sign overflow check: 120000 < 2^31
        expect(buf.availableMs, 120000);
      });
    });

    group('endMs', () {
      test('returns readOffsetMs + availableMs', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 25000,
          totalMs: 0,
          isComplete: false,
        );
        // available = 25000 - 5000 = 20000, end = 5000 + 20000 = 25000
        expect(buf.endMs, 25000);
      });

      test('returns readOffsetMs when available is 0', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 3000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.endMs, 5000);
      });
    });

    group('endMsClamped', () {
      test('returns endMs when it exceeds readOffsetMs', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 25000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.endMsClamped, buf.endMs);
      });

      test('returns readOffsetMs when endMs equals readOffsetMs', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 3000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.endMsClamped, 5000);
      });
    });

    group('isFullyBuffered', () {
      test('returns true when isComplete is true', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 0,
          writeOffsetMs: 120000,
          totalMs: 120000,
          isComplete: true,
        );
        expect(buf.isFullyBuffered, true);
      });

      test('returns true when writeOffsetMs >= totalMs and totalMs > 0', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 0,
          writeOffsetMs: 120000,
          totalMs: 120000,
          isComplete: false,
        );
        expect(buf.isFullyBuffered, true);
      });

      test('returns false when totalMs is 0 (unknown size)', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 0,
          writeOffsetMs: 5000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.isFullyBuffered, false);
      });

      test('returns false when writeOffsetMs < totalMs', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 0,
          writeOffsetMs: 50000,
          totalMs: 120000,
          isComplete: false,
        );
        expect(buf.isFullyBuffered, false);
      });
    });

    group('contains', () {
      test('returns true for positions within [readOffsetMs, endMs]', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 25000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.contains(5000), true);
        expect(buf.contains(15000), true);
        expect(buf.contains(25000), true);
      });

      test('returns false for positions before readOffsetMs', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 25000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.contains(0), false);
        expect(buf.contains(4999), false);
      });

      test('returns false for positions after endMs', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 25000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.contains(25001), false);
        expect(buf.contains(99999), false);
      });

      test('boundary: endMs is readOffsetMs when available is 0', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 3000,
          totalMs: 0,
          isComplete: false,
        );
        expect(buf.contains(5000), true);
        expect(buf.contains(4999), false);
        expect(buf.contains(5001), false);
      });
    });

    group('toString', () {
      test('produces a readable representation', () {
        const buf = AdaptiveRingBuffer(
          capacityMs: 30000,
          readOffsetMs: 5000,
          writeOffsetMs: 25000,
          totalMs: 120000,
          isComplete: false,
        );
        final str = buf.toString();
        expect(str, contains('playhead=5000ms'));
        expect(str, contains('end=25000ms'));
        expect(str, contains('total=120000ms'));
        expect(str, contains('available=20000ms'));
        expect(str, contains('complete=false'));
      });
    });
  });

  // ===========================================================================
  // Tunes4rErrorCode extension
  // ===========================================================================

  group('Tunes4rErrorCode', () {
    test('constants match expected values', () {
      expect(Tunes4rErrorCode.ffiSuccess, 0);
      expect(Tunes4rErrorCode.ffiNullHandleOrUri, -1);
      expect(Tunes4rErrorCode.ffiInvalidUtf8, -2);
      expect(Tunes4rErrorCode.ffiEngineLockError, -3);
      expect(Tunes4rErrorCode.ffiPlaybackError, -4);
      expect(Tunes4rErrorCode.ffiInternalPanic, -99);
    });

    test('isFfiError returns true for non-zero codes', () {
      expect(0.isFfiError, false);
      expect((-1).isFfiError, true);
      expect((-99).isFfiError, true);
    });

    test('ffiErrorMessage returns descriptive strings', () {
      expect(0.ffiErrorMessage, 'Success');
      expect((-1).ffiErrorMessage, 'Null handle or URI');
      expect((-2).ffiErrorMessage, 'Invalid UTF-8 string');
      expect((-99).ffiErrorMessage, 'Internal panic in native engine');
      expect(42.ffiErrorMessage, 'Unknown FFI error (code: 42)');
    });
  });

  // ===========================================================================
  // EngineEventType
  // ===========================================================================

  group('EngineEventType', () {
    test('fromValue returns correct enum for each known value', () {
      expect(EngineEventType.fromValue(0), EngineEventType.none);
      expect(EngineEventType.fromValue(1), EngineEventType.stateChanged);
      expect(EngineEventType.fromValue(2), EngineEventType.seekStarted);
      expect(EngineEventType.fromValue(3), EngineEventType.seekCompleted);
      expect(EngineEventType.fromValue(4), EngineEventType.endOfStream);
      expect(EngineEventType.fromValue(5), EngineEventType.positionReset);
      expect(EngineEventType.fromValue(6), EngineEventType.error);
      expect(EngineEventType.fromValue(7), EngineEventType.seekQueued);
    });

    test('fromValue returns none for unknown value', () {
      expect(EngineEventType.fromValue(-1), EngineEventType.none);
      expect(EngineEventType.fromValue(99), EngineEventType.none);
    });

    test('value matches expected int', () {
      expect(EngineEventType.none.value, 0);
      expect(EngineEventType.stateChanged.value, 1);
      expect(EngineEventType.seekStarted.value, 2);
      expect(EngineEventType.seekCompleted.value, 3);
      expect(EngineEventType.endOfStream.value, 4);
      expect(EngineEventType.positionReset.value, 5);
      expect(EngineEventType.error.value, 6);
      expect(EngineEventType.seekQueued.value, 7);
    });
  });

  // ===========================================================================
  // EngineEvent
  // ===========================================================================

  group('EngineEvent', () {
    test('constructs with typed eventType and intParam', () {
      final event = EngineEvent(
        eventType: EngineEventType.seekCompleted,
        intParam: 5000,
      );
      expect(event.eventType, EngineEventType.seekCompleted);
      expect(event.intParam, 5000);
    });
  });

  // ===========================================================================
  // Tunes4rEngineException
  // ===========================================================================

  group('Tunes4rEngineException', () {
    test('toString includes message', () {
      final e = Tunes4rEngineException('something broke');
      expect(e.toString(), 'Tunes4rEngineException: something broke');
    });

    test('toString includes error code when set', () {
      final e = Tunes4rEngineException('engine failed', errorCode: -1);
      expect(
        e.toString(),
        'Tunes4rEngineException: engine failed (code: -1)',
      );
    });
  });
}
