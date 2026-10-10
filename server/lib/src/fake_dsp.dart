// Fake DSP 206 / 408
//
// A TCP server that answers the DSP protocol well enough to develop and test
// the panel without hardware. It keeps a DeviceState, applies every set
// command to it, stores and recalls presets, and serves the config dump from
// that state (for the DSP 408 the initial state is the dump captured from a
// real device). Startup answers mirror the captured DSP 408 session.

import 'dart:async';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:dsp_core/dsp_core.dart';
import 'package:dsp_core/testing.dart';

class FakeDsp {
  final DeviceModel model;

  /// Artificial response delay.
  final Duration latency;

  /// Print received set commands.
  final bool verbose;

  /// Label used in log output.
  final String label;

  /// Do not answer these command bytes (to test timeouts).
  final Set<int> ignoreCommands = {};

  /// Every frame received from the host, in order.
  final List<List<int>> received = [];

  /// Current settings of the simulated device.
  late DeviceState state;

  /// Stored presets by 0-based slot.
  final Map<int, DeviceState> _presets = {};
  final _random = Random(42);
  final List<Socket> _clients = [];
  ServerSocket? _server;
  String _pendingPresetName = '';

  late final ProtocolService _proto = ProtocolService(model);
  late final ConfigDump _dump = ConfigDump(model);

  FakeDsp({
    this.model = DeviceModel.dsp408,
    this.latency = Duration.zero,
    this.verbose = false,
    String? label,
  }) : label = label ?? 'Fake ${model.name}' {
    state = _initialState();
    for (var i = 0; i < TRacksProto.presetCount; i++) {
      final preset = _initialState();
      preset.currentPreset = i == 0 ? state.currentPreset : 'Preset ${i + 1}';
      _presets[i] = preset;
    }
  }

  DeviceState _initialState() {
    if (model == DeviceModel.dsp408) {
      return _dump.decode([for (final c in capturedConfigChunks()) ...c]).state;
    }
    return DeviceState(model)..currentPreset = 'Default Preset';
  }

  String get deviceInfo => '${model.name.replaceAll(' ', '')} V0104P';

  int get port => _server!.port;
  int get connectionCount => _clients.length;

  Future<void> start({int port = 0, InternetAddress? address}) async {
    _server = await ServerSocket.bind(address ?? InternetAddress.loopbackIPv4, port);
    _server!.listen(_accept);
  }

  Future<void> stop() async {
    await _server?.close();
    _server = null;
    dropConnections();
  }

  /// Close all host connections (simulates a cable pull / device reboot).
  void dropConnections() {
    for (final c in List.of(_clients)) {
      c.destroy();
    }
    _clients.clear();
  }

  /// Set commands (gain, mute, …) received so far, decoded for display.
  List<String> get receivedSetCommands => [
        for (final f in received)
          if (_isSetCommand(f[5])) PacketDescriber.describe(f, model),
      ];

  static bool _isSetCommand(int cmd) => const {
        0x20, 0x21, 0x26, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x38, 0x3a, 0x3b, 0x3d, 0x3e, 0x3f, 0x41,
        0x48,
      }.contains(cmd);

  void _accept(Socket socket) {
    _clients.add(socket);
    final decoder = FrameDecoder();
    socket.listen(
      (data) {
        for (final frame in decoder.add(data)) {
          _handle(socket, frame);
        }
      },
      onDone: () => _clients.remove(socket),
      onError: (_) => _clients.remove(socket),
      cancelOnError: true,
    );
  }

  void _handle(Socket socket, List<int> frame) {
    received.add(frame);
    final cmd = frame[5];
    final args = frame.sublist(6, 5 + frame[4]);
    if (verbose && _isSetCommand(cmd)) {
      stdout.writeln('[$label] ${PacketDescriber.describe(frame, model)}');
    }
    if (ignoreCommands.contains(cmd)) return;

    final arg = args.isNotEmpty ? args[0] : 0;
    final List<int> response = switch (cmd) {
      0x10 => _response(0x10, [0x13]),
      0x13 => _response(0x13, deviceInfo.codeUnits),
      0x2c => _response(0x2c, [0x00, 0x27, 0x0f, 0x00, 0x00, 0x00]),
      0x22 => _response(0x22, [0xff, 0xff, 0xff, 0xff, ...List.filled(18, 0), 0xff, 0xff]),
      0x14 => _response(0x14, [0x14]),
      0x29 => _response(0x29, [arg, ..._padded(_presets[arg]?.currentPreset ?? '', 14)]),
      0x27 => _configChunk(arg),
      0x40 => _response(0x40, _meterPayload()),
      _ => _apply(cmd, args),
    };

    if (latency == Duration.zero) {
      _send(socket, response);
    } else {
      Timer(latency, () => _send(socket, response));
    }
  }

  void _send(Socket socket, List<int> data) {
    if (_clients.contains(socket)) socket.add(data);
  }

  List<int> _configChunk(int index) {
    final chunks = ConfigDump.split(_dump.encode(state));
    if (index >= chunks.length) return _ack();
    return _response(0x24, [index, ...chunks[index]]);
  }

