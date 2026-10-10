// Packet Describer
//
// Human-readable descriptions of protocol frames for the debug log.

import 'device_model.dart';
import 'protocol_service.dart';

class PacketDescriber {
  // 31-band ISO 1/3-octave center frequencies for GEQ decode
  static const _geqFreqs = [
    '20', '25', '31.5', '40', '50', '63', '80', '100', '125', '160',
    '200', '250', '315', '400', '500', '630', '800', '1K', '1.25K', '1.6K',
    '2K', '2.5K', '3.15K', '4K', '5K', '6.3K', '8K', '10K', '12.5K', '16K', '20K',
  ];

  static String _db(double dB) =>
      dB >= 0 ? '+${dB.toStringAsFixed(1)}' : dB.toStringAsFixed(1);

  static String _hz(double hz) =>
      hz >= 1000 ? '${(hz / 1000).toStringAsFixed(hz >= 10000 ? 1 : 2)} kHz' : '${hz.toStringAsFixed(0)} Hz';

  static int _u16(List<int> d, int at) => d[at] | (d[at + 1] << 8);

  static String _slope(int s) =>
      s < ProtocolService.crossoverSlopeNames.length ? ProtocolService.crossoverSlopeNames[s] : '$s';

  /// Decode an Rx frame using the parsed ProtocolMessage for richer output
  static String describeRx(List<int> data, [DeviceModel model = DeviceModel.dsp408]) {
    final message = ProtocolService(model).decode(data);
    if (message is PresetMessage) {
      final label = 'U${(message.index + 1).toString().padLeft(2, '0')}';
      return 'PRESET $label: "${message.name}"';
    }
    if (message is ChannelConfigMessage) {
      return 'CONFIG chunk 0x${message.subIndex.toRadixString(16).padLeft(2, '0')} (${message.chunkData.length} bytes)';
    }
    if (message is DeviceInfoMessage) {
      return 'DEVICE: ${message.deviceName}';
    }
    if (message is KeepaliveMessage) {
      final channels = model.channels;
      final parts = <String>[];
      for (int i = 0; i < message.meterLevels.length && i < channels.length; i++) {
        final db = ProtocolService.levelToDb(message.meterLevels[i]);
        if (db > -100) parts.add('${channels[i]}:${db.toStringAsFixed(1)}');
      }
      final dynamics = [
        for (var o = 0; o < model.outputs.length; o++) ...[
          if (ProtocolService.compressorActive(message.dynamics, o)) '${model.outputs[o]}:COMP',
          if (ProtocolService.limiterActive(message.dynamics, o)) '${model.outputs[o]}:LIMIT',
        ],
      ];
      return 'METERS dBFS ${parts.isEmpty ? "(silence)" : parts.join(" ")}'
          '${dynamics.isEmpty ? '' : ' ${dynamics.join(' ')}'}';
    }
    if (message is PresetCountMessage) {
      return 'PRESET COUNT: ${message.count}';
    }
    if (data.length > 5 && data[5] == 0x01) return 'ACK';
    // Fallback to generic decode
    return describe(data, model);
  }

