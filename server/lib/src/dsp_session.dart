// DSP Session
//
// One instance per configured DSP. Owns the TCP connection, the request
// scheduler and the authoritative DeviceState.
//
// Scheduling rules:
// - Exactly one request is in flight at a time; every request has a timeout.
// - Init, preset load and preset save run as exclusive sequences.
// - Value changes are coalesced per key (e.g. "gain:In A"): only the newest
//   value that has not been sent yet goes to the device.
// - Keepalive (meter polling) is just another request, sent only when idle,
//   so its response can never be mistaken for the response to a command.
// - After a connection failure the session reconnects with backoff.

import 'dart:async';
import 'dart:collection';
import 'dart:io';
import 'dart:math';

import 'package:dsp_core/dsp_core.dart';

import 'device_registry.dart';
import 'log.dart';

sealed class SessionEvent {
  const SessionEvent();
}

/// Connection status, progress or settings changed.
class InfoChanged extends SessionEvent {
  const InfoChanged();
}

/// The whole DeviceState was replaced (after init or preset load).
class StateReplaced extends SessionEvent {
  const StateReplaced();
}

/// A single op was applied. [origin] identifies the client that sent it.
class OpApplied extends SessionEvent {
  final Map<String, Object?> op;
  final Object? origin;
  const OpApplied(this.op, this.origin);
}

class MetersUpdated extends SessionEvent {
  const MetersUpdated();
}

class LogAppended extends SessionEvent {
  final List<LogEntry> entries;
  final bool reset;
  const LogAppended(this.entries, {this.reset = false});
}

/// Persistent settings changed (aliases, refresh interval).
class ConfigChanged extends SessionEvent {
  const ConfigChanged();
}

/// Expected failure with a user-facing message.
class SessionFailure implements Exception {
  final String message;
  const SessionFailure(this.message);

  @override
  String toString() => message;
}

typedef SocketConnector = Future<Socket> Function(String host, int port, Duration timeout);

Future<Socket> _defaultConnector(String host, int port, Duration timeout) =>
    Socket.connect(host, port, timeout: timeout);

/// Progress of a sequence of known length.
class _StepCounter {
  final int total;
  final void Function(double progress) onProgress;
  int _done = 0;

  _StepCounter(this.total, this.onProgress);

  void next() => onProgress((++_done / total).clamp(0.0, 1.0));
}

class _Request {
  final List<int> frame;
  final bool keepalive;
  final Completer<List<int>?> done = Completer();
  Timer? timer;

  _Request(this.frame, this.keepalive);
}

class DspSession {
  static const maxLogEntries = 500;

  DspConfig _config;
  final SocketConnector _connector;
  final Duration requestTimeout;
  final Duration sequenceGap;
  final List<Duration> reconnectDelays;
  final Duration meterInterval;

  DspSession(
    this._config, {
    SocketConnector? connector,
    this.requestTimeout = const Duration(milliseconds: 500),
    this.sequenceGap = const Duration(milliseconds: 50),
    this.reconnectDelays = const [
      Duration(seconds: 1),
      Duration(seconds: 2),
      Duration(seconds: 5),
      Duration(seconds: 10),
      Duration(seconds: 30),
    ],
    this.meterInterval = const Duration(milliseconds: 50),
  }) : _connector = connector ?? _defaultConnector {
    state = _defaultState(_configuredModel ?? DeviceModel.dsp408);
    meters = List.filled(state.model.meterCount, 0.0);
  }

  final _events = StreamController<SessionEvent>.broadcast();
  Stream<SessionEvent> get events => _events.stream;

  // ─── Public state ───

  late DeviceState state;
  DspStatus status = DspStatus.disconnected;
  double progress = 0.0;
  String? message;
  late List<double> meters;
  int packetsSent = 0;
  int packetsReceived = 0;
  bool logRx = false;
  bool logTx = false;

  /// False if the last config dump could not be decoded plausibly.
  bool configReadable = true;

  DspConfig get config => _config;
  String get id => _config.id;
  DeviceModel get model => state.model;

