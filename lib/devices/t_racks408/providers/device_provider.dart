// Device State Provider
//
// Client-side mirror of one DSP's state. The panel server owns the real
// state; this provider applies changes immediately for a smooth UI
// (optimistic update) and sends them to the server as ops. Changes made by
// other clients arrive as ops or full state snapshots.

import 'dart:async';
import 'dart:collection';

import 'package:dsp_core/dsp_core.dart';
import 'package:flutter/foundation.dart';

export 'package:dsp_core/dsp_core.dart'
    show PeqBand, FilterState, GateState, CompressorState, LimiterState, DeviceModel, DeviceState,
        ProtocolService, geqBandCount;

class DeviceProvider extends ChangeNotifier {
  final String dspId;
  final void Function(Map<String, Object?> message) _send;

  /// Whether the DSP accepts device commands right now.
  final bool Function() _isReady;

  DeviceProvider(this.dspId, this._send, this._isReady, {DeviceModel model = DeviceModel.dsp408})
      : _state = DeviceState(model),
        _meterLevels = List.filled(model.meterCount, 0.0);

  static const _proto = ProtocolService();

  DeviceState _state;

  // Meter levels (linear float values, 0.0 = silence), in channel order
  List<double> _meterLevels;

  // Last active slope per filter, restored when a filter is switched back on
  final Map<String, int> _lastSlope = {};

  // Outgoing ops, coalesced per key and flushed every 30 ms
  final LinkedHashMap<String, Map<String, Object?>> _outgoing = LinkedHashMap();
  Timer? _flushTimer;

  // ─── Topology ───

  DeviceModel get model => _state.model;
  List<String> get inputs => model.inputs;
  List<String> get outputs => model.outputs;

  // ─── Getters ───

  Map<int, String> get presets => Map.unmodifiable(_state.presets);
  String get currentPreset => _state.currentPreset;
  String get deviceName => _state.deviceName;

  double getInputGain(String channel) => _state.gains[channel] ?? 0;
  double getOutputGain(String channel) => _state.gains[channel] ?? 0;

  bool getInputMute(String channel) => _state.mutes[channel] ?? false;
  bool getOutputMute(String channel) => _state.mutes[channel] ?? false;

  bool getInputPhase(String channel) => _state.phase[channel] ?? false;
  bool getOutputPhase(String channel) => _state.phase[channel] ?? false;

  /// Display name of a channel: the name stored on the device, or the label
  /// if the device still uses its default name ("InA").
  String getAlias(String channel) {
    final name = _state.channelNames[channel];
    return (name == null || name == model.defaultDeviceName(channel)) ? channel : name;
  }

  /// Matrix gain for an input-output crossing point (read from the device)
  double getMatrixGain(String output, String input) =>
      _state.matrixGains['$output:$input'] ?? 0.0;

  /// Check if a matrix input is routed to an output
  bool getMatrixEnabled(String output, String input) =>
      (_state.matrixRouting[output] ?? 0) & model.matrixBit(input) != 0;

  /// Get meter level for a channel (linear float, 0 = silence)
  double getMeterLevel(String channel) {
    final idx = model.channels.indexOf(channel);
    return (idx < 0 || idx >= _meterLevels.length) ? 0.0 : _meterLevels[idx];
  }

  /// Get all GEQ band values for a channel
  List<double> getGeqBands(String channel) =>
      List.of(_state.geqBands[channel] ?? List.filled(geqBandCount, 0.0));

  /// Get a single GEQ band value
  double getGeqBand(String channel, int bandIndex) => getGeqBands(channel)[bandIndex];

  /// Get the number of PEQ bands for a channel (8 for inputs, 9 for outputs)
  int getPeqBandCount(String channel) => _state.peqBands[channel]?.length ?? 0;

  /// Get all PEQ bands for a channel
  List<PeqBand> getPeqBands(String channel) =>
      (_state.peqBands[channel] ?? []).map((b) => b.copy()).toList();

  /// Get a single PEQ band
  PeqBand getPeqBand(String channel, int band) => _state.peqBands[channel]![band].copy();

  /// Get Hi Pass filter state
  FilterState getHiPass(String channel) => (_state.hiPass[channel] ?? FilterState()).copy();

  /// Get Lo Pass filter state
  FilterState getLoPass(String channel) => (_state.loPass[channel] ?? FilterState(freqRaw: 300)).copy();

  /// Delay in milliseconds
  double getDelayMs(String channel) => ProtocolService.delayRawToMs(_state.delays[channel] ?? 0);

  GateState getGate(String channel) => (_state.gates[channel] ?? GateState()).copy();
  CompressorState getCompressor(String channel) => (_state.compressors[channel] ?? CompressorState()).copy();
  LimiterState getLimiter(String channel) => (_state.limiters[channel] ?? LimiterState()).copy();

