// Config Dump
//
// The current configuration is read with "get config" requests (cmd 0x27,
// sub-index 0, 1, …); each 0x24 response carries a 50-byte chunk. Together
// the chunks form one stream: a header followed by one record per channel.
//
// Layout, decoded from DSP 408 dumps and confirmed for both models by the
// original editor (PROTOCOL-RE.md). All values are u16 little-endian unless
// marked u8.
//
//   Header (16):   flags(2) preset name(14 ASCII)
//   Input (140):   name(8) | gate: attack release hold threshold
//                  | GEQ 31 × value | PEQ 8 × [gain freq Q:u8 type:u8]
//                  | filter: hpfFreq lpfFreq hpfSlope:u8 lpfSlope:u8
//                  | tail: gain phase delay link
//   Output:        name(8) | routing mask | crossing gain × inputs
//                  | filter (6) | PEQ 9 × 6 | compressor: ratio attack
//                  release knee threshold | limiter: attack release ?
//                  threshold | tail: gain phase delay link
//   Switches:      mute bits of the inputs | mute bits of the outputs
//                  | PEQ bypass bits (bit = band), one word per channel byte
//   Rest:          unknown(4) | delay display unit | unknown(6)
//
// DSP 408: records end at 0x580, total 0x5A8 bytes (29 chunks).
// DSP 206: records end at 0x380, total 0x3A0 bytes (19 chunks).

import 'device_model.dart';
import 'device_state.dart';
import 'protocol_service.dart';

class ConfigDump {
  static const int chunkSize = 50;
  static const int headerLength = 16;
  static const int inputRecordLength = 140;

  final DeviceModel model;

  const ConfigDump(this.model);

  int get outputRecordLength => 8 + 2 + 2 * model.inputs.length + 6 + 9 * 6 + 10 + 8 + 8;

  /// Bytes needed to decode every channel record.
  int get requiredLength =>
      headerLength + model.inputs.length * inputRecordLength + model.outputs.length * outputRecordLength;

  /// Mute bits of the inputs; the outputs follow at +2.
  int get muteOffset => requiredLength;

  /// PEQ bypass words, one per channel byte (inputs from 0, outputs from
  /// the model's first output byte).
  int get peqBypassOffset => muteOffset + 4;

  int get _channelSlots => model.channelByte(model.outputs.last)! + 1;

  /// Bytes needed to decode mute and PEQ bypass as well.
  int get switchesLength => peqBypassOffset + 2 * _channelSlots;

  /// Full dump length as the original editor reads it.
  int get totalLength => switchesLength + 4 + 2 + 6;

  /// Number of 0x27 requests needed to read [totalLength] bytes.
  int get chunkCount => (totalLength + chunkSize - 1) ~/ chunkSize;

  int inputOffset(int index) => headerLength + index * inputRecordLength;

  int outputOffset(int index) =>
      headerLength + model.inputs.length * inputRecordLength + index * outputRecordLength;

  // ─── Decode ───

