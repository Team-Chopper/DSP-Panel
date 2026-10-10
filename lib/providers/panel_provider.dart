// Panel Provider
//
// The list of DSPs managed by the server. Holds a DeviceProvider and a
// ConnectionProvider per DSP and routes server messages to them. Only the
// DSP shown in the active tab is subscribed (state, meters, log).

import 'dart:async';
import 'dart:collection';

import 'package:dsp_core/dsp_core.dart';
import 'package:flutter/foundation.dart';

import '../devices/t_racks408/providers/connection_provider.dart';
import '../devices/t_racks408/providers/device_provider.dart';
import '../services/server_connection.dart';

class DspHandle {
  final ConnectionProvider connection;
  final DeviceProvider device;

  DspHandle(this.connection, this.device);

  DspInfo get info => connection.info;
  String get id => info.id;

  void dispose() {
    connection.dispose();
    device.dispose();
  }
}

class PanelProvider extends ChangeNotifier {
  final ServerConnection server;
  final LinkedHashMap<String, DspHandle> _dsps = LinkedHashMap();
  final _errors = StreamController<String>.broadcast();
  StreamSubscription<Map<String, Object?>>? _subscription;
  String? _selected;
  bool _loaded = false;

  PanelProvider(this.server) {
    _subscription = server.messages.listen(_onMessage);
  }

  List<DspHandle> get dsps => List.unmodifiable(_dsps.values);

  /// True once the DSP list has been received from the server.
  bool get loaded => _loaded;

  /// Error messages from the server, for snackbars.
  Stream<String> get errors => _errors.stream;

  String? get selectedId => _selected;

  /// Show [id] in the active tab: subscribe to it, unsubscribe the previous.
  void select(String? id) {
    if (id == _selected) return;
    if (_selected != null) server.send({'t': Msg.unsubscribe, 'dsp': _selected});
    _selected = id;
    if (id != null) server.send({'t': Msg.subscribe, 'dsp': id});
    notifyListeners();
  }

  /// [model]: 'auto' (detect from the device), 'dsp408' or 'dsp206'.
  void addDsp({required String name, required String host, required int port, String model = 'auto'}) =>
      server.send({'t': Msg.addDsp, 'name': name, 'host': host, 'port': port, 'model': model});

  void updateDsp(String id, {String? name, String? host, int? port, bool? enabled, String? model}) =>
      server.send({
        't': Msg.updateDsp,
        'id': id,
        'name': ?name,
        'host': ?host,
        'port': ?port,
        'enabled': ?enabled,
        'model': ?model,
      });

  void removeDsp(String id) => server.send({'t': Msg.removeDsp, 'id': id});

  // ─── Server presets ───

  final List<ServerPresetInfo> _serverPresets = [];

  /// Presets open in the offline editor, keyed by `preset:<id>`.
  final LinkedHashMap<String, DspHandle> _editors = LinkedHashMap();

  List<ServerPresetInfo> get serverPresets => List.unmodifiable(_serverPresets);

  /// Server presets that fit [model].
  List<ServerPresetInfo> serverPresetsFor(DeviceModel model) =>
      [for (final p in _serverPresets) if (p.model == model.id) p];

  /// Offline editor tabs.
  List<DspHandle> get editors => List.unmodifiable(_editors.values);

  /// Save the current settings of [dspId] as a new server preset.
  void savePresetFromDsp(String dspId, String name) =>
      server.send({'t': Msg.presetCreate, 'name': name, 'fromDsp': dspId});

  /// Create an empty server preset for [model].
  void createServerPreset(String name, DeviceModel model) =>
      server.send({'t': Msg.presetCreate, 'name': name, 'model': model.id});

  void renameServerPreset(String presetId, String name) =>
      server.send({'t': Msg.presetRename, 'preset': presetId, 'name': name});

  void deleteServerPreset(String presetId) {
    closeEditor(presetId);
    server.send({'t': Msg.presetDelete, 'preset': presetId});
  }

  /// Send all values of a server preset to a DSP.
  void applyServerPreset(String presetId, String dspId) =>
      server.send({'t': Msg.presetApply, 'preset': presetId, 'dsp': dspId});

