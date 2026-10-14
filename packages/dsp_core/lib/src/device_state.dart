// Device State
//
// The complete state of one DSP as shown in the panel. The server owns the
// authoritative copy; clients hold a mirror that they update optimistically.
// Both sides mutate it only through DeviceOps (device_ops.dart).
//
// Values are kept in protocol units where the protocol has no natural float
// representation (frequencies, Q, times, thresholds, delay), so that a config
// dump can be decoded and re-encoded without loss. ProtocolService has the
// conversions.

import 'device_model.dart';

/// State for a single PEQ band
class PeqBand {
  int freqRaw;   // 0-300, 20 Hz – 20 kHz
  int qRaw;      // 0-100, Q 0.4 – 128
  double gainDb;  // -12.0 to +12.0 dB
  int type;      // 0-8 (Peak, Low Shelf, etc.)
  bool bypass;

  PeqBand({
    this.freqRaw = 150,
    this.qRaw = 35,
    this.gainDb = 0.0,
    this.type = 0,
    this.bypass = false,
  });

  PeqBand copy() => PeqBand(
    freqRaw: freqRaw,
    qRaw: qRaw,
    gainDb: gainDb,
    type: type,
    bypass: bypass,
  );

  Map<String, Object?> toJson() => {
    'freqRaw': freqRaw,
    'qRaw': qRaw,
    'gainDb': gainDb,
    'type': type,
    'bypass': bypass,
  };

  factory PeqBand.fromJson(Map<String, Object?> json) => PeqBand(
    freqRaw: (json['freqRaw'] as num).toInt(),
    qRaw: (json['qRaw'] as num).toInt(),
    gainDb: (json['gainDb'] as num).toDouble(),
    type: (json['type'] as num).toInt(),
    bypass: json['bypass'] as bool,
  );
}

/// State for a Hi/Lo Pass filter
class FilterState {
  int freqRaw;   // 0-300, 20 Hz – 20 kHz
  int slope;     // index into ProtocolService.crossoverSlopeNames; 0 = bypass

  FilterState({this.freqRaw = 0, this.slope = 0});

  /// The filter is active unless its slope is "bypass".
  bool get enabled => slope != 0;

  FilterState copy() => FilterState(freqRaw: freqRaw, slope: slope);

  Map<String, Object?> toJson() => {'freqRaw': freqRaw, 'slope': slope};

  factory FilterState.fromJson(Map<String, Object?> json) => FilterState(
    freqRaw: (json['freqRaw'] as num).toInt(),
    slope: (json['slope'] as num).toInt(),
  );
}

/// Noise gate of an input. Raw protocol values: times = ms - 1,
/// threshold = dB * 2 + 180.
class GateState {
  int attack;
  int release;
  int hold;
  int threshold;

  GateState({this.attack = 0, this.release = 99, this.hold = 99, this.threshold = 0});

  GateState copy() => GateState(attack: attack, release: release, hold: hold, threshold: threshold);

  Map<String, Object?> toJson() =>
      {'attack': attack, 'release': release, 'hold': hold, 'threshold': threshold};

  factory GateState.fromJson(Map<String, Object?> json) => GateState(
    attack: (json['attack'] as num).toInt(),
    release: (json['release'] as num).toInt(),
    hold: (json['hold'] as num).toInt(),
    threshold: (json['threshold'] as num).toInt(),
  );
}

/// Compressor of an output. Raw protocol values: ratio index,
/// times = ms - 1, knee in dB, threshold = dB * 2 + 180.
class CompressorState {
  int ratio;
  int attack;
  int release;
  int knee;
  int threshold;

  CompressorState({
    this.ratio = 0,
    this.attack = 49,
    this.release = 499,
    this.knee = 0,
    this.threshold = 220,
  });

  CompressorState copy() => CompressorState(
      ratio: ratio, attack: attack, release: release, knee: knee, threshold: threshold);

  Map<String, Object?> toJson() => {
    'ratio': ratio,
    'attack': attack,
    'release': release,
    'knee': knee,
    'threshold': threshold,
  };

  factory CompressorState.fromJson(Map<String, Object?> json) => CompressorState(
    ratio: (json['ratio'] as num).toInt(),
    attack: (json['attack'] as num).toInt(),
    release: (json['release'] as num).toInt(),
    knee: (json['knee'] as num).toInt(),
    threshold: (json['threshold'] as num).toInt(),
  );
}

/// Limiter of an output. Raw protocol values like [CompressorState]; the
/// knee (0 … 12 dB) is hidden in the original editor's UI.
class LimiterState {
  int attack;
  int release;
  int knee;
  int threshold;

  LimiterState({this.attack = 49, this.release = 499, this.knee = 0, this.threshold = 220});

