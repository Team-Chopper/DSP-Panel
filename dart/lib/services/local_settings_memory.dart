// In-memory storage for LocalSettings outside a browser.

final _values = <String, String>{};

String? read(String key) => _values[key];

void write(String key, String value) => _values[key] = value;
