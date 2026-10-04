// UI settings remembered per browser (localStorage), e.g. which EQ curves
// are hidden on the gain page. Outside a browser (tests) they are kept in
// memory only.

import 'dart:convert';

import 'local_settings_memory.dart' if (dart.library.js_interop) 'local_settings_web.dart' as store;

class LocalSettings {
  static const _prefix = 'dsp-panel.';

  static String? getString(String key) => store.read('$_prefix$key');

  static void setString(String key, String value) => store.write('$_prefix$key', value);

  static Set<String> getStringSet(String key) {
    final raw = getString(key);
    if (raw == null) return {};
    try {
      return {for (final v in jsonDecode(raw) as List) v as String};
    } on FormatException {
      return {};
    }
  }

  static void setStringSet(String key, Set<String> values) => setString(key, jsonEncode(values.toList()..sort()));
}
