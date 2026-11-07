// Test signal generator: plays sine, pink noise or white noise through the
// audio output of the device the panel runs on (Web Audio in the browser).
// Connect that output to a DSP input to measure the system with the RTA
// (pink noise + Auto-EQ) or to check speakers.
//
// The DSP's own generator (cmd 0x39) is a separate control, see
// DeviceProvider.setTestTone.

import 'package:flutter/foundation.dart';

import 'tone_generator_stub.dart' if (dart.library.js_interop) 'tone_generator_web.dart' as backend;

enum ToneSignal {
  sine('Sine'),
  pink('Pink noise'),
  white('White noise');

  final String label;

  const ToneSignal(this.label);
}

class ToneGenerator extends ChangeNotifier {
  ToneGenerator._();

  /// One generator for the whole panel (there is one audio output).
  static final ToneGenerator instance = ToneGenerator._();

  final backend.ToneBackend _backend = backend.ToneBackend();

  bool get isSupported => _backend.isSupported;

  bool _playing = false;
  bool get isPlaying => _playing;

  ToneSignal _signal = ToneSignal.pink;
  ToneSignal get signal => _signal;

  double _frequency = 1000;
  double get frequency => _frequency;

  /// RMS level in dBFS (-60 … -6).
  double _levelDb = -20;
  double get levelDb => _levelDb;

  static const double minLevelDb = -60;
  static const double maxLevelDb = -6;

  Future<void> start() async {
    if (!isSupported) return;
    try {
      await _backend.start(_signal, _frequency, _levelDb);
      _playing = true;
    } catch (e) {
      debugPrint('ToneGenerator: cannot start audio: $e');
      _playing = false;
    }
    notifyListeners();
  }

  void stop() {
    _backend.stop();
    _playing = false;
    notifyListeners();
  }

  Future<void> toggle() => _playing ? Future.sync(stop) : start();

  Future<void> setSignal(ToneSignal signal) async {
    if (signal == _signal) return;
    _signal = signal;
    if (_playing) await _backend.start(_signal, _frequency, _levelDb);
    notifyListeners();
  }

  void setFrequency(double hz) {
    _frequency = hz.clamp(20.0, 20000.0);
    if (_playing) _backend.setFrequency(_frequency);
    notifyListeners();
  }

  void setLevel(double db) {
    _levelDb = db.clamp(minLevelDb, maxLevelDb);
    if (_playing) _backend.setLevel(_signal, _levelDb);
    notifyListeners();
  }
}
