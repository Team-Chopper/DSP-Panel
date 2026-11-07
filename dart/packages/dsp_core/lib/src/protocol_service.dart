// DSP Protocol Service
//
// Handles encoding and decoding of t.racks DSP 206/408 protocol messages.
// Includes command building, value encodings, checksum calculation and
// message parsing.
//
// Frame layout (verified against captured traffic):
//   10 02 [src] [dst] [len] [cmd] [len-1 payload bytes] 10 03 [checksum]
// Total frame length is len + 8. The host is address 0, the device answers
// from its device ID (1 by default). The checksum is 0x01 XOR every byte
// from [src] up to the last payload byte (the same as XOR over the whole
// frame from 10 02 to 10 03). Payload bytes are NOT escaped, so a payload
// may contain 10 03 — always use [len] to find the end of a frame.
//
// Value encodings are shared by both models and follow the original editor
// (ORIGINAL-SOFTWARE.md): exact frequency and Q formulas, its value ranges
// and its display tables.

import 'dart:math' as math;
import 'dart:typed_data';

import 'device_model.dart';

/// Types of protocol messages
enum MessageType {
  deviceInfo,
  preset,
  channelConfig,
  lockStatus,
  presetSlots,
  currentPreset,
  passwordResult,
  keepalive,
  unknown,
}

/// Base class for protocol messages
abstract class ProtocolMessage {
  final MessageType type;
  final List<int> rawData;

  ProtocolMessage(this.type, this.rawData);

  /// Command byte of the frame.
  int get command => rawData[5];
}

/// Preset information message
class PresetMessage extends ProtocolMessage {
  final int index;
  final String name;

  PresetMessage(this.index, this.name, List<int> rawData)
      : super(MessageType.preset, rawData);
}

/// Channel config chunk message (cmd 0x24)
class ChannelConfigMessage extends ProtocolMessage {
  final int subIndex;
  final List<int> chunkData;

  ChannelConfigMessage(this.subIndex, this.chunkData, List<int> rawData)
      : super(MessageType.channelConfig, rawData);
}

/// Device info message, e.g. "DSP408 V0104P"
class DeviceInfoMessage extends ProtocolMessage {
  final String deviceName;

  DeviceInfoMessage(this.deviceName, List<int> rawData)
      : super(MessageType.deviceInfo, rawData);
}

/// PTS / lock status (answer to 0x2C):
/// `ptsOn limit(u16 BE) count(u16 BE) locked`.
class LockStatusMessage extends ProtocolMessage {
  /// The PTS limit is switched on.
  final bool ptsOn;
  final int limit;
  final int count;

  /// The device is locked with a password.
  final bool locked;

  LockStatusMessage(this.ptsOn, this.limit, this.count, this.locked, List<int> rawData)
      : super(MessageType.lockStatus, rawData);

  /// The PTS limit is reached: the original editor asks for the PTS password.
  bool get ptsExpired => ptsOn && limit <= count;
}

/// Preset slot table (answer to 0x22): one byte per user slot, 0xFF = used.
class PresetSlotsMessage extends ProtocolMessage {
  /// Used slots by 0-based index (0 = U01).
  final List<bool> used;

  PresetSlotsMessage(this.used, List<int> rawData) : super(MessageType.presetSlots, rawData);
}

/// Current preset number (answer to 0x14).
class CurrentPresetMessage extends ProtocolMessage {
  final int number;

  CurrentPresetMessage(this.number, List<int> rawData) : super(MessageType.currentPreset, rawData);
}

/// Result of a password check (answer to 0x2D): `mode result`.
class PasswordResultMessage extends ProtocolMessage {
  /// 0 = unlock password, 1 = PTS password.
  final int mode;
  final bool ok;

  PasswordResultMessage(this.mode, this.ok, List<int> rawData) : super(MessageType.passwordResult, rawData);
}

/// Keepalive/meter message (cmd 0x40 response with one level per channel)
class KeepaliveMessage extends ProtocolMessage {
  /// Meter levels as linear amplitudes (1.0 = full scale), in the model's
  /// channel order (inputs, then outputs).
  final List<double> meterLevels;

