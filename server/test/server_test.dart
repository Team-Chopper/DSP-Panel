import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:dsp_core/dsp_core.dart';
import 'package:dsp_server/src/auth.dart';
import 'package:dsp_server/src/config.dart';
import 'package:dsp_server/src/device_registry.dart';
import 'package:dsp_server/src/dsp_session.dart';
import 'package:dsp_server/src/fake_dsp.dart';
import 'package:dsp_server/src/http_server.dart';
import 'package:dsp_server/src/hub.dart';
import 'package:http/http.dart' as http;
import 'package:test/test.dart';
import 'package:web_socket_channel/io.dart';

/// A WebSocket client that records every message it receives.
class TestClient {
  final IOWebSocketChannel channel;
  final List<Map<String, Object?>> messages = [];

  TestClient(this.channel) {
    channel.stream.listen((data) => messages.add((jsonDecode(data as String) as Map).cast()));
  }

  void send(Map<String, Object?> message) => channel.sink.add(jsonEncode(message));

  Iterable<Map<String, Object?>> ofType(String type) => messages.where((m) => m['t'] == type);

  Future<Map<String, Object?>> waitFor(bool Function(Map<String, Object?> m) test) async {
    final deadline = DateTime.now().add(const Duration(seconds: 10));
    while (true) {
      for (final m in messages) {
        if (test(m)) return m;
      }
      if (DateTime.now().isAfter(deadline)) fail('Message not received. Got: ${messages.map((m) => m['t'])}');
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
  }

  Future<void> close() => channel.sink.close();
}

void main() {
  late Directory dataDir;
  late FakeDsp fake;
  late AuthService auth;
  late Hub hub;
  late PanelServer server;
  late String base;

  setUp(() async {
    dataDir = await Directory.systemTemp.createTemp('dsp_server');
    fake = FakeDsp();
    await fake.start();

    final config = ServerConfig(
      address: '127.0.0.1',
      port: 0,
      tlsEnabled: false,
      dataDir: dataDir.path,
      webRoot: '${dataDir.path}/web',
    );
    await Directory(config.webRoot).create();
    await File('${config.webRoot}/index.html').writeAsString('<html>panel</html>');

    auth = AuthService(enabled: true, dataDir: dataDir.path);
    await auth.init();
    await auth.setPassword('secret-password');
    hub = Hub(
      DeviceRegistry(File('${dataDir.path}/devices.json')),
      sessionFactory: (c) => DspSession(c,
          sequenceGap: Duration.zero,
          requestTimeout: const Duration(milliseconds: 200),
          reconnectDelays: const [Duration(milliseconds: 100)]),
    );
    await hub.start();
    server = PanelServer(config: config, auth: auth, hub: hub);
    await server.start();
    base = '127.0.0.1:${server.port}';
  });

  tearDown(() async {
    await server.stop();
    await hub.shutdown();
    await fake.stop();
    await dataDir.delete(recursive: true);
  });

  Future<String> login() async {
    final response = await http.post(Uri.parse('http://$base/api/login'),
        body: jsonEncode({'password': 'secret-password'}));
    expect(response.statusCode, 200);
    final cookie = response.headers['set-cookie']!;
    expect(cookie, contains('HttpOnly'));
    expect(cookie, contains('SameSite=Strict'));
    return cookie.split(';').first;
  }

  Future<TestClient> connect(String cookie, {String? origin}) async {
    final channel = IOWebSocketChannel.connect(Uri.parse('ws://$base/ws'),
        headers: {'cookie': cookie, 'origin': origin ?? 'http://$base'});
    await channel.ready;
    return TestClient(channel);
  }

  test('serves the web client and session info without login', () async {
    final index = await http.get(Uri.parse('http://$base/'));
    expect(index.body, contains('panel'));
    expect(index.headers['x-content-type-options'], 'nosniff');

    final session = jsonDecode((await http.get(Uri.parse('http://$base/api/session'))).body);
    expect(session, {'authEnabled': true, 'authenticated': false, 'passwordSet': true});
  });

  test('login is required for the WebSocket', () async {
    final wrong = await http.post(Uri.parse('http://$base/api/login'),
        body: jsonEncode({'password': 'nope'}));
    expect(wrong.statusCode, 401);

    expect(() => connect('dsp_session=forged.token.value'), throwsA(anything));

    final cookie = await login();
    final session = jsonDecode((await http.get(Uri.parse('http://$base/api/session'),
        headers: {'cookie': cookie})).body);
    expect(session['authenticated'], isTrue);
    final client = await connect(cookie);
    await client.waitFor((m) => m['t'] == Msg.hello);
    await client.close();
  });

  test('WebSocket from a foreign origin is rejected', () async {
    final cookie = await login();
    expect(() => connect(cookie, origin: 'https://evil.example'), throwsA(anything));
  });

  test('two clients manage a DSP and stay in sync', () async {
    final cookie = await login();
    final a = await connect(cookie);
    final b = await connect(cookie);
    await a.waitFor((m) => m['t'] == Msg.hello);

    a.send({'t': Msg.addDsp, 'name': 'FOH', 'host': '127.0.0.1', 'port': fake.port});
    final info = await b.waitFor((m) =>
        m['t'] == Msg.dspInfo && (m['info'] as Map)['status'] == DspStatus.ready.name);
    final id = (info['info'] as Map)['id'] as String;

    a.send({'t': Msg.subscribe, 'dsp': id});
    b.send({'t': Msg.subscribe, 'dsp': id});
    final state = await b.waitFor((m) => m['t'] == Msg.state);
    expect(((state['state'] as Map)['presets'] as Map).length, 20);

    a.send({'t': Msg.op, 'dsp': id, 'op': DeviceOps.gain('Out 3', -12)});
    final op = await b.waitFor((m) => m['t'] == Msg.op);
    expect(op['op'], DeviceOps.gain('Out 3', -12));
    await b.waitFor((m) => m['t'] == Msg.meters);
    await Future<void>.delayed(const Duration(milliseconds: 100));
    expect(a.ofType(Msg.op), isEmpty, reason: 'sender does not get its own op back');
    expect(fake.receivedSetCommands, contains('SET GAIN Out 3 → -12.0 dB'));

    // Invalid op: error plus resync for the sender only.
    final statesBefore = a.ofType(Msg.state).length;
    a.send({'t': Msg.op, 'dsp': id, 'op': {'op': 'gain', 'ch': 'Out 9', 'db': 0}});
    await a.waitFor((m) => m['t'] == Msg.error);
    await a.waitFor((_) => a.ofType(Msg.state).length > statesBefore);

    // Channel names go to the device; values the dump does not contain
    // (mute) are remembered by the server.
    a.send({'t': Msg.op, 'dsp': id, 'op': DeviceOps.channelName('In A', 'Vocals')});
    a.send({'t': Msg.op, 'dsp': id, 'op': DeviceOps.mute('Out 2', true)});
    await Future<void>.delayed(const Duration(milliseconds: 700));
    expect(fake.state.channelNames['In A'], 'Vocals');
    final saved = DeviceRegistry(File('${dataDir.path}/devices.json')).load();
    expect(saved.single.remembered.mutes['Out 2'], isTrue);
    expect(saved.single.port, fake.port);
    expect(saved.single.model, 'auto');

    a.send({'t': Msg.removeDsp, 'id': id});
    await b.waitFor((m) => m['t'] == Msg.dspRemoved);
    await a.close();
    await b.close();
  });

  test('server presets: save from a DSP, edit offline, load onto the DSP', () async {
    final cookie = await login();
    final a = await connect(cookie);
    final b = await connect(cookie);
    await a.waitFor((m) => m['t'] == Msg.presets);

    a.send({'t': Msg.addDsp, 'name': 'FOH', 'host': '127.0.0.1', 'port': fake.port});
    final info = await a.waitFor((m) =>
        m['t'] == Msg.dspInfo && (m['info'] as Map)['status'] == DspStatus.ready.name);
    final dspId = (info['info'] as Map)['id'] as String;

    // Snapshot of the DSP and an empty preset for the other model.
    a.send({'t': Msg.presetCreate, 'name': 'Hall north', 'fromDsp': dspId});
    a.send({'t': Msg.presetCreate, 'name': 'Club', 'model': 'dsp206'});
    a.send({'t': Msg.presetCreate, 'name': ''});
    await a.waitFor((m) => m['t'] == Msg.error);
    final list = await b.waitFor((m) => m['t'] == Msg.presets && (m['presets'] as List).length == 2);
    final hall = (list['presets'] as List).cast<Map>().firstWhere((p) => p['name'] == 'Hall north');
    expect(hall['model'], 'dsp408');
    final presetDsp = '${Msg.presetPrefix}${hall['id']}';

    // Offline editing, synchronised between clients.
    a.send({'t': Msg.subscribe, 'dsp': presetDsp});
    b.send({'t': Msg.subscribe, 'dsp': presetDsp});
    await b.waitFor((m) => m['t'] == Msg.state && m['dsp'] == presetDsp);
    a.send({'t': Msg.op, 'dsp': presetDsp, 'op': DeviceOps.gain('Out 5', -15)});
    a.send({'t': Msg.op, 'dsp': presetDsp, 'op': DeviceOps.delay('Out 6', 960)});
    await b.waitFor((m) => m['t'] == Msg.op && m['dsp'] == presetDsp);
    expect(fake.receivedSetCommands.where((c) => c.contains('Out 5')), isEmpty, reason: 'offline');

    // Load onto the DSP.
    a.send({'t': Msg.presetApply, 'preset': hall['id'], 'dsp': dspId});
    await a.waitFor((m) =>
        m['t'] == Msg.dspInfo && (m['info'] as Map)['status'] == DspStatus.busy.name);
    await a.waitFor((m) =>
        m['t'] == Msg.dspInfo &&
        (m['info'] as Map)['status'] == DspStatus.ready.name &&
        a.messages.any((x) => x['t'] == Msg.dspInfo && (x['info'] as Map)['status'] == DspStatus.busy.name));
    await Future<void>.delayed(const Duration(milliseconds: 200));
    expect(fake.state.gains['Out 5'], -15.0);
    expect(fake.state.delays['Out 6'], 960);

    // Persisted.
    await hub.presets.save();
    final saved = jsonDecode(File('${dataDir.path}/presets.json').readAsStringSync()) as Map;
    expect((saved['presets'] as List).length, 2);

    a.send({'t': Msg.presetDelete, 'preset': hall['id']});
    await b.waitFor((m) => m['t'] == Msg.presets && (m['presets'] as List).length == 1);
    await a.close();
    await b.close();
  });

  test('rejects invalid DSP settings', () async {
    final client = await connect(await login());
    client.send({'t': Msg.addDsp, 'name': '', 'host': '1.2.3.4', 'port': 9761});
    client.send({'t': Msg.addDsp, 'name': 'X', 'host': 'a b', 'port': 9761});
    client.send({'t': Msg.addDsp, 'name': 'X', 'host': '1.2.3.4', 'port': 70000});
    client.send({'t': 'bogus'});
    await client.waitFor((_) => client.ofType(Msg.error).length == 4);
    expect(hub.sessions, isEmpty);
    await client.close();
  });
}
