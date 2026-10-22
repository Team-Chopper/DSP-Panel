// Writes reference data of the Dart dsp_core for the Rust parity test
// (rust/crates/dsp-core/tests/parity.rs):
//
//   cd rust/tool/parity
//   dart pub get --offline
//   dart run bin/generate.dart ../../crates/dsp-core/tests/fixtures/parity.json
//
// Random op sequences (valid and malformed) with the validation result, the
// linked ops, the device frames and their descriptions, the final states and
// link groups; config dumps; meters; value encodings.

import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:dsp_core/dsp_core.dart';
import 'package:dsp_core/testing.dart';

String hex(List<int> bytes) => bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

/// JSON round trip, as an op travels over the WebSocket.
Object? wire(Object? value) => jsonDecode(jsonEncode(value));

late Random rng;

T pick<T>(List<T> list) => list[rng.nextInt(list.length)];
bool chance(int percent) => rng.nextInt(100) < percent;

/// An int, sometimes just outside [min, max].
int intAround(int min, int max) => chance(8) ? pick([min - 1, max + 1]) : min + rng.nextInt(max - min + 1);

double dbIn(int minTenths, int maxTenths) => (minTenths + rng.nextInt(maxTenths - minTenths + 1)) / 10.0;

String channel(DeviceModel model) =>
    chance(5) ? pick(['In E', 'Out 9', 'In C', 'Out 7']) : pick(model.channels);

Map<String, Object?> randomOp(DeviceModel model) {
  final ch = channel(model);
  final kind = rng.nextInt(21);
  final Map<String, Object?> op;
  switch (kind) {
    case 0:
      op = DeviceOps.gain(ch, dbIn(-700, 200));
    case 1:
      op = DeviceOps.mute(ch, chance(50));
    case 2:
      op = DeviceOps.phase(ch, chance(50));
    case 3:
      op = DeviceOps.delay(ch, intAround(0, ProtocolService.delayRawMax));
    case 4:
      op = DeviceOps.matrixRoute(chance(10) ? ch : pick(model.outputs), intAround(0, model.matrixMask));
    case 5:
      op = DeviceOps.matrixGain(pick(model.outputs), chance(5) ? 'In E' : pick(model.inputs), dbIn(-700, 100));
    case 6:
      op = DeviceOps.geqBand(chance(10) ? ch : pick(model.inputs), intAround(0, 30), dbIn(-150, 150));
    case 7:
      op = DeviceOps.geqReset(chance(10) ? ch : pick(model.inputs));
    case 8:
      final band = PeqBand(
        freqRaw: intAround(0, 300),
        qRaw: intAround(0, 100),
        gainDb: dbIn(-150, 150),
        type: intAround(0, 8),
        bypass: chance(30),
      );
      op = {...DeviceOps.peqBand(ch, intAround(0, model.isInput(ch) ? 7 : 8), PeqBand()), ...band.toJson()};
    case 9:
      op = {'op': 'hiPass', 'ch': ch, 'freqRaw': intAround(0, 300), 'slope': intAround(0, 20)};
    case 10:
      op = {'op': 'loPass', 'ch': ch, 'freqRaw': intAround(0, 300), 'slope': intAround(0, 20)};
    case 11:
      op = {
        'op': 'gate', 'ch': chance(10) ? ch : pick(model.inputs),
        'attack': intAround(0, 998), 'release': intAround(9, 2999), 'hold': intAround(9, 998),
        'threshold': intAround(0, 180),
      };
    case 12:
      op = {
        'op': 'compressor', 'ch': chance(10) ? ch : pick(model.outputs),
        'ratio': intAround(0, 15), 'attack': intAround(0, 998), 'release': intAround(9, 2999),
        'knee': intAround(0, 12), 'threshold': intAround(0, 220),
      };
    case 13:
      op = {
        'op': 'limiter', 'ch': chance(10) ? ch : pick(model.outputs),
        'attack': intAround(0, 998), 'release': intAround(9, 2999), 'knee': intAround(0, 12),
        'threshold': intAround(0, 220),
      };
    case 14:
      op = DeviceOps.eqBypass(ch, chance(50));
    case 15:
      op = DeviceOps.geqBypass(chance(10) ? ch : pick(model.inputs), chance(50));
    case 16:
      op = DeviceOps.delayUnit(intAround(0, 2));
    case 17:
      op = DeviceOps.channelName(ch, pick([null, '', '  ', 'Vox', 'Sub L', 'Kick In', 'TooLongName', 'Bässe', ' Pad ']));
    case 18:
      // Link masks within the group, sometimes without the own bit or empty.
      final linkCh = model.hasChannel(ch) ? ch : pick(model.channels);
      final group = model.groupOf(linkCh);
      op = DeviceOps.link(
          linkCh, chance(10) ? 0 : rng.nextInt(1 << group.length) | (chance(85) ? model.linkBit(linkCh) : 0));
    case 19:
      op = DeviceOps.testTone(intAround(0, 3), intAround(0, 30));
    default:
      // Malformed: wrong types, missing fields, unknown ops.
      op = pick<Map<String, Object?>>([
        {'op': 'gain', 'ch': ch, 'db': 'loud'},
        {'op': 'gain', 'ch': ch},
        {'op': 'delay', 'ch': ch, 'raw': 1.5},
        {'op': 'mute', 'ch': ch, 'on': 1},
        {'op': 'selfDestruct'},
        {'op': 42},
        {'ch': ch},
        {'op': 'peqBand', 'ch': ch, 'band': 0, 'freqRaw': 100},
        {'op': 'matrixGain', 'out': pick(model.outputs), 'in': pick(model.inputs), 'db': true},
        {'op': 'channelName', 'ch': ch, 'name': 5},
        {'op': 'link', 'ch': ch, 'mask': 'all'},
        {'op': 'testTone', 'type': 1},
      ]);
  }
  return op;
}