  /// Decode [stream] (all chunks concatenated in order) into a new state.
  /// Values that fail plausibility checks are listed in [ConfigDumpResult.problems]
  /// and left at their defaults.
  ConfigDumpResult decode(List<int> stream) {
    final state = DeviceState(model);
    final problems = <String>[];
    if (stream.length < requiredLength) {
      problems.add('dump too short: ${stream.length} of $requiredLength bytes');
      return ConfigDumpResult(state, problems);
    }
    const proto = ProtocolService();
    final r = _Reader(stream, problems);

    final presetName = r.ascii(2, 14);
    if (presetName != null && presetName.isNotEmpty) state.currentPreset = presetName;

    for (var i = 0; i < model.inputs.length; i++) {
      final ch = model.inputs[i];
      final o = inputOffset(i);
      _decodeName(r, state, ch, o);
      state.gates[ch] = GateState(
        attack: r.u16(o + 8, max: 9999, what: '$ch gate attack'),
        release: r.u16(o + 10, max: 9999, what: '$ch gate release'),
        hold: r.u16(o + 12, max: 9999, what: '$ch gate hold'),
        threshold: r.u16(o + 14, max: 400, what: '$ch gate threshold'),
      );
      state.geqBands[ch] = [
        for (var b = 0; b < geqBandCount; b++)
          proto.geqValueToDb(r.u16(o + 16 + b * 2, max: 240, fallback: 120, what: '$ch GEQ $b')),
      ];
      state.peqBands[ch] = [for (var b = 0; b < 8; b++) _decodePeq(r, o + 78 + b * 6, '$ch PEQ $b')];
      _decodeFilters(r, state, ch, o + 126);
      _decodeTail(r, state, ch, o + 132);
    }

    final xBase = 10;
    final filterBase = xBase + 2 * model.inputs.length;
    for (var j = 0; j < model.outputs.length; j++) {
      final ch = model.outputs[j];
      final o = outputOffset(j);
      _decodeName(r, state, ch, o);
      state.matrixRouting[ch] = r.u16(o + 8, max: model.matrixMask, what: '$ch routing');
      for (var i = 0; i < model.inputs.length; i++) {
        state.matrixGains['$ch:${model.inputs[i]}'] =
            proto.gainValueToDb(r.u16(o + xBase + i * 2, max: 400, fallback: 280, what: '$ch crossing gain'));
      }
      _decodeFilters(r, state, ch, o + filterBase);
      final peqBase = filterBase + 6;
      state.peqBands[ch] = [for (var b = 0; b < 9; b++) _decodePeq(r, o + peqBase + b * 6, '$ch PEQ $b')];
      final compBase = peqBase + 54;
      state.compressors[ch] = CompressorState(
        ratio: r.u16(compBase + o, max: ProtocolService.compressorRatioNames.length - 1, what: '$ch ratio'),
        attack: r.u16(compBase + o + 2, max: 9999, what: '$ch comp attack'),
        release: r.u16(compBase + o + 4, max: 9999, what: '$ch comp release'),
        knee: r.u16(compBase + o + 6, max: 60, what: '$ch comp knee'),
        threshold: r.u16(compBase + o + 8, max: 400, what: '$ch comp threshold'),
      );
      final limBase = compBase + 10;
      state.limiters[ch] = LimiterState(
        attack: r.u16(limBase + o, max: 9999, what: '$ch limiter attack'),
        release: r.u16(limBase + o + 2, max: 9999, what: '$ch limiter release'),
        unknown: r.u16(limBase + o + 4),
        threshold: r.u16(limBase + o + 6, max: 400, what: '$ch limiter threshold'),
      );
      _decodeTail(r, state, ch, limBase + o + 8);
    }

    // Mute and PEQ bypass follow the records; a shorter dump leaves them off.
    final switchesRead = stream.length >= switchesLength;
    if (switchesRead) {
      final inputMutes = r.u16(muteOffset);
      final outputMutes = r.u16(muteOffset + 2);
      for (var i = 0; i < model.inputs.length; i++) {
        state.mutes[model.inputs[i]] = inputMutes & (1 << i) != 0;
      }
      for (var j = 0; j < model.outputs.length; j++) {
        state.mutes[model.outputs[j]] = outputMutes & (1 << j) != 0;
      }
      for (final ch in model.channels) {
        final bits = r.u16(peqBypassOffset + 2 * model.channelByte(ch)!);
        final bands = state.peqBands[ch]!;
        for (var b = 0; b < bands.length; b++) {
          bands[b].bypass = bits & (1 << b) != 0;
        }
      }
    }

    return ConfigDumpResult(state, problems, switchesRead: switchesRead);
  }

  void _decodeName(_Reader r, DeviceState state, String ch, int offset) {
    final name = r.ascii(offset, 8, what: '$ch name');
    if (name != null) state.channelNames[ch] = name;
  }