  /// Status word after the meter slots: bit n = compressor of output n
  /// working, bit 8 + n = limiter of output n working.
  final int dynamics;

  KeepaliveMessage(this.meterLevels, List<int> rawData, {this.dynamics = 0})
      : super(MessageType.keepalive, rawData);
}

/// Unknown message (including the generic 0x01 acknowledgement)
class UnknownMessage extends ProtocolMessage {
  UnknownMessage(List<int> rawData) : super(MessageType.unknown, rawData);
}

class ProtocolService {
  /// Model whose channel bytes the command builders use.
  final DeviceModel model;

  /// Device ID: the address the device answers from (1 … 250). Command
  /// builders address ID 1; see [withDeviceId].
  final int deviceId;

  const ProtocolService([this.model = DeviceModel.dsp408, this.deviceId = 1]);

  /// Device IDs the original editor can address.
  static const int minDeviceId = 1;
  static const int maxDeviceId = 250;

  // ─── Meters ───

  /// A meter slot holds the upper three bytes of an IEEE 754 float32:
  /// `bits = d1 << 24 | d0 << 16 | d2 << 8` (read like this by the original
  /// editor, which shows 20·log10 of it). Read as float16, `d0 d1` gave the
  /// old "0.5 … 2.0" scale; 1.0 is full scale.
  static double decodeMeterValue(int d0, int d1, int d2) {
    final bits = (d1 << 24) | (d0 << 16) | (d2 << 8);
    final value = (ByteData(4)..setUint32(0, bits)).getFloat32(0);
    // The DSP answers 0xFF 0xFF (NaN) for channels during PEQ recalculation.
    return value.isFinite ? value.abs() : 0.0;
  }

  /// Level in dBFS of a linear meter value, at least [silenceDb].
  static double levelToDb(double level) =>
      level <= 1e-6 ? silenceDb : math.max(silenceDb, 20 * math.log(level) / math.ln10);

  /// Meter level shown for silence.
  static const double silenceDb = -120.0;

  /// Parse [count] channel meter levels from a keepalive response frame.
  /// Each channel is 3 bytes, see [decodeMeterValue].
  static List<double> parseMeterLevels(List<int> data, int count) {
    // Data starts at byte 6 (after 10 02 01 00 LEN 40)
    if (data.length < 6 + count * 3) return List.filled(count, 0.0); // too short

    return [for (int ch = 0; ch < count; ch++) _meterAt(data, ch)];
  }

  static double _meterAt(List<int> data, int slot) =>
      decodeMeterValue(data[6 + slot * 3], data[7 + slot * 3], data[8 + slot * 3]);

  /// Status word (u16 LE) after the meter slots of a 0x40 response, or 0.
  static int parseDynamics(List<int> data) {
    if (data.length < 6) return 0;
    final footer = (5 + data[4]).clamp(6, data.length);
    final at = 6 + (footer - 6) ~/ 3 * 3;
    return at + 2 <= footer ? data[at] | (data[at + 1] << 8) : 0;
  }

  /// The compressor of the output with index [output] reduces the gain.
  static bool compressorActive(int dynamics, int output) => dynamics & (1 << output) != 0;

  /// The limiter of the output with index [output] reduces the gain.
  static bool limiterActive(int dynamics, int output) => dynamics & (0x100 << output) != 0;

  /// Meter levels of [model] in channel order (inputs, then outputs).
  ///
  /// The 0x40 response of the DSP 408 has 12 slots: In A–D, then Out 1–8.
  /// The DSP 206 answers with the same layout: its outputs start at slot 4
  /// (slots 2 and 3 are unused), seen on hardware as Out 1 being shown at
  /// Out 3. Shorter frames are read without the gap.
  static List<double> parseModelMeters(List<int> data, DeviceModel model) {
    if (data.length < 6) return List.filled(model.meterCount, 0.0);
    final footer = data.length >= 5 ? 5 + data[4] : data.length;
    final slots = ((footer.clamp(6, data.length) - 6) ~/ 3);
    const outputSlot = 4;
    if (model.inputs.length < outputSlot && slots >= outputSlot + model.outputs.length) {
      return [
        for (var i = 0; i < model.inputs.length; i++) _meterAt(data, i),
        for (var o = 0; o < model.outputs.length; o++) _meterAt(data, outputSlot + o),
      ];
    }
    return parseMeterLevels(data, model.meterCount);
  }