  DspInfo get info => DspInfo(
        id: _config.id,
        name: _config.name,
        host: _config.host,
        port: _config.port,
        enabled: _config.enabled,
        status: status,
        progress: progress,
        message: message,
        refreshMs: _config.refreshMs,
        logRx: logRx,
        logTx: logTx,
        model: model.id,
        modelSetting: _config.model,
        configReadable: configReadable,
      );

  /// Model set explicitly in the configuration, or null for auto-detection.
  DeviceModel? get _configuredModel =>
      _config.model == 'auto' ? null : DeviceModel.byId(_config.model);

  /// Default state with the values remembered by the server applied.
  DeviceState _defaultState(DeviceModel model) {
    final fresh = DeviceState(model);
    _config.remembered.applyTo(fresh);
    return fresh;
  }

  ProtocolService get _proto => ProtocolService(model);

  List<LogEntry> get logEntries => List.of(_log);

  // ─── Internal state ───

  bool _running = false;
  bool _disposed = false;
  int _generation = 0;
  Socket? _socket;
  StreamSubscription<List<int>>? _subscription;
  Completer<void>? _connectionClosed;
  Completer<void>? _wakeUp;
  final _decoder = FrameDecoder();
  final _stopwatch = Stopwatch();

  _Request? _inFlight;
  int _consecutiveTimeouts = 0;
  bool _exclusive = false;
  final List<Completer<void>> _idleWaiters = [];

  /// Coalesced value changes waiting to be sent, in first-queued order.
  final LinkedHashMap<String, List<List<int>>> _pendingOps = LinkedHashMap();

  /// Frames of the op currently being sent (e.g. gain is sent twice).
  final List<List<int>> _batch = [];

  /// Config chunks collected while a dump is being read.
  Map<int, List<int>>? _chunks;
  Timer? _keepaliveTimer;
  Timer? _meterTimer;
  Timer? _logFlushTimer;
  final Queue<LogEntry> _log = Queue();
  final List<LogEntry> _logPending = [];

  // ─── Lifecycle ───

  /// Start connecting (if enabled). Reconnects automatically until [stop].
  void start() {
    if (_running || _disposed || !_config.enabled) return;
    _running = true;
    unawaited(_connectLoop(++_generation));
  }

  /// Disconnect and stop reconnecting.
  Future<void> stop() async {
    final wasRunning = _running;
    _running = false;
    _generation++;
    _wakeUp?.complete();
    _wakeUp = null;
    await _closeSocket(wasRunning ? 'Disconnected' : null);
    _setStatus(DspStatus.disconnected, progress: 0.0, message: null);
  }

  /// Apply new settings; reconnects if the address, model or enabled flag
  /// changed.
  Future<void> updateConfig({String? name, String? host, int? port, bool? enabled, String? model}) async {
    final reconnect = (host != null && host != _config.host) ||
        (port != null && port != _config.port) ||
        (enabled != null && enabled != _config.enabled) ||
        (model != null && model != _config.model);
    _config = _config.copyWith(name: name, host: host, port: port, enabled: enabled, model: model);
    _events.add(const ConfigChanged());
    if (reconnect) {
      await stop();
      start();
    }
    _emitInfo();
  }

  Future<void> dispose() async {
    await stop();
    _disposed = true;
    _logFlushTimer?.cancel();
    _meterTimer?.cancel();
    await _events.close();
  }

  Future<void> _connectLoop(int generation) async {
    bool active() => _running && generation == _generation;
    var attempt = 0;
    while (active()) {
      try {
        await _open(active);
        await _runInit();
        attempt = 0;
        _setStatus(DspStatus.ready, progress: 1.0, message: null);
        _startKeepalive();
        _pump();
        await _connectionClosed!.future;
        if (active()) _addStatusLog('Connection lost: ${message ?? ''}');
      } catch (e) {
        if (active()) {
          message = _describeError(e);
          _addStatusLog('Error: $message');
          logWarn('[${_config.name}] $message');
        }
      }
      if (!active()) break;
      await _closeSocket(null);

      final delay = reconnectDelays[min(attempt, reconnectDelays.length - 1)];
      attempt++;
      _setStatus(DspStatus.error,
          message: '${message ?? 'Disconnected'} – retrying in ${delay.inSeconds} s');
      final wakeUp = _wakeUp = Completer();
      await Future.any([Future<void>.delayed(delay), wakeUp.future]);
      if (identical(_wakeUp, wakeUp)) _wakeUp = null;
    }
  }

