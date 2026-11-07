// Authentication
//
// One shared password, stored as PBKDF2-HMAC-SHA256 hash in auth.json in the
// data directory. A successful login yields a signed session token (cookie)
// that is valid for 30 days. The token embeds a fingerprint of the password
// hash, so changing the password logs out every session.

import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:crypto/crypto.dart';
import 'package:path/path.dart' as p;

class AuthService {
  static const cookieName = 'dsp_session';
  static const sessionLifetime = Duration(days: 30);
  static const iterations = 210000;
  static const maxFailuresPerMinute = 5;

  final bool enabled;
  final String dataDir;

  List<int> _secret = const [];
  _PasswordHash? _hash;
  final Map<String, List<DateTime>> _failures = {};

  AuthService({required this.enabled, required this.dataDir});

  bool get hasPassword => _hash != null;

  File get _authFile => File(p.join(dataDir, 'auth.json'));
  File get _secretFile => File(p.join(dataDir, 'secret.key'));

  /// Load (or create) the signing secret and the password hash.
  Future<void> init() async {
    await Directory(dataDir).create(recursive: true);
    if (_secretFile.existsSync()) {
      _secret = base64.decode(_secretFile.readAsStringSync().trim());
    } else {
      _secret = _randomBytes(32);
      await _writePrivate(_secretFile, base64.encode(_secret));
    }
    if (_authFile.existsSync()) {
      _hash = _PasswordHash.fromJson(
          (jsonDecode(_authFile.readAsStringSync()) as Map).cast<String, Object?>());
    }
  }

  Future<void> setPassword(String password) async {
    final salt = _randomBytes(16);
    _hash = _PasswordHash(salt, iterations, pbkdf2(utf8.encode(password), salt, iterations));
    await _writePrivate(_authFile, const JsonEncoder.withIndent('  ').convert(_hash!.toJson()));
  }

  bool verifyPassword(String password) {
    final hash = _hash;
    if (hash == null) return false;
    return _constantTimeEquals(pbkdf2(utf8.encode(password), hash.salt, hash.iterations), hash.hash);
  }

  // ─── Rate limiting ───

  /// False if [client] had too many failed logins in the last minute.
  bool allowAttempt(String client) {
    final cutoff = DateTime.now().subtract(const Duration(minutes: 1));
    final list = _failures[client]?..removeWhere((t) => t.isBefore(cutoff));
    if (list != null && list.isEmpty) _failures.remove(client);
    return (list?.length ?? 0) < maxFailuresPerMinute;
  }

  void recordFailure(String client) {
    if (_failures.length > 10000) _failures.clear(); // bound memory
    (_failures[client] ??= []).add(DateTime.now());
  }

  // ─── Session tokens ───

  String issueToken() {
    final expires = DateTime.now().add(sessionLifetime).millisecondsSinceEpoch ~/ 1000;
    final payload = '$expires.${_passwordFingerprint()}';
    return '$payload.${_sign(payload)}';
  }

  bool verifyToken(String? token) {
    if (token == null || _hash == null) return false;
    final parts = token.split('.');
    if (parts.length != 3) return false;
    final payload = '${parts[0]}.${parts[1]}';
    if (!_constantTimeEquals(utf8.encode(_sign(payload)), utf8.encode(parts[2]))) return false;
    final expires = int.tryParse(parts[0]);
    if (expires == null || expires * 1000 < DateTime.now().millisecondsSinceEpoch) return false;
    return parts[1] == _passwordFingerprint();
  }

  String _sign(String payload) =>
      base64Url.encode(Hmac(sha256, _secret).convert(utf8.encode(payload)).bytes).replaceAll('=', '');

  String _passwordFingerprint() =>
      base64Url.encode(sha256.convert(_hash!.hash).bytes.sublist(0, 9));

  // ─── Helpers ───

  /// PBKDF2-HMAC-SHA256 with a 32-byte output (one block).
  static List<int> pbkdf2(List<int> password, List<int> salt, int iterations) {
    final hmac = Hmac(sha256, password);
    var u = hmac.convert([...salt, 0, 0, 0, 1]).bytes;
    final result = List.of(u);
    for (var i = 1; i < iterations; i++) {
      u = hmac.convert(u).bytes;
      for (var j = 0; j < result.length; j++) {
        result[j] ^= u[j];
      }
    }
    return result;
  }

  static bool _constantTimeEquals(List<int> a, List<int> b) {
    if (a.length != b.length) return false;
    var diff = 0;
    for (var i = 0; i < a.length; i++) {
      diff |= a[i] ^ b[i];
    }
    return diff == 0;
  }

  static List<int> _randomBytes(int n) {
    final rnd = Random.secure();
    return List.generate(n, (_) => rnd.nextInt(256));
  }

  static Future<void> _writePrivate(File file, String content) async {
    final tmp = File('${file.path}.tmp');
    await tmp.writeAsString(content, flush: true);
    if (!Platform.isWindows) {
      await Process.run('chmod', ['600', tmp.path]);
    }
    await tmp.rename(file.path);
  }
}

class _PasswordHash {
  final List<int> salt;
  final int iterations;
  final List<int> hash;

  _PasswordHash(this.salt, this.iterations, this.hash);

  Map<String, Object?> toJson() => {
    'algorithm': 'pbkdf2-hmac-sha256',
    'iterations': iterations,
    'salt': base64.encode(salt),
    'hash': base64.encode(hash),
  };

  factory _PasswordHash.fromJson(Map<String, Object?> json) => _PasswordHash(
    base64.decode(json['salt'] as String),
    (json['iterations'] as num).toInt(),
    base64.decode(json['hash'] as String),
  );
}
