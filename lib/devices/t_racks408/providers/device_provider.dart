// Device State Provider
//
// Client-side mirror of one DSP's state. The panel server owns the real
// state; this provider applies changes immediately for a smooth UI
// (optimistic update) and sends them to the server as ops. Changes made by
// other clients arrive as ops or full state snapshots.

import 'dart:async';
import 'dart:collection';
import 'dart:math' as math;

import 'package:dsp_core/dsp_core.dart';
import 'package:flutter/foundation.dart';

export 'package:dsp_core/dsp_core.dart'
    show PeqBand, FilterState, GateState, CompressorState, LimiterState, DeviceModel, DeviceState,
        ProtocolService, geqBandCount;

/// Change notifier for the meters of a [DeviceProvider].
class MeterNotifier extends ChangeNotifier {
  void notify() => notifyListeners();
}

class DeviceProvider extends ChangeNotifier {
  final String dspId;
  final void Function(Map<String, Object?> message) _send;

  /// Whether the DSP accepts device commands right now.
  final bool Function() _isReady;

  DeviceProvider(this.dspId, this._send, this._isReady, {DeviceModel model = DeviceModel.dsp408})
      : _state = DeviceState(model),
        _meterDb = List.filled(model.meterCount, ProtocolService.silenceDb);

  static const _proto = ProtocolService();

  DeviceState _state;

  // Meter levels in dBFS, in channel order
  List<double> _meterDb;

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

  /// Matrix gain for an input-output crossing point in dB (−60 … 0)
  double getMatrixGain(String output, String input) =>
      _state.matrixGains['$output:$input'] ?? 0.0;

  /// Check if a matrix input is routed to an output
  bool getMatrixEnabled(String output, String input) =>
      (_state.matrixRouting[output] ?? 0) & model.matrixBit(input) != 0;