  /// Decode a raw protocol frame into a human-readable description
  static String describe(List<int> data, [DeviceModel model = DeviceModel.dsp408]) {
    if (data.length < 6 || data[0] != 0x10 || data[1] != 0x02) {
      return 'UNKNOWN (${data.length} bytes)';
    }
    const proto = ProtocolService();
    final cmdByte = data[5];
    String ch() => data.length > 6
        ? (model.channelForByte(data[6]) ?? 'ch 0x${data[6].toRadixString(16)}')
        : '?';

    switch (cmdByte) {
      case 0x10: // Handshake
        return 'HANDSHAKE';
      case 0x40:
        return 'KEEPALIVE → request meter levels';
      case 0x12:
        return 'STATUS QUERY';
      case 0x13:
        return 'GET DEVICE INFO';
      case 0x2c:
        return 'GET PRESET COUNT';
      case 0x20:
        if (data.length > 6) return 'LOAD PRESET U${data[6].toString().padLeft(2, '0')}';
        return 'LOAD PRESET';
      case 0x21:
        if (data.length > 6) return 'STORE PRESET → U${data[6].toString().padLeft(2, '0')}';
        return 'STORE PRESET';
      case 0x26:
        return 'STORE PRESET NAME';
      case 0x29:
        if (data.length > 6) {
          return 'GET PRESET U${(data[6] + 1).toString().padLeft(2, '0')} → request name';
        }
        return 'GET PRESET';
      case 0x27:
        if (data.length > 6) {
          return 'GET CONFIG CHUNK 0x${data[6].toRadixString(16).padLeft(2, '0')}';
        }
        return 'GET CONFIG';
      case 0x34: // Gain: ch value(2)
        if (data.length > 8) return 'SET GAIN ${ch()} → ${_db(proto.gainValueToDb(_u16(data, 7)))} dB';
        return 'SET GAIN';
      case 0x35: // Mute
        if (data.length > 7) return 'SET MUTE ${ch()} → ${data[7] == 0x01 ? "MUTED" : "UNMUTED"}';
        return 'SET MUTE';
      case 0x36: // Phase
        if (data.length > 7) return 'SET PHASE ${ch()} → ${data[7] == 0x01 ? "INVERTED" : "NORMAL"}';
        return 'SET PHASE';
      case 0x38: // Delay: ch value(2)
        if (data.length > 8) {
          return 'SET DELAY ${ch()} → ${ProtocolService.delayRawToMs(_u16(data, 7)).toStringAsFixed(2)} ms';
        }
        return 'SET DELAY';
      case 0x48: // GEQ band: ch band value(2)
        if (data.length > 8) {
          final band = data[7];
          final freq = band < _geqFreqs.length ? _geqFreqs[band] : '#$band';
          return 'SET GEQ ${ch()} ${freq}Hz → ${_db(proto.geqValueToDb(data[8]))} dB';
        }
        return 'SET GEQ';
      case 0x33: // PEQ: ch band gain(2) freq(2) Q type bypass
        if (data.length > 14) {
          final type = data[13] < ProtocolService.peqTypeNames.length
              ? ProtocolService.peqTypeNames[data[13]]
              : '${data[13]}';
          final bypass = data[14] == 1 ? ' [BYPASS]' : '';
          return 'SET PEQ ${ch()} band ${data[7] + 1} → ${_db(proto.peqGainToDb(_u16(data, 8)))} dB '
              '${_hz(proto.peqFreqToHz(_u16(data, 10)))} Q ${proto.peqRawToQ(data[12]).toStringAsFixed(2)} '
              '$type$bypass';
        }
        return 'SET PEQ';
      case 0x32: // Hi Pass: ch freq(2) slope
      case 0x31: // Lo Pass: ch freq(2) slope
        final name = cmdByte == 0x32 ? 'HPF' : 'LPF';
        if (data.length > 9) {
          final slope = data[9];
          return slope == 0
              ? 'SET $name ${ch()} → BYPASS'
              : 'SET $name ${ch()} → ${_hz(proto.peqFreqToHz(_u16(data, 7)))} ${_slope(slope)}';
        }
        return 'SET $name';
      case 0x30: // Compressor: ch ratio attack release knee threshold
        if (data.length > 16) {
          final ratio = _u16(data, 7);
          final ratioName = ratio < ProtocolService.compressorRatioNames.length
              ? ProtocolService.compressorRatioNames[ratio]
              : '$ratio';
          return 'SET COMPRESSOR ${ch()} → $ratioName, '
              '${_db(ProtocolService.thresholdRawToDb(_u16(data, 15)))} dB, '
              '${ProtocolService.timeRawToMs(_u16(data, 9))}/${ProtocolService.timeRawToMs(_u16(data, 11))} ms, '
              'knee ${_u16(data, 13)} dB';
        }
        return 'SET COMPRESSOR';
      case 0x3E: // Gate: ch attack release hold threshold
        if (data.length > 14) {
          return 'SET GATE ${ch()} → ${_db(ProtocolService.thresholdRawToDb(_u16(data, 13)))} dB, '
              'attack/release/hold ${ProtocolService.timeRawToMs(_u16(data, 7))}/'
              '${ProtocolService.timeRawToMs(_u16(data, 9))}/${ProtocolService.timeRawToMs(_u16(data, 11))} ms';
        }
        return 'SET GATE';
      case 0x3F: // Limiter: ch attack release ? threshold
        if (data.length > 14) {
          return 'SET LIMITER ${ch()} → ${_db(ProtocolService.thresholdRawToDb(_u16(data, 13)))} dB, '
              '${ProtocolService.timeRawToMs(_u16(data, 7))}/${ProtocolService.timeRawToMs(_u16(data, 9))} ms';
        }
        return 'SET LIMITER';
      case 0x39: // Test tone: type param
        if (data.length > 8) {
          final type = data[6];
          final name = type < ProtocolService.testToneTypeNames.length ? ProtocolService.testToneTypeNames[type] : '?';
          return 'SET TEST TONE → $name${type == 3 ? ' (frequency index ${data[7]})' : ''}';
        }
        return 'SET TEST TONE';
      case 0x3B: // Link: ch mask
        if (data.length > 7) {
          final group = model.groupOf(ch());
          final linked = [
            for (final c in group)
              if (data[7] & model.linkBit(c) != 0) c,
          ];
          return 'SET LINK ${ch()} → ${linked.join(' + ')}';
        }
        return 'SET LINK';
      case 0x3D: // Channel name
        if (data.length > 14) {
          return 'SET NAME ${ch()} → "${String.fromCharCodes(data.sublist(7, 15).where((b) => b != 0))}"';
        }
        return 'SET NAME';
      case 0x3a: // Matrix routing
        if (data.length > 7) {
          final mask = data[7];
          final inputs = [
            for (final input in model.inputs)
              if (mask & model.matrixBit(input) != 0) input,
          ];
          return 'SET MATRIX ${ch()} ← ${inputs.isEmpty ? "none" : inputs.join(" + ")}';
        }
        return 'SET MATRIX';
      case 0x41: // Matrix crossing gain: output input value(2)
        if (data.length > 9) {
          final input = data[7] < model.inputs.length ? model.inputs[data[7]] : 'input ${data[7]}';
          return 'SET MATRIX GAIN ${ch()} ← $input → ${_db(proto.gainValueToDb(_u16(data, 8)))} dB';
        }
        return 'SET MATRIX GAIN';
      case 0x24: // Config chunk response
        if (data.length > 6) {
          return 'CONFIG CHUNK 0x${data[6].toRadixString(16).padLeft(2, '0')} (${data.length} bytes)';
        }
        return 'CONFIG CHUNK';
      default:
        return 'CMD 0x${cmdByte.toRadixString(16).padLeft(2, '0')} (${data.length} bytes)';
    }
  }
}
