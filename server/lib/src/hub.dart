// Hub
//
// Owns all DSP sessions and all connected browser clients, and routes
// messages between them (see packages/dsp_core/lib/src/api.dart).

import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:io';

import 'package:dsp_core/dsp_core.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import 'device_registry.dart';
import 'dsp_session.dart';
import 'log.dart';
import 'preset_store.dart';

typedef SessionFactory = DspSession Function(DspConfig config);

class _Client {
  final WebSocketChannel channel;
  final String remote;
  final Set<String> subscriptions = {};

  _Client(this.channel, this.remote);

  void send(Map<String, Object?> message) => sendRaw(jsonEncode(message));

  void sendRaw(String json) {
    try {
      channel.sink.add(json);
    } catch (_) {
      // Client is gone; its onDone handler removes it.
    }
  }
}

class Hub {
  static const maxMessageLength = 64 * 1024;
  static const maxDevices = 32;

  final DeviceRegistry registry;
  final SessionFactory _sessionFactory;
  final LinkedHashMap<String, DspSession> _sessions = LinkedHashMap();
  final Map<String, StreamSubscription<SessionEvent>> _sessionSubscriptions = {};
  final Set<_Client> _clients = {};
  final PresetStore presets;
  StreamSubscription<PresetEvent>? _presetSubscription;
  Timer? _saveTimer;

  Hub(this.registry, {SessionFactory? sessionFactory, PresetStore? presets})
      : _sessionFactory = sessionFactory ?? DspSession.new,
        presets = presets ?? PresetStore(File('${registry.file.parent.path}/presets.json'));

  int get clientCount => _clients.length;
  Iterable<DspSession> get sessions => _sessions.values;

  Future<void> start() async {
    for (final config in registry.load()) {
      _addSession(config);
    }
    presets.load();
    _presetSubscription = presets.events.listen(_onPresetEvent);
    logInfo('${_sessions.length} DSP(s) configured, ${presets.list.length} server preset(s)');
  }

  Future<void> shutdown() async {
    _saveTimer?.cancel();
    await _save();
    await _presetSubscription?.cancel();
    await presets.dispose();
    for (final client in List.of(_clients)) {
      await client.channel.sink.close();
    }
    for (final sub in _sessionSubscriptions.values) {
      await sub.cancel();
    }
    for (final session in _sessions.values) {
      await session.dispose();
    }
  }

  void _addSession(DspConfig config) {
    final session = _sessionFactory(config);
    _sessions[config.id] = session;
    _sessionSubscriptions[config.id] = session.events.listen((e) => _onSessionEvent(session, e));
    session.start();
  }

  // ─── Clients ───

  void handleConnection(WebSocketChannel channel, {String remote = '?'}) {
    final client = _Client(channel, remote);
    _clients.add(client);
    logInfo('Client connected ($remote), ${_clients.length} total');
    client.send({
      't': Msg.hello,
      'dsps': [for (final s in _sessions.values) s.info.toJson()],
    });
    client.send(_presetListMessage());
    channel.stream.listen(
      (data) => _onMessage(client, data),
      onDone: () {
        _clients.remove(client);
        logInfo('Client disconnected ($remote), ${_clients.length} total');
      },
      onError: (Object e) => logWarn('WebSocket error ($remote): $e'),
      cancelOnError: true,
    );
  }

  void _onMessage(_Client client, Object? data) {
    if (data is! String || data.length > maxMessageLength) {
      client.send({'t': Msg.error, 'message': 'Invalid message'});
      return;
    }
    final Map<String, Object?> msg;
    try {
      msg = (jsonDecode(data) as Map).cast<String, Object?>();
    } catch (_) {
      client.send({'t': Msg.error, 'message': 'Invalid JSON'});
      return;
    }

    try {
      final error = _dispatch(client, msg);
      if (error != null) {
        client.send({'t': Msg.error, 'dsp': msg['dsp'], 'message': error});
      }
    } catch (e, st) {
      logWarn('Error handling ${msg['t']}: $e\n$st');
      client.send({'t': Msg.error, 'message': 'Internal error'});
    }
  }

