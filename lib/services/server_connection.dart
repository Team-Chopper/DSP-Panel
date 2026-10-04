// Server Connection
//
// Login state and the WebSocket to the panel server. The web client is
// always served by the panel server itself, so all URLs are relative to the
// page's own origin.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

enum ServerState {
  /// Asking the server whether a login is needed.
  checking,

  /// Login required (or the session expired).
  loginRequired,

  /// Opening the WebSocket.
  connecting,

  /// Connected; messages flow.
  connected,

  /// Server unreachable; retrying.
  disconnected,
}

class ServerConnection extends ChangeNotifier {
  final Uri base;

  ServerConnection({Uri? base}) : base = base ?? Uri.base;

  ServerState _state = ServerState.checking;
  bool _authEnabled = false;
  bool _passwordSet = true;
  String? _error;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _subscription;
  Timer? _retryTimer;
  int _retryAttempt = 0;
  bool _disposed = false;

  final _messages = StreamController<Map<String, Object?>>.broadcast();

  ServerState get state => _state;
  bool get authEnabled => _authEnabled;
  bool get passwordSet => _passwordSet;
  String? get error => _error;
  Stream<Map<String, Object?>> get messages => _messages.stream;

  Uri get _wsUri => base.replace(
        scheme: base.scheme == 'https' ? 'wss' : 'ws',
        path: '/ws',
        query: null,
        fragment: null,
      );

  Uri _api(String path) => base.replace(path: path, query: null, fragment: null);

  void start() => unawaited(_checkSession());

  Future<void> _checkSession() async {
    _setState(ServerState.checking);
    try {
      final response = await http.get(_api('/api/session')).timeout(const Duration(seconds: 5));
      final json = jsonDecode(response.body) as Map<String, dynamic>;
      _authEnabled = json['authEnabled'] as bool;
      _passwordSet = json['passwordSet'] as bool? ?? true;
      if (_authEnabled && json['authenticated'] != true) {
        _error = null;
        _setState(ServerState.loginRequired);
      } else {
        await _connect();
      }
    } catch (e) {
      _error = 'Server not reachable';
      _scheduleRetry();
    }
  }

  /// Log in with the panel password. Returns an error message or null.
  Future<String?> login(String password) async {
    try {
      final response = await http
          .post(_api('/api/login'),
              headers: {'content-type': 'application/json'},
              body: jsonEncode({'password': password}))
          .timeout(const Duration(seconds: 15));
      final json = jsonDecode(response.body) as Map<String, dynamic>;
      if (response.statusCode != 200) {
        return json['error'] as String? ?? 'Login failed';
      }
      await _connect();
      return null;
    } catch (e) {
      return 'Server not reachable';
    }
  }

  Future<void> logout() async {
    _retryTimer?.cancel();
    try {
      await http.post(_api('/api/logout'));
    } catch (_) {}
    await _closeChannel();
    _setState(ServerState.loginRequired);
  }

  Future<void> _connect() async {
    await _closeChannel();
    _setState(ServerState.connecting);
    final channel = WebSocketChannel.connect(_wsUri);
    _channel = channel;
    try {
      await channel.ready.timeout(const Duration(seconds: 10));
    } catch (e) {
      if (!identical(_channel, channel)) return;
      _channel = null;
      // The upgrade fails with 401 when the login expired; re-check.
      _error = 'Could not connect to the server';
      _scheduleRetry();
      return;
    }
    if (!identical(_channel, channel)) return;
    _retryAttempt = 0;
    _error = null;
    _setState(ServerState.connected);
    _subscription = channel.stream.listen(
      (data) {
        if (data is! String) return;
        try {
          _messages.add((jsonDecode(data) as Map).cast<String, Object?>());
        } catch (e) {
          debugPrint('Invalid server message: $e');
        }
      },
      onDone: () => _onClosed(channel),
      onError: (Object _) => _onClosed(channel),
      cancelOnError: true,
    );
  }

  void _onClosed(WebSocketChannel channel) {
    if (!identical(_channel, channel)) return;
    _channel = null;
    _error = 'Connection to the server lost';
    _scheduleRetry();
  }

  void _scheduleRetry() {
    if (_disposed) return;
    _setState(ServerState.disconnected);
    const delays = [1, 2, 5, 10];
    final delay = delays[_retryAttempt.clamp(0, delays.length - 1)];
    _retryAttempt++;
    _retryTimer?.cancel();
    _retryTimer = Timer(Duration(seconds: delay), () => unawaited(_checkSession()));
  }

  Future<void> _closeChannel() async {
    final channel = _channel;
    _channel = null;
    await _subscription?.cancel();
    _subscription = null;
    await channel?.sink.close();
  }

  /// Send a message; silently dropped while disconnected.
  void send(Map<String, Object?> message) {
    if (_state != ServerState.connected) return;
    _channel?.sink.add(jsonEncode(message));
  }

  void _setState(ServerState state) {
    if (_disposed) return;
    _state = state;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _retryTimer?.cancel();
    unawaited(_closeChannel());
    unawaited(_messages.close());
    super.dispose();
  }
}
