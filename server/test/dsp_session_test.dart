import 'dart:async';

import 'package:dsp_core/dsp_core.dart';
import 'package:dsp_server/src/device_registry.dart';
import 'package:dsp_server/src/dsp_session.dart';
import 'package:dsp_server/src/fake_dsp.dart';
import 'package:test/test.dart';

/// Frames the session sends while initializing a device of [model].
List<List<int>> initSequence(DeviceModel model) => [
      TRacksProto.handshakeCommand,
      TRacksProto.requestDeviceInfo,
      TRacksProto.requestAllPresetsCommand,
      TRacksProto.unknownStartup22,
      TRacksProto.unknownStartup14,
      ...TRacksProto.loadAllPresetsCommands,
      for (var i = 0; i < ConfigDump(model).chunkCount; i++) TRacksProto.configDumpCommand(i),
      ...TRacksProto.statusQueryCommands,
    ];

final initSequence408 = initSequence(DeviceModel.dsp408);

void main() {
  late FakeDsp fake;
  late DspSession session;

  DspSession createSession({int refreshMs = 300, String model = 'auto', RememberedValues? remembered}) =>
      DspSession(
        DspConfig(
            id: 'test', name: 'Test', host: '127.0.0.1', port: fake.port, refreshMs: refreshMs,
            model: model, remembered: remembered),
        requestTimeout: const Duration(milliseconds: 200),
        sequenceGap: Duration.zero,
        reconnectDelays: const [Duration(milliseconds: 100)],
      );

  Future<void> waitFor(bool Function() condition, {Duration timeout = const Duration(seconds: 10)}) async {
    final deadline = DateTime.now().add(timeout);
    while (!condition()) {
      if (DateTime.now().isAfter(deadline)) {
        fail('Timed out; status=${session.status} message=${session.message}');
      }
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
  }

  setUp(() async {
    fake = FakeDsp();
    await fake.start();
  });

  tearDown(() async {
    await session.dispose();
    await fake.stop();
  });

  test('connects, runs init and reads the configuration', () async {
    session = createSession();
    final statuses = <DspStatus>[];
    session.events.where((e) => e is InfoChanged).listen((_) {
      if (statuses.isEmpty || statuses.last != session.status) statuses.add(session.status);
    });
    session.start();
    await waitFor(() => session.status == DspStatus.ready);

    expect(statuses, containsAllInOrder([DspStatus.connecting, DspStatus.initializing, DspStatus.ready]));
    expect(session.state.deviceName, 'DSP408 V0104P');
    expect(session.model, DeviceModel.dsp408);
    expect(session.configReadable, isTrue);
    expect(session.state.presets.length, 20);
    expect(session.state.currentPreset, 'Default Preset');
    expect(session.state.peqBands['In A']![0].freqRaw, 0x29);
    // Every init command was sent exactly once, in order.
    expect(fake.received.take(initSequence408.length).toList(), initSequence408);
  });

  test('polls meters when idle', () async {
    session = createSession(refreshMs: 20);
    session.start();
    await waitFor(() => session.status == DspStatus.ready);
    await waitFor(() => session.meters.any((v) => v > 0));
  });

  test('coalesces rapid value changes per key', () async {
    session = createSession(refreshMs: 300);
    session.start();
    await waitFor(() => session.status == DspStatus.ready);
    fake.received.clear();

    // Slider drag: many values in one go, plus a mute in between.
    for (var i = 0; i <= 50; i++) {
      expect(session.applyOp(DeviceOps.gain('In A', -i / 2)), isNull);
      if (i == 10) session.applyOp(DeviceOps.mute('Out 1', true));
    }
    await waitFor(() => fake.receivedSetCommands.length >= 3);
    await Future<void>.delayed(const Duration(milliseconds: 100));

    final sent = fake.receivedSetCommands;
    // First value goes out immediately; the rest collapse into the newest.
    expect(sent.where((c) => c.startsWith('SET GAIN')).toSet(),
        {'SET GAIN In A → +0.0 dB', 'SET GAIN In A → -25.0 dB'});
    expect(sent, contains('SET MUTE Out 1 → MUTED'));
    expect(sent.length, lessThanOrEqualTo(6));
    expect(session.state.gains['In A'], -25.0);
  });

  test('rejects invalid ops without crashing', () async {
    session = createSession();
    session.start();
    await waitFor(() => session.status == DspStatus.ready);
    expect(session.applyOp({'op': 'gain', 'ch': 'In Z', 'db': 0}), isNotNull);
    expect(session.applyOp({'op': 'peqBand', 'ch': 'In A', 'band': 99}), isNotNull);
    expect(session.applyOp(DeviceOps.gain('In B', -3)), isNull);
    expect(session.status, DspStatus.ready);
  });

  test('reconnects after the device drops the connection', () async {
    session = createSession(refreshMs: 20);
    session.start();
    await waitFor(() => session.status == DspStatus.ready);

    fake.dropConnections();
    await waitFor(() => session.status != DspStatus.ready);
    await waitFor(() => session.status == DspStatus.ready);
    expect(fake.connectionCount, 1);
  });

  test('detects a device that stops answering', () async {
    session = createSession(refreshMs: 20);
    session.start();
    await waitFor(() => session.status == DspStatus.ready);

    fake.ignoreCommands.add(0x40);
    await waitFor(() => session.status == DspStatus.error);
    expect(session.message, contains('not responding'));
    fake.ignoreCommands.clear();
    await waitFor(() => session.status == DspStatus.ready);
  });

  test('loads and saves presets exclusively', () async {
    session = createSession(refreshMs: 20);
    session.start();
    await waitFor(() => session.status == DspStatus.ready);

    fake.received.clear();
    expect(session.loadPreset(3), isNull);
    expect(session.status, DspStatus.busy);
    expect(session.applyOp(DeviceOps.gain('In A', -1)), isNotNull, reason: 'busy');
    await waitFor(() => session.status == DspStatus.ready);
    final loadFrames = fake.received.where((f) => f[5] != 0x40).toList();
    expect(loadFrames, [
      const ProtocolService().buildLoadPresetCommand(3),
      for (var i = 0; i < 29; i++) TRacksProto.configDumpCommand(i),
      ...TRacksProto.statusQueryCommands,
    ]);
    expect(session.state.currentPreset, 'Preset 4');

    expect(session.applyOp(DeviceOps.gain('Out 2', -6)), isNull);
    expect(session.savePreset(4, 'My Show'), isNull);
    await waitFor(() => session.status == DspStatus.ready);
    expect(session.state.presets[4], 'My Show');
    expect(session.state.currentPreset, 'My Show');
    // The pending gain reached the device before the preset was stored.
    final cmds = fake.receivedSetCommands;
    expect(cmds.indexOf('SET GAIN Out 2 → -6.0 dB'), lessThan(cmds.indexOf('STORE PRESET NAME')));

    // Recalling the stored preset reads the saved value back from the device.
    expect(session.loadPreset(0), isNull);
    await waitFor(() => session.status == DspStatus.ready && session.state.currentPreset == 'Default Preset');
    expect(session.state.gains['Out 2'], 0.0);
    expect(session.loadPreset(4), isNull);
    await waitFor(() => session.status == DspStatus.ready && session.state.currentPreset == 'My Show');
    expect(session.state.gains['Out 2'], -6.0);
  });

  group('DSP 206', () {
    setUp(() async {
      await fake.stop();
      fake = FakeDsp(model: DeviceModel.dsp206);
      await fake.start();
    });

    test('is detected and read with its own layout', () async {
      session = createSession(refreshMs: 20);
      session.start();
      await waitFor(() => session.status == DspStatus.ready);
      expect(session.model, DeviceModel.dsp206);
      expect(session.info.model, 'dsp206');
      expect(session.state.deviceName, 'DSP206 V0104P');
      expect(session.configReadable, isTrue);
      expect(session.state.gains.keys, DeviceModel.dsp206.channels);
      expect(fake.received.take(initSequence(DeviceModel.dsp206).length).toList(),
          initSequence(DeviceModel.dsp206));
      await waitFor(() => session.meters.length == 8 && session.meters.any((v) => v > 0));
    });

    test('all functions reach the device and are read back after reconnecting', () async {
      session = createSession(refreshMs: 20);
      session.start();
      await waitFor(() => session.status == DspStatus.ready);

      final ops = [
        DeviceOps.gain('Out 6', -12),
        DeviceOps.mute('In B', true),
        DeviceOps.phase('Out 1', true),
        DeviceOps.delay('Out 2', 960),
        DeviceOps.matrixRoute('Out 3', 3),
        DeviceOps.geqBand('In A', 10, -3.5),
        DeviceOps.peqBand('Out 4', 8, PeqBand(freqRaw: 250, qRaw: 30, gainDb: 4.5, type: 2)),
        DeviceOps.hiPass('Out 1', FilterState(freqRaw: 11, slope: 8)),
        DeviceOps.loPass('Out 1', FilterState(freqRaw: 61, slope: 8)),
        DeviceOps.gate('In A', GateState(attack: 4, release: 199, hold: 49, threshold: 40)),
        DeviceOps.compressor('Out 5', CompressorState(ratio: 9, attack: 9, release: 99, knee: 2, threshold: 170)),
        DeviceOps.limiter('Out 6', LimiterState(attack: 19, release: 199, threshold: 200)),
        DeviceOps.channelName('Out 1', 'Sub L'),
      ];
      for (final op in ops) {
        expect(session.applyOp(op), isNull, reason: '$op');
      }
      expect(session.applyOp(DeviceOps.gain('Out 7', 0)), isNotNull, reason: 'no Out 7 on a 206');
      await waitFor(() => fake.state.channelNames['Out 1'] == 'Sub L');

      expect(fake.state.gains['Out 6'], -12.0);
      expect(fake.state.phase['Out 1'], isTrue);
      expect(fake.state.delays['Out 2'], 960);
      expect(fake.state.compressors['Out 5']!.ratio, 9);

      // Reconnect: everything in the dump comes back from the device.
      fake.dropConnections();
      await waitFor(() => session.status != DspStatus.ready);
      await waitFor(() => session.status == DspStatus.ready);
      final s = session.state;
      expect(s.gains['Out 6'], -12.0);
      expect(s.matrixRouting['Out 3'], 3);
      expect(s.geqBands['In A']![10], -3.5);
      expect(s.peqBands['Out 4']![8].gainDb, 4.5);
      expect(s.hiPass['Out 1']!.toJson(), {'freqRaw': 11, 'slope': 8});
      expect(s.gates['In A']!.threshold, 40);
      expect(s.compressors['Out 5']!.toJson(), CompressorState(ratio: 9, attack: 9, release: 99, knee: 2, threshold: 170).toJson());
      expect(s.limiters['Out 6']!.threshold, 200);
      expect(s.channelNames['Out 1'], 'Sub L');
      // Not in the dump: shown from the values the server remembers.
      expect(s.mutes['In B'], isTrue);
      expect(s.phase['Out 1'], isTrue);
      expect(s.delays['Out 2'], 960);
      expect(session.config.remembered.mutes['In B'], isTrue);
    });

    test('linked channels get every change', () async {
      session = createSession(refreshMs: 300);
      session.start();
      await waitFor(() => session.status == DspStatus.ready);
      for (final op in DeviceOps.linkGroup(session.state, ['Out 1', 'Out 2'])) {
        expect(session.applyOp(op), isNull);
      }
      expect(session.applyOp(DeviceOps.gain('Out 1', -9)), isNull);
      expect(session.applyOp(DeviceOps.hiPass('Out 2', FilterState(freqRaw: 61, slope: 8))), isNull);
      await waitFor(() => fake.state.hiPass['Out 1']!.slope == 8);
      expect(fake.state.gains['Out 2'], -9.0);
      expect(fake.state.links['Out 1'], 3);
      expect(fake.receivedSetCommands, contains('SET LINK Out 2 → Out 1 + Out 2'));
      expect(session.state.gains['Out 2'], -9.0);
    });

    test('a complete state from a server preset is sent to the device', () async {
      session = createSession(refreshMs: 300);
      session.start();
      await waitFor(() => session.status == DspStatus.ready);

      final preset = DeviceState(DeviceModel.dsp206);
      for (final op in [
        DeviceOps.gain('Out 3', -7.5),
        DeviceOps.peqBand('In B', 2, PeqBand(freqRaw: 120, qRaw: 30, gainDb: -3, type: 1)),
        DeviceOps.compressor('Out 6', CompressorState(ratio: 11, threshold: 160)),
        DeviceOps.channelName('Out 1', 'Sub'),
        DeviceOps.mute('In A', true),
      ]) {
        DeviceOps.apply(preset, op);
      }
      expect(session.applyFullState(DeviceState(DeviceModel.dsp408), 'Wrong'), contains('DSP 408'));
      expect(session.applyFullState(preset, 'Club'), isNull);
      expect(session.status, DspStatus.busy);
      await waitFor(() => session.status == DspStatus.ready, timeout: const Duration(seconds: 30));

      expect(fake.state.gains['Out 3'], -7.5);
      expect(fake.state.peqBands['In B']![2].type, 1);
      expect(fake.state.compressors['Out 6']!.ratio, 11);
      expect(fake.state.channelNames['Out 1'], 'Sub');
      expect(fake.state.mutes['In A'], isTrue);
      expect(session.state.currentPreset, 'Club');
      expect(session.config.remembered.mutes['In A'], isTrue);
    });

    test('a wrong configured model is reported, not shown', () async {
      session = createSession(model: 'dsp408');
      session.start();
      await waitFor(() => session.status == DspStatus.ready);
      expect(session.model, DeviceModel.dsp408);
      expect(session.configReadable, isFalse);
      expect(session.info.configReadable, isFalse);
      expect(session.logEntries.map((e) => e.text).join('\n'), contains('configured as DSP 408'));
    });
  });

  test('remembered values are applied on connect', () async {
    session = createSession(remembered: RememberedValues(mutes: {'Out 3': true}, delays: {'In A': 96}));
    session.start();
    await waitFor(() => session.status == DspStatus.ready);
    expect(session.state.mutes['Out 3'], isTrue);
    expect(session.state.delays['In A'], 96);
    expect(session.state.mutes['Out 2'], isFalse);
  });

  test('stop disconnects and does not reconnect', () async {
    session = createSession();
    session.start();
    await waitFor(() => session.status == DspStatus.ready);
    await session.stop();
    expect(session.status, DspStatus.disconnected);
    await Future<void>.delayed(const Duration(milliseconds: 300));
    expect(session.status, DspStatus.disconnected);
    expect(fake.connectionCount, 0);
  });

  test('reports an unreachable device and keeps retrying', () async {
    final port = fake.port;
    await fake.stop();
    session = DspSession(
      DspConfig(id: 'x', name: 'X', host: '127.0.0.1', port: port),
      reconnectDelays: const [Duration(milliseconds: 50)],
    );
    session.start();
    await waitFor(() => session.status == DspStatus.error);
    expect(session.message, contains('Connection failed'));

    fake = FakeDsp();
    await fake.start(port: port);
    await waitFor(() => session.status == DspStatus.ready);
  });
}
