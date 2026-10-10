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
        DeviceOps.matrixGain('Out 1', 'In D', -6.5),
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
        {'op': 'matrixGain', 'out': 'In A', 'in': 'In A', 'db': 0},
        {'op': 'matrixGain', 'out': 'Out 1', 'in': 'Out 2', 'db': 0},
        {'op': 'matrixGain', 'out': 'Out 1', 'in': 'In A', 'db': 'quiet'},
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

    test('matrix gain is clamped to -60 … 0 dB, quantized and sent per crossing', () {
      final state = DeviceState(m206);
      final op = DeviceOps.matrixGain('Out 3', 'In B', -33.3);
      expect(DeviceOps.validate(op, m206), isNull);
      DeviceOps.apply(state, op);
      expect(state.matrixGains['Out 3:In B'], -33.5);
      expect(DeviceOps.commands(state, op), {
        'mixgain:Out 3:In B': [const ProtocolService(m206).buildMatrixGainCommand('Out 3', 'In B', -33.5)],
      });
      DeviceOps.apply(state, DeviceOps.matrixGain('Out 3', 'In B', 4));
      expect(state.matrixGains['Out 3:In B'], 0.0);
      expect(DeviceOps.validate(DeviceOps.matrixGain('Out 1', 'In C', 0), m206), isNotNull);
      // Not shared with linked channels
      final linked = DeviceState(m206)..links['Out 1'] = 3;
      expect(DeviceOps.expandLinked(linked, DeviceOps.matrixGain('Out 1', 'In A', -3)).length, 1);
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
    void applyAll(DeviceState state, List<Map<String, Object?>> ops) {
      for (final op in ops) {
        expect(DeviceOps.validate(op, state.model), isNull, reason: '$op');
        DeviceOps.apply(state, op);
      }
    }

    test('link op: mask must stay in the group and contain the channel (or be empty)', () {
      expect(DeviceOps.validate(DeviceOps.link('Out 1', 0x03), m408), isNull);
      expect(DeviceOps.validate(DeviceOps.link('Out 1', 0x02), m408), isNotNull, reason: 'own bit missing');
      expect(DeviceOps.validate(DeviceOps.link('Out 2', 0x00), m408), isNull, reason: 'slave stored without bits');
      expect(DeviceOps.validate(DeviceOps.link('Out 1', 0x100), m408), isNotNull);
      expect(DeviceOps.validate(DeviceOps.link('In A', 0x10), m408), isNotNull, reason: 'only 4 inputs');
      expect(DeviceOps.validate(DeviceOps.link('Out 6', 0x30), m206), isNull);
    });

    test('master/slave as read from the device: a link 1 → 2 also moves 1 when 2 changes', () {
      // DSP 206 dump: Out 1 lists Out 2, Out 2 only itself.
      final state = DeviceState(m206)
        ..links['Out 1'] = 0x03
        ..links['Out 2'] = 0x02;
      expect(DeviceOps.masterOf(state, 'Out 2'), 'Out 1');
      expect(DeviceOps.masterOf(state, 'Out 1'), isNull);
      expect(DeviceOps.slavesOf(state, 'Out 1'), ['Out 2']);
      expect(DeviceOps.expandLinked(state, DeviceOps.gain('Out 2', -3)),
          [DeviceOps.gain('Out 2', -3), DeviceOps.gain('Out 1', -3)]);
      expect(DeviceOps.expandLinked(state, DeviceOps.gain('Out 1', -3)),
          [DeviceOps.gain('Out 1', -3), DeviceOps.gain('Out 2', -3)]);
    });

    test('symmetric masks of older versions form one group with the first channel as master', () {
      final state = DeviceState(m408)
        ..links['Out 1'] = 7
        ..links['Out 2'] = 7
        ..links['Out 3'] = 7;
      expect(DeviceOps.linkGroupOf(state, 'Out 3'), ['Out 1', 'Out 2', 'Out 3']);
      expect(DeviceOps.masterOf(state, 'Out 2'), 'Out 1');
      expect(DeviceOps.linkedWith(state, 'Out 2'), ['Out 1', 'Out 3']);
    });

    test('linkSlaves: master keeps the mask, slaves only their own bit', () {
      final state = DeviceState(m408);
      applyAll(state, DeviceOps.linkSlaves(state, 'Out 1', ['Out 2'], copySettings: false));
      expect(state.links['Out 1'], 0x03);
      expect(state.links['Out 2'], 0x02);

      // A slave cannot be linked further, nor become a slave of another master.
      expect(DeviceOps.linkCandidates(state, 'Out 2'), isEmpty);
      expect(() => DeviceOps.linkSlaves(state, 'Out 2', ['Out 3']), throwsArgumentError);
      expect(DeviceOps.linkCandidates(state, 'Out 3'), isNot(contains('Out 2')));
      expect(DeviceOps.linkCandidates(state, 'Out 3'), isNot(contains('Out 1')), reason: 'Out 1 is a master');

      // More channels are added to the master.
      applyAll(state, DeviceOps.linkSlaves(state, 'Out 1', ['Out 2', 'Out 4'], copySettings: false));
      expect(state.links['Out 1'], 0x0B);
      expect(DeviceOps.linkedWith(state, 'Out 4'), ['Out 1', 'Out 2']);

      // A slave leaves its master; the rest stays linked.
      applyAll(state, DeviceOps.unlink(state, 'Out 2'));
      expect(DeviceOps.linkedWith(state, 'Out 1'), ['Out 4']);
      expect(DeviceOps.linkedWith(state, 'Out 2'), isEmpty);

      // Unlinking the master releases everything.
      applyAll(state, DeviceOps.unlink(state, 'Out 1'));
      expect(state.links['Out 1'], 0x01);
      expect(DeviceOps.linkedWith(state, 'Out 4'), isEmpty);
      expect(() => DeviceOps.linkSlaves(state, 'In A', ['Out 1']), throwsArgumentError);
    });

    test('linkSlaves copies the master settings to new slaves', () {
      final state = DeviceState(m408);
      DeviceOps.apply(state, DeviceOps.gain('In A', -7));
      DeviceOps.apply(state, DeviceOps.geqBand('In A', 3, 4));
      applyAll(state, DeviceOps.linkSlaves(state, 'In A', ['In C']));
      expect(state.gains['In C'], -7);
      expect(state.geqBands['In C']![3], 4);
      expect(state.gains['In B'], 0);
    });

    test('expandLinked repeats channel ops for linked channels', () {
      final state = DeviceState(m206);
      for (final op in DeviceOps.linkSlaves(state, 'Out 1', ['Out 2'])) {
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

  test('test tone (0x39 type param)', () {
    final state = DeviceState(m206);
    expect(DeviceOps.validate(DeviceOps.testTone(1), m206), isNull);
    expect(DeviceOps.validate(DeviceOps.testTone(4), m206), isNotNull);
    expect(DeviceOps.validate(DeviceOps.testTone(3, 31), m206), isNotNull);
    final op = DeviceOps.testTone(3, 17);
    expect(DeviceOps.expandLinked(state, op), [op]);
    DeviceOps.apply(state, op);
    expect([state.testToneType, state.testToneParam], [3, 17]);
    expect(DeviceOps.commands(state, op)['testTone']!.single, ProtocolService.buildFrame(0x39, [3, 17]));
    expect(DeviceState.fromJson(state.toJson()).testToneParam, 17);
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
      DeviceOps.matrixGain('Out 5', 'In B', -12),
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