  // ─── Framing ───

  /// Calculate checksum for the DSP protocol
  static int calculateChecksum(Iterable<int> dataBytes) {
    int checksum = 1;
    for (int byte in dataBytes) {
      checksum ^= byte;
    }
    return checksum;
  }

  /// Build a host → device frame for a command byte and its payload.
  static List<int> buildFrame(int command, [List<int> payload = const []]) {
    final dataBytes = [0x00, 0x01, payload.length + 1, command, ...payload];
    return [0x10, 0x02, ...dataBytes, 0x10, 0x03, calculateChecksum(dataBytes)];
  }

  /// [frame] addressed to device [id] (destination byte and checksum).
  static List<int> withDeviceId(List<int> frame, int id) {
    if (frame.length < 8 || frame[3] == id) return frame;
    final out = List.of(frame)..[3] = id;
    out[out.length - 1] = calculateChecksum(out.sublist(2, out.length - 3));
    return out;
  }

  static List<int> _u16(int value) => [value & 0xFF, (value >> 8) & 0xFF];

  int _requireChannel(String channel, {bool inputsOnly = false, bool outputsOnly = false}) {
    final ch = model.channelByte(channel);
    if (ch == null ||
        (inputsOnly && !model.isInput(channel)) ||
        (outputsOnly && !model.isOutput(channel))) {
      throw ArgumentError('Invalid channel for ${model.name}: $channel');
    }
    return ch;
  }

  // ─── Channel gain ───

  /// Convert dB value to DSP protocol value
  int gainDbToValue(double dB) {
    if (dB < -20.0) {
      // Coarse range: 0.5dB resolution (2 units per dB)
      return ((dB + 60) * 2).round().clamp(0, 80);
    } else {
      // Fine range: 0.1dB resolution (10 units per dB)
      return (80 + (dB + 20) * 10).round().clamp(80, 400);
    }
  }

  /// Convert DSP protocol value back to dB (inverse of [gainDbToValue])
  double gainValueToDb(int value) {
    if (value < 80) return value / 2.0 - 60.0;
    return (value - 280) / 10.0;
  }

  /// Quantize gain value to appropriate step
  double quantizeGain(double dB) {
    if (dB < -20.0) {
      // Coarse range: round to nearest 0.5dB
      return (dB * 2).round() / 2;
    } else {
      // Fine range: round to nearest 0.1dB
      return (dB * 10).round() / 10;
    }
  }

  // ─── EQ gain (PEQ and GEQ) ───

  /// Convert GEQ dB value (-12.0 to +12.0) to protocol value (0-240)
  /// Linear: value = (dB * 10) + 120, resolution 0.1 dB
  int geqDbToValue(double dB) => (dB * 10 + 120).round().clamp(0, 240);

  /// Convert GEQ protocol value back to dB
  double geqValueToDb(int value) => (value - 120) / 10.0;

  /// Quantize GEQ dB to 0.1 dB resolution
  double quantizeGeqDb(double dB) => (dB * 10).round() / 10.0;

  /// PEQ gain: same encoding as GEQ (0-240, dB = (value-120)/10.0)
  double peqGainToDb(int value) => (value - 120) / 10.0;
  int peqDbToGain(double dB) => (dB * 10 + 120).round().clamp(0, 240);

  // ─── Frequency (PEQ, HPF, LPF) ───

  /// Frequency encoding of the original editor: 1/30-octave steps around
  /// 1 kHz (raw 170), Hz = 1000 * 2^((raw - 170) / 30): raw 0 = 19.7 Hz,
  /// raw 300 = 20.16 kHz.
  static const int freqRawMax = 300;
  static const int _freqRaw1k = 170;

