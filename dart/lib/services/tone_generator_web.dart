// Web Audio backend of the tone generator.

import 'dart:js_interop';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:web/web.dart' as web;

import 'tone_generator.dart';

class ToneBackend {
  web.AudioContext? _context;
  web.GainNode? _gain;
  web.AudioScheduledSourceNode? _source;
  web.OscillatorNode? _oscillator;
  final Map<ToneSignal, web.AudioBuffer> _noise = {};

  bool get isSupported => true;

  web.AudioContext get _ctx => _context ??= web.AudioContext();

  /// Linear gain for an RMS level: noise buffers are normalised to RMS 1,
  /// a sine of amplitude 1 has RMS 1/√2.
  static double _gainFor(ToneSignal signal, double levelDb) {
    final rms = math.pow(10, levelDb / 20).toDouble();
    return signal == ToneSignal.sine ? rms * math.sqrt2 : rms;
  }

  Future<void> start(ToneSignal signal, double frequency, double levelDb) async {
    stop();
    final ctx = _ctx;
    if (ctx.state == 'suspended') await ctx.resume().toDart;
    final gain = ctx.createGain();
    gain.gain.value = _gainFor(signal, levelDb);
    gain.connect(ctx.destination);
    _gain = gain;

    if (signal == ToneSignal.sine) {
      final osc = ctx.createOscillator();
      osc.type = 'sine';
      osc.frequency.value = frequency;
      osc.connect(gain);
      osc.start();
      _oscillator = osc;
      _source = osc;
    } else {
      final src = ctx.createBufferSource();
      src.buffer = _noise[signal] ??= _noiseBuffer(ctx, pink: signal == ToneSignal.pink);
      src.loop = true;
      src.connect(gain);
      src.start();
      _source = src;
    }
  }

  void setFrequency(double hz) => _oscillator?.frequency.value = hz;

  void setLevel(ToneSignal signal, double levelDb) => _gain?.gain.value = _gainFor(signal, levelDb);

  void stop() {
    try {
      _source?.stop();
    } catch (_) {
      // already stopped
    }
    _source?.disconnect();
    _gain?.disconnect();
    _source = null;
    _oscillator = null;
    _gain = null;
  }

  /// 10 s of white or pink noise (Paul Kellet's pink filter), RMS 1.
  static web.AudioBuffer _noiseBuffer(web.AudioContext ctx, {required bool pink}) {
    final rate = ctx.sampleRate.round();
    final length = rate * 10;
    final data = Float32List(length);
    final random = math.Random();
    var b0 = 0.0, b1 = 0.0, b2 = 0.0, b3 = 0.0, b4 = 0.0, b5 = 0.0, b6 = 0.0;
    var sumSquares = 0.0;
    for (var i = 0; i < length; i++) {
      final white = random.nextDouble() * 2 - 1;
      double v;
      if (pink) {
        b0 = 0.99886 * b0 + white * 0.0555179;
        b1 = 0.99332 * b1 + white * 0.0750759;
        b2 = 0.96900 * b2 + white * 0.1538520;
        b3 = 0.86650 * b3 + white * 0.3104856;
        b4 = 0.55000 * b4 + white * 0.5329522;
        b5 = -0.7616 * b5 - white * 0.0168980;
        v = b0 + b1 + b2 + b3 + b4 + b5 + b6 + white * 0.5362;
        b6 = white * 0.115926;
      } else {
        v = white;
      }
      data[i] = v;
      sumSquares += v * v;
    }
    final scale = 1 / math.sqrt(sumSquares / length);
    for (var i = 0; i < length; i++) {
      data[i] *= scale;
    }
    final buffer = ctx.createBuffer(1, length, ctx.sampleRate);
    buffer.copyToChannel(data.toJS, 0);
    return buffer;
  }
}
