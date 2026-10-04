import 'dart:async';

import 'package:dsp/devices/t_racks408/dsp_page.dart';
import 'package:dsp/providers/panel_provider.dart';
import 'package:dsp/services/server_connection.dart';
import 'package:dsp/theme/app_theme.dart';
import 'package:dsp_core/dsp_core.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

/// Server connection without a socket: records what is sent, messages from
/// the "server" are injected with [receive].
class _FakeServer extends ServerConnection {
  final sent = <Map<String, Object?>>[];
  final _incoming = StreamController<Map<String, Object?>>.broadcast(sync: true);

  _FakeServer() : super(base: Uri.parse('http://localhost/'));

  @override
  Stream<Map<String, Object?>> get messages => _incoming.stream;

  @override
  void send(Map<String, Object?> message) => sent.add(message);

  void receive(Map<String, Object?> message) => _incoming.add(message);
}

const _foh = DspInfo(id: 'd1', name: 'FOH', host: '10.0.0.5', port: 9761, enabled: true, status: DspStatus.ready);

Map<String, Object?> _hello(List<DspInfo> dsps) => {
      't': Msg.hello,
      'dsps': [for (final d in dsps) d.toJson()],
    };

Map<String, Object?> _presets(List<ServerPresetInfo> presets) => {
      't': Msg.presets,
      'presets': [for (final p in presets) p.toJson()],
    };