  /// Convert raw frequency value to Hz
  double peqFreqToHz(int raw) =>
      1000.0 * math.pow(2.0, (raw.clamp(0, freqRawMax) - _freqRaw1k) / 30.0);

  /// Convert Hz to raw frequency value
  int peqHzToFreq(double hz) {
    if (hz <= 0) return 0;
    return (_freqRaw1k + 30 * math.log(hz / 1000) / math.ln2).round().clamp(0, freqRawMax);
  }

  // ─── Q ───

  /// Q encoding of the original editor: 12 steps per doubling,
  /// Q = 2^((raw - 16) / 12); raw 0 = 0.40, 16 = 1.0, 100 = 128.
  static const int qRawMax = 100;

  double peqRawToQ(int raw) => math.pow(2.0, (raw.clamp(0, qRawMax) - 16) / 12.0).toDouble();

  int peqQToRaw(double q) {
    if (q <= 0) return 0;
    return (12 * math.log(q) / math.ln2 + 16).round().clamp(0, qRawMax);
  }

  /// Q as the original editor shows it (its display table rounds the
  /// formula to two or three significant digits).
  static String formatQ(int raw) => _qDisplay[raw.clamp(0, qRawMax)];

  static const _qDisplay = [
    '0.40', '0.42', '0.45', '0.47', '0.50', '0.53', '0.56', '0.59', '0.63', '0.67', //
    '0.71', '0.75', '0.79', '0.84', '0.89', '0.94', '1.00', '1.06', '1.12', '1.19', //
    '1.26', '1.3', '1.4', '1.5', '1.6', '1.7', '1.8', '1.9', '2.0', '2.1', //
    '2.2', '2.4', '2.5', '2.7', '2.8', '3.0', '3.2', '3.4', '3.6', '3.8', //
    '4.0', '4.2', '4.5', '4.8', '5.0', '5.3', '5.7', '6.0', '6.3', '6.7', //
    '7.1', '7.6', '8.0', '8.5', '9.0', '9.5', '10.1', '10.7', '11.3', '12', //
    '12.7', '13.5', '14', '15', '16', '17', '18', '19', '20', '21', //
    '23', '24', '25', '27', '29', '30', '32', '34', '36', '38', //
    '40', '43', '45', '48', '51', '54', '57', '60', '64', '68', //
    '72', '76', '81', '85', '91', '96', '102', '108', '114', '121', //
    '128',
  ];

  /// Highest Q (raw) of a PEQ type: shelves and the PEQ low/high-pass types
  /// are limited to Q 3.0 by the original editor, peak and all-pass use the
  /// full range.
  static int maxQRawForType(int type) => (type >= 1 && type <= 6) ? qRawLimited : qRawMax;

  /// Q 3.0, the limit of shelves and PEQ low/high-pass types.
  static const int qRawLimited = 35;

  /// Q 0.71, set by the original editor when a band changes to a limited type.
  static const int qRawTypeReset = 10;

  // ─── Delay and dynamics ───

  /// Delay: raw = ms * 96 (96 kHz sample clock); limited to 680 ms.
  static const int delayRawMax = 680 * 96;
  static double delayRawToMs(int raw) => raw / 96.0;
  static int delayMsToRaw(double ms) => (ms * 96).round().clamp(0, delayRawMax);

  /// Delay display units (cmd 0x15, stored in the config dump).
  static const delayUnitNames = ['ms', 'm', 'ft'];

  /// Speed of sound the original editor uses for the distance display.
  static const double metersPerMs = 0.344;
  static const double feetPerMeter = 1 / 0.3048;

  /// Threshold (gate, compressor, limiter): raw = dB * 2 + 180.
  static double thresholdRawToDb(int raw) => (raw - 180) / 2.0;
  static int thresholdDbToRaw(double dB) => (dB * 2 + 180).round().clamp(0, 0xFFFF);