  /// Handle one client message. Returns an error message or null.
  String? _dispatch(_Client client, Map<String, Object?> msg) {
    final type = msg['t'];

    switch (type) {
      case Msg.addDsp:
        return _addDsp(msg);
      case Msg.updateDsp:
        return _updateDsp(msg);
      case Msg.removeDsp:
        return _removeDsp(msg);
      case Msg.presetCreate:
        return _presetCreate(msg);
      case Msg.presetRename:
        final id = msg['preset'];
        return id is String ? presets.rename(id, msg['name']) : 'preset missing';
      case Msg.presetDelete:
        final id = msg['preset'];
        if (id is! String) return 'preset missing';
        for (final c in _clients) {
          c.subscriptions.remove('${Msg.presetPrefix}$id');
        }
        return presets.delete(id);
      case Msg.presetApply:
        final preset = presets[msg['preset'] is String ? msg['preset'] as String : ''];
        final target = _sessions[msg['dsp']];
        if (preset == null) return 'Unknown preset';
        if (target == null) return 'Unknown DSP';
        return target.applyFullState(preset.state, preset.name);
    }

    // Everything else addresses one DSP, or a server preset in the offline editor.
    final id = msg['dsp'];
    if (id is String && id.startsWith(Msg.presetPrefix)) return _dispatchPreset(client, msg, id);
    final session = id is String ? _sessions[id] : null;
    if (session == null) return 'Unknown DSP';

    switch (type) {
      case Msg.subscribe:
        client.subscriptions.add(session.id);
        client.send({'t': Msg.state, 'dsp': session.id, 'state': session.state.toJson()});
        client.send({
          't': Msg.log,
          'dsp': session.id,
          'reset': true,
          'entries': [for (final e in session.logEntries) e.toJson()],
        });
        return null;
      case Msg.unsubscribe:
        client.subscriptions.remove(session.id);
        return null;
      case Msg.op:
        final op = msg['op'];
        if (op is! Map) return 'op missing';
        final error = session.applyOp(op.cast<String, Object?>(), origin: client);
        if (error != null) {
          // The client already applied the op locally — resynchronise it.
          client.send({'t': Msg.state, 'dsp': session.id, 'state': session.state.toJson()});
        }
        return error;
      case Msg.loadPreset:
        final index = msg['index'];
        return index is int ? session.loadPreset(index) : 'index missing';
      case Msg.savePreset:
        final index = msg['index'];
        final name = msg['name'];
        if (index is! int || name is! String) return 'index/name missing';
        return session.savePreset(index, name);
      case Msg.setRefresh:
        final ms = msg['ms'];
        if (ms is! int) return 'ms missing';
        session.setRefreshInterval(ms);
        return null;
      case Msg.debug:
        final rx = msg['rx'];
        final tx = msg['tx'];
        session.setDebug(rx: rx is bool ? rx : null, tx: tx is bool ? tx : null);
        return null;
      case Msg.clearLog:
        session.clearLog();
        return null;
      default:
        return 'Unknown message type: $type';
    }
  }

  // ─── Server presets ───

  Map<String, Object?> _presetListMessage() =>
      {'t': Msg.presets, 'presets': [for (final p in presets.list) p.toJson()]};

  String? _presetCreate(Map<String, Object?> msg) {
    final name = msg['name'];
    final error = PresetStore.validateName(name);
    if (error != null) return error;
    final fromDsp = msg['fromDsp'];
    final DeviceState state;
    if (fromDsp != null) {
      final session = _sessions[fromDsp];
      if (session == null) return 'Unknown DSP';
      if (session.status != DspStatus.ready) return 'The DSP must be connected to save its settings';
      if (!session.configReadable) return 'The configuration of this DSP could not be read';
      state = session.state;
    } else {
      final model = msg['model'];
      if (model is! String || !DeviceModel.isKnownId(model)) return 'Choose a model for the new preset';
      state = DeviceState(DeviceModel.byId(model));
    }
    try {
      final preset = presets.create(name as String, state);
      logInfo('Server preset created: ${preset.name} (${preset.state.model.name})');
    } on StateError catch (e) {
      return e.message;
    }
    return null;
  }

  String? _dispatchPreset(_Client client, Map<String, Object?> msg, String dspId) {
    final presetId = dspId.substring(Msg.presetPrefix.length);
    final preset = presets[presetId];
    if (preset == null) return 'Unknown preset';
    switch (msg['t']) {
      case Msg.subscribe:
        client.subscriptions.add(dspId);
        client.send({'t': Msg.state, 'dsp': dspId, 'state': preset.state.toJson()});
        return null;
      case Msg.unsubscribe:
        client.subscriptions.remove(dspId);
        return null;
      case Msg.op:
        final op = msg['op'];
        if (op is! Map) return 'op missing';
        final error = presets.applyOp(presetId, op.cast<String, Object?>(), origin: client);
        if (error != null) {
          client.send({'t': Msg.state, 'dsp': dspId, 'state': preset.state.toJson()});
        }
        return error;
      default:
        return 'Not available for an offline preset';
    }
  }

