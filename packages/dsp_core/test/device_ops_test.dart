import 'dart:convert';

import 'package:dsp_core/dsp_core.dart';
import 'package:test/test.dart';

void main() {
  const m408 = DeviceModel.dsp408;
  const m206 = DeviceModel.dsp206;

  group('validate', () {
    test('accepts well-formed ops', () {
      final ops = [
        DeviceOps.gain('In A', -3.5),
        DeviceOps.mute('Out 8', true),
        DeviceOps.phase('In B', true),
        DeviceOps.delay('Out 2', 960),
        DeviceOps.matrixRoute('Out 2', 0x0F),
        DeviceOps.geqBand('In D', 30, 4.2),
        DeviceOps.geqReset('In A'),
        DeviceOps.peqBand('Out 1', 8, PeqBand(gainDb: 3)),
        DeviceOps.hiPass('In A', FilterState(freqRaw: 61, slope: 8)),
        DeviceOps.loPass('Out 3', FilterState(freqRaw: 280, slope: 20)),
        DeviceOps.gate('In C', GateState()),
        DeviceOps.compressor('Out 4', CompressorState(ratio: 9)),
        DeviceOps.limiter('Out 5', LimiterState(threshold: 200)),
        DeviceOps.channelName('Out 1', 'Sub L'),
        DeviceOps.channelName('Out 1', null),
      ];
      for (final op in ops) {
        // Round-trip through JSON as it would over the WebSocket.
        final decoded = jsonDecode(jsonEncode(op));
        expect(DeviceOps.validate(decoded, m408), isNull, reason: '$op');
      }
    });

    test('rejects malformed ops', () {
      final bad = <Object?>[
        null,
        'gain',
        {'op': 'gain', 'ch': 'In E', 'db': 0},
        {'op': 'gain', 'ch': 'In A', 'db': 'loud'},
        {'op': 'gain', 'ch': 'In A', 'db': double.nan},
        {'op': 'mute', 'ch': 'In A'},
        {'op': 'delay', 'ch': 'In A', 'raw': -1},
        {'op': 'delay', 'ch': 'In A', 'raw': ProtocolService.delayRawMax + 1},
        {'op': 'matrixRoute', 'out': 'In A', 'mask': 1},
        {'op': 'matrixRoute', 'out': 'Out 1', 'mask': 16},
        {'op': 'geqBand', 'ch': 'Out 1', 'band': 0, 'db': 0},
        {'op': 'peqBand', 'ch': 'In A', 'band': 8, ...PeqBand().toJson()},
        {'op': 'peqBand', 'ch': 'In A', 'band': 0, ...PeqBand(freqRaw: 301).toJson()},
        {'op': 'hiPass', 'ch': 'In A', 'freqRaw': 100, 'slope': 21},
        {'op': 'gate', 'ch': 'Out 1', ...GateState().toJson()},
        {'op': 'compressor', 'ch': 'In A', ...CompressorState().toJson()},
        {'op': 'compressor', 'ch': 'Out 1', ...CompressorState(ratio: 16).toJson()},
        {'op': 'limiter', 'ch': 'Out 1', 'attack': 0, 'release': 0, 'threshold': 221},
        {'op': 'channelName', 'ch': 'In A', 'name': 'too long name'},
        {'op': 'channelName', 'ch': 'In A', 'name': 'Bässe'},
        {'op': 'matrixGain', 'out': 'Out 1', 'in': 'In A', 'db': 0},
        {'op': 'alias', 'ch': 'In A', 'alias': 'x'},
        {'op': 'selfDestruct'},
      ];
      for (final op in bad) {
        expect(DeviceOps.validate(op, m408), isNotNull, reason: '$op');
      }
    });

    test('channels are validated against the model', () {
      expect(DeviceOps.validate(DeviceOps.gain('In C', 0), m206), isNotNull);
      expect(DeviceOps.validate(DeviceOps.gain('Out 7', 0), m206), isNotNull);
      expect(DeviceOps.validate(DeviceOps.matrixRoute('Out 1', 0x04), m206), isNotNull);
      expect(DeviceOps.validate(DeviceOps.matrixRoute('Out 1', 0x03), m206), isNull);
      expect(DeviceOps.validate(DeviceOps.gain('Out 6', 0), m206), isNull);
    });
  });

  group('apply and commands', () {
    test('gain is clamped, quantized and sent twice', () {
      final state = DeviceState();
      final op = DeviceOps.gain('Out 2', -33.3);
      DeviceOps.apply(state, op);
      expect(state.gains['Out 2'], -33.5);
      final cmds = DeviceOps.commands(state, op);
      expect(cmds.keys, ['gain:Out 2']);
      expect(cmds['gain:Out 2']!.length, 2);

      DeviceOps.apply(state, DeviceOps.gain('Out 2', 50));
      expect(state.gains['Out 2'], 12.0);
    });

    test('commands use the channel bytes of the state\'s model', () {
      final state = DeviceState(m206);
      final op = DeviceOps.phase('Out 1', true);
      DeviceOps.apply(state, op);
      expect(DeviceOps.commands(state, op)['phase:Out 1']!.single,
          const ProtocolService(m206).buildPhaseCommand('Out 1', true));
    });

    test('geq reset produces one coalescing key per band', () {
      final state = DeviceState();
      DeviceOps.apply(state, DeviceOps.geqBand('In A', 3, 5));
      final op = DeviceOps.geqReset('In A');
      DeviceOps.apply(state, op);
      expect(state.geqBands['In A'], List.filled(31, 0.0));
      expect(DeviceOps.commands(state, op).length, 31);
    });

    test('filters send their slope; slope 0 is bypass', () {
      final state = DeviceState();
      final op = DeviceOps.hiPass('In A', FilterState(freqRaw: 61, slope: 0));
      DeviceOps.apply(state, op);
      expect(state.hiPass['In A']!.enabled, isFalse);
      expect(DeviceOps.commands(state, op)['hpf:In A']!.single,
          const ProtocolService().buildHiPassCommand('In A', 61, 0));
    });

    test('limiter keeps the unknown parameter', () {
      final state = DeviceState();
      state.limiters['Out 1']!.unknown = 7;
      DeviceOps.apply(state, DeviceOps.limiter('Out 1', LimiterState(threshold: 200)));
      expect(state.limiters['Out 1']!.unknown, 7);
      expect(state.limiters['Out 1']!.threshold, 200);
    });

    test('channel name; empty restores the default device name', () {
      final state = DeviceState();
      final op = DeviceOps.channelName('In A', 'Vox');
      DeviceOps.apply(state, op);
      expect(state.channelNames['In A'], 'Vox');
      expect(DeviceOps.commands(state, op)['name:In A']!.single,
          const ProtocolService().buildChannelNameCommand('In A', 'Vox'));
      DeviceOps.apply(state, DeviceOps.channelName('In A', '  '));
      expect(state.channelNames['In A'], 'InA');
    });
  });

  group('linking', () {
    test('link op: mask must stay in the group and contain the channel', () {
      expect(DeviceOps.validate(DeviceOps.link('Out 1', 0x03), m408), isNull);
      expect(DeviceOps.validate(DeviceOps.link('Out 1', 0x02), m408), isNotNull, reason: 'own bit missing');
      expect(DeviceOps.validate(DeviceOps.link('Out 1', 0x100), m408), isNotNull);
      expect(DeviceOps.validate(DeviceOps.link('In A', 0x10), m408), isNotNull, reason: 'only 4 inputs');
      expect(DeviceOps.validate(DeviceOps.link('Out 6', 0x30), m206), isNull);
    });

    test('linkGroup sets the same mask on every member and frees others', () {
      final state = DeviceState(m408);
      for (final op in DeviceOps.linkGroup(state, ['Out 1', 'Out 2', 'Out 3'])) {
        DeviceOps.apply(state, op);
      }
      expect([for (final o in ['Out 1', 'Out 2', 'Out 3']) state.links[o]], [7, 7, 7]);
      expect(DeviceOps.linkedWith(state, 'Out 2'), ['Out 1', 'Out 3']);

      // Re-link Out 3 with Out 4: Out 1 and Out 2 stay linked together.
      for (final op in DeviceOps.linkGroup(state, ['Out 3', 'Out 4'])) {
        DeviceOps.apply(state, op);
      }
      expect(DeviceOps.linkedWith(state, 'Out 1'), ['Out 2']);
      expect(DeviceOps.linkedWith(state, 'Out 4'), ['Out 3']);

      // A single channel unlinks it.
      for (final op in DeviceOps.linkGroup(state, ['Out 3'])) {
        DeviceOps.apply(state, op);
      }
      expect(DeviceOps.linkedWith(state, 'Out 3'), isEmpty);
      expect(DeviceOps.linkedWith(state, 'Out 4'), isEmpty);
      expect(() => DeviceOps.linkGroup(state, ['In A', 'Out 1']), throwsArgumentError);
    });

    test('expandLinked repeats channel ops for linked channels', () {
      final state = DeviceState(m206);
      for (final op in DeviceOps.linkGroup(state, ['Out 1', 'Out 2'])) {
        DeviceOps.apply(state, op);
      }
      final ops = DeviceOps.expandLinked(state, DeviceOps.gain('Out 1', -6));
      expect(ops, [DeviceOps.gain('Out 1', -6), DeviceOps.gain('Out 2', -6)]);
      // Routing is not mirrored: a left/right pair is fed from different inputs.
      expect(DeviceOps.expandLinked(state, DeviceOps.matrixRoute('Out 2', 3)), [DeviceOps.matrixRoute('Out 2', 3)]);
      expect(DeviceOps.expandLinked(state, DeviceOps.gain('Out 3', 0)).length, 1);
      // Names and links themselves are never mirrored.
      expect(DeviceOps.expandLinked(state, DeviceOps.channelName('Out 1', 'Sub')).length, 1);
    });

    test('link command layout (0x3B ch mask)', () {
      final state = DeviceState(m206);
      final op = DeviceOps.link('Out 2', 0x03);
      DeviceOps.apply(state, op);
      expect(DeviceOps.commands(state, op)['link:Out 2']!.single,
          ProtocolService.buildFrame(0x3B, [0x03, 0x03]));
    });
  });

  group('copyChannel', () {
    test('copies everything except name, mute, link and routing', () {
      final state = DeviceState(m408);
      final edits = [
        DeviceOps.gain('Out 1', -4),
        DeviceOps.mute('Out 1', true),
        DeviceOps.phase('Out 1', true),
        DeviceOps.delay('Out 1', 480),
        DeviceOps.peqBand('Out 1', 3, PeqBand(gainDb: 5, freqRaw: 200, qRaw: 40, type: 1)),
        DeviceOps.hiPass('Out 1', FilterState(freqRaw: 61, slope: 8)),
        DeviceOps.matrixRoute('Out 1', 0x05),
        DeviceOps.compressor('Out 1', CompressorState(ratio: 7)),
        DeviceOps.channelName('Out 1', 'Sub L'),
      ];
      for (final op in edits) {
        DeviceOps.apply(state, op);
      }
      for (final op in DeviceOps.copyChannel(state, 'Out 1', 'Out 6')) {
        expect(DeviceOps.validate(op, m408), isNull, reason: '$op');
        DeviceOps.apply(state, op);
      }
      expect(state.gains['Out 6'], -4);
      expect(state.phase['Out 6'], isTrue);
      expect(state.delays['Out 6'], 480);
      expect(state.peqBands['Out 6']![3].toJson(), state.peqBands['Out 1']![3].toJson());
      expect(state.hiPass['Out 6']!.slope, 8);
      expect(state.matrixRouting['Out 6'], DeviceState(m408).matrixRouting['Out 6'], reason: 'routing is not copied');
      expect(state.compressors['Out 6']!.ratio, 7);
      expect(state.mutes['Out 6'], isFalse);
      expect(state.channelNames['Out 6'], 'Out6');
    });

    test('only within inputs or within outputs', () {
      final state = DeviceState(m408);
      expect(() => DeviceOps.copyChannel(state, 'In A', 'Out 1'), throwsArgumentError);
      expect(() => DeviceOps.copyChannel(state, 'In A', 'In A'), throwsArgumentError);
      expect(DeviceOps.copyChannel(state, 'In A', 'In B').where((op) => op['op'] == 'geqBand').length, 31);
    });
  });

  test('fullState reproduces a state from defaults', () {
    final source = DeviceState(m206);
    for (final op in [
      DeviceOps.gain('In B', -12.5),
      DeviceOps.mute('Out 4', true),
      DeviceOps.delay('Out 2', 96),
      DeviceOps.geqBand('In A', 7, 3.5),
      DeviceOps.loPass('Out 1', FilterState(freqRaw: 61, slope: 10)),
      DeviceOps.gate('In A', GateState(threshold: 40)),
      DeviceOps.limiter('Out 6', LimiterState(threshold: 200)),
      DeviceOps.channelName('In A', 'Vox'),
      DeviceOps.link('Out 1', 3),
      DeviceOps.link('Out 2', 3),
      DeviceOps.matrixRoute('Out 5', 3),
    ]) {
      DeviceOps.apply(source, op);
    }
    final target = DeviceState(m206);
    for (final op in DeviceOps.fullState(source)) {
      expect(DeviceOps.validate(op, m206), isNull, reason: '$op');
      DeviceOps.apply(target, op);
    }
    // Panel-side metadata (presets, device name) is not part of the device settings.
    target.currentPreset = source.currentPreset;
    expect(jsonEncode(target.toJson()), jsonEncode(source.toJson()));
  });

  test('DeviceState survives a JSON round trip', () {
    final state = DeviceState(m206)
      ..presets[0] = 'Default'
      ..currentPreset = 'Default'
      ..deviceName = 'DSP206 V0104P';
    DeviceOps.apply(state, DeviceOps.gain('In B', -12.3));
    DeviceOps.apply(state, DeviceOps.peqBand('Out 4', 2, PeqBand(freqRaw: 212, qRaw: 30, gainDb: -4.5, type: 2, bypass: true)));
    DeviceOps.apply(state, DeviceOps.hiPass('Out 4', FilterState(freqRaw: 100, slope: 3)));
    DeviceOps.apply(state, DeviceOps.delay('Out 6', 4800));
    DeviceOps.apply(state, DeviceOps.compressor('Out 5', CompressorState(ratio: 5)));
    DeviceOps.apply(state, DeviceOps.channelName('In B', 'Keys'));
    state.matrixGains['Out 1:In B'] = -1;

    final copy = DeviceState.fromJson(jsonDecode(jsonEncode(state.toJson())) as Map<String, Object?>);
    expect(copy.model, m206);
    expect(jsonEncode(copy.toJson()), jsonEncode(state.toJson()));
    expect(copy.peqBands['Out 4']![2].bypass, isTrue);
    expect(copy.presets[0], 'Default');
  });
}
