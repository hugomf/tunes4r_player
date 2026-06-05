import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:tunes4r_player/tunes4r_player.dart';

void main() {
  runApp(const Tunes4rPlayerExampleApp());
}

class Tunes4rPlayerExampleApp extends StatefulWidget {
  const Tunes4rPlayerExampleApp({super.key});

  @override
  State<Tunes4rPlayerExampleApp> createState() =>
      _Tunes4rPlayerExampleAppState();
}

enum _SourceType { none, file, youtube, live }

class _Tunes4rPlayerExampleAppState extends State<Tunes4rPlayerExampleApp> {
  AudioEngine? _engine;
  bool _ready = false;
  String _status = 'Initializing...';
  String _error = '';
  String _lastSeekEvent = '';

  // ── Active source ────────────────────────────────────────────────────
  _SourceType _activeSource = _SourceType.none;

  // ── Engine state ─────────────────────────────────────────────────────
  PlaybackState _currentState = PlaybackState.stopped;
  int _positionMs = 0;
  int _durationMs = 0;
  bool _canSeek = false;

  // ── Buffer state ─────────────────────────────────────────────────────
  AdaptiveRingBuffer _buffer = const AdaptiveRingBuffer(
    capacityMs: 0,
    readOffsetMs: 0,
    writeOffsetMs: 0,
    totalMs: 0,
    isComplete: false,
  );

  // ── Current source tracking ──────────────────────────────────────────
  String _filePath = '';
  final _ytController = TextEditingController(text: 'dQw4w9WgXcQ');
  final _liveController = TextEditingController(
    text:
        'https://wdr-1live-live.icecastssl.wdr.de/wdr/1live/live/mp3/128/stream.mp3',
  );

  StreamSubscription<PlaybackState>? _stateSub;
  StreamSubscription<PlaybackPosition>? _positionSub;
  StreamSubscription<EngineEvent>? _eventSub;
  StreamSubscription<AdaptiveRingBuffer>? _bufferSub;

  @override
  void initState() {
    super.initState();
    _init();
  }

  Future<void> _init() async {
    try {
      _engine = await AudioEngine.createWithInit();

      _stateSub = _engine!.stateStream.listen((state) {
        if (!mounted) return;
        setState(() {
          _currentState = state;
          _status = 'State: ${_stateLabel(state)}';
          if (state == PlaybackState.stopped) {
            _activeSource = _SourceType.none;
          }
          if (state == PlaybackState.error || state == PlaybackState.stopped) {
            _error = _engine?.loadError ?? _engine?.lastError ?? '';
          }
        });
      });

      _positionSub = _engine!.positionStream.listen((p) {
        if (!mounted) return;
        setState(() {
          _positionMs = p.currentMs;
          _durationMs = p.totalMs;
          _canSeek = _engine?.canSeek ?? false;
        });
      });

      _bufferSub = _engine!.downloadBufferStream.listen((b) {
        if (!mounted) return;
        setState(() => _buffer = b);
      });

      _eventSub = _engine!.playbackEventStream.listen((event) {
        if (!mounted) return;
        final pos = event.intParam;
        switch (event.eventType) {
          case engineEventSeekStarted:
            setState(() => _lastSeekEvent = 'Seek started: ${pos}ms');
            break;
          case engineEventSeekCompleted:
            setState(() => _lastSeekEvent = 'Seek completed: ${pos}ms');
            break;
          case engineEventEndOfStream:
            setState(() => _lastSeekEvent = 'End of stream');
            break;
          case engineEventError:
            setState(() => _lastSeekEvent = 'Error: $pos');
            break;
        }
      });

      // Extract bundled asset so the file section has something to play.
      final byteData = await rootBundle.load('assets/music.mp3');
      final tempDir = Directory.systemTemp;
      final tempFile = File('${tempDir.path}/music.mp3');
      await tempFile.writeAsBytes(byteData.buffer.asUint8List());
      _filePath = tempFile.path;

      setState(() {
        _ready = true;
        _status = 'Engine ready';
      });
    } catch (e) {
      setState(() {
        _error = e.toString();
        _status = 'Init failed';
      });
    }
  }

  // ── Helpers ──────────────────────────────────────────────────────────

  String _stateLabel(PlaybackState s) {
    switch (s) {
      case PlaybackState.stopped:
        return 'stopped';
      case PlaybackState.connecting:
        return 'connecting';
      case PlaybackState.buffering:
        return 'buffering';
      case PlaybackState.decoding:
        return 'decoding';
      case PlaybackState.playing:
        return 'playing';
      case PlaybackState.paused:
        return 'paused';
      case PlaybackState.error:
        return 'error';
    }
  }

