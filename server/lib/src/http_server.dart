// HTTP(S) server
//
//   GET  /api/session  {authEnabled, authenticated, passwordSet}
//   POST /api/login    {password} → sets the session cookie
//   POST /api/logout   clears the session cookie
//   GET  /ws           WebSocket (see hub.dart)
//   GET  /*            Flutter web client (static files)

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:mime/mime.dart';
import 'package:shelf/shelf.dart';
import 'package:shelf/shelf_io.dart' as shelf_io;
import 'package:shelf_router/shelf_router.dart';
import 'package:shelf_static/shelf_static.dart';
import 'package:shelf_web_socket/shelf_web_socket.dart';

import 'auth.dart';
import 'config.dart';
import 'hub.dart';
import 'log.dart';

class PanelServer {
  final ServerConfig config;
  final AuthService auth;
  final Hub hub;
  HttpServer? _server;

  PanelServer({required this.config, required this.auth, required this.hub});

  int get port => _server!.port;

  Future<void> start() async {
    SecurityContext? context;
    if (config.tlsEnabled) {
      context = SecurityContext()
        ..useCertificateChain(config.tlsCert)
        ..usePrivateKey(config.tlsKey);
    }
    _server = await shelf_io.serve(buildHandler(), config.address, config.port,
        securityContext: context, poweredByHeader: null);
    _server!.autoCompress = true;
    logInfo('Listening on ${config.tlsEnabled ? 'https' : 'http'}://${config.address}:${_server!.port}');
  }

  Future<void> stop() async => _server?.close(force: true);

  Handler buildHandler() {
    final router = Router()
      ..get('/api/session', _session)
      ..post('/api/login', _login)
      ..post('/api/logout', _logout)
      ..get('/ws', _webSocket);

    return const Pipeline()
        .addMiddleware(_securityHeaders())
        .addHandler(Cascade().add(router.call).add(_staticHandler()).handler);
  }

  // ─── API ───

  bool _isAuthenticated(Request request) =>
      !auth.enabled || auth.verifyToken(_cookie(request, AuthService.cookieName));

  Response _json(Object body, {int status = 200, Map<String, String>? headers}) => Response(
        status,
        body: jsonEncode(body),
        headers: {'content-type': 'application/json', 'cache-control': 'no-store', ...?headers},
      );

  Response _session(Request request) => _json({
        'authEnabled': auth.enabled,
        'authenticated': _isAuthenticated(request),
        'passwordSet': auth.hasPassword,
      });

  Future<Response> _login(Request request) async {
    if (!auth.enabled) return _json({'ok': true});
    final client = _remote(request);
    if (!auth.allowAttempt(client)) {
      return _json({'error': 'Too many failed attempts – please wait a minute.'}, status: 429);
    }
    if (!auth.hasPassword) {
      return _json({'error': 'No password has been set yet (run: dsp-server passwd).'}, status: 503);
    }

    String? password;
    try {
      final body = await request.readAsString().timeout(const Duration(seconds: 5));
      if (body.length <= 4096) {
        final json = jsonDecode(body);
        if (json is Map && json['password'] is String) password = json['password'] as String;
      }
    } catch (_) {}
    if (password == null) return _json({'error': 'Password missing'}, status: 400);

    if (!auth.verifyPassword(password)) {
      auth.recordFailure(client);
      logWarn('Failed login from $client');
      return _json({'error': 'Wrong password'}, status: 401);
    }
    logInfo('Login from $client');
    return _json({'ok': true}, headers: {
      'set-cookie': _sessionCookie(auth.issueToken(), AuthService.sessionLifetime.inSeconds),
    });
  }

  Response _logout(Request request) =>
      _json({'ok': true}, headers: {'set-cookie': _sessionCookie('', 0)});

  String _sessionCookie(String value, int maxAge) =>
      '${AuthService.cookieName}=$value; Path=/; Max-Age=$maxAge; HttpOnly; SameSite=Strict'
      '${config.tlsEnabled ? '; Secure' : ''}';

  FutureOr<Response> _webSocket(Request request) {
    if (!_isSameOrigin(request)) {
      logWarn('Rejected WebSocket from foreign origin ${request.headers['origin']}');
      return Response.forbidden('Origin not allowed');
    }
    if (!_isAuthenticated(request)) return Response(401, body: 'Not logged in');
    final remote = _remote(request);
    return webSocketHandler(
      (channel, _) => hub.handleConnection(channel, remote: remote),
      pingInterval: const Duration(seconds: 20),
    )(request);
  }

  /// Browsers always send Origin on WebSocket requests. It must match the
  /// Host we were addressed with, otherwise another website could drive the
  /// panel with the user's cookie (cross-site WebSocket hijacking).
  static bool _isSameOrigin(Request request) {
    final origin = request.headers['origin'];
    if (origin == null) return true; // non-browser client
    final host = request.headers['host'];
    final uri = Uri.tryParse(origin);
    if (uri == null || host == null) return false;
    final originHost = uri.hasPort ? '${uri.host}:${uri.port}' : uri.host;
    String normalize(String h) => h.toLowerCase().replaceAll(RegExp(r':(443|80)$'), '');
    return normalize(originHost) == normalize(host);
  }

  static String? _cookie(Request request, String name) {
    final header = request.headers['cookie'];
    if (header == null) return null;
    for (final part in header.split(';')) {
      final index = part.indexOf('=');
      if (index > 0 && part.substring(0, index).trim() == name) {
        return part.substring(index + 1).trim();
      }
    }
    return null;
  }

  static String _remote(Request request) {
    final info = request.context['shelf.io.connection_info'];
    return info is HttpConnectionInfo ? info.remoteAddress.address : 'unknown';
  }

  // ─── Static files ───

  Handler _staticHandler() {
    if (!Directory(config.webRoot).existsSync()) {
      logWarn('Web root ${config.webRoot} not found – only the API is served');
      return (request) => Response.notFound('Web client not installed (${config.webRoot})');
    }
    final mime = MimeTypeResolver()
      ..addExtension('wasm', 'application/wasm')
      ..addExtension('mjs', 'text/javascript')
      ..addExtension('json', 'application/json');
    return createStaticHandler(config.webRoot,
        defaultDocument: 'index.html', contentTypeResolver: mime);
  }

  static Middleware _securityHeaders() => (inner) => (request) async {
        final response = await inner(request);
        return response.change(headers: {
          'x-content-type-options': 'nosniff',
          'referrer-policy': 'no-referrer',
          'content-security-policy': "frame-ancestors 'none'",
          'x-frame-options': 'DENY',
          // Always revalidate so a server update is picked up immediately.
          if (!response.headers.containsKey('cache-control')) 'cache-control': 'no-cache',
        });
      };
}
