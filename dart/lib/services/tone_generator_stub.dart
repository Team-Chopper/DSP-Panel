// Tone generator backend outside a browser (tests): not supported.

import 'tone_generator.dart';

class ToneBackend {
  bool get isSupported => false;

  Future<void> start(ToneSignal signal, double frequency, double levelDb) async {}

  void setFrequency(double hz) {}

  void setLevel(ToneSignal signal, double levelDb) {}

  void stop() {}
}