  /// Open a server preset in an offline editor tab and show it.
  void openEditor(String presetId) {
    final key = '${Msg.presetPrefix}$presetId';
    final preset = _serverPresets.where((p) => p.id == presetId).firstOrNull;
    if (preset == null) return;
    _editors.putIfAbsent(key, () {
      final info = DspInfo(
        id: key,
        name: preset.name,
        host: '',
        port: 0,
        enabled: true,
        status: DspStatus.ready,
        progress: 1,
        model: preset.model,
      );
      final connection = ConnectionProvider(key, server.send, info, offline: true);
      final device = DeviceProvider(key, server.send, () => true, model: DeviceModel.byId(preset.model));
      return DspHandle(connection, device);
    });
    select(key);
  }

  void closeEditor(String presetId) {
    final key = '${Msg.presetPrefix}$presetId';
    final handle = _editors.remove(key);
    if (handle == null) return;
    if (_selected == key) select(_dsps.keys.firstOrNull);
    notifyListeners();
    Timer(const Duration(seconds: 1), handle.dispose);
  }

  DspHandle _createHandle(DspInfo info) {
    late final ConnectionProvider connection;
    connection = ConnectionProvider(info.id, server.send, info);
    final device = DeviceProvider(info.id, server.send, () => connection.isConnected,
        model: DeviceModel.byId(info.model));
    return DspHandle(connection, device);
  }

  void _onMessage(Map<String, Object?> msg) {
    final dsp = _dsps[msg['dsp']] ?? _editors[msg['dsp']];
    switch (msg['t']) {
      case Msg.presets:
        _serverPresets
          ..clear()
          ..addAll([for (final p in msg['presets'] as List) ServerPresetInfo.fromJson((p as Map).cast())]);
        // Close editors of deleted presets, rename the others.
        for (final key in _editors.keys.toList()) {
          final id = key.substring(Msg.presetPrefix.length);
          final preset = _serverPresets.where((p) => p.id == id).firstOrNull;
          if (preset == null) {
            closeEditor(id);
          } else {
            final editor = _editors[key]!;
            editor.connection.updateInfo(DspInfo(
              id: key, name: preset.name, host: '', port: 0, enabled: true,
              status: DspStatus.ready, progress: 1, model: preset.model,
            ));
          }
        }
        notifyListeners();
      case Msg.hello:
        final infos = [
          for (final i in msg['dsps'] as List) DspInfo.fromJson((i as Map).cast()),
        ];
        final ids = infos.map((i) => i.id).toSet();
        for (final id in _dsps.keys.where((id) => !ids.contains(id)).toList()) {
          _dsps.remove(id)!.dispose();
        }
        for (final info in infos) {
          _upsert(info);
        }
        _loaded = true;
        // After a reconnect, subscribe again to the DSP on screen.
        final selected = _selected;
        _selected = null;
        if (selected != null && (_dsps.containsKey(selected) || _editors.containsKey(selected))) {
          select(selected);
        }
        notifyListeners();
      case Msg.dspInfo:
        final isNew = _upsert(DspInfo.fromJson((msg['info'] as Map).cast()));
        if (isNew) notifyListeners();
      case Msg.dspRemoved:
        final id = msg['id'];
        if (id == _selected) _selected = null;
        final removed = _dsps.remove(id);
        notifyListeners();
        // Dispose after the tab using it has been rebuilt.
        if (removed != null) Timer(const Duration(seconds: 1), removed.dispose);
      case Msg.state:
        dsp?.device.applyState((msg['state'] as Map).cast());
      case Msg.op:
        dsp?.device.applyRemoteOp(msg['op']);
      case Msg.meters:
        if (dsp == null) return;
        dsp.connection.updateCounters((msg['tx'] as num).toInt(), (msg['rx'] as num).toInt());
        dsp.device.applyMeters([for (final v in msg['v'] as List) (v as num).toDouble()],
            dynamics: (msg['dyn'] as num?)?.toInt() ?? 0);
      case Msg.log:
        dsp?.connection.applyLog(
          [for (final e in msg['entries'] as List) LogEntry.fromJson((e as Map).cast())],
          reset: msg['reset'] == true,
        );
      case Msg.error:
        _errors.add(msg['message'] as String? ?? 'Error');
    }
  }

  /// Insert or update a DSP. Returns true if it is new.
  bool _upsert(DspInfo info) {
    final existing = _dsps[info.id];
    if (existing != null) {
      final renamed = existing.info.name != info.name || existing.info.model != info.model;
      existing.connection.updateInfo(info);
      if (renamed) notifyListeners();
      return false;
    }
    _dsps[info.id] = _createHandle(info);
    return true;
  }

  @override
  void dispose() {
    _subscription?.cancel();
    _errors.close();
    for (final d in [..._dsps.values, ..._editors.values]) {
      d.dispose();
    }
    super.dispose();
  }
}
