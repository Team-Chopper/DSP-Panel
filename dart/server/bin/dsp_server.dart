// DSP panel server (t.racks DSP 408 / DSP 206)
//
//   dsp-server [serve] --config /etc/dsp-panel/config.yaml
//   dsp-server passwd  --config /etc/dsp-panel/config.yaml

import 'dart:async';
import 'dart:io';

import 'package:args/args.dart';
import 'package:dsp_server/src/auth.dart';
import 'package:dsp_server/src/config.dart';
import 'package:dsp_server/src/device_registry.dart';
import 'package:dsp_server/src/http_server.dart';
import 'package:dsp_server/src/hub.dart';
import 'package:dsp_server/src/log.dart';
import 'package:path/path.dart' as p;

const defaultConfigPath = '/etc/dsp-panel/config.yaml';

Future<void> main(List<String> arguments) async {
  final parser = ArgParser()
    ..addOption('config', abbr: 'c', help: 'Config file (default: $defaultConfigPath if it exists)')
    ..addOption('port', help: 'Override listen port')
    ..addOption('address', help: 'Override listen address')
    ..addOption('data-dir', help: 'Override data directory')
    ..addOption('web-root', help: 'Override web client directory')
    ..addFlag('tls', defaultsTo: null, help: 'Override TLS on/off')
    ..addFlag('auth', defaultsTo: null, help: 'Override login on/off')
    ..addFlag('verbose', abbr: 'v', negatable: false, help: 'Debug output')
    ..addFlag('help', abbr: 'h', negatable: false);

  final ArgResults args;
  try {
    args = parser.parse(arguments);
  } on FormatException catch (e) {
    stderr.writeln('${e.message}\n\n${parser.usage}');
    exit(64);
  }
  if (args.flag('help')) {
    stdout.writeln('DSP panel server (t.racks DSP 408 / DSP 206)\n\n'
        'Usage: dsp-server [serve|passwd] [options]\n\n${parser.usage}');
    return;
  }
  verboseLogging = args.flag('verbose');

  final ServerConfig config;
  try {
    config = _loadConfig(args);
  } catch (e) {
    stderr.writeln('Config error: $e');
    exit(78);
  }

  final command = args.rest.isEmpty ? 'serve' : args.rest.first;
  switch (command) {
    case 'serve':
      await _serve(config);
    case 'passwd':
      await _passwd(config);
    default:
      stderr.writeln('Unknown command "$command"\n\n${parser.usage}');
      exit(64);
  }
}

ServerConfig _loadConfig(ArgResults args) {
  var path = args.option('config');
  if (path == null && File(defaultConfigPath).existsSync()) path = defaultConfigPath;
  var config = path != null ? ServerConfig.load(path) : const ServerConfig();

  final port = args.option('port');
  return config.copyWith(
    port: port != null ? int.parse(port) : null,
    address: args.option('address'),
    dataDir: _absolute(args.option('data-dir')),
    webRoot: _absolute(args.option('web-root')),
    tlsEnabled: args['tls'] as bool?,
    authEnabled: args['auth'] as bool?,
  );
}

String? _absolute(String? path) => path == null ? null : p.normalize(p.absolute(path));

Future<void> _serve(ServerConfig config) async {
  logInfo('Starting DSP panel server ($config)');

  final auth = AuthService(enabled: config.authEnabled, dataDir: config.dataDir);
  await auth.init();
  if (auth.enabled && !auth.hasPassword) {
    logWarn('Login is enabled but no password is set – run "dsp-server passwd".');
  }
  if (!auth.enabled) {
    logWarn('Login is disabled – everyone who can reach this server can control the DSPs.');
  }

  final hub = Hub(DeviceRegistry(File(p.join(config.dataDir, 'devices.json'))));
  await hub.start();

  final server = PanelServer(config: config, auth: auth, hub: hub);
  try {
    await server.start();
  } catch (e) {
    stderr.writeln('Could not start server: $e');
    if (config.tlsEnabled) {
      stderr.writeln('Check tls.cert (${config.tlsCert}) and tls.key (${config.tlsKey}).');
    }
    await hub.shutdown();
    exit(1);
  }

  final done = Completer<void>();
  Future<void> shutdown(ProcessSignal signal) async {
    if (done.isCompleted) return;
    logInfo('Received $signal, shutting down');
    await server.stop();
    await hub.shutdown();
    done.complete();
  }

  ProcessSignal.sigint.watch().listen(shutdown);
  if (!Platform.isWindows) ProcessSignal.sigterm.watch().listen(shutdown);
  await done.future;
  exit(0);
}

Future<void> _passwd(ServerConfig config) async {
  if (!stdin.hasTerminal) {
    // Allow scripting: read the password from the first line of stdin.
    final password = stdin.readLineSync();
    if (password == null || password.length < 8) {
      stderr.writeln('Password must have at least 8 characters.');
      exit(1);
    }
    await _storePassword(config, password);
    return;
  }

  String prompt(String text) {
    stdout.write(text);
    stdin.echoMode = false;
    try {
      return stdin.readLineSync() ?? '';
    } finally {
      stdin.echoMode = true;
      stdout.writeln();
    }
  }

  final password = prompt('New password: ');
  if (password.length < 8) {
    stderr.writeln('Password must have at least 8 characters.');
    exit(1);
  }
  if (prompt('Repeat password: ') != password) {
    stderr.writeln('Passwords do not match.');
    exit(1);
  }
  await _storePassword(config, password);
}

Future<void> _storePassword(ServerConfig config, String password) async {
  final auth = AuthService(enabled: true, dataDir: config.dataDir);
  await auth.init();
  await auth.setPassword(password);
  stdout.writeln('Password saved in ${p.join(config.dataDir, 'auth.json')}. '
      'Existing logins are now invalid.');
  if (!config.authEnabled) {
    stdout.writeln('Note: login is currently disabled (auth.enabled: false).');
  }
}