  static String _describeError(Object e) {
    if (e is SocketException) {
      return 'Connection failed: ${e.osError?.message ?? e.message}';
    }
    if (e is TimeoutException) return 'Connection timed out';
    if (e is SessionFailure) return e.message;
    return e.toString();
  }

  Future<void> _open(bool Function() active) async {
    _setStatus(DspStatus.connecting, progress: 0.0, message: null);
    _stopwatch
      ..reset()
      ..start();
    _addStatusLog('Connecting to ${_config.host}:${_config.port} …');
    packetsSent = 0;
    packetsReceived = 0;
    _consecutiveTimeouts = 0;
    _decoder.reset();

    final socket = await _connector(_config.host, _config.port, const Duration(seconds: 5));
    if (!active()) {
      socket.destroy();
      throw const SessionFailure('Cancelled');
    }
    socket.setOption(SocketOption.tcpNoDelay, true);
    _socket = socket;
    final closed = _connectionClosed = Completer<void>();
    _subscription = socket.listen(
      _onData,
      onError: (Object e) => _connectionLost(socket, 'Socket error: $e'),
      onDone: () => _connectionLost(socket, 'Connection closed by the DSP'),
      cancelOnError: true,
    );
    socket.done.then((_) {}, onError: (Object e) => _connectionLost(socket, 'Socket error: $e'));
    // Ensure the future is observed even if nobody awaits it.
    closed.future.ignore();
    _addStatusLog('Connected – initializing');
  }

  void _connectionLost(Socket socket, String reason) {
    if (!identical(socket, _socket)) return; // stale callback of an old socket
    message = reason;
    final closed = _connectionClosed;
    if (closed != null && !closed.isCompleted) closed.complete();
  }

  Future<void> _closeSocket(String? reason) async {
    _stopKeepalive();
    final socket = _socket;
    _socket = null;
    await _subscription?.cancel();
    _subscription = null;
    socket?.destroy();

    // Fail whatever is in flight and drop queued work.
    final inFlight = _inFlight;
    _inFlight = null;
    inFlight?.timer?.cancel();
    if (inFlight != null && !inFlight.done.isCompleted) inFlight.done.complete(null);
    _pendingOps.clear();
    _batch.clear();
    _chunks = null;
    _notifyIdle();

    final closed = _connectionClosed;
    if (closed != null && !closed.isCompleted) closed.complete();
    if (reason != null) _addStatusLog(reason);
  }

  // ─── Init ───

  /// Handshake, device info (→ model), preset names and the full config
  /// dump. The state is only published once everything has been read.
  Future<void> _runInit() async {
    _setStatus(DspStatus.initializing, progress: 0.0);
    final steps = _StepCounter(5 + TRacksProto.presetCount + ConfigDump(model).chunkCount + 2,
        (p) => _setStatus(DspStatus.initializing, progress: p));
    final socket = _socket;

    _exclusive = true;
    try {
      await _require(socket, TRacksProto.handshakeCommand);
      steps.next();

      final info = await _require(socket, TRacksProto.requestDeviceInfo);
      final deviceName = (ProtocolService().decode(info) as DeviceInfoMessage?)?.deviceName ?? '';
      steps.next();
      final detected = DeviceModel.detect(deviceName);
      final model = _configuredModel ?? detected ?? DeviceModel.dsp408;
      if (_configuredModel != null && detected != null && detected != _configuredModel) {
        _addStatusLog('Device reports "$deviceName" but is configured as ${_configuredModel!.name}');
      } else if (detected == null) {
        _addStatusLog('Unknown device "$deviceName" – using ${model.name}');
      }
      _addStatusLog('Device: $deviceName (${model.name})');
      // Collect preset names into a fresh state of the right model; it is
      // published after the config dump.
      state = _defaultState(model)..deviceName = deviceName;
      meters = List.filled(model.meterCount, 0.0);

      // The original editor sends these; their meaning is unknown and a
      // missing answer is not an error.
      for (final cmd in [
        TRacksProto.requestAllPresetsCommand,
        TRacksProto.unknownStartup22,
        TRacksProto.unknownStartup14,
      ]) {
        await _optional(socket, cmd);
        steps.next();
      }
      for (final cmd in TRacksProto.loadAllPresetsCommands) {
        await _require(socket, cmd);
        steps.next();
      }

      final stream = await _readConfig(socket, steps.next);
      _applyDump(stream);

      for (final cmd in TRacksProto.statusQueryCommands) {
        await _optional(socket, cmd);
        steps.next();
      }
    } finally {
      _exclusive = false;
      _chunks = null;
    }
    _events.add(const StateReplaced());
  }