List<String> describeAll(Map<String, List<List<int>>> commands, DeviceModel model) => [
      for (final frames in commands.values)
        for (final frame in frames) PacketDescriber.describe(frame, model),
    ];

Map<String, Object?> commandsJson(Map<String, List<List<int>>> commands) => {
      for (final e in commands.entries) e.key: [for (final f in e.value) hex(f)],
    };

Map<String, Object?> linkSnapshot(DeviceState state) {
  final model = state.model;
  return {
    for (final ch in model.channels)
      ch: {
        'editGroup': DeviceOps.editGroupOf(state, ch),
        'master': DeviceOps.masterOf(state, ch),
        'slaves': DeviceOps.slavesOf(state, ch),
        'group': DeviceOps.linkGroupOf(state, ch),
        'candidates': DeviceOps.linkCandidates(state, ch),
      },
  };
}

Map<String, Object?> attempt(List<Map<String, Object?>> Function() run) {
  try {
    return {'ops': wire(run())};
  } on ArgumentError catch (e) {
    return {'error': '${e.message}'};
  }
}

Map<String, Object?> sequence(DeviceModel model, int length) {
  final state = DeviceState(model);
  final steps = <Object?>[];
  for (var i = 0; i < length; i++) {
    final op = wire(randomOp(model)) as Map<String, Object?>;
    final error = DeviceOps.validate(op, model);
    final step = <String, Object?>{'op': op, 'error': error};
    if (error == null) {
      final expanded = DeviceOps.expandLinked(state, op);
      step['expanded'] = wire(expanded);
      for (final o in expanded) {
        DeviceOps.apply(state, o);
      }
      final commands = DeviceOps.commands(state, op);
      step['commands'] = commandsJson(commands);
      step['describe'] = describeAll(commands, model);
    }
    if (i % 40 == 39) {
      step['state'] = wire(state.toJson());
      step['links'] = linkSnapshot(state);
      final master = pick(model.channels);
      final group = model.groupOf(master);
      final slaves = [for (final c in group) if (chance(30)) c];
      step['linkSlaves'] = {'master': master, 'slaves': slaves, ...attempt(() => DeviceOps.linkSlaves(state, master, slaves))};
      final leaving = pick(model.channels);
      step['unlink'] = {'ch': leaving, ...attempt(() => DeviceOps.unlink(state, leaving))};
      final from = pick(model.channels), to = pick(model.channels);
      step['copy'] = {'from': from, 'to': to, ...attempt(() => DeviceOps.copyChannel(state, from, to))};
    }
    steps.add(step);
  }
  return {
    'model': model.id,
    'steps': steps,
    'state': wire(state.toJson()),
    'fullState': wire(DeviceOps.fullState(state)),
    'dump': hex(ConfigDump(model).encode(state)),
  };
}

