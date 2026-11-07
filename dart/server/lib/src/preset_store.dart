// Preset Store
//
// Presets kept on the server (in addition to the 20 slots on each DSP).
// A preset is a complete DeviceState of one model. It can be edited offline
// — the panel addresses it like a DSP with the id "preset:<id>" — and
// loaded onto any DSP of the same model.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:dsp_core/dsp_core.dart';

import 'device_registry.dart';
import 'log.dart';

class ServerPreset {
  final String id;
  String name;
  DeviceState state;
  int updated;

  ServerPreset(this.id, this.name, this.state, this.updated);

  ServerPresetInfo get info =>
      ServerPresetInfo(id: id, name: name, model: state.model.id, updated: updated);

  Map<String, Object?> toJson() => {'id': id, 'name': name, 'updated': updated, 'state': state.toJson()};

  factory ServerPreset.fromJson(Map<String, Object?> json) => ServerPreset(
    json['id'] as String,
    json['name'] as String,
    DeviceState.fromJson((json['state'] as Map).cast<String, Object?>()),
    (json['updated'] as num).toInt(),
  );
}

sealed class PresetEvent {
  const PresetEvent();
}

/// The list of presets (names, models) changed.
class PresetListChanged extends PresetEvent {
  const PresetListChanged();
}

/// An op was applied to a preset by [origin].
class PresetOpApplied extends PresetEvent {
  final String presetId;
  final Map<String, Object?> op;
  final Object? origin;
  const PresetOpApplied(this.presetId, this.op, this.origin);
}

class PresetStore {
  static const maxPresets = 500;

  final File file;
  final Map<String, ServerPreset> _presets = {};
  final _events = StreamController<PresetEvent>.broadcast();
  Timer? _saveTimer;

  PresetStore(this.file);

  Stream<PresetEvent> get events => _events.stream;

  List<ServerPresetInfo> get list =>
      [for (final p in _presets.values) p.info]..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));

  ServerPreset? operator [](String id) => _presets[id];

  void load() {
    if (!file.existsSync()) return;
    final json = jsonDecode(file.readAsStringSync()) as Map<String, Object?>;
    for (final p in (json['presets'] as List? ?? const [])) {
      try {
        final preset = ServerPreset.fromJson((p as Map).cast<String, Object?>());
        _presets[preset.id] = preset;
      } catch (e) {
        logWarn('Skipping unreadable preset in ${file.path}: $e');
      }
    }
  }

  static String? validateName(Object? name) {
    if (name is! String || name.trim().isEmpty || name.trim().length > 40) {
      return 'Preset name must be 1–40 characters';
    }
    return null;
  }

  /// Create a preset from [state] (copied). Returns the new preset.
  ServerPreset create(String name, DeviceState state) {
    if (_presets.length >= maxPresets) throw StateError('At most $maxPresets presets');
    final copy = DeviceState.fromJson(state.toJson())..currentPreset = name.trim();
    final preset = ServerPreset(DspConfig.newId(), name.trim(), copy, DateTime.now().millisecondsSinceEpoch);
    _presets[preset.id] = preset;
    _changed(listChanged: true);
    return preset;
  }

  String? rename(String id, Object? name) {
    final preset = _presets[id];
    if (preset == null) return 'Unknown preset';
    final error = validateName(name);
    if (error != null) return error;
    preset
      ..name = (name as String).trim()
      ..state.currentPreset = name.trim()
      ..updated = DateTime.now().millisecondsSinceEpoch;
    _changed(listChanged: true);
    return null;
  }

  String? delete(String id) {
    if (_presets.remove(id) == null) return 'Unknown preset';
    _changed(listChanged: true);
    return null;
  }

  /// Apply an op from the offline editor (linked channels included).
  String? applyOp(String id, Map<String, Object?> op, {Object? origin}) {
    final preset = _presets[id];
    if (preset == null) return 'Unknown preset';
    final error = DeviceOps.validate(op, preset.state.model);
    if (error != null) return error;
    for (final expanded in DeviceOps.expandLinked(preset.state, op)) {
      DeviceOps.apply(preset.state, expanded);
      _events.add(PresetOpApplied(id, expanded, origin));
    }
    preset.updated = DateTime.now().millisecondsSinceEpoch;
    _changed();
    return null;
  }

  void _changed({bool listChanged = false}) {
    if (listChanged) _events.add(const PresetListChanged());
    _saveTimer?.cancel();
    _saveTimer = Timer(const Duration(milliseconds: 500), () => unawaited(save()));
  }

  Future<void> save() async {
    _saveTimer?.cancel();
    try {
      await file.parent.create(recursive: true);
      final tmp = File('${file.path}.tmp');
      await tmp.writeAsString(jsonEncode({'presets': [for (final p in _presets.values) p.toJson()]}), flush: true);
      await tmp.rename(file.path);
    } catch (e) {
      logWarn('Could not save ${file.path}: $e');
    }
  }

  Future<void> dispose() async {
    await save();
    await _events.close();
  }
}
