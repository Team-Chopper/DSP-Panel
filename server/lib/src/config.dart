// Server configuration (config.yaml)
//
// listen:
//   address: 0.0.0.0
//   port: 8443
// tls:
//   enabled: true
//   cert: /var/lib/dsp-panel/tls/cert.pem
//   key: /var/lib/dsp-panel/tls/key.pem
// auth:
//   enabled: true
// data_dir: /var/lib/dsp-panel
// web_root: /opt/dsp-panel/web

import 'dart:io';

import 'package:path/path.dart' as p;
import 'package:yaml/yaml.dart';

class ServerConfig {
  final String address;
  final int port;
  final bool tlsEnabled;
  final String tlsCert;
  final String tlsKey;
  final bool authEnabled;
  final String dataDir;
  final String webRoot;

  const ServerConfig({
    this.address = '0.0.0.0',
    this.port = 8443,
    this.tlsEnabled = true,
    this.tlsCert = '/var/lib/dsp-panel/tls/cert.pem',
    this.tlsKey = '/var/lib/dsp-panel/tls/key.pem',
    this.authEnabled = true,
    this.dataDir = '/var/lib/dsp-panel',
    this.webRoot = '/opt/dsp-panel/web',
  });

  ServerConfig copyWith({
    String? address,
    int? port,
    bool? tlsEnabled,
    String? tlsCert,
    String? tlsKey,
    bool? authEnabled,
    String? dataDir,
    String? webRoot,
  }) =>
      ServerConfig(
        address: address ?? this.address,
        port: port ?? this.port,
        tlsEnabled: tlsEnabled ?? this.tlsEnabled,
        tlsCert: tlsCert ?? this.tlsCert,
        tlsKey: tlsKey ?? this.tlsKey,
        authEnabled: authEnabled ?? this.authEnabled,
        dataDir: dataDir ?? this.dataDir,
        webRoot: webRoot ?? this.webRoot,
      );

  /// Load a YAML config file. Missing keys keep their defaults; relative
  /// paths are resolved against the directory of the config file.
  static ServerConfig load(String path) {
    final file = File(path);
    final yaml = loadYaml(file.readAsStringSync());
    if (yaml == null) return const ServerConfig();
    if (yaml is! YamlMap) {
      throw FormatException('$path: expected a mapping at top level');
    }
    final base = p.dirname(file.absolute.path);
    String resolve(String value) => p.isAbsolute(value) ? value : p.normalize(p.join(base, value));

    T? get<T>(YamlMap? map, String key) {
      final value = map?[key];
      if (value == null) return null;
      if (value is! T) throw FormatException('$path: "$key" has the wrong type');
      return value;
    }

    YamlMap? section(String key) => get<YamlMap>(yaml, key);

    const d = ServerConfig();
    final listen = section('listen');
    final tls = section('tls');
    final auth = section('auth');
    final config = ServerConfig(
      address: get<String>(listen, 'address') ?? d.address,
      port: get<int>(listen, 'port') ?? d.port,
      tlsEnabled: get<bool>(tls, 'enabled') ?? d.tlsEnabled,
      tlsCert: resolve(get<String>(tls, 'cert') ?? d.tlsCert),
      tlsKey: resolve(get<String>(tls, 'key') ?? d.tlsKey),
      authEnabled: get<bool>(auth, 'enabled') ?? d.authEnabled,
      dataDir: resolve(get<String>(yaml, 'data_dir') ?? d.dataDir),
      webRoot: resolve(get<String>(yaml, 'web_root') ?? d.webRoot),
    );
    if (config.port < 1 || config.port > 65535) {
      throw FormatException('$path: listen.port must be 1-65535');
    }
    return config;
  }

  @override
  String toString() => 'listen=$address:$port tls=$tlsEnabled auth=$authEnabled '
      'data_dir=$dataDir web_root=$webRoot';
}