  String _formatMs(int ms) {
    final s = ms ~/ 1000;
    final m = s ~/ 60;
    final r = s % 60;
    return '$m:${r.toString().padLeft(2, '0')}';
  }

  void _commitSeek(double v) {
    if (!_canSeek || _durationMs <= 0) return;
    _engine?.seek(v.toInt());
  }

  // ── Section actions ──────────────────────────────────────────────────

  void _playFile() {
    if (_engine == null || _filePath.isEmpty) return;
    _engine!.play(_filePath);
    setState(() => _activeSource = _SourceType.file);
  }

  Future<void> _playYoutube() async {
    if (_engine == null) return;
    final input = _ytController.text.trim();
    if (input.isEmpty) return;
    final uri = input.contains('youtu')
        ? input
        : 'https://www.youtube.com/watch?v=$input';
    _engine!.play(uri);
    setState(() => _activeSource = _SourceType.youtube);
  }

  void _playLive() {
    if (_engine == null) return;
    final url = _liveController.text.trim();
    if (url.isEmpty) return;
    _engine!.playLive(url, cacheMaxMs: 30 * 60 * 1000);
    setState(() => _activeSource = _SourceType.live);
  }

  // ── Buffered seek slider ─────────────────────────────────────────────

  /// Slider that shows buffer progress and allows seeking.
  /// Only renders the seek bar when [source] is the active source.
  Widget _bufferedSlider(_SourceType source) {
    final isActive = source == _activeSource;
    final isLive = source == _SourceType.live;
    return _BufferedSlider(
      isActive: isActive,
      isLive: isLive,
      positionMs: _positionMs,
      durationMs: _durationMs,
      canSeek: _canSeek,
      buffer: _buffer,
      onSeek: _commitSeek,
    );
  }

  /// Row of standard transport buttons: Play, Pause, Resume, Stop.
  Widget _transportRow({
    required VoidCallback? onPlay,
    required VoidCallback? onPause,
    required VoidCallback? onResume,
    required VoidCallback? onStop,
  }) {
    return Row(
      children: [
        Expanded(child: ElevatedButton(onPressed: onPlay, child: const Text('Play'))),
        const SizedBox(width: 6),
        Expanded(child: ElevatedButton(onPressed: onPause, child: const Text('Pause'))),
        const SizedBox(width: 6),
        Expanded(child: ElevatedButton(onPressed: onResume, child: const Text('Resume'))),
        const SizedBox(width: 6),
        Expanded(child: ElevatedButton(onPressed: onStop, child: const Text('Stop'))),
      ],
    );
  }

  // ── Sections ─────────────────────────────────────────────────────────

