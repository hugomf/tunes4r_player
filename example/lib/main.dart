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
        setState(() => _status = 'State: $state');
      });
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

  @override
  void dispose() {
    _engine?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      home: Scaffold(
        appBar: AppBar(title: const Text('Tunes4R Player Example')),
        body: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Text(_status),
              if (_error.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.all(16),
                  child: SelectableText(
                    _error,
                    style: const TextStyle(color: Colors.red),
                  ),
                ),
              if (_ready) ...[
                ElevatedButton(
                  onPressed: () => _engine?.play(
                    'http://stream.live.vc.bbcmedia.co.uk/bbc_world_service',
                  ),
                  child: const Text('Play Stream'),
                ),
                const SizedBox(height: 8),
                ElevatedButton(
                  onPressed: () => _engine?.pause(),
                  child: const Text('Pause'),
                ),
                const SizedBox(height: 8),
                ElevatedButton(
                  onPressed: () => _engine?.resume(),
                  child: const Text('Resume'),
                ),
                const SizedBox(height: 8),
                ElevatedButton(
                  onPressed: () => _engine?.stop(),
                  child: const Text('Stop'),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}