  LimiterState copy() => LimiterState(attack: attack, release: release, knee: knee, threshold: threshold);

  Map<String, Object?> toJson() => {'attack': attack, 'release': release, 'knee': knee, 'threshold': threshold};

  /// Older versions stored the knee as "unknown".
  factory LimiterState.fromJson(Map<String, Object?> json) => LimiterState(
    attack: (json['attack'] as num).toInt(),
    release: (json['release'] as num).toInt(),
    knee: ((json['knee'] ?? json['unknown'] ?? 0) as num).toInt(),
    threshold: (json['threshold'] as num).toInt(),
  );
}

class DeviceState {
  final DeviceModel model;

  /// Preset names by 0-based index (0 = U01)
  final Map<int, String> presets;
  String currentPreset;
  String deviceName;

  /// Gain in dB for every channel
  final Map<String, double> gains;
  final Map<String, bool> mutes;
  final Map<String, bool> phase;

  /// Delay per channel, raw (ms * 96)
  final Map<String, int> delays;

  /// Matrix routing: active input bitmask per output (cmd 0x3a)
  final Map<String, int> matrixRouting;

  /// Matrix gain per crossing point in dB (−60 … 0), key "Out 1:In A"
  /// (cmd 0x41)
  final Map<String, double> matrixGains;

  /// 31-band GEQ per input channel (-12 to +12 dB)
  final Map<String, List<double>> geqBands;

  /// PEQ bands: 8 per input, 9 per output
  final Map<String, List<PeqBand>> peqBands;
  final Map<String, FilterState> hiPass;
  final Map<String, FilterState> loPass;

  final Map<String, GateState> gates;            // inputs
  final Map<String, CompressorState> compressors; // outputs
  final Map<String, LimiterState> limiters;       // outputs

  /// Channel names stored on the device ("InA", "Kick" …)
  final Map<String, String> channelNames;

  /// Channel link mask per channel as stored on the device: a master holds
  /// its own bit and the bits of its slaves, a slave only its own bit.
  final Map<String, int> links;

  /// Whole PEQ of a channel bypassed (cmd 0x3C).
  final Map<String, bool> eqBypass;

  /// GEQ of an input bypassed (cmd 0x49).
  final Map<String, bool> geqBypass;

  /// Signal generator of the DSP (cmd 0x39, kept in the config dump): type
  /// (0 = analog input, i.e. off, 1 = pink, 2 = white, 3 = sine) and
  /// parameter (sine frequency index).
  int testToneType = 0;
  int testToneParam = 0;

  /// Delay display unit stored on the device (cmd 0x15): 0 = ms, 1 = m,
  /// 2 = ft.
  int delayUnit = 0;

  /// Used user preset slots by 0-based index (answer to 0x22); empty if
  /// the device did not answer.
  final Set<int> presetSlots = {};

  /// Current preset number reported by the device (0x14; 0 = none/factory,
  /// n = Un), or -1 if unknown.
  int currentPresetNumber = -1;

  DeviceState._(
    this.model, {
    required this.presets,
    required this.currentPreset,
    required this.deviceName,
    required this.gains,
    required this.mutes,
    required this.phase,
    required this.delays,
    required this.matrixRouting,
    required this.matrixGains,
    required this.geqBands,
    required this.peqBands,
    required this.hiPass,
    required this.loPass,
    required this.gates,
    required this.compressors,
    required this.limiters,
    required this.channelNames,
    required this.links,
    required this.eqBypass,
    required this.geqBypass,
  });

  /// Default state of [model] before anything has been read from the device.
  factory DeviceState([DeviceModel model = DeviceModel.dsp408]) {
    final channels = model.channels;
    return DeviceState._(
      model,
      presets: {},
      currentPreset: 'Unknown',
      deviceName: '',
      gains: {for (final ch in channels) ch: 0.0},
      mutes: {for (final ch in channels) ch: false},
      phase: {for (final ch in channels) ch: false},
      delays: {for (final ch in channels) ch: 0},
      matrixRouting: {
        for (var i = 0; i < model.outputs.length; i++)
          model.outputs[i]: 1 << (i % model.inputs.length),
      },
      matrixGains: {
        for (final o in model.outputs)
          for (final i in model.inputs) '$o:$i': 0.0,
      },
      geqBands: {for (final ch in model.inputs) ch: List.filled(geqBandCount, 0.0)},
      peqBands: {
        for (final ch in channels)
          ch: List.generate(peqBandCountFor(model, ch), (_) => PeqBand()),
      },
      hiPass: {for (final ch in channels) ch: FilterState(freqRaw: 0)},
      loPass: {for (final ch in channels) ch: FilterState(freqRaw: 300)},
      gates: {for (final ch in model.inputs) ch: GateState()},
      compressors: {for (final ch in model.outputs) ch: CompressorState()},
      limiters: {for (final ch in model.outputs) ch: LimiterState()},
      channelNames: {for (final ch in channels) ch: model.defaultDeviceName(ch)},
      links: {
        for (var i = 0; i < model.inputs.length; i++) model.inputs[i]: 1 << i,
        for (var i = 0; i < model.outputs.length; i++) model.outputs[i]: 1 << i,
      },
      eqBypass: {for (final ch in channels) ch: false},
      geqBypass: {for (final ch in model.inputs) ch: false},
    );
  }