  /// Apply a set command to the simulated state; answers with an ACK.
  List<int> _apply(int cmd, List<int> a) {
    int u16(int at) => a[at] | (a[at + 1] << 8);
    String? ch() => a.isEmpty ? null : model.channelForByte(a[0]);
    final c = ch();

    switch (cmd) {
      case 0x34 when c != null:
        state.gains[c] = _proto.gainValueToDb(u16(1));
      case 0x35 when c != null:
        state.mutes[c] = a[1] == 1;
      case 0x36 when c != null:
        state.phase[c] = a[1] == 1;
      case 0x38 when c != null:
        state.delays[c] = u16(1);
      case 0x3a when c != null:
        state.matrixRouting[c] = a[1];
      case 0x41 when c != null && a.length >= 4 && a[1] < model.inputs.length:
        state.matrixGains['$c:${model.inputs[a[1]]}'] = _proto.gainValueToDb(u16(2));
      case 0x48 when c != null && state.geqBands.containsKey(c):
        state.geqBands[c]![a[1]] = _proto.geqValueToDb(u16(2));
      case 0x33 when c != null:
        state.peqBands[c]![a[1]] = PeqBand(
          gainDb: _proto.peqGainToDb(u16(2)),
          freqRaw: u16(4),
          qRaw: a[6],
          type: a[7],
          bypass: a[8] == 1,
        );
      case 0x32 when c != null:
        state.hiPass[c] = FilterState(freqRaw: u16(1), slope: a[3]);
      case 0x31 when c != null:
        state.loPass[c] = FilterState(freqRaw: u16(1), slope: a[3]);
      case 0x30 when c != null:
        state.compressors[c] = CompressorState(
            ratio: u16(1), attack: u16(3), release: u16(5), knee: u16(7), threshold: u16(9));
      case 0x3e when c != null:
        state.gates[c] = GateState(attack: u16(1), release: u16(3), hold: u16(5), threshold: u16(7));
      case 0x3f when c != null:
        state.limiters[c] = LimiterState(attack: u16(1), release: u16(3), unknown: u16(5), threshold: u16(7));
      case 0x3b when c != null:
        state.links[c] = a[1];
      case 0x3d when c != null:
        state.channelNames[c] = String.fromCharCodes(a.sublist(1, 9).where((b) => b != 0));
      case 0x26:
        _pendingPresetName = String.fromCharCodes(a).trim();
      case 0x21:
        final slot = a[0] - 1;
        state.currentPreset = _pendingPresetName;
        _presets[slot] = _copy(state);
      case 0x20:
        final stored = _presets[a[0] - 1];
        if (stored != null) state = _copy(stored);
    }
    return _ack();
  }

  DeviceState _copy(DeviceState s) => DeviceState.fromJson(s.toJson());

  static List<int> _padded(String text, int length) =>
      text.padRight(length).substring(0, length).codeUnits;

  static List<int> _ack() => _response(0x01, const []);

  /// Device → host frame.
  static List<int> _response(int cmd, List<int> payload) {
    final dataBytes = [0x01, 0x00, payload.length + 1, cmd, ...payload];
    return [0x10, 0x02, ...dataBytes, 0x10, 0x03, ProtocolService.calculateChecksum(dataBytes)];
  }

  List<int> _meterPayload() {
    final t = DateTime.now().millisecondsSinceEpoch / 1000.0;
    final payload = <int>[];
    var dynamics = 0;
    // Like the hardware, both models answer with 12 slots: four input slots
    // (unused ones are zero on the DSP 206), then the outputs.
    final channels = [
      for (var i = 0; i < 4; i++) i < model.inputs.length ? model.inputs[i] : null,
      for (var o = 0; o < 8; o++) o < model.outputs.length ? model.outputs[o] : null,
    ];
    for (var i = 0; i < channels.length; i++) {
      final ch = channels[i];
      if (ch == null) {
        payload.addAll([0, 0, 0]);
        continue;
      }
      // Slowly pulsing levels (about -40 … -6 dBFS) with a little noise,
      // different per channel, shifted by the channel gain; muted channels
      // stay quiet.
      final db = -23 + 15 * sin(t * (0.7 + i * 0.13) + i) + _random.nextDouble() * 3 + (state.gains[ch] ?? 0);
      final level = state.mutes[ch] == true ? 0.0 : pow(10, db / 20).toDouble();
      payload.addAll(_meterBytes(level));
      final o = model.outputs.indexOf(ch);
      if (o >= 0 && level > 0) {
        final comp = state.compressors[ch]!;
        if (comp.ratio != 0 && db > ProtocolService.thresholdRawToDb(comp.threshold)) dynamics |= 1 << o;
        if (db > ProtocolService.thresholdRawToDb(state.limiters[ch]!.threshold)) dynamics |= 0x100 << o;
      }
    }
    return [...payload, dynamics & 0xFF, dynamics >> 8];
  }

  /// The upper three bytes of [level] as float32, in the order the DSP
  /// sends them (see ProtocolService.decodeMeterValue).
  static List<int> _meterBytes(double level) {
    final bits = (ByteData(4)..setFloat32(0, level)).getUint32(0);
    return [(bits >> 16) & 0xFF, bits >> 24, (bits >> 8) & 0xFF];
  }
}