  // ─── Changes ───

  /// Apply an op locally and queue it for the server. Ops for the same
  /// [key] are coalesced: only the newest is sent.
  void _change(String key, Map<String, Object?> op) {
    if (!_isReady()) return;
    if (DeviceOps.validate(op, model) != null) return;
    // Linked channels follow locally right away; the server does the same
    // expansion, so only the original op is sent.
    for (final expanded in DeviceOps.expandLinked(_state, op)) {
      DeviceOps.apply(_state, expanded);
    }
    notifyListeners();
    _outgoing[key] = op;
    _flushTimer ??= Timer(const Duration(milliseconds: 30), _flush);
  }

  /// Key under which an op is coalesced before sending.
  static String _keyFor(Map<String, Object?> op) =>
      [op['op'], op['ch'] ?? op['out'], op['band']].whereType<Object>().join(':');

  // ─── Linking and copying ───

  /// Channels [channel] is linked with (on the device, cmd 0x3B).
  List<String> linkedWith(String channel) => DeviceOps.linkedWith(_state, channel);

  /// Link [channels] (all inputs or all outputs) into one group; a single
  /// channel is unlinked from its group.
  void setLinkGroup(List<String> channels) {
    for (final op in DeviceOps.linkGroup(_state, channels)) {
      _change(_keyFor(op), op);
    }
  }

  /// Copy the settings of [from] to each of [targets] (same group).
  void copyChannel(String from, List<String> targets) {
    for (final to in targets) {
      for (final op in DeviceOps.copyChannel(_state, from, to)) {
        _change(_keyFor(op), op);
      }
    }
  }

  void _flush() {
    _flushTimer = null;
    for (final op in _outgoing.values) {
      _send({'t': Msg.op, 'dsp': dspId, 'op': op});
    }
    _outgoing.clear();
  }

  /// Set gain for a channel
  void setGain(String channel, double dB) =>
      _change('gain:$channel', DeviceOps.gain(channel, dB));

  /// Toggle mute for a channel
  void toggleMute(String channel) {
    if (!_state.mutes.containsKey(channel)) return;
    _change('mute:$channel', DeviceOps.mute(channel, !_state.mutes[channel]!));
  }

  /// Toggle phase invert for a channel
  void togglePhase(String channel) {
    if (!_state.phase.containsKey(channel)) return;
    _change('phase:$channel', DeviceOps.phase(channel, !_state.phase[channel]!));
  }

  /// Set delay in milliseconds (0 – 680 ms, 1/96 ms steps)
  void setDelayMs(String channel, double ms) =>
      _change('delay:$channel', DeviceOps.delay(channel, ProtocolService.delayMsToRaw(ms)));

  /// Toggle a matrix input for an output
  void toggleMatrixInput(String output, String input) {
    final mask = (_state.matrixRouting[output] ?? 0) ^ model.matrixBit(input);
    _change('matrix:$output', DeviceOps.matrixRoute(output, mask));
  }

  /// Set a single GEQ band value
  void setGeqBand(String channel, int bandIndex, double dB) =>
      _change('geq:$channel:$bandIndex', DeviceOps.geqBand(channel, bandIndex, dB));

  /// Reset all GEQ bands to flat (0 dB)
  void resetGeqBands(String channel) {
    _outgoing.removeWhere((key, _) => key.startsWith('geq:$channel:'));
    _change('geqReset:$channel', DeviceOps.geqReset(channel));
  }

  /// Set a PEQ band parameter
  void setPeqBand(String channel, int band, {
    double? gainDb,
    int? freqRaw,
    int? qRaw,
    int? type,
    bool? bypass,
  }) {
    final bands = _state.peqBands[channel];
    if (bands == null || band < 0 || band >= bands.length) return;
    final b = bands[band].copy();
    if (gainDb != null) b.gainDb = gainDb.clamp(-12.0, 12.0);
    if (freqRaw != null) b.freqRaw = freqRaw.clamp(0, ProtocolService.freqRawMax);
    if (qRaw != null) b.qRaw = qRaw.clamp(0, ProtocolService.qRawMax);
    if (type != null) b.type = type;
    if (bypass != null) b.bypass = bypass;
    _change('peq:$channel:$band', DeviceOps.peqBand(channel, band, b));
  }

  /// Set Hi Pass filter. [enabled] switches between bypass and the last
  /// active slope.
  void setHiPass(String channel, {int? freqRaw, bool? enabled, int? slope}) {
    final f = _state.hiPass[channel]?.copy();
    if (f == null) return;
    _updateFilter('hpf:$channel', f, freqRaw: freqRaw, enabled: enabled, slope: slope);
    _change('hpf:$channel', DeviceOps.hiPass(channel, f));
  }