  Map<String, Object?> toJson() => {
    'model': model.id,
    'presets': {for (final e in presets.entries) '${e.key}': e.value},
    'currentPreset': currentPreset,
    'deviceName': deviceName,
    'gains': gains,
    'mutes': mutes,
    'phase': phase,
    'delays': delays,
    'matrixRouting': matrixRouting,
    'matrixGains': matrixGains,
    'geqBands': geqBands,
    'peqBands': {
      for (final e in peqBands.entries)
        e.key: [for (final b in e.value) b.toJson()],
    },
    'hiPass': {for (final e in hiPass.entries) e.key: e.value.toJson()},
    'loPass': {for (final e in loPass.entries) e.key: e.value.toJson()},
    'gates': {for (final e in gates.entries) e.key: e.value.toJson()},
    'compressors': {for (final e in compressors.entries) e.key: e.value.toJson()},
    'limiters': {for (final e in limiters.entries) e.key: e.value.toJson()},
    'channelNames': channelNames,
    'links': links,
    'eqBypass': eqBypass,
    'geqBypass': geqBypass,
    'testTone': [testToneType, testToneParam],
    'delayUnit': delayUnit,
    'presetSlots': [...presetSlots]..sort(),
    'currentPresetNumber': currentPresetNumber,
  };

  factory DeviceState.fromJson(Map<String, Object?> json) {
    Map<String, Object?> map(String key) => ((json[key] as Map?) ?? const {}).cast<String, Object?>();
    Map<String, Object?> obj(Object? v) => (v as Map).cast<String, Object?>();
    final state = DeviceState(DeviceModel.byId(json['model'] as String?));

    map('presets').forEach((k, v) => state.presets[int.parse(k)] = v as String);
    state.currentPreset = json['currentPreset'] as String;
    state.deviceName = json['deviceName'] as String;
    map('gains').forEach((k, v) => state.gains[k] = (v as num).toDouble());
    map('mutes').forEach((k, v) => state.mutes[k] = v as bool);
    map('phase').forEach((k, v) => state.phase[k] = v as bool);
    map('delays').forEach((k, v) => state.delays[k] = (v as num).toInt());
    map('matrixRouting').forEach((k, v) => state.matrixRouting[k] = (v as num).toInt());
    map('matrixGains').forEach((k, v) => state.matrixGains[k] = (v as num).toDouble());
    map('geqBands').forEach((k, v) =>
        state.geqBands[k] = [for (final x in v as List) (x as num).toDouble()]);
    map('peqBands').forEach((k, v) => state.peqBands[k] = [
          for (final b in v as List) PeqBand.fromJson(obj(b)),
        ]);
    map('hiPass').forEach((k, v) => state.hiPass[k] = FilterState.fromJson(obj(v)));
    map('loPass').forEach((k, v) => state.loPass[k] = FilterState.fromJson(obj(v)));
    map('gates').forEach((k, v) => state.gates[k] = GateState.fromJson(obj(v)));
    map('compressors').forEach((k, v) => state.compressors[k] = CompressorState.fromJson(obj(v)));
    map('limiters').forEach((k, v) => state.limiters[k] = LimiterState.fromJson(obj(v)));
    map('channelNames').forEach((k, v) => state.channelNames[k] = v as String);
    map('links').forEach((k, v) => state.links[k] = (v as num).toInt());
    map('eqBypass').forEach((k, v) => state.eqBypass[k] = v as bool);
    map('geqBypass').forEach((k, v) => state.geqBypass[k] = v as bool);
    final tone = json['testTone'];
    if (tone is List && tone.length == 2) {
      state.testToneType = (tone[0] as num).toInt();
      state.testToneParam = (tone[1] as num).toInt();
    }
    state.delayUnit = (json['delayUnit'] as num? ?? 0).toInt();
    state.presetSlots.addAll([for (final i in (json['presetSlots'] as List? ?? const [])) (i as num).toInt()]);
    state.currentPresetNumber = (json['currentPresetNumber'] as num? ?? -1).toInt();
    return state;
  }
}