  PeqBand _decodePeq(_Reader r, int o, String what) {
    const proto = ProtocolService();
    return PeqBand(
      gainDb: proto.peqGainToDb(r.u16(o, max: 240, fallback: 120, what: '$what gain')),
      freqRaw: r.u16(o + 2, max: ProtocolService.freqRawMax, fallback: 150, what: '$what freq'),
      qRaw: r.u8(o + 4, max: 255, fallback: 35, what: '$what Q'),
      type: r.u8(o + 5, max: ProtocolService.peqTypeNames.length - 1, what: '$what type'),
    );
  }

  void _decodeFilters(_Reader r, DeviceState state, String ch, int o) {
    const maxSlope = 20; // ProtocolService.crossoverSlopeNames.length - 1
    state.hiPass[ch] = FilterState(
      freqRaw: r.u16(o, max: ProtocolService.freqRawMax, what: '$ch HPF freq'),
      slope: r.u8(o + 4, max: maxSlope, what: '$ch HPF slope'),
    );
    state.loPass[ch] = FilterState(
      freqRaw: r.u16(o + 2, max: ProtocolService.freqRawMax, fallback: 300, what: '$ch LPF freq'),
      slope: r.u8(o + 5, max: maxSlope, what: '$ch LPF slope'),
    );
  }

  void _decodeTail(_Reader r, DeviceState state, String ch, int o) {
    state.gains[ch] = const ProtocolService().quantizeGain(
        const ProtocolService().gainValueToDb(r.u16(o, max: 400, fallback: 280, what: '$ch gain')));
    state.phase[ch] = r.u16(o + 2, max: 1, what: '$ch phase') == 1;
    state.delays[ch] = r.u16(o + 4, max: ProtocolService.delayRawMax, what: '$ch delay');
    state.links[ch] = r.u16(o + 6, max: 0xFF, what: '$ch link');
  }

  // ─── Encode ───

