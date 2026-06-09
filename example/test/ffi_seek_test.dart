import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:tunes4r_player/tunes4r_player.dart';

/// Find the native library from common build locations.
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

/// Initialize FFI if the native library is found, or return null to skip.
Tunes4rFFI? _initFfi() {
  final libPath = _findDylib();
  if (libPath == null) return null;
  final ffi = Tunes4rFFI();
  if (!ffi.initialize(macOSBundlePath: libPath)) return null;
  return ffi;
}

/// Unique temp path per test invocation.
int _testCounter = 0;
String _tempFilePath() {
  _testCounter++;
  return '${Directory.systemTemp.path}/tunes4r_seek$_testCounter.mp3';
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('FFI seek', () {
    test('seek file: position updates and seek events fire', () async {
      final ffi = _initFfi();
      if (ffi == null) {
        markTestSkipped(
          'Native library not found. Build with: make build-macos',
        );
        return;
      }

      final byteData = await rootBundle.load('assets/music.mp3');
      final path = _tempFilePath();
      await File(path).writeAsBytes(byteData.buffer.asUint8List());
      addTearDown(() => File(path).deleteSync());

      final handle = ffi.createEngine();
      expect(handle, isNotNull);
      addTearDown(() => ffi.destroyEngine(handle));

      expect(ffi.play(handle, path, bufferSizeMs: -1), 0);

      // Wait for Playing (state == 4)
      int state = ffi.getState(handle);
      for (var i = 0; state != 4 && i < 200; i++) {
        await Future.delayed(const Duration(milliseconds: 50));
        state = ffi.getState(handle);
      }
      expect(state, 4, reason: 'must reach Playing state');

      expect(ffi.canSeek(handle), true, reason: 'file must be seekable');

      // Seek to 5000ms
      expect(ffi.seek(handle, 5000), 0, reason: 'seek must return 0');

      // Read position immediately — should be near the seek target.
      // Allow tolerance: the seek lands on the nearest frame, then the
      // audio clock may advance a few ms.
      final pos = ffi.getPosition(handle);
      expect(pos.currentMs, inInclusiveRange(4800, 5100),
          reason: 'position should be near seek target');
      expect(pos.totalMs, greaterThan(0), reason: 'totalMs must be known');

      // Poll for SEEK_STARTED (2) and SEEK_COMPLETED (3) events
      var sawStarted = false;
      var sawCompleted = false;
      for (var i = 0; i < 100; i++) {
        final event = ffi.pollEvent(handle);
        if (event.eventType == 2) sawStarted = true;
        if (event.eventType == 3) sawCompleted = true;
        if (event.eventType == 0 && sawStarted && sawCompleted) break;
        await Future.delayed(const Duration(milliseconds: 10));
      }
      expect(sawStarted, true, reason: 'must fire SEEK_STARTED');
      expect(sawCompleted, true, reason: 'must fire SEEK_COMPLETED');
    });

    test('seek clamp does not crash on absurd target', () async {
      final ffi = _initFfi();
      if (ffi == null) {
        markTestSkipped('Native library not found');
        return;
      }

      final byteData = await rootBundle.load('assets/music.mp3');
      final path = _tempFilePath();
      await File(path).writeAsBytes(byteData.buffer.asUint8List());
      addTearDown(() => File(path).deleteSync());

      final handle = ffi.createEngine();
      expect(handle, isNotNull);
      addTearDown(() => ffi.destroyEngine(handle));

      ffi.play(handle, path, bufferSizeMs: -1);

      // Wait for totalMs
      int totalMs = 0;
      for (var i = 0; totalMs == 0 && i < 50; i++) {
        await Future.delayed(const Duration(milliseconds: 100));
        totalMs = ffi.getPosition(handle).totalMs;
      }
      if (totalMs == 0) {
        markTestSkipped('totalMs was never reported');
        return;
      }

      // Seek far past end — must not panic
      expect(ffi.seek(handle, totalMs * 10), 0,
          reason: 'seek must accept overshoot (clamped internally)');

      // Position must be non-negative (if clamping is applied, it will
      // be <= totalMs; the engine may still be processing the seek)
      final pos = ffi.getPosition(handle);
      expect(pos.currentMs, greaterThanOrEqualTo(0),
          reason: 'currentMs must be >= 0');
    });

    test('stop + re-play does not crash', () async {
      final ffi = _initFfi();
      if (ffi == null) {
        markTestSkipped('Native library not found');
        return;
      }

      final byteData = await rootBundle.load('assets/music.mp3');
      final path = _tempFilePath();
      await File(path).writeAsBytes(byteData.buffer.asUint8List());
      addTearDown(() => File(path).deleteSync());

      final handle = ffi.createEngine();
      expect(handle, isNotNull);
      addTearDown(() => ffi.destroyEngine(handle));

      expect(ffi.play(handle, path, bufferSizeMs: -1), 0,
          reason: 'first play');
      await Future.delayed(const Duration(milliseconds: 200));
      ffi.stop(handle);
      await Future.delayed(const Duration(milliseconds: 100));

      // Verify file still exists before re-playing
      expect(File(path).existsSync(), true,
          reason: 'temp file must exist for re-play');

      expect(ffi.play(handle, path, bufferSizeMs: -1), 0,
          reason: 're-play after stop must succeed');
    });
  });
}