  /// Set Lo Pass filter (see [setHiPass])
  void setLoPass(String channel, {int? freqRaw, int? slope, bool? enabled}) {
    final f = _state.loPass[channel]?.copy();
    if (f == null) return;
    _updateFilter('lpf:$channel', f, freqRaw: freqRaw, enabled: enabled, slope: slope);
    _change('lpf:$channel', DeviceOps.loPass(channel, f));
  }

  /// Slope of the HPF ([highPass]) or LPF: the active one, or the one the
  /// filter gets when it is switched on again.
  int filterSlope(String channel, {required bool highPass}) {
    final f = highPass ? getHiPass(channel) : getLoPass(channel);
    if (f.slope != 0) return f.slope;
    return _lastSlope['${highPass ? 'hpf' : 'lpf'}:$channel'] ?? ProtocolService.defaultSlope;
  }

  /// Choose the filter curve. A bypassed filter stays bypassed and uses the
  /// curve when it is switched on.
  void setFilterSlope(String channel, int slope, {required bool highPass}) {
    final f = highPass ? getHiPass(channel) : getLoPass(channel);
    if (f.slope == 0) {
      _lastSlope['${highPass ? 'hpf' : 'lpf'}:$channel'] = slope;
      notifyListeners();
    } else if (highPass) {
      setHiPass(channel, slope: slope);
    } else {
      setLoPass(channel, slope: slope);
    }
  }

  void _updateFilter(String key, FilterState f, {int? freqRaw, bool? enabled, int? slope}) {
    if (f.slope != 0) _lastSlope[key] = f.slope;
    if (freqRaw != null) f.freqRaw = freqRaw.clamp(0, ProtocolService.freqRawMax);
    if (slope != null) f.slope = slope;
    if (enabled != null) {
      f.slope = enabled ? (_lastSlope[key] ?? ProtocolService.defaultSlope) : 0;
    }
  }

  void setGate(String channel, GateState gate) =>
      _change('gate:$channel', DeviceOps.gate(channel, gate));

  void setCompressor(String channel, CompressorState compressor) =>
      _change('comp:$channel', DeviceOps.compressor(channel, compressor));

  void setLimiter(String channel, LimiterState limiter) =>
      _change('limit:$channel', DeviceOps.limiter(channel, limiter));

  /// Names are stored on the device: at most 8 ASCII characters.
  Future<void> loadAliases() async {}

  /// Set the channel name stored on the device
  Future<void> setAlias(String channel, String alias) async =>
      _change('name:$channel', DeviceOps.channelName(channel, alias));

  /// Restore the device's default channel name
  Future<void> resetAlias(String channel) async =>
      _change('name:$channel', DeviceOps.channelName(channel, null));

  /// Quantize like the device does (exposed for widgets that format values).
  double quantizeGain(double dB) => _proto.quantizeGain(dB);

  // ─── Updates from the server ───

  /// Replace the whole state (after subscribing, init or preset load).
  void applyState(Map<String, Object?> json) {
    final next = DeviceState.fromJson(json);
    if (next.model != model) {
      _outgoing.clear();
      _meterLevels = List.filled(next.model.meterCount, 0.0);
    }
    _state = next;
    // Keep local changes that have not been sent yet.
    for (final op in _outgoing.values) {
      if (DeviceOps.validate(op, model) != null) continue;
      for (final expanded in DeviceOps.expandLinked(_state, op)) {
        DeviceOps.apply(_state, expanded);
      }
    }
    notifyListeners();
  }

  /// Snapshot of the whole state (e.g. to show it in another view).
  DeviceState get stateSnapshot => DeviceState.fromJson(_state.toJson());

  /// Apply a change made by another client. The server sends linked
  /// channels as separate ops, so no expansion here.
  void applyRemoteOp(Object? op) {
    if (DeviceOps.validate(op, model) != null) return;
    DeviceOps.apply(_state, (op as Map).cast<String, Object?>());
    notifyListeners();
  }

  /// New meter levels; only rebuild if they changed noticeably.
  void applyMeters(List<double> levels) {
    if (levels.length != _meterLevels.length) {
      _meterLevels = levels;
      notifyListeners();
      return;
    }
    bool changed = false;
    for (int i = 0; i < levels.length; i++) {
      if ((levels[i] - _meterLevels[i]).abs() > 0.005) {
        changed = true;
        break;
      }
    }
    _meterLevels = levels;
    if (changed) notifyListeners();
  }

  @override
  void dispose() {
    _flushTimer?.cancel();
    super.dispose();
  }
}