void main(List<String> args) {
  if (args.isEmpty) {
    stderr.writeln('usage: dart run bin/generate.dart <out.json>');
    exit(64);
  }
  rng = Random(408206);
  final out = <String, Object?>{};

  out['sequences'] = [
    for (var round = 0; round < 3; round++)
      for (final model in DeviceModel.all) sequence(model, 400),
  ];

  // Config dumps: captured streams with both layouts, plus truncated ones.
  final defaultStream = [for (final c in capturedConfigChunks()) ...c];
  final testStream = [for (final c in capturedTestPresetChunks()) ...c];
  out['dumps'] = [
    for (final stream in [defaultStream, testStream])
      for (final model in DeviceModel.all)
        for (final length in [stream.length, 900, 1408, 1430, 100])
          () {
            final bytes = stream.sublist(0, min(length, stream.length));
            final result = ConfigDump(model).decode(bytes);
            return {
              'model': model.id,
              'stream': hex(bytes),
              'problems': result.problems,
              'switchesRead': result.switchesRead,
              'restRead': result.restRead,
              'state': wire(result.state.toJson()),
              'encoded': hex(ConfigDump(model).encode(result.state)),
            };
          }(),
  ];

  // Device answers of the captured session: decoded messages and descriptions.
  out['rx'] = [
    for (final hexFrame in capturedSession408)
      () {
        final frame = hexToBytes(hexFrame);
        final rx = frame[2] == 0x01;
        return {
          'frame': hexFrame,
          'rx': rx,
          for (final model in DeviceModel.all) ...{
            'describe_${model.id}': rx
                ? PacketDescriber.describeRx(frame, model)
                : PacketDescriber.describe(frame, model),
            if (frame[5] == 0x40 && rx) ...{
              'meters_${model.id}': ProtocolService.parseModelMeters(frame, model),
              'dbfs_${model.id}': [
                for (final v in ProtocolService.parseModelMeters(frame, model)) ProtocolService.levelToDb(v),
              ],
              'dynamics': ProtocolService.parseDynamics(frame),
            },
          },
        };
      }(),
  ];

  // Value encodings.
  const p = ProtocolService();
  out['encodings'] = {
    'gain': [
      for (var i = -1400; i <= 400; i++)
        [i / 20, p.quantizeGain(i / 20), p.gainDbToValue(p.quantizeGain(i / 20))],
    ],
    'geq': [for (var i = -300; i <= 300; i++) [i / 20, p.quantizeGeqDb(i / 20), p.geqDbToValue(i / 20)]],
    'freq': [for (var raw = 0; raw <= 300; raw++) p.peqFreqToHz(raw)],
    'hz': [for (var i = 0; i <= 2000; i++) () {
      final hz = 15 * pow(1.0045, i).toDouble();
      return [hz, p.peqHzToFreq(hz)];
    }()],
    'q': [for (var raw = 0; raw <= 100; raw++) p.peqRawToQ(raw)],
    'qRaw': [for (var i = 1; i <= 1000; i++) [i / 7.0, p.peqQToRaw(i / 7.0)]],
    'delay': [for (var i = 0; i <= 2000; i++) [i * 0.37, ProtocolService.delayMsToRaw(i * 0.37)]],
    'threshold': [for (var i = -200; i <= 60; i++) [i / 2.0, ProtocolService.thresholdDbToRaw(i / 2.0)]],
    'channelName': [
      for (final name in ['', 'A', 'Sub L', 'TooLongName', 'Bässe', '\u{1F600}x'])
        [name, hex(ProtocolService.encodeChannelName(name))],
    ],
    'presetName': [
      for (final name in ['', 'Test', 'Fourteen Chars', 'Much longer than fourteen', 'Ümlaut'])
        [name, hex(p.buildStorePresetNameCommand(name))],
    ],
    'password': [
      for (final pw in ['', 'ab', 'ab12', 'abcdef', 'ßx'])
        [pw, hex(p.buildPasswordCommand(0, pw))],
    ],
  };

  File(args.first).writeAsStringSync(jsonEncode(out));
  stdout.writeln('wrote ${args.first}');
}
