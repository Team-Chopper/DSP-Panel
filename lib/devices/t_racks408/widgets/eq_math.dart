// EQ response math shared by the EQ overview and the EQ graphs.
//
// Magnitudes of the analog prototypes the DSP filters are designed from:
// PEQ types as RBJ "Audio EQ Cookbook" biquads (peak, shelves, 2nd-order
// pass filters with the band's Q) and 1st-order pass filters; the HPF/LPF
// crossover slopes as Butterworth (-3 dB at the cut-off), Linkwitz-Riley
// (-6 dB) and Bessel (-3 dB) filters of the slope's order. At 96 kHz the
// digital filters follow these curves closely over the audio range.

import 'dart:math' as math;

import '../providers/device_provider.dart';

const _proto = ProtocolService();

/// ISO 1/3-octave centre frequencies of the 31 GEQ bands.
const geqCenterFrequencies = [
  20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0,
  200.0, 250.0, 315.0, 400.0, 500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0,
  2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10000.0, 12500.0, 16000.0, 20000.0,
];

// ─── Polynomials at s = j·w (no allocations: curves evaluate thousands) ───

/// |p(jw)|² of a polynomial with real coefficients (ascending powers):
/// j^k cycles 1, j, -1, -j.
double _abs2(List<double> c, double w) {
  var re = 0.0, im = 0.0, power = 1.0;
  for (var k = 0; k < c.length; k++) {
    final term = c[k] * power;
    switch (k & 3) {
      case 0:
        re += term;
      case 1:
        im += term;
      case 2:
        re -= term;
      default:
        im -= term;
    }
    power *= w;
  }
  return re * re + im * im;
}

/// |num(jw) / den(jw)| in dB.
double _ratioDb(List<double> num, List<double> den, double w) {
  final n = _abs2(num, w), d = _abs2(den, w);
  if (d <= 0 || n <= 0) return n <= 0 ? -120 : 0;
  return 10 * math.log(n / d) / math.ln10;
}

/// |(a0 + a1 s + a2 s²) / (b0 + b1 s + b2 s²)| in dB at s = j·w.
double _biquadDb(double a0, double a1, double a2, double b0, double b1, double b2, double w) {
  final w2 = w * w;
  final nr = a0 - a2 * w2, ni = a1 * w;
  final dr = b0 - b2 * w2, di = b1 * w;
  final n = nr * nr + ni * ni, d = dr * dr + di * di;
  if (d <= 0 || n <= 0) return n <= 0 ? -120 : 0;
  return 10 * math.log(n / d) / math.ln10;
}

// ─── Crossover (HPF/LPF) ───

/// Filter family and order of a crossover slope index.
(String family, int order) crossoverShape(int slope) {
  final name = ProtocolService.crossoverSlopeNames[slope.clamp(0, ProtocolService.crossoverSlopeNames.length - 1)];
  return (name.split(' ').first, ProtocolService.slopeDbPerOctave(slope) ~/ 6);
}

/// Reverse Bessel polynomial coefficients of [order], frequency-scaled so
/// that the low-pass is -3 dB at w = 1.
final Map<int, List<double>> _besselCache = {};

List<double> _bessel(int order) => _besselCache.putIfAbsent(order, () {
      double fact(int n) => n <= 1 ? 1 : n * fact(n - 1);
      final base = [
        for (var k = 0; k <= order; k++)
          fact(2 * order - k) / (math.pow(2, order - k) * fact(k) * fact(order - k)),
      ];
      List<double> scaled(double a) => [for (var k = 0; k <= order; k++) base[k] * math.pow(a, k)];
      double gainAt1(double a) => _ratioDb([base[0]], scaled(a), 1);
      // Bisection on the scale factor for -3.01 dB at w = 1.
      var lo = 0.01, hi = 100.0;
      for (var i = 0; i < 80; i++) {
        final mid = math.sqrt(lo * hi);
        if (gainAt1(mid) > -3.0103) {
          lo = mid;
        } else {
          hi = mid;
        }
      }
      return scaled(math.sqrt(lo * hi));
    });

