import 'dart:ffi';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:tunes4r_player/tunes4r_player.dart';

/// Real YouTube streaming seek test.
///
/// This test requires network access and a working YouTube extractor.
/// It is skipped by default. Run with:
///   flutter test --dart-define=YT_TEST=1 test/yt_stream_seek_test.dart
///
/// You must also have the native library built (make build-macos).

final _enabled = const bool.fromEnvironment('YT_TEST', defaultValue: false);

/// Find the native library.
String? _findDylib() {
  final cwd = Directory.current.path;
  final candidates = [
    '$cwd/../macos/Frameworks/libtunes4r.dylib',
    '$cwd/../rust/target/debug/libtunes4r.dylib',
    '$cwd/../rust/target/release/libtunes4r.dylib',
    '$cwd/macos/Frameworks/libtunes4r.dylib',
  ];
  for (final p in candidates) {
    final f = File(p);
    if (f.existsSync()) return f.absolute.path;
  }
  return null;
}

void main() {
  if (!_enabled) {
    test('YouTube stream seek (skipped — set --dart-define=YT_TEST=1 to run)',
        () => markTestSkipped('Skipped by default'));
    return;
  }

  group('YouTube stream seek', () {
    late Tunes4rFFI ffi;
    late Pointer<Void> handle;

    setUp(() {
      final libPath = _findDylib();
      if (libPath == null) {
        throw 'Native library not found. Build with: make build-macos';
      }
      ffi = Tunes4rFFI();
      if (!ffi.initialize(macOSBundlePath: libPath)) {
        throw 'Failed to initialize FFI: ${ffi.initError}';
      }
      handle = ffi.createEngine();
      if (handle == nullptr) throw 'createEngine returned null';
    });

    tearDown(() {
      ffi.destroyEngine(handle);
    });

    test('buffered seeks are instant after 30 seconds of playback', () async {
      // Play Rick Astley — Never Gonna Give You Up
      const url = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ';
      final rc = ffi.play(handle, url, bufferSizeMs: -1);
      expect(rc, 0, reason: 'play must succeed');

      // Play for 30 seconds to fill buffer
      debugPrint('[yt-test] Playing for 30 seconds to fill buffer...');
      await Future.delayed(const Duration(seconds: 30));

      // Check buffer status
      final buf = ffi.getDownloadBuffer(handle);
      debugPrint(
        '[yt-test] Buffer after 30s: read=${buf.readOffsetMs} write=${buf.writeOffsetMs} total=${buf.totalMs}',
      );
      if (buf.writeOffsetMs < 20000) {
        throw 'Buffer did not reach 20s after 30s playback (reached ${buf.writeOffsetMs}ms).';
      }

      debugPrint('[yt-test] Buffer ready (${buf.writeOffsetMs}ms). Starting seeks...');

      // Perform seeks within the buffered range: 5s, 10s, 20s, 8s, 15s
      // Each seek plays for 5 seconds before next seek
      final targets = [5000, 10000, 20000, 8000, 15000];
      for (final target in targets) {
        final start = DateTime.now();

        expect(ffi.seek(handle, target), 0,
            reason: 'seek to ${target}ms failed');

        // Wait for position to reach within 500ms of target
        int ms = 0;
        for (var i = 0; i < 50; i++) {
          await Future.delayed(const Duration(milliseconds: 50));
          ms = ffi.getPosition(handle).currentMs;
          if ((ms - target).abs() < 500) break;
        }

        final elapsed = DateTime.now().difference(start).inMilliseconds;
        debugPrint(
          '[yt-test] Seek $target ms → position $ms ms (${elapsed}ms)',
        );

        // Seek should be fast (< 2 seconds) since data is cached
        expect((ms - target).abs() < 2000, true,
            reason: 'seek to ${target}ms: position ($ms) too far from target');
        expect(elapsed < 5000, true,
            reason: 'seek to ${target}ms took ${elapsed}ms (expected < 5s)');

        // Play for 5 seconds at this position
        debugPrint('[yt-test] Playing for 5 seconds at ${target}ms...');
        await Future.delayed(const Duration(seconds: 5));
      }

      // Verify events
      var seekEvents = <int>[];
      for (var i = 0; i < 50; i++) {
        final event = ffi.pollEvent(handle);
        if (event.eventType == 0) break;
        if (event.eventType == 2 || event.eventType == 3) {
          seekEvents.add(event.eventType);
        }
      }
      expect(seekEvents.where((t) => t == 2).length, greaterThanOrEqualTo(5),
          reason: 'should have at least 5 SEEK_STARTED events');
      expect(seekEvents.where((t) => t == 3).length, greaterThanOrEqualTo(5),
          reason: 'should have at least 5 SEEK_COMPLETED events');
    }, timeout: const Timeout(Duration(seconds: 180)));
  });
}