  /// Attack, release and hold times: raw = ms - 1.
  static int timeRawToMs(int raw) => raw + 1;
  static int timeMsToRaw(int ms) => (ms - 1).clamp(0, 0xFFFF);

  /// Ranges of the original editor (raw): attack 0 … 998 (1 … 999 ms),
  /// release 9 … 2999 (10 … 3000 ms), gate hold 9 … 998 (10 … 999 ms).
  static const int attackRawMax = 998;
  static const int releaseRawMin = 9;
  static const int releaseRawMax = 2999;
  static const int holdRawMin = 9;
  static const int holdRawMax = 998;

  /// Thresholds: gate −90 … 0 dB, compressor and limiter −90 … +20 dB.
  static const int gateThresholdRawMax = 180;
  static const int thresholdRawMax = 220;

  /// Knee of the compressor and the limiter in dB.
  static const int kneeMax = 12;

  /// PEQ filter types
  static const peqTypeNames = [
    'Peak', 'Low Shelf', 'High Shelf',
    'LP -6dB', 'LP -12dB', 'HP -6dB', 'HP -12dB',
    'All Pass 1', 'All Pass 2',
  ];

  /// Hi/Lo Pass crossover slopes; index 0 = bypass (filter off).
  /// BW = Butterworth, BL = Bessel, LK = Linkwitz-Riley.
  static const crossoverSlopeNames = [
    'Bypass',
    'BW -6', 'BL -6',
    'BW -12', 'BL -12', 'LK -12',
    'BW -18', 'BL -18',
    'BW -24', 'BL -24', 'LK -24',
    'BW -30', 'BL -30',
    'BW -36', 'BL -36', 'LK -36',
    'BW -42', 'BL -42',
    'BW -48', 'BL -48', 'LK -48',
  ];

  /// Slope in dB/octave for a crossover slope index (0 for bypass).
  static int slopeDbPerOctave(int slope) {
    if (slope <= 0 || slope >= crossoverSlopeNames.length) return 0;
    return int.parse(crossoverSlopeNames[slope].split(' -').last);
  }

  /// Default slope when a filter is switched on (BW -24).
  static const int defaultSlope = 8;

  /// Compressor ratios by index.
  static const compressorRatioNames = [
    '1:1', '1:1.1', '1:1.3', '1:1.5', '1:1.7', '1:2', '1:2.5', '1:3',
    '1:3.5', '1:4', '1:5', '1:6', '1:8', '1:10', '1:20', 'Limit',
  ];

  // ─── Command builders ───

  /// Gain (0x34): `ch value(2B LE)`
  List<int> buildGainCommand(String channelLabel, double dB) {
    final channel = _requireChannel(channelLabel);
    return buildFrame(0x34, [channel, ..._u16(gainDbToValue(quantizeGain(dB)))]);
  }

  /// Mute (0x35): `ch 0|1`
  List<int> buildMuteCommand(String channelLabel, bool mute) {
    final channel = _requireChannel(channelLabel);
    return buildFrame(0x35, [channel, mute ? 0x01 : 0x00]);
  }

  /// Phase invert (0x36): `ch 0|1`
  List<int> buildPhaseCommand(String channelLabel, bool invert) {
    final channel = _requireChannel(channelLabel);
    return buildFrame(0x36, [channel, invert ? 0x01 : 0x00]);
  }

  /// Delay (0x38): `ch value(2B LE)`, value = ms * 96
  List<int> buildDelayCommand(String channelLabel, int raw) {
    final channel = _requireChannel(channelLabel);
    return buildFrame(0x38, [channel, ..._u16(raw.clamp(0, delayRawMax))]);
  }

  /// GEQ band (0x48): `ch band value(2B LE)` — inputs only
  List<int> buildGeqBandCommand(String channel, int bandIndex, double dB) {
    final ch = _requireChannel(channel, inputsOnly: true);
    if (bandIndex < 0 || bandIndex >= geqBandCount) {
      throw ArgumentError('Invalid band index: $bandIndex');
    }
    return buildFrame(0x48, [ch, bandIndex, ..._u16(geqDbToValue(quantizeGeqDb(dB)))]);
  }

