// EQ response math shared by the EQ overview and the EQ graphs.
//
// These are display approximations (asymptotic filter slopes, bell and
// shelf shapes), not the exact transfer functions of the device.

import 'dart:math' as math;

import '../providers/device_provider.dart';

const _proto = ProtocolService();

/// ISO 1/3-octave centre frequencies of the 31 GEQ bands.
const geqCenterFrequencies = [
  20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0,
  200.0, 250.0, 315.0, 400.0, 500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0,
  2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10000.0, 12500.0, 16000.0, 20000.0,
];

/// HPF/LPF response in dB at [freq].
double filterResponseDb(double freq, double cutoffHz, double slopeDbPerOct, bool isHighPass) {
  if (cutoffHz <= 0) return 0.0;
  if (isHighPass) {
    if (freq >= cutoffHz) return 0.0;
    return -slopeDbPerOct * math.log(cutoffHz / freq) / math.ln2;
  }
  if (freq <= cutoffHz) return 0.0;
  return -slopeDbPerOct * math.log(freq / cutoffHz) / math.ln2;
}

/// Approximate response of one PEQ band in dB at [freq].
double peqResponseDb(double freq, double centerFreq, double q, double gainDb, int type) {
  if (gainDb.abs() < 0.01 && type == 0) return 0.0;
  if (q < 0.01) return 0.0;

  final ratio = freq / centerFreq;
  final logRatio = math.log(ratio);

  switch (type) {
    case 0: // Peak
      final bw = 1.0 / q;
      final x = logRatio / (bw * 0.5 * math.ln2);
      return gainDb / (1.0 + x * x);
    case 1: // Low Shelf
      final x = logRatio / (math.ln2 * 0.5);
      return gainDb / (1.0 + math.exp(x * 3));
    case 2: // High Shelf
      final x = logRatio / (math.ln2 * 0.5);
      return gainDb / (1.0 + math.exp(-x * 3));
    case 3: // LP -6dB
      return freq <= centerFreq ? 0 : -6.0 * logRatio / math.ln2;
    case 4: // LP -12dB
      return freq <= centerFreq ? 0 : -12.0 * logRatio / math.ln2;
    case 5: // HP -6dB
      return freq >= centerFreq ? 0 : 6.0 * logRatio / math.ln2;
    case 6: // HP -12dB
      return freq >= centerFreq ? 0 : 12.0 * logRatio / math.ln2;
    default: // All pass: no magnitude change
      return 0;
  }
}

/// GEQ response in dB at [freq]: bands interpolated on a log-frequency axis.
double geqResponseDb(double freq, List<double> bands) {
  if (bands.isEmpty) return 0;
  if (freq <= geqCenterFrequencies.first) return bands.first;
  if (freq >= geqCenterFrequencies.last) return bands.last;
  for (var i = 0; i < geqCenterFrequencies.length - 1; i++) {
    final f0 = geqCenterFrequencies[i], f1 = geqCenterFrequencies[i + 1];
    if (freq <= f1) {
      final t = math.log(freq / f0) / math.log(f1 / f0);
      return bands[i] + (bands[i + 1] - bands[i]) * t;
    }
  }
  return 0;
}

/// Total response of a channel in dB at [freq]: PEQ bands (not bypassed),
/// HPF/LPF and, for inputs, the GEQ.
double channelResponseDb(DeviceProvider device, String channel, double freq) {
  var db = 0.0;
  for (final band in device.getPeqBands(channel)) {
    if (band.bypass) continue;
    db += peqResponseDb(freq, _proto.peqFreqToHz(band.freqRaw), _proto.peqRawToQ(band.qRaw), band.gainDb, band.type);
  }
  final hp = device.getHiPass(channel);
  if (hp.enabled) {
    db += filterResponseDb(freq, _proto.peqFreqToHz(hp.freqRaw),
        ProtocolService.slopeDbPerOctave(hp.slope).toDouble(), true);
  }
  final lp = device.getLoPass(channel);
  if (lp.enabled) {
    db += filterResponseDb(freq, _proto.peqFreqToHz(lp.freqRaw),
        ProtocolService.slopeDbPerOctave(lp.slope).toDouble(), false);
  }
  if (device.inputs.contains(channel)) db += geqResponseDb(freq, device.getGeqBands(channel));
  return db;
}