void main() {
  group('PanelProvider', () {
    late _FakeServer server;
    late PanelProvider panel;

    setUp(() {
      server = _FakeServer();
      panel = PanelProvider(server);
      server.receive(_hello([_foh]));
    });

    test('lists server presets per model', () {
      server.receive(_presets(const [
        ServerPresetInfo(id: 'p1', name: 'Club', model: 'dsp408', updated: 1),
        ServerPresetInfo(id: 'p2', name: 'Bar', model: 'dsp206', updated: 2),
      ]));
      expect(panel.serverPresetsFor(DeviceModel.dsp408).map((p) => p.name), ['Club']);
      expect(panel.serverPresetsFor(DeviceModel.dsp206).map((p) => p.name), ['Bar']);

      panel.savePresetFromDsp('d1', 'Saved');
      panel.applyServerPreset('p1', 'd1');
      expect(server.sent, [
        {'t': Msg.presetCreate, 'name': 'Saved', 'fromDsp': 'd1'},
        {'t': Msg.presetApply, 'preset': 'p1', 'dsp': 'd1'},
      ]);
    });

    test('offline editor: subscribes, receives the state, sends changes', () {
      server.receive(_presets(const [ServerPresetInfo(id: 'p2', name: 'Bar', model: 'dsp206', updated: 2)]));
      panel.openEditor('p2');
      expect(panel.editors.single.id, 'preset:p2');
      expect(panel.selectedId, 'preset:p2');
      expect(server.sent.last, {'t': Msg.subscribe, 'dsp': 'preset:p2'});

      final editor = panel.editors.single;
      expect(editor.connection.offline, isTrue);
      expect(editor.device.model, DeviceModel.dsp206);
      server.receive({'t': Msg.state, 'dsp': 'preset:p2', 'state': (DeviceState(DeviceModel.dsp206)..currentPreset = 'Bar').toJson()});
      expect(editor.device.currentPreset, 'Bar');
      server.receive({'t': Msg.op, 'dsp': 'preset:p2', 'op': DeviceOps.gain('Out 6', -4)});
      expect(editor.device.getOutputGain('Out 6'), -4.0);

      // Deleted on the server → the editor closes.
      server.receive(_presets(const []));
      expect(panel.editors, isEmpty);
      expect(panel.selectedId, 'd1');
    });
  });

  group('DSP page', () {
    late _FakeServer server;
    late PanelProvider panel;

    Future<void> pumpPage(WidgetTester tester, DspHandle handle) async {
      tester.view.physicalSize = const Size(1400, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MultiProvider(
        providers: [
          ChangeNotifierProvider<ServerConnection>.value(value: server),
          ChangeNotifierProvider.value(value: panel),
        ],
        child: MaterialApp(theme: buildAppTheme(), home: Scaffold(body: DspPage(handle: handle))),
      ));
      await tester.pump();
    }

    setUp(() {
      server = _FakeServer();
      panel = PanelProvider(server);
    });

    testWidgets('disconnected: blurred content with a connect button', (tester) async {
      server.receive(_hello([
        const DspInfo(id: 'd1', name: 'FOH', host: '10.0.0.5', port: 9761, enabled: false),
      ]));
      await pumpPage(tester, panel.dsps.single);

      expect(find.byType(ImageFiltered), findsOneWidget);
      expect(tester.widget<ImageFiltered>(find.byType(ImageFiltered)).enabled, isTrue);
      expect(find.text('Disconnected'), findsWidgets);
      final connect = find.text('Connect');
      expect(connect, findsNWidgets(2)); // section bar and overlay
      await tester.tap(connect.last);
      expect(server.sent.last, {'t': Msg.updateDsp, 'id': 'd1', 'enabled': true});
    });

    testWidgets('connected: all sections render, disconnect without a question', (tester) async {
      server.receive(_hello([_foh]));
      final handle = panel.dsps.single;
      await pumpPage(tester, handle);
      expect(tester.widget<ImageFiltered>(find.byType(ImageFiltered)).enabled, isFalse);
      expect(find.text('Connected'), findsOneWidget);
      expect(find.text('Inputs · EQ'), findsOneWidget);
      expect(find.text('INPUTS'), findsOneWidget);

      for (final (section, marker) in [
        ('EQ', 'PEQ · In A'),
        ('GEQ', 'Flat'),
        ('Gate', 'Gate · In A'),
        ('Comp', 'Compressor · Out 1'),
        ('Limit', 'Limiter · Out 1'),
        ('Matrix', 'OUT \\ IN'),
        ('Delay', 'Unit'),
        ('Gain', 'Outputs · EQ'),
      ]) {
        await tester.tap(find.text(section).first);
        await tester.pumpAndSettle();
        expect(find.text(marker), findsWidgets, reason: section);
        expect(tester.takeException(), isNull, reason: section);
      }

      await tester.tap(find.text('Disconnect'));
      expect(server.sent.last, {'t': Msg.updateDsp, 'id': 'd1', 'enabled': false});
      await tester.pump(const Duration(milliseconds: 100));
    });

    testWidgets('the DSP 206 layout renders on every page', (tester) async {
      server.receive(_hello([
        const DspInfo(
            id: 'd2', name: 'Mon', host: '10.0.0.6', port: 9761, enabled: true, status: DspStatus.ready, model: 'dsp206'),
      ]));
      final handle = panel.dsps.single;
      handle.device.applyState(DeviceState(DeviceModel.dsp206).toJson());
      await pumpPage(tester, handle);
      for (final section in ['EQ', 'GEQ', 'Gate', 'Comp', 'Limit', 'Matrix', 'Delay', 'Gain']) {
        await tester.tap(find.text(section).first);
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull, reason: section);
      }
      expect(find.text('Out 6'), findsWidgets);
      expect(find.text('Out 7'), findsNothing);
      await tester.pump(const Duration(milliseconds: 100));
    });

    testWidgets('gain page: typed gain, hidden EQ curves are remembered', (tester) async {
      server.receive(_hello([_foh]));
      final handle = panel.dsps.single;
      await pumpPage(tester, handle);

      // Type the gain of In B directly into its field.
      final field = find.widgetWithText(TextField, '0.0').at(1);
      await tester.tap(field);
      await tester.pump();
      await tester.enterText(field, '-7,5');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump(const Duration(milliseconds: 100));
      expect(handle.device.getInputGain('In B'), -7.5);

      // Hide the curve of Out 2; a new gain page remembers it.
      final checkboxes = find.byType(Checkbox);
      expect(tester.widget<Checkbox>(checkboxes.at(5)).value, isTrue);
      await tester.tap(checkboxes.at(5)); // In A–D, then Out 1, Out 2
      await tester.pump();
      expect(tester.widget<Checkbox>(checkboxes.at(5)).value, isFalse);
      await tester.tap(find.text('Matrix').first);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Gain').first);
      await tester.pumpAndSettle();
      expect(tester.widget<Checkbox>(find.byType(Checkbox).at(5)).value, isFalse);
      await tester.pump(const Duration(milliseconds: 100));
    });

    testWidgets('matrix: toggling a linked output does not change its partner', (tester) async {
      server.receive(_hello([_foh]));
      final device = panel.dsps.single.device;
      device.setLinkGroup(['Out 1', 'Out 2']);
      final before = device.getMatrixEnabled('Out 2', 'In C');
      device.toggleMatrixInput('Out 1', 'In C');
      expect(device.getMatrixEnabled('Out 1', 'In C'), !before);
      expect(device.getMatrixEnabled('Out 2', 'In C'), before);
      await tester.pump(const Duration(milliseconds: 100));
    });

    testWidgets('offline editor shows "Load onto DSP"', (tester) async {
      server.receive(_hello([_foh]));
      server.receive(_presets(const [ServerPresetInfo(id: 'p1', name: 'Club', model: 'dsp408', updated: 1)]));
      panel.openEditor('p1');
      await pumpPage(tester, panel.editors.single);
      expect(tester.widget<ImageFiltered>(find.byType(ImageFiltered)).enabled, isFalse);
      expect(find.text('Load onto DSP …'), findsOneWidget);
      expect(find.text('Offline · DSP 408'), findsOneWidget);

      await tester.tap(find.text('Load onto DSP …'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('FOH'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Load'));
      await tester.pumpAndSettle();
      expect(server.sent.last, {'t': Msg.presetApply, 'preset': 'p1', 'dsp': 'd1'});
    });
  });
}
