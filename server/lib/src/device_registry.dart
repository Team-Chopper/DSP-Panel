// Device Registry
//
// The list of configured DSPs, persisted as JSON in the data directory.

import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:dsp_core/dsp_core.dart';

/// Values the panel cannot read back from the device yet (their position in
/// the config dump is unknown, see TODO.md). The server remembers what was
/// last set through the panel and shows that after connecting.
class RememberedValues {
  final Map<String, bool> mutes;
  final Map<String, bool> phase;
  final Map<String, int> delays;
  final Map<String, List<bool>> peqBypass;

  RememberedValues({
    Map<String, bool>? mutes,
    Map<String, bool>? phase,
    Map<String, int>? delays,
    Map<String, List<bool>>? peqBypass,
  })  : mutes = mutes ?? {},
        phase = phase ?? {},
        delays = delays ?? {},
        peqBypass = peqBypass ?? {};

  /// Copy the remembered values into [state] (channels the model has).
  void applyTo(DeviceState state) {
    final model = state.model;
    for (final e in mutes.entries) {
      if (model.hasChannel(e.key)) state.mutes[e.key] = e.value;
    }
    for (final e in phase.entries) {
      if (model.hasChannel(e.key)) state.phase[e.key] = e.value;
    }
    for (final e in delays.entries) {
      if (model.hasChannel(e.key)) state.delays[e.key] = e.value;
    }
    peqBypass.forEach((ch, flags) {
      final bands = state.peqBands[ch];
      if (bands == null) return;
      for (var i = 0; i < bands.length && i < flags.length; i++) {
        bands[i].bypass = flags[i];
      }
    });
  }

  /// Remember the values an op changed. Returns true if anything changed.
  bool record(DeviceState state, Map<String, Object?> op) {
    final ch = op['ch'] as String?;
    if (ch == null) return false;
    switch (op['op']) {
      case 'mute':
        mutes[ch] = state.mutes[ch]!;
      case 'phase':
        phase[ch] = state.phase[ch]!;
      case 'delay':
        delays[ch] = state.delays[ch]!;
      case 'peqBand':
        peqBypass[ch] = [for (final b in state.peqBands[ch]!) b.bypass];
      default:
        return false;
    }
    return true;
  }

  Map<String, Object?> toJson() => {
    'mutes': mutes,
    'phase': phase,
    'delays': delays,
    'peqBypass': peqBypass,
  };

  factory RememberedValues.fromJson(Map<String, Object?>? json) {
    Map<String, Object?> map(String key) => ((json?[key] as Map?) ?? const {}).cast<String, Object?>();
    return RememberedValues(
      mutes: {for (final e in map('mutes').entries) if (e.value is bool) e.key: e.value as bool},
      phase: {for (final e in map('phase').entries) if (e.value is bool) e.key: e.value as bool},
      delays: {for (final e in map('delays').entries) if (e.value is int) e.key: e.value as int},
      peqBypass: {
        for (final e in map('peqBypass').entries)
          if (e.value is List) e.key: [for (final b in e.value as List) b == true],
      },
    );
  }
}

/// Persistent settings of one DSP.
class DspConfig {
  final String id;
  final String name;
  final String host;
  final int port;
  final bool enabled;
  final int refreshMs;

  /// 'auto' (detect from the device) or a DeviceModel id.
  final String model;
  final RememberedValues remembered;

  DspConfig({
    required this.id,
    required this.name,
    required this.host,
    this.port = 9761,
    this.enabled = true,
    this.refreshMs = 300,
    this.model = 'auto',
    RememberedValues? remembered,
  }) : remembered = remembered ?? RememberedValues();

  DspConfig copyWith({
    String? name,
    String? host,
    int? port,
    bool? enabled,
    int? refreshMs,
    String? model,
  }) =>
      DspConfig(
        id: id,
        name: name ?? this.name,
        host: host ?? this.host,
        port: port ?? this.port,
        enabled: enabled ?? this.enabled,
        refreshMs: refreshMs ?? this.refreshMs,
        model: model ?? this.model,
        remembered: remembered,
      );

  Map<String, Object?> toJson() => {
    'id': id,
    'name': name,
    'host': host,
    'port': port,
    'enabled': enabled,
    'refreshMs': refreshMs,
    'model': model,
    'remembered': remembered.toJson(),
  };

  factory DspConfig.fromJson(Map<String, Object?> json) {
    final model = json['model'] as String? ?? 'auto';
    return DspConfig(
      id: json['id'] as String,
      name: json['name'] as String,
      host: json['host'] as String,
      port: (json['port'] as num?)?.toInt() ?? 9761,
      enabled: json['enabled'] as bool? ?? true,
      refreshMs: (json['refreshMs'] as num?)?.toInt() ?? 300,
      model: isValidModelSetting(model) ? model : 'auto',
      remembered: RememberedValues.fromJson((json['remembered'] as Map?)?.cast<String, Object?>()),
    );
  }

  static String newId() {
    final rnd = Random.secure();
    return List.generate(8, (_) => rnd.nextInt(16).toRadixString(16)).join();
  }

  static bool isValidModelSetting(String model) => model == 'auto' || DeviceModel.isKnownId(model);

  /// Validate user input for name/host/port/model. Returns an error or null.
  static String? validateFields({String? name, String? host, int? port, String? model}) {
    if (name != null && (name.trim().isEmpty || name.length > 40)) {
      return 'Name must be 1–40 characters';
    }
    if (host != null &&
        (host.isEmpty || host.length > 253 || !RegExp(r'^[A-Za-z0-9.\-:\[\]]+$').hasMatch(host))) {
      return 'Invalid IP address or host name';
    }
    if (port != null && (port < 1 || port > 65535)) {
      return 'Port must be between 1 and 65535';
    }
    if (model != null && !isValidModelSetting(model)) {
      return 'Unknown model';
    }
    return null;
  }
}

class DeviceRegistry {
  final File file;

  DeviceRegistry(this.file);

  List<DspConfig> load() {
    if (!file.existsSync()) return [];
    final json = jsonDecode(file.readAsStringSync()) as Map<String, Object?>;
    return [
      for (final d in (json['devices'] as List? ?? const []))
        DspConfig.fromJson((d as Map).cast<String, Object?>()),
    ];
  }

  /// Write atomically: temp file + rename.
  Future<void> save(List<DspConfig> devices) async {
    await file.parent.create(recursive: true);
    final tmp = File('${file.path}.tmp');
    const encoder = JsonEncoder.withIndent('  ');
    await tmp.writeAsString(encoder.convert({
      'devices': [for (final d in devices) d.toJson()],
    }), flush: true);
    await tmp.rename(file.path);
  }
}
