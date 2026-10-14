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

  /// Device ID of the DSP (1 … 250), the address byte of the frames.
  final int deviceId;

  /// Password of a locked DSP (4 characters) or ''.
  final String password;

  DspConfig({
    required this.id,
    required this.name,
    required this.host,
    this.port = 9761,
    this.enabled = true,
    this.refreshMs = defaultRefreshMs,
    this.model = 'auto',
    this.deviceId = 1,
    this.password = '',
  });

  DspConfig copyWith({
    String? name,
    String? host,
    int? port,
    bool? enabled,
    int? refreshMs,
    String? model,
    int? deviceId,
    String? password,
  }) =>
      DspConfig(
        id: id,
        name: name ?? this.name,
        host: host ?? this.host,
        port: port ?? this.port,
        enabled: enabled ?? this.enabled,
        refreshMs: refreshMs ?? this.refreshMs,
        model: model ?? this.model,
        deviceId: deviceId ?? this.deviceId,
        password: password ?? this.password,
      );

  Map<String, Object?> toJson() => {
    'id': id,
    'name': name,
    'host': host,
    'port': port,
    'enabled': enabled,
    'refreshMs': refreshMs,
    'model': model,
    'deviceId': deviceId,
    if (password.isNotEmpty) 'password': password,
  };

  /// Older versions also stored "remembered" mute/phase/delay/PEQ bypass
  /// values; they are read from the device now and the key is ignored.
  factory DspConfig.fromJson(Map<String, Object?> json) {
    final model = json['model'] as String? ?? 'auto';
    final deviceId = (json['deviceId'] as num?)?.toInt() ?? 1;
    final password = json['password'] as String? ?? '';
    return DspConfig(
      id: json['id'] as String,
      name: json['name'] as String,
      host: json['host'] as String,
      port: (json['port'] as num?)?.toInt() ?? 9761,
      enabled: json['enabled'] as bool? ?? true,
      refreshMs: _refreshFromJson(json['refreshMs']),
      model: isValidModelSetting(model) ? model : 'auto',
      deviceId: deviceId.clamp(ProtocolService.minDeviceId, ProtocolService.maxDeviceId),
      password: isValidPassword(password) ? password.toUpperCase() : '',
    );
  }

  /// Stored interval; 40 ms was the default of versions 0.7–0.8 and is
  /// replaced by the original editor's 300 ms.
  static int _refreshFromJson(Object? value) {
    final ms = (value as num?)?.toInt();
    return (ms == null || ms == 40) ? defaultRefreshMs : ms.clamp(5, 300);
  }

  /// Empty (no password) or 4 printable ASCII characters, like the
  /// original editor's lock password.
  static bool isValidPassword(String password) =>
      password.isEmpty || (password.length == 4 && password.codeUnits.every((c) => c > 0x20 && c <= 0x7E));

  static String newId() {
    final rnd = Random.secure();
    return List.generate(8, (_) => rnd.nextInt(16).toRadixString(16)).join();
  }

  static bool isValidModelSetting(String model) => model == 'auto' || DeviceModel.isKnownId(model);

  /// Validate user input for name/host/port/model/device ID/password.
  /// Returns an error or null.
  static String? validateFields(
      {String? name, String? host, int? port, String? model, int? deviceId, String? password}) {
    if (deviceId != null && (deviceId < ProtocolService.minDeviceId || deviceId > ProtocolService.maxDeviceId)) {
      return 'Device ID must be between ${ProtocolService.minDeviceId} and ${ProtocolService.maxDeviceId}';
    }
    if (password != null && !isValidPassword(password)) {
      return 'The password has 4 characters (empty = none)';
    }
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