  Widget _fileSection() {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                const Icon(Icons.audio_file, size: 20),
                const SizedBox(width: 8),
                Text('Audio File', style: Theme.of(context).textTheme.titleMedium),
              ],
            ),
            const SizedBox(height: 4),
            Text(_filePath.isNotEmpty ? _filePath.split('/').last : 'No file loaded',
                style: Theme.of(context).textTheme.bodySmall),
            const SizedBox(height: 8),
            _transportRow(
              onPlay: _playFile,
              onPause: () => _engine?.pause(),
              onResume: () => _engine?.resume(),
              onStop: () => _engine?.stop(),
            ),
            const SizedBox(height: 8),
            _bufferedSlider(_SourceType.file),
          ],
        ),
      ),
    );
  }

  Widget _youtubeSection() {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                const Icon(Icons.play_circle_fill, size: 20, color: Colors.red),
                const SizedBox(width: 8),
                Text('YouTube Stream', style: Theme.of(context).textTheme.titleMedium),
              ],
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _ytController,
                    decoration: const InputDecoration(
                      hintText: 'Video ID or URL',
                      border: OutlineInputBorder(),
                      isDense: true,
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                ElevatedButton.icon(
                  onPressed: _playYoutube,
                  icon: const Icon(Icons.play_arrow, size: 18),
                  label: const Text('Play'),
                ),
              ],
            ),
            const SizedBox(height: 8),
            _transportRow(
              onPlay: _playYoutube,
              onPause: () => _engine?.pause(),
              onResume: () => _engine?.resume(),
              onStop: () => _engine?.stop(),
            ),
            const SizedBox(height: 8),
            _bufferedSlider(_SourceType.youtube),
          ],
        ),
      ),
    );
  }

  Widget _liveSection() {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                const Icon(Icons.live_tv, size: 20, color: Colors.green),
                const SizedBox(width: 8),
                Text('Live Stream', style: Theme.of(context).textTheme.titleMedium),
              ],
            ),
            const SizedBox(height: 4),
            Text('30 min ring buffer — seek backward within cached window',
                style: Theme.of(context).textTheme.bodySmall),
            const SizedBox(height: 8),
            Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _liveController,
                    decoration: const InputDecoration(
                      hintText: 'Stream URL',
                      border: OutlineInputBorder(),
                      isDense: true,
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                ElevatedButton.icon(
                  onPressed: _playLive,
                  icon: const Icon(Icons.play_arrow, size: 18),
                  label: const Text('Play'),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                Expanded(child: ElevatedButton(onPressed: _playLive, child: const Text('Play'))),
                const SizedBox(width: 6),
                Expanded(child: ElevatedButton(onPressed: () => _engine?.pause(), child: const Text('Pause'))),
                const SizedBox(width: 6),
                Expanded(child: ElevatedButton(onPressed: () => _engine?.stop(), child: const Text('Stop'))),
              ],
            ),
            const SizedBox(height: 8),
            _bufferedSlider(_SourceType.live),
            const SizedBox(height: 4),
            Text(
              'canSeek: $_canSeek  ·  cache: ${_formatMs(_durationMs)}',
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      home: Scaffold(
        appBar: AppBar(title: const Text('Tunes4R Player Example')),
        body: Padding(
          padding: const EdgeInsets.all(16),
          child: SingleChildScrollView(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(_status, style: Theme.of(context).textTheme.titleMedium),
                if (_currentState == PlaybackState.connecting ||
                    _currentState == PlaybackState.buffering)
                  Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: LinearProgressIndicator(
                      backgroundColor: Colors.grey.shade300,
                    ),
                  ),
                if (_error.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 8),
                    child: SelectableText(
                      _error,
                      style: const TextStyle(color: Colors.red),
                    ),
                  ),
                if (_lastSeekEvent.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 8),
                    child: Text(
                      _lastSeekEvent,
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(color: Colors.blueGrey),
                    ),
                  ),
                if (_ready) ...[
                  const SizedBox(height: 8),
                  _fileSection(),
                  const SizedBox(height: 12),
                  _youtubeSection(),
                  const SizedBox(height: 12),
                  _liveSection(),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }

  @override
  void dispose() {
    _stateSub?.cancel();
    _positionSub?.cancel();
    _bufferSub?.cancel();
    _eventSub?.cancel();
    _ytController.dispose();
    _liveController.dispose();
    _engine?.dispose();
    super.dispose();
  }
}

// ── BufferedSlider widget ─────────────────────────────────────────────────

/// Seek slider that shows buffer progress and allows dragging/tapping to seek.
/// Extracted from the inline `_bufferedSlider` method to decouple from
/// `_activeSource` and remove SRP violation (ARCH-6).
class _BufferedSlider extends StatefulWidget {
  final bool isActive;
  final bool isLive;
  final int positionMs;
  final int durationMs;
  final bool canSeek;
  final AdaptiveRingBuffer buffer;
  final void Function(double ms) onSeek;

  const _BufferedSlider({
    required this.isActive,
    required this.isLive,
    required this.positionMs,
    required this.durationMs,
    required this.canSeek,
    required this.buffer,
    required this.onSeek,
  });

  @override
  State<_BufferedSlider> createState() => _BufferedSliderState();
}

class _BufferedSliderState extends State<_BufferedSlider> {
  bool _isDragging = false;
  double _dragValue = 0;

  String _formatMs(int ms) {
    final s = ms ~/ 1000;
    final m = s ~/ 60;
    final r = s % 60;
    return '$m:${r.toString().padLeft(2, '0')}';
  }

