import 'package:dsp_core/dsp_core.dart';
import 'package:dsp_core/testing.dart';
import 'package:test/test.dart';

void main() {
  const dump408 = ConfigDump(DeviceModel.dsp408);
  const dump206 = ConfigDump(DeviceModel.dsp206);
  const p = ProtocolService();

  List<int> join(List<List<int>> chunks) => [for (final c in chunks) ...c];

  final defaultStream = join(capturedConfigChunks());
  final testStream = join(capturedTestPresetChunks());

  test('layout sizes', () {
    expect(dump408.outputRecordLength, 104);
    expect(dump408.requiredLength, 1408);
    expect(dump408.totalLength, 1448);
    expect(dump408.chunkCount, 29);
    expect(dump206.outputRecordLength, 100);
    expect(dump206.requiredLength, 896);
    expect(dump206.chunkCount, 18);
    // The captured dumps have exactly the expected size.
    expect(defaultStream.length, dump408.totalLength);
    expect(testStream.length, dump408.totalLength);
  });

  group('DSP 408 "Default Preset" dump', () {
    final result = dump408.decode(defaultStream);
    final s = result.state;

    test('is plausible', () => expect(result.problems, isEmpty));

    test('header and channel names', () {
      expect(s.currentPreset, 'Default Preset');
      expect(s.channelNames['In A'], 'InA');
      expect(s.channelNames['Out 8'], 'Out8');
    });

    test('gains, routing and crossing gains', () {
      expect(s.gains.values.toSet(), {0.0});
      expect(s.matrixRouting, {
        'Out 1': 1, 'Out 2': 2, 'Out 3': 4, 'Out 4': 8,
        'Out 5': 1, 'Out 6': 2, 'Out 7': 4, 'Out 8': 8,
      });
      expect(s.matrixGains.values.toSet(), {0.0});
    });

    test('gate, EQ and filters', () {
      expect(s.gates['In A']!.toJson(), GateState().toJson());
      expect(ProtocolService.thresholdRawToDb(s.gates['In A']!.threshold), -90.0);
      expect(s.geqBands['In D'], List.filled(31, 0.0));
      // Default PEQ frequencies are spread over the whole band.
      expect(s.peqBands['In A']!.map((b) => b.freqRaw), [41, 71, 101, 140, 170, 200, 240, 270]);
      expect(s.peqBands['Out 1']!.map((b) => b.freqRaw), [31, 63, 95, 127, 158, 190, 222, 253, 285]);
      expect(s.peqBands['Out 1']!.every((b) => b.gainDb == 0 && b.qRaw == 35 && b.type == 0), isTrue);
      for (final ch in DeviceModel.dsp408.channels) {
        expect(s.hiPass[ch]!.toJson(), {'freqRaw': 0, 'slope': 0}, reason: ch);
        expect(s.loPass[ch]!.toJson(), {'freqRaw': 300, 'slope': 0}, reason: ch);
      }
    });

    test('dynamics and links', () {
      final c = s.compressors['Out 4']!;
      expect(c.toJson(), {'ratio': 0, 'attack': 49, 'release': 499, 'knee': 0, 'threshold': 220});
      expect(s.limiters['Out 8']!.toJson(), {'attack': 49, 'release': 499, 'unknown': 0, 'threshold': 220});
      expect(s.links['In C'], 4);
      expect(s.links['Out 8'], 0x80);
    });

    test('re-encodes to the identical bytes', () {
      expect(dump408.encode(s), defaultStream);
    });
  });

  group('DSP 408 "Test" dump (changed preset)', () {
    final result = dump408.decode(testStream);
    final s = result.state;

    test('is plausible', () => expect(result.problems, isEmpty));

    test('contains the changes made in the preset', () {
      expect(s.currentPreset, 'Test');
      expect([for (final o in ['Out 1', 'Out 2', 'Out 3']) s.matrixRouting[o]], [3, 3, 3]);
      expect(s.gains['Out 8'], -60.0);
      expect(s.hiPass['Out 8']!.toJson(), {'freqRaw': 55, 'slope': 1}); // 71 Hz BW -6
      expect(s.loPass['Out 8']!.toJson(), {'freqRaw': 280, 'slope': 20}); // 12.6 kHz LK -48
      expect(s.loPass['In A']!.slope, 20);
      expect(s.peqBands['Out 8']![0].type, 7); // All Pass 1
      expect(s.peqBands['Out 8']![2].gainDb, closeTo(6.9, 1e-9));
      expect(s.peqBands['In A']![7].gainDb, 12.0);
      expect(p.peqFreqToHz(s.hiPass['Out 8']!.freqRaw).round(), 71);
    });

    test('re-encodes to the identical bytes', () {
      expect(dump408.encode(s), testStream);
    });
  });

  group('DSP 206 (assumed layout)', () {
    test('round trip of a modified state', () {
      final state = DeviceState(DeviceModel.dsp206)
        ..currentPreset = 'Club'
        ..gains['Out 6'] = -12.5
        ..matrixRouting['Out 3'] = 3
        ..channelNames['Out 1'] = 'Sub L';
      state.hiPass['Out 2'] = FilterState(freqRaw: 61, slope: 8);
      state.compressors['Out 2'] = CompressorState(ratio: 9, attack: 9, release: 99, knee: 2, threshold: 170);
      state.gates['In B'] = GateState(attack: 4, release: 199, hold: 49, threshold: 40);
      state.geqBands['In A']![10] = -3.5;

      final stream = dump206.encode(state);
      expect(stream.length, dump206.totalLength);
      final decoded = dump206.decode(stream);
      expect(decoded.problems, isEmpty);
      expect(decoded.state.toJson(), state.toJson());
    });

    test('a 408 dump decoded with the 206 layout is reported as implausible', () {
      final result = dump206.decode(defaultStream);
      expect(result.isPlausible, isFalse);
    });

    test('too short', () {
      expect(dump206.decode(List.filled(100, 0)).problems.single, contains('too short'));
    });
  });

  test('split produces 50-byte chunks', () {
    final chunks = ConfigDump.split(defaultStream);
    expect(chunks.length, 29);
    expect(chunks.last.length, 48);
    expect(chunks.first, capturedConfigChunks().first);
  });
}
