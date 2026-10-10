// Inspect a captured DSP session (helper for the tasks in TODO.md).
//
//   dart run tool/inspect_capture.dart <capture.txt> [--model dsp206]
//
// The capture file has one frame per line as hex, optionally preceded by a
// line number ("103\t10020100342400…"), e.g. exported from Wireshark with
// "Copy as hex stream". The tool prints every frame decoded and, if the
// capture contains a config dump, the decoded channel records and every
// value the decoder considered implausible.

import 'dart:io';

import 'package:dsp_core/dsp_core.dart';

void main(List<String> args) {
  if (args.isEmpty) {
    stderr.writeln('usage: dart run tool/inspect_capture.dart <capture.txt> [--model dsp206]');
    exit(64);
  }
  final modelIndex = args.indexOf('--model');
  var model = modelIndex >= 0 ? DeviceModel.byId(args[modelIndex + 1]) : null;

  final chunks = <int, List<int>>{};
  for (final line in File(args.first).readAsLinesSync()) {
    final hex = line.trim().split(RegExp(r'\s+')).last.toLowerCase();
    if (!hex.startsWith('1002') || hex.length.isOdd) continue;
    final bytes = [for (var i = 0; i < hex.length; i += 2) int.parse(hex.substring(i, i + 2), radix: 16)];
    final valid = FrameDecoder().add(bytes).length == 1;
    final rx = bytes[2] == 0x01;
    final message = ProtocolService(model ?? DeviceModel.dsp408).decode(bytes);
    if (message is DeviceInfoMessage) model ??= DeviceModel.detect(message.deviceName);
    if (message is ChannelConfigMessage) chunks[message.subIndex] = message.chunkData;
    final desc = rx
        ? PacketDescriber.describeRx(bytes, model ?? DeviceModel.dsp408)
        : PacketDescriber.describe(bytes, model ?? DeviceModel.dsp408);
    stdout.writeln('${rx ? 'RX' : 'TX'} ${valid ? ' ' : '!'} $desc');
  }

  if (chunks.isEmpty) return;
  model ??= DeviceModel.dsp408;
  final stream = [for (var i = 0; chunks.containsKey(i); i++) ...chunks[i]!];
  final dump = ConfigDump(model);
  stdout.writeln('\nConfig dump: ${chunks.length} chunks, ${stream.length} bytes '
      '(${model.name} layout expects ${dump.totalLength})');
  final result = dump.decode(stream);
  final s = result.state;
  const p = ProtocolService();
  String hz(int raw) => p.peqFreqToHz(raw).toStringAsFixed(0);
  stdout.writeln('Preset: "${s.currentPreset}"');
  for (final ch in model.channels) {
    final hp = s.hiPass[ch]!, lp = s.loPass[ch]!;
    stdout.writeln('${ch.padRight(6)} name="${s.channelNames[ch]}" gain=${s.gains[ch]} dB '
        '${s.mutes[ch]! ? 'MUTE ' : ''}${s.phase[ch]! ? 'PHASE ' : ''}'
        'delay=${ProtocolService.delayRawToMs(s.delays[ch]!).toStringAsFixed(2)} ms '
        'HPF ${hz(hp.freqRaw)} Hz ${ProtocolService.crossoverSlopeNames[hp.slope]} '
        'LPF ${hz(lp.freqRaw)} Hz ${ProtocolService.crossoverSlopeNames[lp.slope]} '
        '${model.isOutput(ch) ? 'routing=0x${s.matrixRouting[ch]!.toRadixString(16)}' : ''}');
  }
  if (!result.switchesRead) stdout.writeln('\nDump ends before the mute/PEQ bypass bits.');
  if (result.isPlausible) {
    stdout.writeln('\nAll values plausible.');
  } else {
    stdout.writeln('\nImplausible values (layout probably differs):');
    for (final problem in result.problems.take(40)) {
      stdout.writeln('  $problem');
    }
  }

  // Raw words around the record tails, to locate unknown fields.
  stdout.writeln('\nRecord tails [gain phase delay link]:');
  for (var i = 0; i < model.inputs.length; i++) {
    _tail(stream, dump.inputOffset(i) + 132, model.inputs[i]);
  }
  for (var j = 0; j < model.outputs.length; j++) {
    _tail(stream, dump.outputOffset(j) + dump.outputRecordLength - 8, model.outputs[j]);
  }
}

void _tail(List<int> s, int at, String ch) {
  if (at + 8 > s.length) return;
  final words = [for (var k = 0; k < 8; k += 2) s[at + k] | (s[at + k + 1] << 8)];
  stdout.writeln('  ${ch.padRight(6)} $words');
}