  /// Matrix routing (0x3a): `output input_bitmask`
  List<int> buildMatrixRoutingCommand(String output, int inputBitmask) {
    final ch = _requireChannel(output, outputsOnly: true);
    return buildFrame(0x3a, [ch, inputBitmask & model.matrixMask]);
  }

  /// Highest matrix crossing gain (the original editor allows −60 … 0 dB).
  static const double matrixGainMaxDb = 0.0;

  /// Matrix crossing gain (0x41): `output input_index value(2B LE)`, value
  /// encoded like the channel gain (found in the original editor).
  List<int> buildMatrixGainCommand(String output, String input, double dB) {
    final ch = _requireChannel(output, outputsOnly: true);
    final index = model.inputs.indexOf(input);
    if (index < 0) throw ArgumentError('Invalid input for ${model.name}: $input');
    final value = gainDbToValue(quantizeGain(math.min(dB, matrixGainMaxDb)));
    return buildFrame(0x41, [ch, index, ..._u16(value)]);
  }

  /// PEQ band (0x33):
  /// `ch band gain(2B LE) freq(2B LE) Q type bypass`
  List<int> buildPeqBandCommand(String channel, int band, {
    required double gainDb,
    required int freqRaw,
    required int qRaw,
    required int type,
    required bool bypass,
  }) {
    final ch = _requireChannel(channel);
    if (band < 0 || band >= peqBandCountFor(model, channel)) {
      throw ArgumentError('Invalid PEQ band: $band');
    }
    return buildFrame(0x33, [
      ch, band, ..._u16(peqDbToGain(gainDb)),
      ..._u16(freqRaw.clamp(0, freqRawMax)),
      qRaw & 0xFF, type, bypass ? 1 : 0,
    ]);
  }

  /// Hi Pass (0x32): `ch freq(2B LE) slope` — slope 0 = bypass
  List<int> buildHiPassCommand(String channel, int freqRaw, int slope) {
    final ch = _requireChannel(channel);
    return buildFrame(0x32, [ch, ..._u16(freqRaw.clamp(0, freqRawMax)), slope]);
  }

  /// Lo Pass (0x31): `ch freq(2B LE) slope` — slope 0 = bypass
  List<int> buildLoPassCommand(String channel, int freqRaw, int slope) {
    final ch = _requireChannel(channel);
    return buildFrame(0x31, [ch, ..._u16(freqRaw.clamp(0, freqRawMax)), slope]);
  }

  /// Compressor (0x30):
  /// `ch ratio(2B) attack(2B) release(2B) knee(2B) threshold(2B)`
  List<int> buildCompressorCommand(String channel, {
    required int ratio,
    required int attack,
    required int release,
    required int knee,
    required int threshold,
  }) {
    final ch = _requireChannel(channel);
    return buildFrame(0x30, [
      ch, ..._u16(ratio), ..._u16(attack), ..._u16(release), ..._u16(knee), ..._u16(threshold),
    ]);
  }

  /// Gate (0x3E): `ch attack(2B) release(2B) hold(2B) threshold(2B)` — inputs only
  List<int> buildGateCommand(String channel, {
    required int attack,
    required int release,
    required int hold,
    required int threshold,
  }) {
    final ch = _requireChannel(channel, inputsOnly: true);
    return buildFrame(0x3E, [
      ch, ..._u16(attack), ..._u16(release), ..._u16(hold), ..._u16(threshold),
    ]);
  }

  /// Limiter (0x3F): `ch attack(2B) release(2B) knee(2B) threshold(2B)`
  List<int> buildLimiterCommand(String channel, {
    required int attack,
    required int release,
    required int knee,
    required int threshold,
  }) {
    final ch = _requireChannel(channel);
    return buildFrame(0x3F, [
      ch, ..._u16(attack), ..._u16(release), ..._u16(knee), ..._u16(threshold),
    ]);
  }