/// HPF/LPF response in dB at [freq] for a crossover slope index (0 =
/// bypass).
double filterResponseDb(double freq, double cutoffHz, int slope, bool isHighPass) {
  if (cutoffHz <= 0 || slope <= 0) return 0.0;
  final (family, order) = crossoverShape(slope);
  if (order <= 0) return 0.0;
  // A high-pass is the low-pass mirrored on a log-frequency axis.
  final w = isHighPass ? cutoffHz / freq : freq / cutoffHz;
  switch (family) {
    case 'LK':
      // Linkwitz-Riley: two cascaded Butterworth filters of half the order.
      return -20 * math.log(1 + math.pow(w, order)) / math.ln10;
    case 'BL':
      final den = _bessel(order);
      return _ratioDb([den[0]], den, w);
    default:
      // Butterworth
      return -10 * math.log(1 + math.pow(w, 2 * order)) / math.ln10;
  }
}

// ─── PEQ band ───

/// Response of one PEQ band in dB at [freq].
double peqResponseDb(double freq, double centerFreq, double q, double gainDb, int type) {
  if (centerFreq <= 0) return 0.0;
  q = q.clamp(0.05, 200.0);
  final w = freq / centerFreq;
  final a = math.pow(10, gainDb / 40).toDouble();
  switch (type) {
    case 0: // Peak: (s² + s·A/Q + 1) / (s² + s/(A·Q) + 1)
      if (gainDb.abs() < 0.01) return 0.0;
      return _biquadDb(1, a / q, 1, 1, 1 / (a * q), 1, w);
    case 1: // Low shelf: A·(s² + √A/Q·s + A) / (A·s² + √A/Q·s + 1)
      if (gainDb.abs() < 0.01) return 0.0;
      final r = math.sqrt(a) / q;
      return gainDb / 2 + _biquadDb(a, r, 1, 1, r, a, w);
    case 2: // High shelf: A·(A·s² + √A/Q·s + 1) / (s² + √A/Q·s + A)
      if (gainDb.abs() < 0.01) return 0.0;
      final r = math.sqrt(a) / q;
      return gainDb / 2 + _biquadDb(1, r, a, a, r, 1, w);
    case 3: // LP -6 dB: 1 / (1 + s)
      return _biquadDb(1, 0, 0, 1, 1, 0, w);
    case 4: // LP -12 dB: 1 / (s² + s/Q + 1)
      return _biquadDb(1, 0, 0, 1, 1 / q, 1, w);
    case 5: // HP -6 dB: s / (1 + s)
      return _biquadDb(0, 1, 0, 1, 1, 0, w);
    case 6: // HP -12 dB: s² / (s² + s/Q + 1)
      return _biquadDb(0, 0, 1, 1, 1 / q, 1, w);
    default: // All pass: no magnitude change
      return 0;
  }
}

/// Whether a band changes the magnitude at all (used to skip flat curves).
bool peqBandAudible(PeqBand b) => !b.bypass && (b.type >= 3 && b.type <= 6 || b.gainDb.abs() >= 0.05 && b.type <= 2);

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

/// Free PEQ bands for an Auto-EQ: peak bands at 0 dB (bypassed or not).
/// Bands in use (shelves, pass filters, peaks with gain) are not touched.
List<int> freePeqBands(List<PeqBand> bands) => [
      for (var i = 0; i < bands.length; i++)
        if (bands[i].type == 0 && bands[i].gainDb.abs() < 0.05) i,
    ];