  /// Meter level of a channel in dBFS ([ProtocolService.silenceDb] = silence)
  double getMeterDb(String channel) {
    final idx = model.channels.indexOf(channel);
    return (idx < 0 || idx >= _meterDb.length) ? ProtocolService.silenceDb : _meterDb[idx];
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
      [op['op'], op['ch'] ?? op['out'], op['in'], op['band']].whereType<Object>().join(':');

  // ─── Linking and copying ───

  /// Channels [channel] is linked with (on the device, cmd 0x3B).
  List<String> linkedWith(String channel) => DeviceOps.linkedWith(_state, channel);

  /// Master [channel] is a slave of, or null.
  String? linkMaster(String channel) => DeviceOps.masterOf(_state, channel);

  /// Slaves of [channel] (empty unless it is a master).
  List<String> linkSlaves(String channel) => DeviceOps.slavesOf(_state, channel);

  /// Channels that can become slaves of [master].
  List<String> linkCandidates(String master) => DeviceOps.linkCandidates(_state, master);

  /// Make [slaves] the slaves of [master] (empty = unlink its slaves);
  /// newly linked slaves get the master's settings if [copySettings] is set.
  void setLinkSlaves(String master, List<String> slaves, {bool copySettings = true}) {
    for (final op in DeviceOps.linkSlaves(_state, master, slaves, copySettings: copySettings)) {
      _change(_keyFor(op), op);
    }
  }

  /// Set a raw link mask (used to lift a link for a measurement and restore
  /// it exactly afterwards).
  void setLinkMask(String channel, int mask) => _change('link:$channel', DeviceOps.link(channel, mask));

  /// Raw link mask of [channel] as stored on the device.
  int linkMask(String channel) => _state.links[channel] ?? model.linkBit(channel);

  /// Set mute to [on] (linked channels follow).
  void setMute(String channel, bool on) {
    if (!_state.mutes.containsKey(channel)) return;
    _change('mute:$channel', DeviceOps.mute(channel, on));
  }

  /// Remove [channel] from its link group.
  void unlink(String channel) {
    for (final op in DeviceOps.unlink(_state, channel)) {
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

  /// Set the gain of a matrix crossing point (−60 … 0 dB)
  void setMatrixGain(String output, String input, double dB) =>
      _change('mixgain:$output:$input', DeviceOps.matrixGain(output, input, dB));

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

  /// Signal generator of the DSP: type (0 = off, 1 = pink, 2 = white,
  /// 3 = sine) and the sine frequency index.
  int get testToneType => _state.testToneType;
  int get testToneParam => _state.testToneParam;

  void setTestTone(int type, [int? param]) =>
      _change('testTone', DeviceOps.testTone(type, param ?? _state.testToneParam));

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
      _meterDb = List.filled(next.model.meterCount, ProtocolService.silenceDb);
      _peakDb.clear();
      meters.notify();
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

  /// New meter levels in dBFS (channel order) and the compressor/limiter
  /// status word of the outputs ([dynamics], see KeepaliveMessage); notifies
  /// [meters] if they (or a hold indicator) changed noticeably.
  void applyMeters(List<double> levels, {int dynamics = 0}) {
    final now = DateTime.now();
    final dt = _lastMeterTime == null ? 0.0 : now.difference(_lastMeterTime!).inMicroseconds / 1e6;
    _lastMeterTime = now;
    if (levels.length != _meterDb.length) {
      _meterDb = levels;
      _peakDb.clear();
      meters.notify();
      return;
    }
    var changed = false;
    final channels = model.channels;
    for (var i = 0; i < levels.length && i < channels.length; i++) {
      final ch = channels[i];
      final db = levels[i];
      if ((db - _meterDb[i]).abs() > 0.2) changed = true;
      // Peak hold: jumps up, falls slowly after a short hold.
      final peak = _peakDb[ch] ?? ProtocolService.silenceDb;
      if (db >= peak) {
        _peakDb[ch] = db;
        _peakHeldUntil[ch] = now.add(_peakHold);
      } else if (now.isAfter(_peakHeldUntil[ch] ?? now)) {
        final fallen = math.max(db, peak - _peakFallDbPerSecond * dt);
        if ((fallen - peak).abs() > 0.1) changed = true;
        _peakDb[ch] = fallen;
      }
      // Indicators stay lit for a moment.
      if (db >= clipDb) _clipUntil[ch] = now.add(_indicatorHold);
      final output = model.outputs.indexOf(ch);
      if (output >= 0) {
        if (ProtocolService.compressorActive(dynamics, output)) _compUntil[ch] = now.add(_indicatorHold);
        if (ProtocolService.limiterActive(dynamics, output)) _limitUntil[ch] = now.add(_indicatorHold);
      }
      // Compare with what was shown last time, so an indicator that times
      // out between two frames goes dark.
      if (_shownClip.contains(ch) != isClipping(ch) ||
          _shownComp.contains(ch) != isCompressing(ch) ||
          _shownLimit.contains(ch) != isLimiting(ch)) {
        changed = true;
      }
    }
    _meterDb = levels;
    if (changed) {
      _shownClip
        ..clear()
        ..addAll(channels.where(isClipping));
      _shownComp
        ..clear()
        ..addAll(channels.where(isCompressing));
      _shownLimit
        ..clear()
        ..addAll(channels.where(isLimiting));
      meters.notify();
    }
  }

  /// Notifies when meter levels or indicators change. Kept apart from the
  /// provider itself so meter updates (every 25–40 ms) only repaint the
  /// meters, not whole pages with their EQ curves.
  final MeterNotifier meters = MeterNotifier();

  final Set<String> _shownClip = {};
  final Set<String> _shownComp = {};
  final Set<String> _shownLimit = {};

  static const _peakHold = Duration(milliseconds: 900);
  static const _peakFallDbPerSecond = 20.0;
  static const _indicatorHold = Duration(milliseconds: 1500);

  /// Level in dBFS at which the clip indicator lights up.
  static const clipDb = -0.5;

  DateTime? _lastMeterTime;
  final Map<String, double> _peakDb = {};
  final Map<String, DateTime> _peakHeldUntil = {};
  final Map<String, DateTime> _clipUntil = {};
  final Map<String, DateTime> _compUntil = {};
  final Map<String, DateTime> _limitUntil = {};

  /// Peak-hold level of a channel's meter in dBFS.
  double getMeterPeakDb(String channel) => _peakDb[channel] ?? ProtocolService.silenceDb;

  /// The channel reached full scale in the last 1.5 s.
  bool isClipping(String channel) => DateTime.now().isBefore(_clipUntil[channel] ?? DateTime(0));

  /// The DSP reported the compressor of this output working in the last 1.5 s.
  bool isCompressing(String channel) => DateTime.now().isBefore(_compUntil[channel] ?? DateTime(0));

  /// The DSP reported the limiter of this output working in the last 1.5 s.
  bool isLimiting(String channel) => DateTime.now().isBefore(_limitUntil[channel] ?? DateTime(0));

  @override
  void dispose() {
    _flushTimer?.cancel();
    meters.dispose();
    super.dispose();
  }
}
