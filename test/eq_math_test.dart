import 'package:dsp/devices/t_racks408/widgets/eq_math.dart';
import 'package:dsp_core/dsp_core.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  int slope(String name) => ProtocolService.crossoverSlopeNames.indexOf(name);

  test('PEQ types', () {
    expect(peqResponseDb(1000, 1000, 2, 6, 0), closeTo(6, 0.01), reason: 'peak gain at the centre');
    expect(peqResponseDb(20, 1000, 2, 6, 0), closeTo(0, 0.05));
    expect(peqResponseDb(20, 1000, 0.7, -9, 1), closeTo(-9, 0.1), reason: 'low shelf below');
    expect(peqResponseDb(20000, 1000, 0.7, -9, 1), closeTo(0, 0.1));
    expect(peqResponseDb(20000, 1000, 0.7, 4, 2), closeTo(4, 0.1), reason: 'high shelf above');
    expect(peqResponseDb(1000, 1000, 1, 0, 3), closeTo(-3.01, 0.01), reason: 'LP -6 at cut-off');
    // Bilinear at 48 kHz: the frequency axis is pre-warped.
    expect(peqResponseDb(4000, 1000, 0.707, 0, 4), closeTo(-24.48, 0.05), reason: 'LP -12 two octaves up');
    expect(peqResponseDb(250, 1000, 0.707, 0, 6), closeTo(-24.12, 0.05), reason: 'HP -12 two octaves down');
    expect(peqResponseDb(300, 1000, 1, 0, 7), 0, reason: 'all pass');
  });

  test('peaks and shelves like the original editor (Zölzer K form)', () {
    // A cut is the exact inverse of the boost.
    for (final f in [100.0, 700.0, 1000.0, 1400.0, 8000.0]) {
      for (final type in [0, 1, 2]) {
        expect(peqResponseDb(f, 1000, 2, -6, type), closeTo(-peqResponseDb(f, 1000, 2, 6, type), 1e-9));
      }
    }
    // At the same Q the K form is an RBJ peak with Q / 10^(|G|/40), i.e.
    // wider: half an octave from the centre a +12 dB, Q 1 band has about
    // 10.4 dB (RBJ: 7.75 dB), a -12 dB band the mirror image.
    expect(peqResponseDb(1414, 1000, 1, 12, 0), closeTo(10.37, 0.05));
    expect(peqResponseDb(1414, 1000, 1, -12, 0), closeTo(-10.37, 0.05));
    // Shelves reach the full gain far from the corner.
    expect(peqResponseDb(20, 2000, 0.71, 6, 1), closeTo(6, 0.05));
    expect(peqResponseDb(20000, 200, 0.71, -6, 2), closeTo(-6, 0.05));
  });

  test('PEQ fit puts corrections into free bands only', () {
    const proto = ProtocolService();
    // Wanted: -6 dB around 1 kHz (one 1/3-octave band), +4 dB at 100 Hz.
    final target = [
      for (final f in geqCenterFrequencies) f == 1000 ? -6.0 : (f == 100 ? 4.0 : 0.0),
    ];
    final bands = [
      PeqBand(type: 1, gainDb: 3), // low shelf in use: must stay
      PeqBand(),
      PeqBand(),
      PeqBand(),
    ];
    final fitted = fitPeqCorrection(target: target, centers: geqCenterFrequencies, bands: bands, maxBoost: 6);
    expect(fitted[0].toJson(), bands[0].toJson());
    expect(proto.peqFreqToHz(fitted[1].freqRaw), closeTo(1000, 30));
    expect(fitted[1].gainDb, closeTo(-6, 0.5));
    expect(proto.peqFreqToHz(fitted[2].freqRaw), closeTo(100, 5));
    expect(fitted[2].gainDb, greaterThan(2));
    expect(freePeqBands(bands), [1, 2, 3]);

    // No boosts allowed: the +4 dB is left alone.
    final cutsOnly = fitPeqCorrection(target: target, centers: geqCenterFrequencies, bands: bands, maxBoost: 0);
    expect([for (final b in cutsOnly.skip(1)) b.gainDb > 0], everyElement(isFalse));
  });

  test('crossover slopes', () {
    expect(filterResponseDb(1000, 1000, slope('BW -24'), false), closeTo(-3.01, 0.01));
    expect(filterResponseDb(1000, 1000, slope('LK -24'), true), closeTo(-6.02, 0.01));
    expect(filterResponseDb(1000, 1000, slope('BL -24'), false), closeTo(-3.01, 0.05));
    expect(filterResponseDb(4000, 1000, slope('BW -24'), false), closeTo(-48.2, 0.3), reason: '24 dB/oct');
    expect(filterResponseDb(100, 1000, slope('BW -12'), false), closeTo(0, 0.01));
    expect(filterResponseDb(100, 1000, 0, true), 0, reason: 'bypass');
  });
}
