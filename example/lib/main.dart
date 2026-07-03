import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:tunes4r_player/tunes4r_player.dart';

import 'progress_slider.dart';

String formatMs(int ms) {
  final s = ms ~/ 1000;
  final m = s ~/ 60;
  final r = s % 60;
  return '$m:${r.toString().padLeft(2, '0')}';
}

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

  // ── Seek slider state ────────────────────────────────────────────────
  bool _isDragging = false;
  DateTime? _lastSeekEnd;
  int _sliderTargetMs = 0;

  int get _displayPositionMs {
    if (_isDragging) return _sliderTargetMs;
    if (_lastSeekEnd != null &&
        DateTime.now().difference(_lastSeekEnd!).inMilliseconds < 500) {
      return _sliderTargetMs;
    }
    return _positionMs;
  }

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
          if (state == PlaybackState.stopped) {
            _activeSource = _SourceType.none;
            _error = _engine?.loadError ?? _error;
          }
          if (state == PlaybackState.finished) {
            _error = _engine?.loadError ?? '';
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
          case EngineEventType.seekStarted:
            setState(() => _lastSeekEvent = 'Seek started: ${pos}ms');
            break;
          case EngineEventType.seekCompleted:
            setState(() => _lastSeekEvent = 'Seek completed: ${pos}ms');
            break;
          case EngineEventType.endOfStream:
            setState(() => _lastSeekEvent = 'End of stream');
            break;
          case EngineEventType.error:
            setState(() => _lastSeekEvent = 'Error: $pos');
            break;
          case EngineEventType.none:
          case EngineEventType.stateChanged:
          case EngineEventType.positionReset:
          case EngineEventType.seekQueued:
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
      });
    } catch (e) {
      setState(() {
        _error = e.toString();
      });
    }
  }

  // ── Seek actions ────────────────────────────────────────────────────

  void _onSeekChange(double v) {
    setState(() {
      _isDragging = true;
      _sliderTargetMs = v.toInt();
    });
  }

  void _onSeekEnd(double v) {
    if (!_canSeek || _durationMs <= 0) return;
    setState(() {
      _isDragging = false;
      _lastSeekEnd = DateTime.now();
      _sliderTargetMs = v.toInt();
    });
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
    final uri = Uri.tryParse(input);
    final url = (uri != null && uri.host.isNotEmpty)
        ? input
        : 'https://www.youtube.com/watch?v=$input';
    _engine!.play(url);
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

  /// Single ProgressSlider shown only when the matching [source] is active.
  Widget _sliderForSource(_SourceType source, {SliderMode? mode}) {
    if (source != _activeSource) return const SizedBox.shrink();
    final m = mode ??
        (source == _SourceType.live ? SliderMode.live : SliderMode.file);
    final isLive = source == _SourceType.live;
    return ProgressSlider(
      positionMs: _displayPositionMs,
      durationMs: _durationMs,
      bufferedToMs:
          isLive ? _buffer.writeOffsetMs : _buffer.endMsClamped,
      bufferCapacityMs: isLive ? _buffer.capacityMs : _buffer.totalMs,
      mode: m,
      enabled: _canSeek && _durationMs > 0,
      onSeekChange: _onSeekChange,
      onSeekEnd: _onSeekEnd,
    );
  }

  /// Row of standard transport buttons: Play, Pause, Resume (optional), Stop.
  Widget _transportRow({
    required VoidCallback? onPlay,
    required VoidCallback? onPause,
    VoidCallback? onResume,
    required VoidCallback? onStop,
  }) {
    return Row(
      children: [
        Expanded(child: ElevatedButton(onPressed: onPlay, child: const Text('Play'))),
        const SizedBox(width: 6),
        Expanded(child: ElevatedButton(onPressed: onPause, child: const Text('Pause'))),
        if (onResume != null) ...[
          const SizedBox(width: 6),
          Expanded(child: ElevatedButton(onPressed: onResume, child: const Text('Resume'))),
        ],
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
            _sliderForSource(_SourceType.file),
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
            _sliderForSource(_SourceType.youtube, mode: SliderMode.streaming),
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
            _transportRow(
              onPlay: _playLive,
              onPause: () => _engine?.pause(),
              onStop: () => _engine?.stop(),
            ),
            const SizedBox(height: 8),
            _sliderForSource(_SourceType.live),
            const SizedBox(height: 4),
            Text(
              'canSeek: $_canSeek  ·  cache: ${formatMs(_durationMs)}',
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
                if (_activeSource == _SourceType.youtube ||
                    _activeSource == _SourceType.live)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 12),
                    child: _StreamStateBadge(state: _currentState),
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

class _StreamStateBadge extends StatefulWidget {
  final PlaybackState state;
  const _StreamStateBadge({required this.state});

  @override
  State<_StreamStateBadge> createState() => _StreamStateBadgeState();
}

class _StreamStateBadgeState extends State<_StreamStateBadge>
    with SingleTickerProviderStateMixin {
  late final AnimationController _ctrl;
  late final Animation<double> _pulse;

  @override
  void initState() {
    super.initState();
    _ctrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 800),
    )..repeat(reverse: true);
    _pulse = CurvedAnimation(parent: _ctrl, curve: Curves.easeInOut);
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  bool get _isPulsing => switch (widget.state) {
    PlaybackState.connecting => true,
    _ => false,
  };

  Color get _color => switch (widget.state) {
    PlaybackState.stopped => Colors.grey,
    PlaybackState.connecting => Colors.amber.shade700,
    PlaybackState.playing => Colors.green.shade600,
    PlaybackState.paused => Colors.orange.shade600,
    PlaybackState.finished => Colors.blue.shade600,
  };

  IconData get _icon => switch (widget.state) {
    PlaybackState.stopped => Icons.stop_circle_outlined,
    PlaybackState.connecting => Icons.wifi_find,
    PlaybackState.playing => Icons.play_circle_fill,
    PlaybackState.paused => Icons.pause_circle_filled,
    PlaybackState.finished => Icons.check_circle_outline,
  };

  @override
  Widget build(BuildContext context) {
    final animating = _isPulsing;

    return AnimatedBuilder(
      animation: _pulse,
      builder: (_, _) {
        final opacity = animating ? 0.6 + 0.4 * _pulse.value : 1.0;
        return Opacity(
          opacity: opacity,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
            decoration: BoxDecoration(
              color: _color.withValues(alpha: 0.12),
              borderRadius: BorderRadius.circular(20),
              border: Border.all(color: _color.withValues(alpha: 0.4)),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(_icon, size: 18, color: _color),
                const SizedBox(width: 8),
                Text(
                  widget.state.name,
                  style: TextStyle(
                    color: _color,
                    fontWeight: FontWeight.w600,
                    fontSize: 14,
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}