/// Auto-EQ for a channel without GEQ: approximates [target] (the wanted
/// change in dB at the 1/3-octave [centers]) with peak filters in the free
/// bands of [bands]. Greedy: the largest remaining deviation gets the next
/// free band, its width sets Q; stops when the rest is below [minDb].
/// Returns the new band list (unchanged bands are copies).
List<PeqBand> fitPeqCorrection({
  required List<double> target,
  required List<double> centers,
  required List<PeqBand> bands,
  double maxBoost = 6,
  double maxCut = 12,
  double minDb = 1.0,
}) {
  final result = [for (final b in bands) b.copy()];
  final residual = List.of(target);
  for (final index in freePeqBands(bands)) {
    // Largest deviation that can be corrected (boosts may be limited to 0).
    double usable(int i) => residual[i].clamp(-maxCut, maxBoost).abs();
    var k = 0;
    for (var i = 1; i < residual.length; i++) {
      if (usable(i) > usable(k)) k = i;
    }
    final peak = residual[k];
    if (usable(k) < minDb) break;
    if (peak.abs() < minDb) break;
    // Width: neighbours with the same sign and at least half the deviation.
    var lo = k, hi = k;
    bool similar(int i) => residual[i].sign == peak.sign && residual[i].abs() >= peak.abs() / 2;
    while (lo > 0 && similar(lo - 1)) {
      lo--;
    }
    while (hi < residual.length - 1 && similar(hi + 1)) {
      hi++;
    }
    final octaves = (hi - lo + 1) / 3;
    final n = math.pow(2, octaves).toDouble();
    final q = (math.sqrt(n) / (n - 1)).clamp(0.5, 8.0);
    final gain = ((peak.clamp(-maxCut, maxBoost)) * 10).round() / 10;
    final band = PeqBand(freqRaw: _proto.peqHzToFreq(centers[k]), qRaw: _proto.peqQToRaw(q), gainDb: gain);
    result[index] = band;
    final fc = _proto.peqFreqToHz(band.freqRaw), bq = _proto.peqRawToQ(band.qRaw);
    for (var i = 0; i < residual.length; i++) {
      residual[i] -= peqResponseDb(centers[i], fc, bq, gain, 0);
    }
  }
  return result;
}

/// The EQ settings of one channel, read once, for evaluating its total
/// response at many frequencies (a curve has ~160 points).
class ChannelEq {
  final List<(double hz, double q, double gainDb, int type)> _bands;
  final (double hz, int slope)? _hp;
  final (double hz, int slope)? _lp;
  final List<double>? _geq;

  ChannelEq._(this._bands, this._hp, this._lp, this._geq);

  factory ChannelEq.of(DeviceProvider device, String channel) {
    final hp = device.getHiPass(channel);
    final lp = device.getLoPass(channel);
    return ChannelEq._(
      [
        for (final b in device.getPeqBands(channel))
          if (peqBandAudible(b)) (_proto.peqFreqToHz(b.freqRaw), _proto.peqRawToQ(b.qRaw), b.gainDb, b.type),
      ],
      hp.enabled ? (_proto.peqFreqToHz(hp.freqRaw), hp.slope) : null,
      lp.enabled ? (_proto.peqFreqToHz(lp.freqRaw), lp.slope) : null,
      device.inputs.contains(channel) ? device.getGeqBands(channel) : null,
    );
  }

  /// Total response in dB at [freq]: PEQ bands (not bypassed), HPF/LPF and,
  /// for inputs, the GEQ.
  double db(double freq) {
    var db = 0.0;
    for (final (hz, q, gain, type) in _bands) {
      db += peqResponseDb(freq, hz, q, gain, type);
    }
    final hp = _hp, lp = _lp, geq = _geq;
    if (hp != null) db += filterResponseDb(freq, hp.$1, hp.$2, true);
    if (lp != null) db += filterResponseDb(freq, lp.$1, lp.$2, false);
    if (geq != null) db += geqResponseDb(freq, geq);
    return db;
  }
}

/// Total response of a channel in dB at [freq] (see [ChannelEq] for many
/// frequencies).
double channelResponseDb(DeviceProvider device, String channel, double freq) =>
    ChannelEq.of(device, channel).db(freq);