  /// Read the config dump: as many chunks as the model's layout needs. A
  /// chunk the device does not answer ends the dump.
  Future<List<int>> _readConfig(Socket? socket, void Function() step) async {
    final dump = ConfigDump(model);
    final chunks = _chunks = {};
    for (var i = 0; i < dump.chunkCount; i++) {
      final response = await _requestWithRetries(socket, TRacksProto.configDumpCommand(i));
      step();
      if (response == null) {
        if (!identical(_socket, socket)) throw SessionFailure(message ?? 'Connection lost');
        _addStatusLog('Config chunk $i not answered – dump ends here');
        break;
      }
      await Future<void>.delayed(sequenceGap);
    }
    _chunks = null;
    return [for (var i = 0; chunks.containsKey(i); i++) ...chunks[i]!];
  }

  /// Decode [stream] into the state. Keeps preset names and device name,
  /// and applies the values the server remembers.
  void _applyDump(List<int> stream) {
    final result = ConfigDump(model).decode(stream);
    configReadable = result.isPlausible;
    final next = configReadable ? result.state : DeviceState(model);
    next.presets.addAll(state.presets);
    next.deviceName = state.deviceName;
    _config.remembered.applyTo(next);
    state = next;
    if (configReadable) {
      _addStatusLog('Configuration read – active preset: ${state.currentPreset}');
    } else {
      _addStatusLog('Configuration could not be decoded (${result.problems.length} implausible '
          'values, e.g. ${result.problems.take(3).join('; ')}) – showing defaults');
      logWarn('[${_config.name}] config dump not recognised: ${result.problems.take(5).join('; ')}');
    }
    _emitInfo();
  }

  /// Request that must be answered (with retries), else the step fails.
  Future<List<int>> _require(Socket? socket, List<int> frame) async {
    final response = await _requestWithRetries(socket, frame);
    if (response != null) {
      await Future<void>.delayed(sequenceGap);
      return response;
    }
    throw SessionFailure(!identical(_socket, socket)
        ? (message ?? 'Connection lost during initialization')
        : 'Initialization failed: DSP is not responding');
  }

  /// Request whose answer is optional (single attempt).
  Future<void> _optional(Socket? socket, List<int> frame) async {
    if (!identical(_socket, socket)) throw SessionFailure(message ?? 'Connection lost');
    if (await _request(frame) == null) {
      _consecutiveTimeouts = 0; // an unanswered optional request is fine
      logDebug('[${_config.name}] no answer to optional ${PacketDescriber.describe(frame)}');
    }
    await Future<void>.delayed(sequenceGap);
  }

  Future<List<int>?> _requestWithRetries(Socket? socket, List<int> frame, {int retries = 2}) async {
    for (var attempt = 0; attempt <= retries; attempt++) {
      if (socket == null || !identical(_socket, socket)) return null;
      final response = await _request(frame);
      if (response != null) return response;
    }
    return null;
  }

