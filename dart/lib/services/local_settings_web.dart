// localStorage backend for LocalSettings in the browser.

import 'package:web/web.dart' as web;

String? read(String key) {
  try {
    return web.window.localStorage.getItem(key);
  } catch (_) {
    return null; // storage disabled (e.g. private mode)
  }
}

void write(String key, String value) {
  try {
    web.window.localStorage.setItem(key, value);
  } catch (_) {
    // storage disabled or full: the setting is not remembered
  }
}
