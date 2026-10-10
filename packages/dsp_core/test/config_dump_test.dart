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

  test('layout sizes (offsets as used by the original editor)', () {
    expect(dump408.outputRecordLength, 104);
    expect(dump408.requiredLength, 1408);
    expect(dump408.muteOffset, 0x580);
    expect(dump408.peqBypassOffset, 0x584);
    expect(dump408.switchesLength, 0x59C);
    expect(dump408.totalLength, 0x5A8);
    expect(dump408.chunkCount, 29);
    expect(dump206.outputRecordLength, 100);
    expect(dump206.requiredLength, 896);
    expect(dump206.muteOffset, 0x380);
    expect(dump206.peqBypassOffset, 0x384);
    expect(dump206.switchesLength, 0x394);
    expect(dump206.totalLength, 0x3A0);
    expect(dump206.chunkCount, 19);
    // The captured dumps have exactly the expected size.
    expect(defaultStream.length, dump408.totalLength);
    expect(testStream.length, dump408.totalLength);
  });

  group('mute, phase, delay and PEQ bypass', () {
    DeviceState changed(DeviceModel model) {
      final state = DeviceState(model);
      for (final op in [
        DeviceOps.mute(model.inputs.last, true),
        DeviceOps.mute(model.outputs.last, true),
        DeviceOps.phase('In A', true),
        DeviceOps.phase('Out 2', true),
        DeviceOps.delay('Out 3', 960),
        DeviceOps.delay('In B', 65280),
        DeviceOps.peqBand('In A', 0, PeqBand(bypass: true)),
        DeviceOps.peqBand(model.outputs.last, 8, PeqBand(bypass: true)),
      ]) {
        DeviceOps.apply(state, op);
      }
      return state;
    }

    for (final dump in [dump408, dump206]) {
      test('${dump.model.name}: written and read at their offsets', () {
        final model = dump.model;
        final state = changed(model);
        final stream = dump.encode(state);
        int u16(int at) => stream[at] | (stream[at + 1] << 8);

        expect(u16(dump.muteOffset), 1 << (model.inputs.length - 1));
        expect(u16(dump.muteOffset + 2), 1 << (model.outputs.length - 1));
        expect(u16(dump.peqBypassOffset), 1, reason: 'In A band 1');
        expect(u16(dump.peqBypassOffset + 2 * model.channelByte(model.outputs.last)!), 1 << 8);
        // Channel tail: gain, phase, delay, link
        expect(u16(dump.inputOffset(0) + 134), 1);
        expect(u16(dump.outputOffset(1) + dump.outputRecordLength - 6), 1);
        expect(u16(dump.outputOffset(2) + dump.outputRecordLength - 4), 960);

        final decoded = dump.decode(stream);
        expect(decoded.problems, isEmpty);
        expect(decoded.switchesRead, isTrue);
        expect(decoded.state.toJson(), state.toJson());
      });
    }

    test('a dump that ends after the records leaves the switches off', () {
      final stream = dump206.encode(changed(DeviceModel.dsp206)).sublist(0, dump206.requiredLength);
      final result = dump206.decode(stream);
      expect(result.isPlausible, isTrue);
      expect(result.switchesRead, isFalse);
      expect(result.state.mutes.values.toSet(), {false});
      expect(result.state.phase['Out 2'], isTrue, reason: 'phase is part of the records');
    });
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

    test('nothing muted, inverted, delayed or bypassed', () {
      expect(result.switchesRead, isTrue);
      expect(s.mutes.values.toSet(), {false});
      expect(s.phase.values.toSet(), {false});
      expect(s.delays.values.toSet(), {0});
      expect(s.peqBands.values.expand((b) => b).any((b) => b.bypass), isFalse);
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

  group('DSP 206', () {
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