  /// Send [frames] one by one, each after the previous response (with
  /// retries). Returns false if the device stopped answering.
  Future<bool> _runSequence(
    List<List<int>> frames, {
    required void Function(int done, int total) onProgress,
    int retries = 2,
  }) async {
    // Abort if the connection is replaced while the sequence is running.
    final socket = _socket;
    bool connected() => socket != null && identical(_socket, socket);

    for (var i = 0; i < frames.length; i++) {
      List<int>? response;
      for (var attempt = 0; attempt <= retries && response == null; attempt++) {
        if (!connected()) return false;
        response = await _request(frames[i]);
      }
      if (response == null) return false;
      onProgress(i + 1, frames.length);
      await Future<void>.delayed(sequenceGap);
    }
    return connected();
  }

  // ─── Requests ───

  Future<List<int>?> _request(List<int> frame, {bool keepalive = false}) {
    final socket = _socket;
    if (socket == null || _inFlight != null) return Future.value(null);

    final request = _Request(frame, keepalive);
    _inFlight = request;
    socket.add(frame);
    packetsSent++;
    if (logTx) {
      _addLog(LogEntry.packet(
        rawHex: 'Tx: ${_proto.formatAsHex(frame)} → ${_proto.formatAsAscii(frame)}',
        decoded: 'Tx: ${PacketDescriber.describe(frame, model)}',
        timestamp: _elapsed,
      ));
    }
    request.timer = Timer(requestTimeout, () {
      if (!identical(_inFlight, request)) return;
      _inFlight = null;
      request.done.complete(null);
      _consecutiveTimeouts++;
      logDebug('[${_config.name}] timeout for ${PacketDescriber.describe(frame, model)}');
      if (_consecutiveTimeouts >= 5) {
        _connectionLost(socket, 'DSP is not responding');
      }
      _afterRequest();
    });
    return request.done.future;
  }

  void _afterRequest() {
    _notifyIdle();
    scheduleMicrotask(_pump);
  }

  void _notifyIdle() {
    if (_inFlight != null) return;
    for (final waiter in _idleWaiters) {
      if (!waiter.isCompleted) waiter.complete();
    }
    _idleWaiters.clear();
  }

  Future<void> _waitForIdle() {
    if (_inFlight == null) return Future.value();
    final waiter = Completer<void>();
    _idleWaiters.add(waiter);
    return waiter.future;
  }

  void _onData(List<int> data) {
    for (final frame in _decoder.add(data)) {
      _onFrame(frame);
    }
  }

  void _onFrame(List<int> frame) {
    packetsReceived++;
    final message = _proto.decode(frame);
    if (message == null) return; // not a device response

    if (logRx) {
      _addLog(LogEntry.packet(
        rawHex: 'Rx: ${_proto.formatAsHex(frame)} → ${_proto.formatAsAscii(frame)}',
        decoded: 'Rx: ${PacketDescriber.describeRx(frame, model)}',
        timestamp: _elapsed,
      ));
    }

    switch (message) {
      case KeepaliveMessage(:final meterLevels):
        meters = meterLevels;
        _scheduleMeters();
      case PresetMessage(:final index, :final name):
        state.presets[index] = name;
        if (status == DspStatus.ready) _events.add(const StateReplaced());
      case ChannelConfigMessage(:final subIndex, :final chunkData):
        _chunks?[subIndex] = chunkData;
      default:
        break;
    }

    // Meter frames only answer keepalives; any other response answers the
    // command in flight.
    final request = _inFlight;
    if (request != null && request.keepalive == (message.command == 0x40)) {
      request.timer?.cancel();
      _inFlight = null;
      _consecutiveTimeouts = 0;
      request.done.complete(frame);
      _afterRequest();
    }
  }

  /// Send the next queued value change, if the line is free.
  void _pump() {
    if (_socket == null || _inFlight != null || _exclusive) return;
    if (status != DspStatus.ready && status != DspStatus.busy) return;

    if (_batch.isEmpty && _pendingOps.isNotEmpty) {
      final key = _pendingOps.keys.first;
      _batch.addAll(_pendingOps.remove(key)!);
    }
    if (_batch.isNotEmpty) {
      unawaited(_request(_batch.removeAt(0)));
    }
  }

  bool get _isIdle =>
      _inFlight == null && _batch.isEmpty && _pendingOps.isEmpty && !_exclusive;