  /// Channel link (0x3B): `ch mask` — one byte, bits relative to the
  /// channel's group (In A = bit 0, Out 1 = bit 0). The original editor
  /// sends it for the master only: own bit + slaves.
  List<int> buildLinkCommand(String channel, int mask) {
    final ch = _requireChannel(channel);
    return buildFrame(0x3B, [ch, mask & 0xFF]);
  }

  /// Copy channel (0x2A): `src dst` (absolute channel bytes, same group).
  /// The device copies the settings itself; the original editor reads the
  /// whole configuration afterwards.
  List<int> buildCopyChannelCommand(String from, String to) {
    final src = _requireChannel(from);
    final dst = _requireChannel(to);
    if (model.isInput(from) != model.isInput(to)) {
      throw ArgumentError('Only inputs or only outputs can be copied');
    }
    return buildFrame(0x2A, [src, dst]);
  }

  /// Bypass the whole PEQ of a channel (0x3C): `ch 1|0`.
  List<int> buildEqBypassCommand(String channel, bool bypass) =>
      buildFrame(0x3C, [_requireChannel(channel), bypass ? 1 : 0]);

  /// Bypass the GEQ of an input (0x49): `ch 1|0`.
  List<int> buildGeqBypassCommand(String channel, bool bypass) =>
      buildFrame(0x49, [_requireChannel(channel, inputsOnly: true), bypass ? 1 : 0]);

  /// Delay display unit (0x15): 0 = ms, 1 = m, 2 = ft.
  List<int> buildDelayUnitCommand(int unit) => buildFrame(0x15, [unit & 0xFF]);

  /// Password check (0x2D): `mode password(4 ASCII)`, mode 0 = unlock,
  /// 1 = PTS. The original editor converts the password to upper case.
  List<int> buildPasswordCommand(int mode, String password) {
    final text = password.toUpperCase().padRight(4).substring(0, 4);
    return buildFrame(0x2D, [mode & 0xFF, ...text.codeUnits.map((c) => c & 0x7F)]);
  }

  /// Disconnect (0x11): sent by the original editor when it ends a session.
  static List<int> buildDisconnectCommand() => buildFrame(0x11);

  /// Heartbeat (0x12): sent by the original editor while the link is idle,
  /// and after a dump or a store.
  static List<int> buildHeartbeatCommand() => buildFrame(0x12);

  /// Signal generator types of the test tone command (index = type byte);
  /// "Analog input" switches the generator off.
  static const testToneTypeNames = ['Analog input', 'Pink noise', 'White noise', 'Sine'];

  /// Test tone (0x39): `type param` — type 0 = analog input (generator off),
  /// 1 = pink noise, 2 = white noise, 3 = sine with param = index of the
  /// 31 ISO 1/3-octave frequencies (0 = 20 Hz … 30 = 20 kHz). The device
  /// keeps both in the config dump.
  List<int> buildTestToneCommand(int type, int param) => buildFrame(0x39, [type & 0xFF, param & 0xFF]);

  /// Channel name (0x3D): `ch name(8 ASCII, zero-padded)`
  List<int> buildChannelNameCommand(String channel, String name) {
    final ch = _requireChannel(channel);
    return buildFrame(0x3D, [ch, ...encodeChannelName(name)]);
  }

  /// 8 bytes of printable ASCII, zero-padded; other characters become '?'.
  static List<int> encodeChannelName(String name) {
    final codes = name.codeUnits
        .take(8)
        .map((c) => (c >= 0x20 && c <= 0x7E) ? c : 0x3F)
        .toList();
    return [...codes, ...List.filled(8 - codes.length, 0)];
  }

  /// Build keepalive command
  List<int> buildKeepaliveCommand() => buildFrame(0x40);

  /// Build a load preset command for the given preset index (0-based).
  ///
  /// Protocol: `10 02 00 01 02 20 [index] 10 03 [checksum]`
  /// The protocol uses 1-based preset indices.
  List<int> buildLoadPresetCommand(int presetIndex) =>
      buildFrame(0x20, [presetIndex + 1]);

