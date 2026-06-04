import 'dart:async';

import 'package:flutter/material.dart';
import 'package:tunes4r_player/tunes4r_player.dart';

void main() {
  runApp(const Tunes4rPlayerExampleApp());
}

class Tunes4rPlayerExampleApp extends StatefulWidget {
  const Tunes4rPlayerExampleApp({super.key});

  @override
  State<Tunes4rPlayerExampleApp> createState() => _Tunes4rPlayerExampleAppState();
}

class _Tunes4rPlayerExampleAppState extends State<Tunes4rPlayerExampleApp> {
  AudioEngine? _engine;
  bool _ready = false;
  String _status = 'Initializing...';
  String _error = '';

  final _uriCtrl = TextEditingController(
    text: 'https://ice1.somafm.com/groovesalad-128-mp3',
  );

  // Seek / position state
  int _positionMs = 0;
  int _durationMs = 0;
  bool _canSeek = false;
  Timer? _positionPoll;

  // Drag state — when the user is scrubbing, the slider should follow
  // the finger, not the position polled from the engine. Otherwise the
  // two would fight and the slider would jitter.
  bool _isDragging = false;
  double _dragValue = 0;

  @override
  void initState() {
    super.initState();
    _init();
  }

  Future<void> _init() async {
    try {
      _engine = await AudioEngine.createWithInit();
      _engine!.stateStream.listen((state) {
        if (!mounted) return;
        setState(() {
          _status = 'State: ${_stateLabel(state)}';
          if (state == PlaybackState.stopped || state == PlaybackState.error) {
            _positionMs = 0;
          }
          // Pull a fresh error message on every state change so the
          // UI shows *why* the engine entered an error state.
          if (state == PlaybackState.error || state == PlaybackState.stopped) {
            _error = _engine?.loadError ?? _engine?.lastError ?? '';
          }
        });
      });
      _startPositionPoll();
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

  void _startPositionPoll() {
    _positionPoll ??= Timer.periodic(const Duration(milliseconds: 200), (_) {
      if (!mounted || _engine == null) return;
      try {
        final newDuration = _engine!.durationMs;
        final newPosition = _engine!.positionMs;
        final newCanSeek = _engine!.canSeek;
        // Also surface any load error that appeared since the last tick.
        final newError = _engine!.loadError ?? _engine!.lastError ?? '';
        if (newDuration != _durationMs ||
            newPosition != _positionMs ||
            newCanSeek != _canSeek ||
            newError != _error) {
          setState(() {
            _durationMs = newDuration;
            _positionMs = newPosition;
            _canSeek = newCanSeek;
            _error = newError;
          });
        }
      } catch (_) {}
    });
  }

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

  void _play() {
    final uri = _uriCtrl.text.trim();
    if (uri.isEmpty) return;
    _engine?.play(uri);
  }

  void _commitSeek(double v) {
    if (!_canSeek || _durationMs <= 0) return;
    _engine?.seek(v.toInt());
  }

  @override
  void dispose() {
    _positionPoll?.cancel();
    _uriCtrl.dispose();
    _engine?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final sliderMax = _durationMs > 0 ? _durationMs.toDouble() : 1.0;
    final sliderValue = _isDragging
        ? _dragValue
        : _positionMs.toDouble().clamp(0.0, sliderMax).toDouble();
    final sliderEnabled = _canSeek && _durationMs > 0;

    return MaterialApp(
      home: Scaffold(
        appBar: AppBar(title: const Text('Tunes4R Player Example')),
        body: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(_status, style: Theme.of(context).textTheme.titleMedium),
              if (_error.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 8),
                  child: SelectableText(
                    _error,
                    style: const TextStyle(color: Colors.red),
                  ),
                ),
              if (_ready) ...[
                const SizedBox(height: 12),
                TextField(
                  controller: _uriCtrl,
                  decoration: const InputDecoration(
                    labelText: 'URI',
                    hintText: 'file:///path/to/song.mp3 or http://…',
                    border: OutlineInputBorder(),
                  ),
                ),
                const SizedBox(height: 8),
                Row(
                  children: [
                    Expanded(
                      child: ElevatedButton(
                        onPressed: _play,
                        child: const Text('Play'),
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      child: ElevatedButton(
                        onPressed: () => _engine?.pause(),
                        child: const Text('Pause'),
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      child: ElevatedButton(
                        onPressed: () => _engine?.resume(),
                        child: const Text('Resume'),
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      child: ElevatedButton(
                        onPressed: () => _engine?.stop(),
                        child: const Text('Stop'),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 16),
                Row(
                  children: [
                    SizedBox(
                      width: 56,
                      child: Text(
                        _formatMs(_positionMs),
                        textAlign: TextAlign.right,
                        style: const TextStyle(fontFeatures: [FontFeature.tabularFigures()]),
                      ),
                    ),
                    Expanded(
                      child: Slider(
                        value: sliderValue,
                        min: 0,
                        max: sliderMax,
                        onChanged: sliderEnabled
                            ? (v) {
                                setState(() {
                                  _isDragging = true;
                                  _dragValue = v;
                                });
                              }
                            : null,
                        onChangeEnd: sliderEnabled
                            ? (v) {
                                _commitSeek(v);
                                setState(() {
                                  _isDragging = false;
                                });
                              }
                            : null,
                      ),
                    ),
                    SizedBox(
                      width: 56,
                      child: Text(_formatMs(_durationMs)),
                    ),
                  ],
                ),
                Text(
                  'canSeek: $_canSeek  ·  duration: ${_durationMs}ms',
                  style: Theme.of(context).textTheme.bodySmall,
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}
