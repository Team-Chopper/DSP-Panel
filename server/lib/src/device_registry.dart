// Device Registry
//
// The list of configured DSPs, persisted as JSON in the data directory.

import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:dsp_core/dsp_core.dart';

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

  DspConfig({
    required this.id,
    required this.name,
    required this.host,
    this.port = 9761,
    this.enabled = true,
    this.refreshMs = defaultRefreshMs,
    this.model = 'auto',
  });

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
      );

  Map<String, Object?> toJson() => {
    'id': id,
    'name': name,
    'host': host,
    'port': port,
    'enabled': enabled,
    'refreshMs': refreshMs,
    'model': model,
  };

  /// Older versions also stored "remembered" mute/phase/delay/PEQ bypass
  /// values; they are read from the device now and the key is ignored.
  factory DspConfig.fromJson(Map<String, Object?> json) {
    final model = json['model'] as String? ?? 'auto';
    return DspConfig(
      id: json['id'] as String,
      name: json['name'] as String,
      host: json['host'] as String,
      port: (json['port'] as num?)?.toInt() ?? 9761,
      enabled: json['enabled'] as bool? ?? true,
      refreshMs: _refreshFromJson(json['refreshMs']),
      model: isValidModelSetting(model) ? model : 'auto',
    );
  }

  /// Stored interval; 300 ms was the old default (too slow for the meters)
  /// and is replaced by the current default.
  static int _refreshFromJson(Object? value) {
    final ms = (value as num?)?.toInt();
    return (ms == null || ms == 300) ? defaultRefreshMs : ms.clamp(5, 300);
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
