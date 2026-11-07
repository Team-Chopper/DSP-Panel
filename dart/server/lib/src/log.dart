// Minimal logging to stdout/stderr (captured by journald under systemd).

import 'dart:io';

bool verboseLogging = false;

void logInfo(String message) => stdout.writeln('[INFO] $message');

void logWarn(String message) => stderr.writeln('[WARN] $message');

void logDebug(String message) {
  if (verboseLogging) stdout.writeln('[DEBUG] $message');
}