  void _onPresetEvent(PresetEvent event) {
    switch (event) {
      case PresetListChanged():
        _broadcast(_presetListMessage());
      case PresetOpApplied(:final presetId, :final op, :final origin):
        final dspId = '${Msg.presetPrefix}$presetId';
        _toSubscribers(dspId, {'t': Msg.op, 'dsp': dspId, 'op': op}, except: origin);
    }
  }

  String? _addDsp(Map<String, Object?> msg) {
    final name = msg['name'];
    final host = msg['host'];
    final port = msg['port'] ?? 9761;
    final model = msg['model'] ?? 'auto';
    if (name is! String || host is! String || port is! int || model is! String) {
      return 'Name, host and port are required';
    }
    final error = DspConfig.validateFields(name: name, host: host.trim(), port: port, model: model);
    if (error != null) return error;
    if (_sessions.length >= maxDevices) return 'At most $maxDevices DSPs';

    final config = DspConfig(
        id: DspConfig.newId(), name: name.trim(), host: host.trim(), port: port, model: model);
    _addSession(config);
    _broadcast({'t': Msg.dspInfo, 'info': _sessions[config.id]!.info.toJson()});
    _scheduleSave();
    logInfo('DSP added: ${config.name} (${config.host}:${config.port})');
    return null;
  }

  String? _updateDsp(Map<String, Object?> msg) {
    final session = _sessions[msg['id']];
    if (session == null) return 'Unknown DSP';
    final name = msg['name'];
    final host = msg['host'];
    final port = msg['port'];
    final enabled = msg['enabled'];
    final model = msg['model'];
    if ((name != null && name is! String) ||
        (host != null && host is! String) ||
        (port != null && port is! int) ||
        (enabled != null && enabled is! bool) ||
        (model != null && model is! String)) {
      return 'Invalid values';
    }
    final error = DspConfig.validateFields(
        name: name as String?, host: (host as String?)?.trim(), port: port as int?, model: model as String?);
    if (error != null) return error;

    unawaited(session.updateConfig(
      name: name?.trim(),
      host: host?.trim(),
      port: port,
      model: model,
      enabled: enabled as bool?,
    ));
    return null;
  }

  String? _removeDsp(Map<String, Object?> msg) {
    final id = msg['id'];
    final session = _sessions.remove(id);
    if (session == null) return 'Unknown DSP';
    unawaited(_sessionSubscriptions.remove(id)?.cancel());
    unawaited(session.dispose());
    for (final c in _clients) {
      c.subscriptions.remove(id);
    }
    _broadcast({'t': Msg.dspRemoved, 'id': id});
    _scheduleSave();
    logInfo('DSP removed: ${session.config.name}');
    return null;
  }

  // ─── Session events ───

  void _onSessionEvent(DspSession session, SessionEvent event) {
    switch (event) {
      case InfoChanged():
        _broadcast({'t': Msg.dspInfo, 'info': session.info.toJson()});
      case StateReplaced():
        _toSubscribers(session.id, {'t': Msg.state, 'dsp': session.id, 'state': session.state.toJson()});
      case OpApplied(:final op, :final origin):
        _toSubscribers(session.id, {'t': Msg.op, 'dsp': session.id, 'op': op}, except: origin);
      case MetersUpdated():
        _toSubscribers(session.id, {
          't': Msg.meters,
          'dsp': session.id,
          'v': [for (final v in session.meters) (ProtocolService.levelToDb(v) * 10).round() / 10],
          'dyn': session.dynamics,
          'tx': session.packetsSent,
          'rx': session.packetsReceived,
        });
      case LogAppended(:final entries, :final reset):
        _toSubscribers(session.id, {
          't': Msg.log,
          'dsp': session.id,
          'reset': reset,
          'entries': [for (final e in entries) e.toJson()],
        });
      case ConfigChanged():
        _scheduleSave();
    }
  }

  void _broadcast(Map<String, Object?> message) {
    final json = jsonEncode(message);
    for (final c in _clients) {
      c.sendRaw(json);
    }
  }

  void _toSubscribers(String dspId, Map<String, Object?> message, {Object? except}) {
    String? json;
    for (final c in _clients) {
      if (identical(c, except) || !c.subscriptions.contains(dspId)) continue;
      c.sendRaw(json ??= jsonEncode(message));
    }
  }

  // ─── Persistence ───

  void _scheduleSave() {
    _saveTimer?.cancel();
    _saveTimer = Timer(const Duration(milliseconds: 500), () => unawaited(_save()));
  }

  Future<void> _save() async {
    try {
      await registry.save([for (final s in _sessions.values) s.config]);
    } catch (e) {
      logWarn('Could not save ${registry.file.path}: $e');
    }
  }
}
