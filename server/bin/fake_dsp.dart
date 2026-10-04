// Development tool: simulate DSP 206 / DSP 408 devices.
//
//   dart run bin/fake_dsp.dart --port 19761 --models 408,206
//
// starts a simulated DSP 408 on port 19761 and a DSP 206 on port 19762.

import 'dart:io';

import 'package:args/args.dart';
import 'package:dsp_core/dsp_core.dart';
import 'package:dsp_server/src/fake_dsp.dart';

Future<void> main(List<String> arguments) async {
  final parser = ArgParser()
    ..addOption('port', defaultsTo: '19761', help: 'TCP port of the first device')
    ..addOption('models', defaultsTo: '408', help: 'Comma-separated models, one device each (408, 206)')
    ..addOption('latency', defaultsTo: '5', help: 'Response delay in ms')
    ..addFlag('quiet', abbr: 'q', help: 'Do not print received set commands')
    ..addFlag('help', abbr: 'h', negatable: false);
  final args = parser.parse(arguments);
  if (args.flag('help')) {
    stdout.writeln('Simulated t.racks DSP 206/408 devices for development.\n\n${parser.usage}');
    return;
  }

  final firstPort = int.parse(args.option('port')!);
  final latency = Duration(milliseconds: int.parse(args.option('latency')!));
  final models = [
    for (final m in args.option('models')!.split(','))
      DeviceModel.byId('dsp${m.trim().replaceAll(RegExp(r'[^0-9]'), '')}'),
  ];

  final devices = <FakeDsp>[];
  for (var i = 0; i < models.length; i++) {
    final dsp = FakeDsp(model: models[i], latency: latency, verbose: !args.flag('quiet'),
        label: 'Fake ${models[i].name} #${i + 1}');
    await dsp.start(port: firstPort + i, address: InternetAddress.anyIPv4);
    devices.add(dsp);
    stdout.writeln('${dsp.label} listening on port ${dsp.port}');
  }

  ProcessSignal.sigint.watch().listen((_) async {
    for (final d in devices) {
      await d.stop();
    }
    exit(0);
  });
}