  @override
  Widget build(BuildContext context) {
    final total = widget.isActive && widget.durationMs > 0
        ? widget.durationMs.toDouble()
        : 1.0;
    final maxSeek = widget.isLive
        ? widget.buffer.writeOffsetMs.toDouble().clamp(0.0, total)
        : total;
    final value = widget.isActive
        ? (_isDragging
            ? _dragValue.clamp(0.0, maxSeek)
            : widget.positionMs.toDouble().clamp(0.0, total))
        : 0.0;
    final enabled = widget.isActive && widget.canSeek && widget.durationMs > 0;

    return SizedBox(
      height: 48,
      child: Row(
        children: [
          SizedBox(
            width: 52,
            child: Text(
              _formatMs(widget.positionMs),
              textAlign: TextAlign.right,
              style: const TextStyle(
                fontFeatures: [FontFeature.tabularFigures()],
                fontSize: 13,
              ),
            ),
          ),
          const SizedBox(width: 4),
          Expanded(
            child: LayoutBuilder(
              builder: (context, constraints) {
                final sliderWidth = constraints.maxWidth;
                return GestureDetector(
                  onTapDown: enabled
                      ? (d) {
                          final pos = (d.localPosition.dx / sliderWidth * total)
                              .clamp(0.0, maxSeek);
                          widget.onSeek(pos);
                        }
                      : null,
                  onHorizontalDragUpdate: enabled
                      ? (d) {
                          final raw = d.localPosition.dx / sliderWidth * total;
                          setState(() => _dragValue = raw.clamp(0.0, maxSeek));
                          _isDragging = true;
                        }
                      : null,
                  onHorizontalDragEnd: enabled
                      ? (_) {
                          widget.onSeek(_dragValue);
                          setState(() => _isDragging = false);
                        }
                      : null,
                  child: CustomPaint(
                    size: Size(double.infinity, 32),
                    painter: _BufferedSliderPainter(
                      position: value,
                      total: total,
                      bufferWrite: widget.buffer.writeOffsetMs.toDouble(),
                      bufferTotal: widget.buffer.totalMs.toDouble(),
                      enabled: enabled,
                    ),
                  ),
                );
              },
            ),
          ),
          const SizedBox(width: 4),
          SizedBox(
            width: 52,
            child: Text(
              _formatMs(widget.durationMs),
              style: const TextStyle(fontSize: 13),
            ),
          ),
        ],
      ),
    );
  }
}

// ── BufferedSliderPainter ────────────────────────────────────────────────

/// Custom painter that draws a buffered-progress slider bar.
class _BufferedSliderPainter extends CustomPainter {
  final double position;
  final double total;
  final double bufferWrite;
  final double bufferTotal;
  final bool enabled;

  _BufferedSliderPainter({
    required this.position,
    required this.total,
    required this.bufferWrite,
    required this.bufferTotal,
    required this.enabled,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final h = size.height;
    final w = size.width;
    final trackHeight = 8.0;
    final trackY = (h - trackHeight) / 2;
    final fraction = total > 0 ? (position / total).clamp(0.0, 1.0) : 0.0;
    final posX = fraction * w;
    final cornerRadius = const Radius.circular(4);

    // 1. Background — unbuffered (dark gray)
    canvas.drawRRect(
      RRect.fromRectAndRadius(Rect.fromLTWH(0, trackY, w, trackHeight), cornerRadius),
      Paint()..color = Colors.grey.shade700,
    );

    // 2. Buffered region — cyan fill + diagonal hatch stripes
    if (bufferTotal > 0) {
      final bufFraction = (bufferWrite / bufferTotal).clamp(0.0, 1.0);
      final bufW = w * bufFraction;
      if (bufW > 0) {
        // Light cyan fill
        canvas.drawRRect(
          RRect.fromRectAndRadius(Rect.fromLTWH(0, trackY, bufW, trackHeight), cornerRadius),
          Paint()..color = Colors.cyan.withValues(alpha: 0.15),
        );
        // Diagonal hatch stripes
        canvas.save();
        canvas.clipRect(Rect.fromLTWH(0, trackY, bufW, trackHeight));
        final stripePaint = Paint()
          ..color = Colors.cyan.withValues(alpha: 0.5)
          ..strokeWidth = 1.5;
        for (double x = -trackHeight; x < bufW + trackHeight; x += 6) {
          canvas.drawLine(Offset(x, trackY + trackHeight), Offset(x + 6, trackY), stripePaint);
        }
        canvas.restore();
      }
    }

    // 3. Played region — solid blue fill (overrides buffer)
    if (posX > 0) {
      final playedRect = Rect.fromLTWH(0, trackY, posX, trackHeight);
      // Rounded only on left side; right edge stays square
      final playedRRect = RRect.fromRectAndCorners(
        playedRect,
        topLeft: const Radius.circular(4),
        bottomLeft: const Radius.circular(4),
      );
      canvas.drawRRect(playedRRect, Paint()..color = Colors.blue);
    }

    // 4. Thumb handle — diamond
    if (enabled && w > 0) {
      final thumbSize = 7.0;
      final cy = trackY + trackHeight / 2;
      final path = Path()
        ..moveTo(posX, cy - thumbSize)            // top
        ..lineTo(posX + thumbSize * 0.7, cy)      // right
        ..lineTo(posX, cy + thumbSize)            // bottom
        ..lineTo(posX - thumbSize * 0.7, cy)      // left
        ..close();
      canvas.drawPath(path, Paint()..color = Colors.white);
      canvas.drawPath(
        path,
        Paint()
          ..color = Colors.blue.shade800
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.5,
      );
    }
  }

  @override
  bool shouldRepaint(_BufferedSliderPainter old) =>
      old.position != position ||
      old.total != total ||
      old.bufferWrite != bufferWrite ||
      old.bufferTotal != bufferTotal ||
      old.enabled != enabled;
}