  /// Build the store preset name command (cmd 0x26).
  ///
  /// Protocol: `10 02 00 01 0f 26 [14-char name] 10 03 [checksum]`
  /// Name is padded with spaces to exactly 14 characters. Characters outside
  /// printable ASCII are replaced with '?'.
  List<int> buildStorePresetNameCommand(String name) {
    final padded = name.length >= 14 ? name.substring(0, 14) : name.padRight(14);
    final nameBytes = padded.codeUnits
        .map((c) => (c >= 0x20 && c <= 0x7E) ? c : 0x3F)
        .toList();
    return buildFrame(0x26, nameBytes);
  }

  /// Build the store preset slot command (cmd 0x21).
  ///
  /// Protocol: `10 02 00 01 02 21 [slot] 10 03 [checksum]`
  /// Slot is 1-based (U01 = 1, U20 = 20).
  List<int> buildStorePresetSlotCommand(int presetIndex) =>
      buildFrame(0x21, [presetIndex + 1]);

  // ─── Decoding ───

  /// Decode a complete protocol frame (as produced by FrameDecoder).
  ProtocolMessage? decode(List<int> data) {
    if (data.length < 8 || data[0] != 0x10 || data[1] != 0x02) {
      return null;
    }

    try {
      // End of payload, i.e. position of the 10 03 footer
      final footerIndex = 5 + data[4];
      if (footerIndex > data.length) return null;

      // Only responses of our device (source = device ID, destination = host)
      // carry state; the original editor ignores other addresses.
      final isResponse = data[2] == deviceId && data[3] == 0x00;
      if (!isResponse || data.length <= 7) return null;

      final cmdByte = data[5];
      final dataByte = footerIndex > 6 ? data[6] : null;

      // Keepalive / meter response
      if (cmdByte == 0x40) {
        final levels = parseModelMeters(data, model);
        return KeepaliveMessage(levels, data, dynamics: parseDynamics(data));
      }

      // Device info: the whole payload after the command byte
      if (cmdByte == 0x13 && footerIndex > 6) {
        final deviceName = _ascii(data.sublist(6, footerIndex));
        return DeviceInfoMessage(deviceName, data);
      }

      // Preset data
      if (cmdByte == 0x29 && dataByte != null) {
        final presetName = _ascii(data.sublist(7, footerIndex));
        return PresetMessage(dataByte, presetName, data);
      }

      // PTS / lock status: ptsOn limit(BE) count(BE) locked
      if (cmdByte == 0x2c && footerIndex >= 12) {
        return LockStatusMessage(
            data[6] != 0, (data[7] << 8) | data[8], (data[9] << 8) | data[10], data[11] != 0, data);
      }

      // Preset slot table: one byte per user slot, 0xFF = used
      if (cmdByte == 0x22 && footerIndex > 6) {
        return PresetSlotsMessage([for (final b in data.sublist(6, footerIndex)) b == 0xFF], data);
      }

      // Current preset number
      if (cmdByte == 0x14 && dataByte != null) {
        return CurrentPresetMessage(dataByte, data);
      }

      // Password check: mode result
      if (cmdByte == 0x2d && footerIndex >= 8) {
        return PasswordResultMessage(data[6], data[7] == 1, data);
      }

      // Channel config chunk (cmd 0x24)
      if (cmdByte == 0x24 && dataByte != null) {
        final chunkData = data.sublist(7, footerIndex);
        return ChannelConfigMessage(dataByte, chunkData, data);
      }

      return UnknownMessage(data);
    } catch (e) {
      return null;
    }
  }

  static String _ascii(List<int> bytes) => String.fromCharCodes(
          bytes.where((b) => b != 0).map((b) => (b >= 0x20 && b <= 0x7E) ? b : 0x3F))
      .trim();

  /// Format data as hex string for display
  String formatAsHex(List<int> data) {
    return data.map((b) => b.toRadixString(16).padLeft(2, '0')).join(' ');
  }

  /// Format data as ASCII string for display
  String formatAsAscii(List<int> data) {
    return data
        .map((b) => (b >= 0x20 && b <= 0x7E) ? String.fromCharCode(b) : '.')
        .join('');
  }
}
