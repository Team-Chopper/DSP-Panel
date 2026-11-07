// DSP Session
//
// One instance per configured DSP. Owns the TCP connection, the request
// scheduler and the authoritative DeviceState. The protocol flow follows
// the original editor (ORIGINAL-SOFTWARE.md).
//
// Scheduling rules:
// - Exactly one request is in flight at a time; every request has a timeout
//   and is sent once more if it is not answered. A request that stays
//   unanswered breaks the link (like the original editor); the session then
//   reconnects with backoff.
// - Init, preset load/save, linking and copying run as exclusive sequences.
// - Value changes are coalesced per key (e.g. "gain:In A"): only the newest
//   value that has not been sent yet goes to the device.
// - Meter polling is just another request, sent only when idle, so its
//   response can never be mistaken for the response to a command. While no
//   client shows the DSP, the session sends the idle heartbeat (0x12)
//   instead.

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
  final Duration timeout;

  /// Unanswered request: the link is broken (else the caller gets null).
  final bool fatal;
  int retriesLeft;
  final Completer<List<int>?> done = Completer();
  Timer? timer;

  _Request(this.frame, this.keepalive, this.timeout, this.retriesLeft, this.fatal);
}

class DspSession {
  static const maxLogEntries = 500;

  DspConfig _config;
  final SocketConnector _connector;

  /// Answer timeout of a request (the original editor: 1 s on TCP); an
  /// unanswered request is sent once more.
  final Duration requestTimeout;

  /// Answer timeout of the handshake and device info (4 s, 3 attempts).
  final Duration handshakeTimeout;

  /// Answer timeouts of a preset store and recall.
  final Duration storeTimeout;
  final Duration recallTimeout;

  /// Idle time after which the heartbeat (0x12) is sent.
  final Duration heartbeatIdle;
  final Duration sequenceGap;
  final List<Duration> reconnectDelays;
  final Duration meterInterval;

  DspSession(
    this._config, {
    SocketConnector? connector,
    this.requestTimeout = const Duration(seconds: 1),
    this.handshakeTimeout = const Duration(seconds: 4),
    this.storeTimeout = const Duration(seconds: 10),
    this.recallTimeout = const Duration(seconds: 50),
    this.heartbeatIdle = const Duration(seconds: 2),
    this.sequenceGap = const Duration(milliseconds: 50),
    this.reconnectDelays = const [
      Duration(seconds: 1),
      Duration(seconds: 2),
      Duration(seconds: 5),
      Duration(seconds: 10),
      Duration(seconds: 30),
    ],
    this.meterInterval = const Duration(milliseconds: 25),
  }) : _connector = connector ?? _defaultConnector {
    state = DeviceState(_configuredModel ?? DeviceModel.dsp408);
    meters = List.filled(state.model.meterCount, 0.0);
  }

  final _events = StreamController<SessionEvent>.broadcast();
  Stream<SessionEvent> get events => _events.stream;

  // ─── Public state ───

  late DeviceState state;
  DspStatus status = DspStatus.disconnected;
  double progress = 0.0;
  String? message;
  /// Linear meter levels (1.0 = full scale) in channel order.
  late List<double> meters;