  // ─── Keepalive & meters ───

  void _startKeepalive() {
    _stopKeepalive();
    _keepaliveTimer = Timer.periodic(Duration(milliseconds: _config.refreshMs), (_) {
      if (status == DspStatus.ready && _isIdle) {
        unawaited(_request(_proto.buildKeepaliveCommand(), keepalive: true));
      }
    });
  }

  void _stopKeepalive() {
    _keepaliveTimer?.cancel();
    _keepaliveTimer = null;
  }

  void _scheduleMeters() {
    if (_meterTimer?.isActive ?? false) return;
    _meterTimer = Timer(meterInterval, () {
      if (!_events.isClosed) _events.add(const MetersUpdated());
    });
  }

  /// Change the keepalive/meter polling interval (5–300 ms).
  void setRefreshInterval(int ms) {
    _config = _config.copyWith(refreshMs: ms.clamp(5, 300));
    if (_keepaliveTimer != null) _startKeepalive();
    _events.add(const ConfigChanged());
    _emitInfo();
  }

  // ─── Operations ───

  /// Validate and apply an op from a client. Returns an error message or null.
  String? applyOp(Map<String, Object?> op, {Object? origin}) {
    final error = DeviceOps.validate(op, model);
    if (error != null) return error;
    if (status != DspStatus.ready) {
      return status == DspStatus.busy ? 'DSP is busy' : 'DSP is not connected';
    }

    // Linked channels get the same change. The DSP may mirror linked
    // channels itself (0x3B), but that is not verified; sending the values
    // explicitly is correct either way.
    var remembered = false;
    for (final expanded in DeviceOps.expandLinked(state, op)) {
      DeviceOps.apply(state, expanded);
      // A newer value for a key replaces the queued one but keeps its place.
      DeviceOps.commands(state, expanded).forEach((key, frames) => _pendingOps[key] = frames);
      _events.add(OpApplied(expanded, origin));
      remembered |= _config.remembered.record(state, expanded);
    }
    if (remembered) _events.add(const ConfigChanged());
    _pump();
    return null;
  }

  /// Send every value of [target] to the device (loading a preset stored on
  /// the server). Returns an error message or null.
  String? applyFullState(DeviceState target, String label) {
    if (status != DspStatus.ready) return 'DSP is not ready';
    if (target.model != model) {
      return 'This preset is for a ${target.model.name}, the DSP is a ${model.name}';
    }
    final ops = DeviceOps.fullState(target);
    _pendingOps.clear();
    _batch.clear();
    unawaited(_exclusiveJob('Loading "$label" …', () async {
      final socket = _socket;
      final steps = _StepCounter(ops.length, (p) => _setStatus(DspStatus.busy, progress: p));
      for (final op in ops) {
        DeviceOps.apply(state, op);
        for (final frames in DeviceOps.commands(state, op).values) {
          for (final frame in frames) {
            if (await _requestWithRetries(socket, frame) == null) {
              throw SessionFailure('DSP stopped answering while loading "$label"');
            }
          }
        }
        _config.remembered.record(state, op);
        steps.next();
      }
      state.currentPreset = label;
      _events.add(const StateReplaced());
      _events.add(const ConfigChanged());
      _addStatusLog('Server preset "$label" loaded (${ops.length} settings)');
    }));
    return null;
  }

  /// Load a preset (0-based index). Returns an error message or null.
  String? loadPreset(int index) {
    if (status != DspStatus.ready) return 'DSP is not ready';
    if (index < 0 || index >= TRacksProto.presetCount) return 'Invalid preset';
    // Values queued before the load would overwrite the loaded preset.
    _pendingOps.clear();
    _batch.clear();
    final label = 'U${(index + 1).toString().padLeft(2, '0')}';
    unawaited(_exclusiveJob('Loading preset $label …', () async {
      final socket = _socket;
      final steps = _StepCounter(1 + ConfigDump(model).chunkCount + 2,
          (p) => _setStatus(DspStatus.busy, progress: p));
      try {
        await _require(socket, _proto.buildLoadPresetCommand(index));
        steps.next();
        final stream = await _readConfig(socket, steps.next);
        _applyDump(stream);
        for (final cmd in TRacksProto.statusQueryCommands) {
          await _optional(socket, cmd);
          steps.next();
        }
        _events.add(const StateReplaced());
        _addStatusLog('Preset $label loaded: ${state.currentPreset}');
      } on SessionFailure {
        _addStatusLog('Preset $label could not be loaded completely');
      } finally {
        _chunks = null;
      }
    }));
    return null;
  }

