import 'dart:typed_data';

import 'package:dsp_core/dsp_core.dart';
import 'package:test/test.dart';

/// Gain frame as built by the original app (protocol_service.dart before the
/// move), used as golden reference.
List<int> legacyGainFrame(int channel, double dB) {
  final q = dB < -20.0 ? (dB * 2).round() / 2 : (dB * 10).round() / 10;
  final value = q < -20.0 ? ((q + 60) * 2).round() : (80 + (q + 20) * 10).round();
  final dataBytes = [0x00, 0x01, 0x04, 0x34, channel, value & 0xFF, (value >> 8) & 0xFF];
  var checksum = 1;
  for (final b in dataBytes) {
    checksum ^= b;
  }
  return [0x10, 0x02, ...dataBytes, 0x10, 0x03, checksum];
}

/// Host → device frame for [payload] (command byte first).
List<int> frame(List<int> payload) {
  final data = [0x00, 0x01, payload.length, ...payload];
  return [0x10, 0x02, ...data, 0x10, 0x03, ProtocolService.calculateChecksum(data)];
}

void main() {
  const p408 = ProtocolService(DeviceModel.dsp408);
  const p206 = ProtocolService(DeviceModel.dsp206);

  group('command builders match the original byte sequences (DSP 408)', () {
    test('mute (hard-coded table in the original app)', () {
      expect(p408.buildMuteCommand('In A', true),
          [0x10, 0x02, 0x00, 0x01, 0x03, 0x35, 0x00, 0x01, 0x10, 0x03, 0x37]);
      expect(p408.buildMuteCommand('In D', false),
          [0x10, 0x02, 0x00, 0x01, 0x03, 0x35, 0x03, 0x00, 0x10, 0x03, 0x35]);
      expect(p408.buildMuteCommand('Out 5', true),
          [0x10, 0x02, 0x00, 0x01, 0x03, 0x35, 0x08, 0x01, 0x10, 0x03, 0x3F]);
      expect(p408.buildMuteCommand('Out 8', false),
          [0x10, 0x02, 0x00, 0x01, 0x03, 0x35, 0x0B, 0x00, 0x10, 0x03, 0x3D]);
    });

    test('keepalive', () {
      expect(p408.buildKeepaliveCommand(), [0x10, 0x02, 0x00, 0x01, 0x01, 0x40, 0x10, 0x03, 0x41]);
    });

    test('captured request constants equal the generic frame builder', () {
      for (var i = 0; i < 20; i++) {
        expect(ProtocolService.buildFrame(0x29, [i]), TRacksProto.loadAllPresetsCommands[i]);
      }
      for (var i = 0; i < 29; i++) {
        expect(TRacksProto.configDumpCommand(i), TRacksProto.configDumpCommands[i]);
      }
      expect(ProtocolService.buildFrame(0x10), TRacksProto.handshakeCommand);
      expect(ProtocolService.buildFrame(0x13), TRacksProto.requestDeviceInfo);
      expect(ProtocolService.buildFrame(0x2c), TRacksProto.requestLockStatus);
      expect(ProtocolService.buildFrame(0x22), TRacksProto.requestPresetSlots);
      expect(ProtocolService.buildFrame(0x14), TRacksProto.requestCurrentPreset);
      expect(ProtocolService.buildHeartbeatCommand(), TRacksProto.statusQueryCommands.first);
      expect(ProtocolService.buildFrame(0x12), TRacksProto.statusQueryCommands.first);
    });

    test('gain over the full range', () {
      for (var tenth = -600; tenth <= 120; tenth += 5) {
        final dB = tenth / 10;
        for (var ch = 0; ch < DeviceModel.dsp408.channels.length; ch++) {
          expect(p408.buildGainCommand(DeviceModel.dsp408.channels[ch], dB), legacyGainFrame(ch, dB),
              reason: '${DeviceModel.dsp408.channels[ch]} $dB dB');
        }
      }
    });

    test('load and store preset', () {
      expect(p408.buildLoadPresetCommand(0), [0x10, 0x02, 0x00, 0x01, 0x02, 0x20, 0x01, 0x10, 0x03, 0x23]);
      final name = p408.buildStorePresetNameCommand('Test');
      expect(name.length, 14 + 9);
      expect(name[4], 0x0f);
      expect(String.fromCharCodes(name.sublist(6, 20)), 'Test          ');
      expect(p408.buildStorePresetNameCommand('Ümlaut')[6], 0x3F);
    });
  });

  group('command layouts documented for the DSP 206 (FlexiFlizz)', () {
    test('channel bytes: outputs follow the inputs directly', () {
      expect(p206.buildMuteCommand('In B', true), frame([0x35, 0x01, 0x01]));
      expect(p206.buildMuteCommand('Out 1', true), frame([0x35, 0x02, 0x01]));
      expect(p206.buildMuteCommand('Out 6', true), frame([0x35, 0x07, 0x01]));
      expect(p408.buildMuteCommand('Out 1', true), frame([0x35, 0x04, 0x01]));
    });

    test('phase, delay, filters, PEQ, GEQ, matrix', () {
      expect(p206.buildPhaseCommand('Out 2', true), frame([0x36, 0x03, 0x01]));
      expect(p206.buildDelayCommand('Out 3', ProtocolService.delayMsToRaw(10)), frame([0x38, 0x04, 0xC0, 0x03]));
      expect(p206.buildHiPassCommand('Out 1', 55, 1), frame([0x32, 0x02, 55, 0x00, 0x01]));
      expect(p206.buildLoPassCommand('In A', 300, 0), frame([0x31, 0x00, 0x2C, 0x01, 0x00]));
      expect(
        p206.buildPeqBandCommand('Out 6', 8, gainDb: 3, freqRaw: 200, qRaw: 16, type: 2, bypass: true),
        frame([0x33, 0x07, 0x08, 150, 0x00, 200, 0x00, 16, 2, 1]),
      );
      expect(p206.buildGeqBandCommand('In B', 30, -12), frame([0x48, 0x01, 30, 0x00, 0x00]));
      expect(p206.buildMatrixRoutingCommand('Out 4', 0x03), frame([0x3a, 0x05, 0x03]));
    });

    test('matrix crossing gain (0x41, found in the original editor)', () {
      // output, input index, value like the channel gain (280 = 0 dB)
      expect(p408.buildMatrixGainCommand('Out 2', 'In C', 0), frame([0x41, 0x05, 0x02, 0x18, 0x01]));
      expect(p408.buildMatrixGainCommand('Out 8', 'In A', -60), frame([0x41, 0x0B, 0x00, 0x00, 0x00]));
      expect(p206.buildMatrixGainCommand('Out 1', 'In B', -6.04), frame([0x41, 0x02, 0x01, 0xDC, 0x00]));
      // limited to 0 dB
      expect(p206.buildMatrixGainCommand('Out 1', 'In A', 6), p206.buildMatrixGainCommand('Out 1', 'In A', 0));
      expect(() => p206.buildMatrixGainCommand('Out 1', 'In C', 0), throwsArgumentError);
      expect(() => p206.buildMatrixGainCommand('In A', 'In B', 0), throwsArgumentError);
      expect(PacketDescriber.describe(p408.buildMatrixGainCommand('Out 2', 'In C', -3)),
          'SET MATRIX GAIN Out 2 ← In C → -3.0 dB');
    });

    test('dynamics and channel name', () {
      expect(
        p206.buildCompressorCommand('Out 1', ratio: 9, attack: 49, release: 499, knee: 3, threshold: 170),
        frame([0x30, 0x02, 9, 0, 49, 0, 0xF3, 0x01, 3, 0, 170, 0]),
      );
      expect(
        p206.buildGateCommand('In A', attack: 0, release: 99, hold: 99, threshold: 0),
        frame([0x3E, 0x00, 0, 0, 99, 0, 99, 0, 0, 0]),
      );
      expect(
        p206.buildLimiterCommand('Out 2', attack: 49, release: 499, knee: 6, threshold: 220),
        frame([0x3F, 0x03, 49, 0, 0xF3, 0x01, 6, 0, 220, 0]),
      );
      expect(p206.buildChannelNameCommand('Out 1', 'Sub L'),
          frame([0x3D, 0x02, ...'Sub L'.codeUnits, 0, 0, 0]));
    });

    test('channels the model does not have are rejected', () {
      expect(() => p206.buildGainCommand('In C', 0), throwsArgumentError);
      expect(() => p206.buildGainCommand('Out 7', 0), throwsArgumentError);
      expect(() => p206.buildGeqBandCommand('Out 1', 0, 0), throwsArgumentError);
      expect(() => p206.buildGateCommand('Out 1', attack: 0, release: 0, hold: 0, threshold: 0),
          throwsArgumentError);
      expect(() => p408.buildGeqBandCommand('In A', 31, 0), throwsArgumentError);
      expect(() => p408.buildMatrixRoutingCommand('In A', 1), throwsArgumentError);
      expect(() => p408.buildPeqBandCommand('In A', 8,
          gainDb: 0, freqRaw: 0, qRaw: 0, type: 0, bypass: false), throwsArgumentError);
    });
  });

  group('value encodings', () {
    const p = ProtocolService();

    test('channel gain: decoding inverts encoding across both ranges', () {
      for (var tenth = -600; tenth <= 120; tenth++) {
        final q = p.quantizeGain(tenth / 10);
        expect(p.gainValueToDb(p.gainDbToValue(q)), closeTo(q, 1e-9), reason: '$q dB');
      }
      expect(p.gainValueToDb(0), -60.0);
      expect(p.gainValueToDb(60), -30.0);
      expect(p.gainValueToDb(80), -20.0);
      expect(p.gainValueToDb(280), 0.0);
    });

    test('frequency: 1000 * 2^((raw - 170) / 30), 19.7 Hz – 20.16 kHz', () {
      expect(p.peqFreqToHz(0), closeTo(19.69, 0.01));
      expect(p.peqFreqToHz(140), closeTo(500, 1e-9));
      expect(p.peqFreqToHz(170), closeTo(1000, 1e-9));
      expect(p.peqFreqToHz(300), closeTo(20159, 1));
      expect(p.peqHzToFreq(1000), 170);
      expect(p.peqHzToFreq(5), 0);
      expect(p.peqHzToFreq(50000), 300);
      for (var raw = 0; raw <= 300; raw++) {
        expect(p.peqHzToFreq(p.peqFreqToHz(raw)), raw);
      }
    });

    test('Q: 2^((raw - 16) / 12) and the display table of the original editor', () {
      expect(p.peqRawToQ(16), closeTo(1.0, 1e-9));
      expect(p.peqRawToQ(0), closeTo(0.397, 0.001));
      expect(p.peqRawToQ(100), closeTo(128, 1e-9));
      expect(p.peqQToRaw(1.0), 16);
      for (var raw = 0; raw <= ProtocolService.qRawMax; raw++) {
        expect(p.peqQToRaw(p.peqRawToQ(raw)), raw);
        // The table rounds the formula (to 2–3 significant digits).
        final shown = double.parse(ProtocolService.formatQ(raw));
        expect((shown - p.peqRawToQ(raw)).abs() / p.peqRawToQ(raw), lessThan(0.045), reason: 'raw $raw');
      }
      expect([0, 10, 16, 35, 56, 100].map(ProtocolService.formatQ), ['0.40', '0.71', '1.00', '3.0', '10.1', '128']);
      // Shelves and PEQ low/high-pass types are limited to Q 3.
      expect([for (var t = 0; t < 9; t++) ProtocolService.maxQRawForType(t)], [100, 35, 35, 35, 35, 35, 35, 100, 100]);
      expect(p.peqRawToQ(ProtocolService.qRawTypeReset), closeTo(0.71, 0.01));
    });

    test('delay, threshold, times', () {
      expect(ProtocolService.delayMsToRaw(10), 960);
      expect(ProtocolService.delayRawToMs(960), 10.0);
      expect(ProtocolService.delayMsToRaw(5000), ProtocolService.delayRawMax);
      expect(ProtocolService.thresholdDbToRaw(0), 180);
      expect(ProtocolService.thresholdDbToRaw(20), 220);
      expect(ProtocolService.thresholdRawToDb(0), -90.0);
      expect(ProtocolService.timeMsToRaw(50), 49);
      expect(ProtocolService.timeRawToMs(499), 500);
    });

    test('slopes', () {
      expect(ProtocolService.crossoverSlopeNames.length, 21);
      expect(ProtocolService.crossoverSlopeNames[0], 'Bypass');
      expect(ProtocolService.crossoverSlopeNames[ProtocolService.defaultSlope], 'BW -24');
      expect(ProtocolService.slopeDbPerOctave(0), 0);
      expect(ProtocolService.slopeDbPerOctave(5), 12);
      expect(ProtocolService.slopeDbPerOctave(20), 48);
    });
  });

  group('decode', () {
    List<int> response(int cmd, List<int> payload) {
      final dataBytes = [0x01, 0x00, payload.length + 1, cmd, ...payload];
      return [0x10, 0x02, ...dataBytes, 0x10, 0x03, ProtocolService.calculateChecksum(dataBytes)];
    }

    test('preset name', () {
      final msg = p408.decode(response(0x29, [0x03, ...'Live Show     '.codeUnits]));
      expect(msg, isA<PresetMessage>());
      expect((msg as PresetMessage).index, 3);
      expect(msg.name, 'Live Show');
    });

    test('device info keeps the first character', () {
      final msg = p408.decode(response(0x13, 'DSP408 V0104P'.codeUnits));
      expect((msg as DeviceInfoMessage).deviceName, 'DSP408 V0104P');
    });

    /// Meter slot for [value]: upper three bytes of the float32, sent as
    /// [bits 16–23, bits 24–31, bits 8–15].
    List<int> slot(double value) {
      final bits = (ByteData(4)..setFloat32(0, value)).getUint32(0);
      return [(bits >> 16) & 0xFF, bits >> 24, (bits >> 8) & 0xFF];
    }

    test('meter slots are truncated float32 values', () {
      // 1.0 = 0x3F800000: the old float16 reading of these bytes was 1.875.
      expect(slot(1.0), [0x80, 0x3F, 0x00]);
      expect(ProtocolService.decodeMeterValue(0x80, 0x3F, 0x00), 1.0);
      expect(ProtocolService.decodeMeterValue(0xFF, 0xFF, 0x00), 0.0, reason: 'NaN while recalculating');
      final v = ProtocolService.decodeMeterValue(0x00, 0x38, 0x00); // the old "0.5 noise floor"
      expect(ProtocolService.levelToDb(v), closeTo(-90.3, 0.1));
      expect(ProtocolService.levelToDb(0), ProtocolService.silenceDb);
      expect(ProtocolService.levelToDb(0.5), closeTo(-6.02, 0.01));
    });

    test('meter levels per model and the dynamics word', () {
      List<int> meters(int n, int dynamics) =>
          response(0x40, [for (var i = 0; i < n; i++) ...slot(1.0), dynamics & 0xFF, dynamics >> 8]);
      final m408 = p408.decode(meters(12, 0x0201)) as KeepaliveMessage;
      expect(m408.meterLevels, List.filled(12, 1.0));
      expect(m408.dynamics, 0x0201);
      expect(ProtocolService.compressorActive(m408.dynamics, 0), isTrue);
      expect(ProtocolService.limiterActive(m408.dynamics, 0), isFalse);
      expect(ProtocolService.limiterActive(m408.dynamics, 1), isTrue);
      expect((p206.decode(meters(8, 0)) as KeepaliveMessage).meterLevels, List.filled(8, 1.0));
      // A frame without the status word
      final short = response(0x40, [for (var i = 0; i < 12; i++) ...slot(0.5)]);
      expect((p408.decode(short) as KeepaliveMessage).dynamics, 0);
    });

    test('DSP 206 meters in the 12-slot layout: outputs start at slot 4', () {
      final frame = response(0x40, [for (var i = 0; i < 12; i++) ...slot(i + 1.0), 0, 0]);
      final levels = (p206.decode(frame) as KeepaliveMessage).meterLevels;
      expect(levels, [1, 2, 5, 6, 7, 8, 9, 10]);
      expect((p408.decode(frame) as KeepaliveMessage).meterLevels, [for (var i = 1; i <= 12; i++) i]);
    });

    test('config chunk whose payload contains 10 03', () {
      final msg = p408.decode(response(0x24, [0x05, 0x10, 0x03, 0x77]));
      expect((msg as ChannelConfigMessage).subIndex, 5);
      expect(msg.chunkData, [0x10, 0x03, 0x77]);
    });

    test('requests (host → device) are not decoded', () {
      expect(p408.decode(TRacksProto.handshakeCommand), isNull);
    });

    test('lock status, preset slots, current preset and password result', () {
      // Captured answer of a DSP 408: PTS off, limit 9999, count 0, not locked.
      final lock = p408.decode(response(0x2c, [0x00, 0x27, 0x0f, 0x00, 0x00, 0x00])) as LockStatusMessage;
      expect([lock.ptsOn, lock.limit, lock.count, lock.locked, lock.ptsExpired], [false, 9999, 0, false, false]);
      final expired = p408.decode(response(0x2c, [0x01, 0x00, 0x0A, 0x00, 0x0A, 0x01])) as LockStatusMessage;
      expect([expired.ptsExpired, expired.locked], [true, true]);

      final slots = p408.decode(response(0x22, [0xff, 0xff, 0xff, 0xff, ...List.filled(14, 0), 0xff, 0xff]));
      expect((slots as PresetSlotsMessage).used.where((u) => u).length, 6);
      expect(slots.used.length, 20);
      expect((p408.decode(response(0x14, [0x14])) as CurrentPresetMessage).number, 20);
      final password = p408.decode(response(0x2d, [0x00, 0x01])) as PasswordResultMessage;
      expect([password.mode, password.ok], [0, true]);
    });

    test('answers of another device ID are ignored', () {
      List<int> from(int id) {
        final dataBytes = [id, 0x00, 0x02, 0x14, 0x03];
        return [0x10, 0x02, ...dataBytes, 0x10, 0x03, ProtocolService.calculateChecksum(dataBytes)];
      }
      expect(p408.decode(from(1)), isA<CurrentPresetMessage>());
      expect(p408.decode(from(7)), isNull);
      expect(const ProtocolService(DeviceModel.dsp408, 7).decode(from(7)), isA<CurrentPresetMessage>());
    });
  });

  group('original editor commands', () {
    test('device ID addressing keeps the checksum valid', () {
      final frame = ProtocolService.withDeviceId(p408.buildKeepaliveCommand(), 0x21);
      expect(frame, [0x10, 0x02, 0x00, 0x21, 0x01, 0x40, 0x10, 0x03, 0x01 ^ 0x21 ^ 0x01 ^ 0x40]);
      // XOR over the whole frame from 10 02 to 10 03 gives the same checksum.
      expect(frame.sublist(0, frame.length - 1).fold<int>(0, (x, b) => x ^ b), frame.last);
      expect(ProtocolService.withDeviceId(p408.buildKeepaliveCommand(), 1), p408.buildKeepaliveCommand());
    });

    test('copy, bypass, delay unit, password, disconnect', () {
      expect(p408.buildCopyChannelCommand('Out 1', 'Out 3'), frame([0x2A, 0x04, 0x06]));
      expect(p206.buildCopyChannelCommand('In A', 'In B'), frame([0x2A, 0x00, 0x01]));
      expect(() => p408.buildCopyChannelCommand('In A', 'Out 1'), throwsArgumentError);
      expect(p206.buildEqBypassCommand('Out 6', true), frame([0x3C, 0x07, 0x01]));
      expect(p408.buildGeqBypassCommand('In D', false), frame([0x49, 0x03, 0x00]));
      expect(() => p408.buildGeqBypassCommand('Out 1', true), throwsArgumentError);
      expect(p408.buildDelayUnitCommand(2), frame([0x15, 0x02]));
      expect(p408.buildPasswordCommand(1, 'ab12'), frame([0x2D, 0x01, ...'AB12'.codeUnits]));
      expect(ProtocolService.buildDisconnectCommand(), frame([0x11]));
      expect(PacketDescriber.describe(p408.buildCopyChannelCommand('Out 1', 'Out 3')), 'COPY CHANNEL Out 1 → Out 3');
      expect(PacketDescriber.describe(p206.buildEqBypassCommand('Out 6', true), DeviceModel.dsp206),
          'SET EQ BYPASS Out 6 → BYPASS');
      expect(PacketDescriber.describe(p408.buildLoadPresetCommand(19)), 'LOAD PRESET U20');
    });
  });

  test('packet describer', () {
    expect(PacketDescriber.describe(p408.buildGainCommand('Out 3', -6)), 'SET GAIN Out 3 → -6.0 dB');
    expect(PacketDescriber.describe(p206.buildGainCommand('Out 3', -6), DeviceModel.dsp206),
        'SET GAIN Out 3 → -6.0 dB');
    expect(PacketDescriber.describe(p408.buildHiPassCommand('In A', 100, 0)), 'SET HPF In A → BYPASS');
    expect(PacketDescriber.describe(p408.buildLoPassCommand('In A', 100, 9)), 'SET LPF In A → 198 Hz BL -24');
    expect(PacketDescriber.describe(p408.buildMatrixRoutingCommand('Out 1', 0x05)),
        'SET MATRIX Out 1 ← In A + In C');
    expect(PacketDescriber.describe(p408.buildDelayCommand('Out 2', 960)), 'SET DELAY Out 2 → 10.00 ms');
  });
}
