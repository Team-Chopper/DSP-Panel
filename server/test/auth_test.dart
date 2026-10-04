import 'dart:io';

import 'package:dsp_server/src/auth.dart';
import 'package:test/test.dart';

void main() {
  late Directory dir;

  setUp(() async => dir = await Directory.systemTemp.createTemp('dsp_auth'));
  tearDown(() async => dir.delete(recursive: true));

  test('PBKDF2 matches the RFC 7914 test vector', () {
    // RFC 7914 §11: PBKDF2-HMAC-SHA256, P="passwd", S="salt", c=1, dkLen=64 (first 32 bytes)
    final key = AuthService.pbkdf2('passwd'.codeUnits, 'salt'.codeUnits, 1);
    expect(key.map((b) => b.toRadixString(16).padLeft(2, '0')).join(),
        '55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc');
  });

  test('password and session tokens', () async {
    final auth = AuthService(enabled: true, dataDir: dir.path);
    await auth.init();
    expect(auth.hasPassword, isFalse);
    expect(auth.verifyPassword(''), isFalse);

    await auth.setPassword('correct horse');
    expect(auth.verifyPassword('correct horse'), isTrue);
    expect(auth.verifyPassword('wrong'), isFalse);

    final token = auth.issueToken();
    expect(auth.verifyToken(token), isTrue);
    expect(auth.verifyToken('${token}x'), isFalse);
    expect(auth.verifyToken(token.replaceFirst(RegExp(r'^\d+'), '9999999999')), isFalse);
    expect(auth.verifyToken(null), isFalse);

    // Survives a restart (same secret and hash on disk) …
    final restarted = AuthService(enabled: true, dataDir: dir.path);
    await restarted.init();
    expect(restarted.verifyToken(token), isTrue);
    expect(restarted.verifyPassword('correct horse'), isTrue);

    // … but not a password change.
    await restarted.setPassword('battery staple');
    expect(restarted.verifyToken(token), isFalse);
  });

  test('rate limiting after repeated failures', () async {
    final auth = AuthService(enabled: true, dataDir: dir.path);
    for (var i = 0; i < AuthService.maxFailuresPerMinute; i++) {
      expect(auth.allowAttempt('10.0.0.1'), isTrue);
      auth.recordFailure('10.0.0.1');
    }
    expect(auth.allowAttempt('10.0.0.1'), isFalse);
    expect(auth.allowAttempt('10.0.0.2'), isTrue);
  });
}