  /// Compressor/limiter status of the outputs from the last meter frame
  /// (see KeepaliveMessage.dynamics).
  int dynamics = 0;
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
        deviceId: _config.deviceId,
        hasPassword: _config.password.isNotEmpty,
      );

  /// Model set explicitly in the configuration, or null for auto-detection.
  DeviceModel? get _configuredModel =>
      _config.model == 'auto' ? null : DeviceModel.byId(_config.model);

  ProtocolService get _proto => ProtocolService(model, _config.deviceId);

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
  bool _exclusive = false;
  final List<Completer<void>> _idleWaiters = [];

  /// Coalesced value changes waiting to be sent, in first-queued order.
  final LinkedHashMap<String, List<List<int>>> _pendingOps = LinkedHashMap();

  /// Frames of the op currently being sent.
  final List<List<int>> _batch = [];

  /// Config chunks collected while a dump is being read.
  Map<int, List<int>>? _chunks;
  Timer? _keepaliveTimer;
  Timer? _meterTimer;
  Timer? _logFlushTimer;
  final Queue<LogEntry> _log = Queue();
  final List<LogEntry> _logPending = [];

  /// Poll the meters (a client shows this DSP); otherwise only the
  /// heartbeat keeps the link alive.
  bool _metersWanted = true;

  /// Last time a request was sent or answered.
  DateTime _lastActivity = DateTime.now();

  // ─── Lifecycle ───

  /// Start connecting (if enabled). Reconnects automatically until [stop].
  void start() {
    if (_running || _disposed || !_config.enabled) return;
    _running = true;
    unawaited(_connectLoop(++_generation));
  }

  /// Disconnect (with 0x11, like the original editor) and stop reconnecting.
  Future<void> stop() async {
    final wasRunning = _running;
    _running = false;
    _generation++;
    _wakeUp?.complete();
    _wakeUp = null;
    await _closeSocket(wasRunning ? 'Disconnected' : null, sayGoodbye: true);
    _setStatus(DspStatus.disconnected, progress: 0.0, message: null);
  }

  /// Apply new settings; reconnects if the address, model, device ID,
  /// password or enabled flag changed.
  Future<void> updateConfig({
    String? name,
    String? host,
    int? port,
    bool? enabled,
    String? model,
    int? deviceId,
    String? password,
  }) async {
    final reconnect = (host != null && host != _config.host) ||
        (port != null && port != _config.port) ||
        (enabled != null && enabled != _config.enabled) ||
        (model != null && model != _config.model) ||
        (deviceId != null && deviceId != _config.deviceId) ||
        (password != null && password.toUpperCase() != _config.password);
    _config = _config.copyWith(
        name: name,
        host: host,
        port: port,
        enabled: enabled,
        model: model,
        deviceId: deviceId,
        password: password?.toUpperCase());
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

  /// Close the connection. [sayGoodbye]: send the disconnect command (0x11)
  /// first, like the original editor when it ends a session.
  Future<void> _closeSocket(String? reason, {bool sayGoodbye = false}) async {
    _stopKeepalive();
    final socket = _socket;
    if (sayGoodbye && socket != null) await _sendDisconnect(socket);
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

  /// Send 0x11 (no answer expected).
  Future<void> _sendDisconnect(Socket socket) async {
    try {
      socket.add(ProtocolService.withDeviceId(ProtocolService.buildDisconnectCommand(), _config.deviceId));
      packetsSent++;
      await socket.flush().timeout(const Duration(milliseconds: 500));
    } catch (_) {
      // The connection is going away anyway.
    }
  }

  // ─── Init ───

  /// Like the original editor: handshake, device info (model and version
  /// check), lock/PTS status (with the stored password if the DSP is
  /// locked), then everything [_readAll] reads. The state is only published
  /// once everything has been read.
  Future<void> _runInit() async {
    _setStatus(DspStatus.initializing, progress: 0.0);
    final steps = _StepCounter(3 + _readAllSteps, (p) => _setStatus(DspStatus.initializing, progress: p));
    final socket = _socket;

    _exclusive = true;
    try {
      await _handshake(socket, TRacksProto.handshakeCommand);
      steps.next();

      final info = await _handshake(socket, TRacksProto.requestDeviceInfo);
      final deviceName =
          (ProtocolService(DeviceModel.dsp408, _config.deviceId).decode(info) as DeviceInfoMessage?)?.deviceName ?? '';
      steps.next();
      final model = await _checkDevice(socket, deviceName);
      _addStatusLog('Device: $deviceName (${model.name})');
      // Collect slots and preset names into a fresh state of the right
      // model; it is published after the config dump.
      state = DeviceState(model)..deviceName = deviceName;
      meters = List.filled(model.meterCount, 0.0);
      dynamics = 0;

      await _checkLock(socket);
      steps.next();
      await _readAll(socket, steps.next);
    } finally {
      _exclusive = false;
      _chunks = null;
    }
    _events.add(const StateReplaced());
  }

  /// The model of the device; like the original editor, a device that does
  /// not report itself as a DSP 408/206 with protocol version V010 (or not
  /// as the configured model) is refused.
  Future<DeviceModel> _checkDevice(Socket? socket, String deviceName) async {
    final detected = DeviceModel.detect(deviceName);
    final configured = _configuredModel;
    String? wrong;
    if (detected == null) {
      wrong = 'Wrong device: the DSP reports "$deviceName"';
    } else if (configured != null && configured != detected) {
      wrong = 'Wrong device: the DSP reports "$deviceName", configured as ${configured.name}';
    } else if (!detected.supportsVersion(deviceName)) {
      wrong = 'Wrong device version: "$deviceName"';
    }
    if (wrong != null) {
      if (socket != null) await _sendDisconnect(socket);
      throw SessionFailure(wrong);
    }
    return detected!;
  }

  /// Ask for the lock/PTS status (0x2C). A locked DSP, or one whose PTS
  /// limit is reached, needs its password (0x2D); without a stored
  /// password, or with a wrong one, the session ends like the original
  /// editor's Cancel (0x11).
  Future<void> _checkLock(Socket? socket) async {
    final answer = await _optional(socket, TRacksProto.requestLockStatus);
    final lock = answer == null ? null : _proto.decode(answer);
    if (lock is! LockStatusMessage || !(lock.ptsExpired || lock.locked)) return;

    final mode = lock.ptsExpired ? 1 : 0;
    final what = mode == 1 ? 'The PTS limit of the DSP is reached' : 'The DSP is locked';
    _addStatusLog('$what – checking the password');
    Future<Never> refuse(String reason) async {
      if (socket != null) await _sendDisconnect(socket);
      throw SessionFailure('$what – $reason');
    }

    if (_config.password.isEmpty) await refuse('enter its password in the DSP settings');
    final check = await _request(_proto.buildPasswordCommand(mode, _config.password),
        timeout: const Duration(milliseconds: 3100), retries: 0, fatal: false);
    final reply = check == null ? null : _proto.decode(check);
    if (reply is! PasswordResultMessage) await refuse('the password check was not answered');
    if (!reply.ok) await refuse('wrong password');
    _addStatusLog('Password accepted');
    await Future<void>.delayed(sequenceGap);
  }

  /// Requests of [_readAll].
  int get _readAllSteps => 2 + TRacksProto.presetCount + ConfigDump(model).chunkCount + 2;

  /// Preset slot table (0x22), current preset (0x14), preset names (0x29),
  /// the config dump (0x27) and the heartbeats (0x12) – what the original
  /// editor reads when it connects and after linking or copying.
  Future<void> _readAll(Socket? socket, void Function() step) async {
    await _optional(socket, TRacksProto.requestPresetSlots);
    step();
    await _optional(socket, TRacksProto.requestCurrentPreset);
    step();
    for (final cmd in TRacksProto.loadAllPresetsCommands) {
      await _require(socket, cmd);
      step();
    }
    final stream = await _readConfig(socket, step);
    _applyDump(stream);
    for (final cmd in TRacksProto.statusQueryCommands) {
      await _optional(socket, cmd);
      step();
    }
  }

  /// Read the config dump: as many chunks as the model's layout needs. A
  /// chunk the device does not answer ends the dump.
  Future<List<int>> _readConfig(Socket? socket, void Function() step) async {
    final dump = ConfigDump(model);
    final chunks = _chunks = {};
    for (var i = 0; i < dump.chunkCount; i++) {
      final response = await _request(TRacksProto.configDumpCommand(i), fatal: false);
      step();
      if (response == null) {
        if (socket == null || !identical(_socket, socket)) throw SessionFailure(message ?? 'Connection lost');
        _addStatusLog('Config chunk $i not answered – dump ends here');
        break;
      }
      await Future<void>.delayed(sequenceGap);
    }
    _chunks = null;
    return [for (var i = 0; chunks.containsKey(i); i++) ...chunks[i]!];
  }

  /// Decode [stream] into the state. Keeps the device name and what was
  /// read before the dump (preset names, slots, current preset).
  void _applyDump(List<int> stream) {
    final result = ConfigDump(model).decode(stream);
    configReadable = result.isPlausible;
    final next = configReadable ? result.state : DeviceState(model);
    next.presets.addAll(state.presets);
    next.presetSlots.addAll(state.presetSlots);
    next.currentPresetNumber = state.currentPresetNumber;
    next.deviceName = state.deviceName;
    state = next;
    if (configReadable) {
      _addStatusLog('Configuration read – active preset: ${state.currentPreset}');
      if (!result.switchesRead) {
        _addStatusLog('The dump ends before the mute and PEQ bypass bits – shown as off');
      }
    } else {
      _addStatusLog('Configuration could not be decoded (${result.problems.length} implausible '
          'values, e.g. ${result.problems.take(3).join('; ')}) – showing defaults');
      logWarn('[${_config.name}] config dump not recognised: ${result.problems.take(5).join('; ')}');
    }
    _emitInfo();
  }

  /// Handshake request: 4 s timeout, 3 attempts, else the init fails.
  Future<List<int>> _handshake(Socket? socket, List<int> frame) async {
    if (socket == null || !identical(_socket, socket)) throw SessionFailure(message ?? 'Connection lost');
    final response = await _request(frame, timeout: handshakeTimeout, retries: 2, fatal: false);
    if (response != null) {
      await Future<void>.delayed(sequenceGap);
      return response;
    }
    throw SessionFailure(!identical(_socket, socket)
        ? (message ?? 'Connection lost during initialization')
        : 'Initialization failed: DSP is not responding');
  }

  /// Request that must be answered, else the link is broken.
  Future<List<int>> _require(Socket? socket, List<int> frame, {Duration? timeout}) async {
    if (socket == null || !identical(_socket, socket)) throw SessionFailure(message ?? 'Connection lost');
    final response = await _request(frame, timeout: timeout);
    if (response != null) {
      await Future<void>.delayed(sequenceGap);
      return response;
    }
    throw SessionFailure(message ?? 'DSP is not responding');
  }

  /// Request whose answer is optional (a device that does not answer it is
  /// tolerated). Returns the answer or null.
  Future<List<int>?> _optional(Socket? socket, List<int> frame) async {
    if (socket == null || !identical(_socket, socket)) throw SessionFailure(message ?? 'Connection lost');
    final response = await _request(frame, fatal: false);
    if (response == null) {
      if (!identical(_socket, socket)) throw SessionFailure(message ?? 'Connection lost');
      logDebug('[${_config.name}] no answer to optional ${PacketDescriber.describe(frame)}');
    }
    await Future<void>.delayed(sequenceGap);
    return response;
  }

  /// Send [frames] one by one, each after the previous answer. Returns
  /// false if the device stopped answering.
  Future<bool> _runSequence(
    List<List<int>> frames, {
    required void Function(int done, int total) onProgress,
    Duration? Function(List<int> frame)? timeoutFor,
  }) async {
    // Abort if the connection is replaced while the sequence is running.
    final socket = _socket;
    bool connected() => socket != null && identical(_socket, socket);

    for (var i = 0; i < frames.length; i++) {
      if (!connected()) return false;
      final response = await _request(frames[i], timeout: timeoutFor?.call(frames[i]));
      if (response == null) return false;
      onProgress(i + 1, frames.length);
      await Future<void>.delayed(sequenceGap);
    }
    return connected();
  }

  // ─── Requests ───

  /// Send [frame] (addressed to the device ID) and wait for the answer.
  /// An unanswered request is sent [retries] more times; after that it
  /// returns null and, if [fatal], the link counts as broken.
  Future<List<int>?> _request(List<int> frame,
      {bool keepalive = false, Duration? timeout, int retries = 1, bool fatal = true}) {
    final socket = _socket;
    if (socket == null || _inFlight != null) return Future.value(null);

    final request = _Request(
        ProtocolService.withDeviceId(frame, _config.deviceId), keepalive, timeout ?? requestTimeout, retries, fatal);
    _inFlight = request;
    _send(socket, request);
    return request.done.future;
  }

  void _send(Socket socket, _Request request) {
    final frame = request.frame;
    socket.add(frame);
    packetsSent++;
    _lastActivity = DateTime.now();
    if (logTx) {
      _addLog(LogEntry.packet(
        rawHex: 'Tx: ${_proto.formatAsHex(frame)} → ${_proto.formatAsAscii(frame)}',
        decoded: 'Tx: ${PacketDescriber.describe(frame, model)}',
        timestamp: _elapsed,
      ));
    }
    request.timer = Timer(request.timeout, () {
      if (!identical(_inFlight, request)) return;
      logDebug('[${_config.name}] timeout for ${PacketDescriber.describe(frame, model)}');
      if (request.retriesLeft > 0 && identical(_socket, socket)) {
        request.retriesLeft--;
        _send(socket, request);
        return;
      }
      _inFlight = null;
      request.done.complete(null);
      if (request.fatal) _connectionLost(socket, 'DSP is not responding (link broken)');
      _afterRequest();
    });
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
    if (message == null) return; // not an answer of our device

    if (logRx) {
      _addLog(LogEntry.packet(
        rawHex: 'Rx: ${_proto.formatAsHex(frame)} → ${_proto.formatAsAscii(frame)}',
        decoded: 'Rx: ${PacketDescriber.describeRx(frame, model, _config.deviceId)}',
        timestamp: _elapsed,
      ));
    }

    switch (message) {
      case KeepaliveMessage(:final meterLevels, :final dynamics):
        meters = meterLevels;
        this.dynamics = dynamics;
        _scheduleMeters();
      case PresetMessage(:final index, :final name):
        state.presets[index] = name;
        if (status == DspStatus.ready) _events.add(const StateReplaced());
      case PresetSlotsMessage(:final used):
        state.presetSlots
          ..clear()
          ..addAll([for (var i = 0; i < used.length; i++) if (used[i]) i]);
      case CurrentPresetMessage(:final number):
        state.currentPresetNumber = number;
      case ChannelConfigMessage(:final subIndex, :final chunkData):
        _chunks?[subIndex] = chunkData;
      default:
        break;
    }

    // Meter frames only answer meter requests; any other response answers
    // the command in flight.
    final request = _inFlight;
    if (request != null && request.keepalive == (message.command == 0x40)) {
      request.timer?.cancel();
      _inFlight = null;
      _lastActivity = DateTime.now();
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

  // ─── Meters & heartbeat ───

  /// Poll the meters only while a client shows this DSP; otherwise the
  /// idle heartbeat (0x12) keeps the link alive, like the original editor.
  void setMetersWanted(bool wanted) => _metersWanted = wanted;

  void _startKeepalive() {
    _stopKeepalive();
    _keepaliveTimer = Timer.periodic(Duration(milliseconds: _config.refreshMs), (_) {
      if (status != DspStatus.ready || !_isIdle) return;
      if (_metersWanted) {
        unawaited(_request(_proto.buildKeepaliveCommand(), keepalive: true));
      } else if (DateTime.now().difference(_lastActivity) >= heartbeatIdle) {
        unawaited(_request(ProtocolService.buildHeartbeatCommand()));
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

  /// Change the meter polling interval (5–300 ms).
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

    // Like the original editor: the change goes to the device once, for
    // the edited channel; linked channels follow in the state (the DSP is
    // expected to apply the change to them itself).
    for (final expanded in DeviceOps.expandLinked(state, op)) {
      DeviceOps.apply(state, expanded);
      _events.add(OpApplied(expanded, origin));
    }
    // A newer value for a key replaces the queued one but keeps its place.
    DeviceOps.commands(state, op).forEach((key, frames) => _pendingOps[key] = frames);
    _pump();
    return null;
  }

  /// Link [slaves] to [master] like the original editor's link dialog:
  /// every newly linked slave gets the master's settings on the device
  /// (0x2A), the master gets its new mask (0x3B), then everything is read
  /// again. An empty list unlinks [master]. Returns an error or null.
  String? linkChannels(String master, List<String> slaves) {
    if (status != DspStatus.ready) return 'DSP is not ready';
    final ({int mask, List<String> added, List<String> reset}) change;
    try {
      change = DeviceOps.linkChange(state, master, slaves);
    } on ArgumentError catch (e) {
      return '${e.message}';
    }
    final proto = _proto;
    final frames = [
      for (final s in change.added) proto.buildCopyChannelCommand(master, s),
      proto.buildLinkCommand(master, change.mask),
      // Masks of older panel versions that still list other channels.
      for (final s in change.reset) proto.buildLinkCommand(s, model.linkBit(s)),
    ];
    final label = slaves.isEmpty ? 'Unlinking $master …' : 'Linking ${[master, ...slaves].join(' + ')} …';
    _deviceJob(label, frames, 'Link ${slaves.isEmpty ? 'released' : 'set'}: ${[master, ...slaves].join(' + ')}');
    return null;
  }

  /// Copy [from] to every channel of [targets] on the device (0x2A), then
  /// read everything again, like the original editor. Returns an error or
  /// null.
  String? copyChannels(String from, List<String> targets) {
    if (status != DspStatus.ready) return 'DSP is not ready';
    if (targets.isEmpty) return 'No channel to copy to';
    if (targets.contains(from)) return '$from cannot be copied to itself';
    final List<List<int>> frames;
    try {
      frames = [for (final to in targets) _proto.buildCopyChannelCommand(from, to)];
    } on ArgumentError catch (e) {
      return '${e.message}';
    }
    _deviceJob('Copying $from …', frames, 'Copied $from to ${targets.join(', ')}');
    return null;
  }

  /// Send [frames] exclusively (after the queued value changes), then read
  /// everything again.
  void _deviceJob(String label, List<List<int>> frames, String doneLog) {
    unawaited(_exclusiveJob(label, () async {
      final socket = _socket;
      final steps = _StepCounter(frames.length + _readAllSteps, (p) => _setStatus(DspStatus.busy, progress: p));
      for (final frame in frames) {
        await _require(socket, frame);
        steps.next();
      }
      await _readAll(socket, steps.next);
      _events.add(const StateReplaced());
      _addStatusLog(doneLog);
    }, drainFirst: true));
  }

  /// Send every value of [target] to the device (loading a preset stored on
  /// the server), like the original editor loads a preset file: the name
  /// (0x26), every value as a single command, then the heartbeat. Returns
  /// an error message or null.
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
      final steps = _StepCounter(ops.length + 2, (p) => _setStatus(DspStatus.busy, progress: p));
      await _require(socket, _proto.buildStorePresetNameCommand(label));
      steps.next();
      for (final op in ops) {
        DeviceOps.apply(state, op);
        for (final frames in DeviceOps.commands(state, op).values) {
          for (final frame in frames) {
            await _require(socket, frame);
          }
        }
        steps.next();
      }
      await _optional(socket, ProtocolService.buildHeartbeatCommand());
      steps.next();
      state.currentPreset = label;
      _events.add(const StateReplaced());
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
        await _require(socket, _proto.buildLoadPresetCommand(index), timeout: recallTimeout);
        steps.next();
        state.currentPresetNumber = index + 1;
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
          onProgress: (done, total) => _setStatus(DspStatus.busy, progress: done / total),
          timeoutFor: (frame) => frame[5] == 0x21 ? storeTimeout : null);
      if (ok) {
        // Same sanitising as the name command
        final stored = String.fromCharCodes(
            _proto.buildStorePresetNameCommand(trimmed).sublist(6, 20)).trim();
        state.presets[index] = stored;
        state.presetSlots.add(index);
        state.currentPresetNumber = index + 1;
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
      // Let queued value changes reach the device first.
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