  /// Encode [state] as a dump stream of [totalLength] bytes. Fields that are
  /// not decoded (limiter "?", the rest after the switches) are written as
  /// zero.
  List<int> encode(DeviceState state) {
    const proto = ProtocolService();
    final out = List<int>.filled(totalLength, 0);
    void u16(int at, int v) {
      out[at] = v & 0xFF;
      out[at + 1] = (v >> 8) & 0xFF;
    }
    void name(int at, String ch) {
      final bytes = ProtocolService.encodeChannelName(state.channelNames[ch] ?? model.defaultDeviceName(ch));
      out.setRange(at, at + 8, bytes);
    }
    void peq(int at, PeqBand b) {
      u16(at, proto.peqDbToGain(b.gainDb));
      u16(at + 2, b.freqRaw);
      out[at + 4] = b.qRaw;
      out[at + 5] = b.type;
    }
    void filters(int at, String ch) {
      u16(at, state.hiPass[ch]!.freqRaw);
      u16(at + 2, state.loPass[ch]!.freqRaw);
      out[at + 4] = state.hiPass[ch]!.slope;
      out[at + 5] = state.loPass[ch]!.slope;
    }
    void tail(int at, String ch) {
      u16(at, proto.gainDbToValue(state.gains[ch]!));
      u16(at + 2, state.phase[ch]! ? 1 : 0);
      u16(at + 4, state.delays[ch]!);
      u16(at + 6, state.links[ch]!);
    }
    int bits(Iterable<bool> flags) {
      var mask = 0, bit = 1;
      for (final on in flags) {
        if (on) mask |= bit;
        bit <<= 1;
      }
      return mask;
    }

    out[0] = 0xFF;
    out[1] = 0xFF;
    final preset = state.currentPreset == 'Unknown' ? '' : state.currentPreset;
    out.setRange(2, 16, preset.padRight(14).substring(0, 14).codeUnits.map((c) => c & 0x7F));

    for (var i = 0; i < model.inputs.length; i++) {
      final ch = model.inputs[i];
      final o = inputOffset(i);
      name(o, ch);
      final g = state.gates[ch]!;
      u16(o + 8, g.attack);
      u16(o + 10, g.release);
      u16(o + 12, g.hold);
      u16(o + 14, g.threshold);
      for (var b = 0; b < geqBandCount; b++) {
        u16(o + 16 + b * 2, proto.geqDbToValue(state.geqBands[ch]![b]));
      }
      for (var b = 0; b < 8; b++) {
        peq(o + 78 + b * 6, state.peqBands[ch]![b]);
      }
      filters(o + 126, ch);
      tail(o + 132, ch);
    }

    final filterBase = 10 + 2 * model.inputs.length;
    for (var j = 0; j < model.outputs.length; j++) {
      final ch = model.outputs[j];
      final o = outputOffset(j);
      name(o, ch);
      u16(o + 8, state.matrixRouting[ch]!);
      for (var i = 0; i < model.inputs.length; i++) {
        u16(o + 10 + i * 2, proto.gainDbToValue(state.matrixGains['$ch:${model.inputs[i]}'] ?? 0));
      }
      filters(o + filterBase, ch);
      final peqBase = o + filterBase + 6;
      for (var b = 0; b < 9; b++) {
        peq(peqBase + b * 6, state.peqBands[ch]![b]);
      }
      final c = state.compressors[ch]!;
      final compBase = peqBase + 54;
      u16(compBase, c.ratio);
      u16(compBase + 2, c.attack);
      u16(compBase + 4, c.release);
      u16(compBase + 6, c.knee);
      u16(compBase + 8, c.threshold);
      final l = state.limiters[ch]!;
      u16(compBase + 10, l.attack);
      u16(compBase + 12, l.release);
      u16(compBase + 14, l.unknown);
      u16(compBase + 16, l.threshold);
      tail(compBase + 18, ch);
    }

    u16(muteOffset, bits(model.inputs.map((ch) => state.mutes[ch]!)));
    u16(muteOffset + 2, bits(model.outputs.map((ch) => state.mutes[ch]!)));
    for (final ch in model.channels) {
      u16(peqBypassOffset + 2 * model.channelByte(ch)!, bits(state.peqBands[ch]!.map((b) => b.bypass)));
    }
    return out;
  }

  /// Split a dump stream into the chunks the device sends.
  static List<List<int>> split(List<int> stream) => [
        for (var i = 0; i < stream.length; i += chunkSize)
          stream.sublist(i, (i + chunkSize).clamp(0, stream.length)),
      ];
}

class ConfigDumpResult {
  final DeviceState state;

  /// Values that were out of range; empty if the dump looks plausible.
  final List<String> problems;

  /// False if the dump ended before the mute and PEQ bypass bits.
  final bool switchesRead;

  ConfigDumpResult(this.state, this.problems, {this.switchesRead = false});

  bool get isPlausible => problems.isEmpty;
}

/// Bounds-checked reader that records implausible values.
class _Reader {
  final List<int> data;
  final List<String> problems;

  _Reader(this.data, this.problems);

  int u8(int at, {int max = 0xFF, int fallback = 0, String? what}) =>
      _check(data[at], max, fallback, what);

  int u16(int at, {int max = 0xFFFF, int fallback = 0, String? what}) =>
      _check(data[at] | (data[at + 1] << 8), max, fallback, what);

  int _check(int value, int max, int fallback, String? what) {
    if (value <= max) return value;
    problems.add('${what ?? 'value'} = $value (max $max)');
    return fallback;
  }

  /// Printable ASCII up to the first zero, or null if the bytes are not text.
  String? ascii(int at, int length, {String? what}) {
    final bytes = data.sublist(at, at + length);
    final end = bytes.indexOf(0);
    final text = end < 0 ? bytes : bytes.sublist(0, end);
    if (text.any((b) => b < 0x20 || b > 0x7E) || bytes.skip(text.length).any((b) => b != 0)) {
      if (what != null) problems.add('$what is not text');
      return null;
    }
    return String.fromCharCodes(text).trimRight();
  }
}