  /// Store the current settings in a preset slot. Returns an error or null.
  String? savePreset(int index, String name) {
    if (status != DspStatus.ready) return 'DSP is not ready';
    if (index < 0 || index >= TRacksProto.presetCount) return 'Invalid preset';
    final trimmed = name.trim();
    if (trimmed.isEmpty || trimmed.length > 14) return 'Name must be 1–14 characters';

    final label = 'U${(index + 1).toString().padLeft(2, '0')}';
    unawaited(_exclusiveJob('Saving preset $label …', () async {
      final ok = await _runSequence(TRacksProto.storePresetSequence(index, trimmed),
          onProgress: (done, total) => _setStatus(DspStatus.busy, progress: done / total));
      if (ok) {
        // Same sanitising as the name command
        final stored = String.fromCharCodes(
            _proto.buildStorePresetNameCommand(trimmed).sublist(6, 20)).trim();
        state.presets[index] = stored;
        state.currentPreset = stored;
        _events.add(const StateReplaced());
        _addStatusLog('Preset $label saved: "$stored"');
      } else {
        _addStatusLog('Preset $label could not be saved');
      }
    }, drainFirst: true));
    return null;
  }

  Future<void> _exclusiveJob(String label, Future<void> Function() body,
      {bool drainFirst = false}) async {
    _setStatus(DspStatus.busy, progress: 0.0, message: label);
    _addStatusLog(label);
    if (drainFirst) {
      // Let queued value changes reach the device before saving.
      while (_socket != null && !_isIdle) {
        await _waitForIdle();
        await Future<void>.delayed(Duration.zero);
      }
    }
    _exclusive = true;
    await _waitForIdle();
    try {
      await body();
    } catch (e) {
      _addStatusLog('Error: $e');
    } finally {
      _exclusive = false;
      if (_socket != null && status == DspStatus.busy) {
        _setStatus(DspStatus.ready, progress: 1.0, message: null);
      }
      _pump();
    }
  }

  // ─── Debug log ───

  void setDebug({bool? rx, bool? tx}) {
    logRx = rx ?? logRx;
    logTx = tx ?? logTx;
    _emitInfo();
  }

  double get _elapsed => _stopwatch.elapsedMicroseconds / 1e6;

  void _addStatusLog(String text) => _addLog(LogEntry.status(text, _elapsed));

  void _addLog(LogEntry entry) {
    _log.add(entry);
    while (_log.length > maxLogEntries) {
      _log.removeFirst();
    }
    _logPending.add(entry);
    _logFlushTimer ??= Timer(const Duration(milliseconds: 200), _flushLog);
  }

  void _flushLog() {
    _logFlushTimer = null;
    if (_logPending.isEmpty || _events.isClosed) return;
    _events.add(LogAppended(List.of(_logPending)));
    _logPending.clear();
  }

  void _clearLog() {
    _log.clear();
    _logPending.clear();
    if (!_events.isClosed) _events.add(const LogAppended([], reset: true));
  }

  void clearLog() => _clearLog();

  // ─── Status ───

  void _setStatus(DspStatus newStatus, {double? progress, String? message = _keep}) {
    final newProgress = progress ?? this.progress;
    final newMessage = identical(message, _keep) ? this.message : message;
    if (newStatus == status && newProgress == this.progress && newMessage == this.message) return;
    status = newStatus;
    this.progress = newProgress;
    this.message = newMessage;
    _emitInfo();
  }

  static const String _keep = '\u0000keep';

  void _emitInfo() {
    if (!_events.isClosed) _events.add(const InfoChanged());
  }
}
