// Device Operations
//
// Every change to a DeviceState is expressed as an "op": a small JSON map
// such as {"op": "gain", "ch": "In A", "db": -3.5}. The client applies an op
// locally (optimistic UI) and sends it to the server; the server validates
// it, applies it to its own state, turns it into device commands and
// forwards it to the other clients.
//
// Ops carry absolute values (mute on/off, full routing mask, full PEQ band)
// so that concurrent clients cannot toggle each other back and forth.
// Dynamics, delay and filter values are raw protocol values (see
// ProtocolService for the conversions).

import 'device_model.dart';
import 'device_state.dart';
import 'protocol_service.dart';

class DeviceOps {
  static const _proto = ProtocolService();

  // ─── Constructors (used by the client) ───

  static Map<String, Object?> gain(String ch, double db) =>
      {'op': 'gain', 'ch': ch, 'db': db};
  static Map<String, Object?> mute(String ch, bool on) =>
      {'op': 'mute', 'ch': ch, 'on': on};
  static Map<String, Object?> phase(String ch, bool on) =>
      {'op': 'phase', 'ch': ch, 'on': on};
  static Map<String, Object?> delay(String ch, int raw) =>
      {'op': 'delay', 'ch': ch, 'raw': raw};
  static Map<String, Object?> matrixRoute(String out, int mask) =>
      {'op': 'matrixRoute', 'out': out, 'mask': mask};
  static Map<String, Object?> matrixGain(String out, String input, double db) =>
      {'op': 'matrixGain', 'out': out, 'in': input, 'db': db};
  static Map<String, Object?> geqBand(String ch, int band, double db) =>
      {'op': 'geqBand', 'ch': ch, 'band': band, 'db': db};
  static Map<String, Object?> geqReset(String ch) => {'op': 'geqReset', 'ch': ch};
  static Map<String, Object?> peqBand(String ch, int band, PeqBand b) =>
      {'op': 'peqBand', 'ch': ch, 'band': band, ...b.toJson()};
  static Map<String, Object?> hiPass(String ch, FilterState f) =>
      {'op': 'hiPass', 'ch': ch, ...f.toJson()};
  static Map<String, Object?> loPass(String ch, FilterState f) =>
      {'op': 'loPass', 'ch': ch, ...f.toJson()};
  static Map<String, Object?> gate(String ch, GateState g) =>
      {'op': 'gate', 'ch': ch, ...g.toJson()};
  static Map<String, Object?> compressor(String ch, CompressorState c) =>
      {'op': 'compressor', 'ch': ch, ...c.toJson()};
  static Map<String, Object?> limiter(String ch, LimiterState l) =>
      {'op': 'limiter', 'ch': ch, 'attack': l.attack, 'release': l.release, 'threshold': l.threshold};

  /// Device channel name; null or empty restores the default ("InA").
  static Map<String, Object?> channelName(String ch, String? name) =>
      {'op': 'channelName', 'ch': ch, 'name': name};

  /// Link mask of a channel (bits of the channels of its group it is the
  /// master of, including itself; a slave or unlinked channel has only its
  /// own bit).
  static Map<String, Object?> link(String ch, int mask) => {'op': 'link', 'ch': ch, 'mask': mask};

  /// Signal generator of the DSP (0 = off, 1 = pink, 2 = white, 3 = sine
  /// with [param] = frequency index).
  static Map<String, Object?> testTone(int type, [int param = 0]) =>
      {'op': 'testTone', 'type': type, 'param': param};

  /// Highest sine frequency index of the generator (31 ISO frequencies).
  static const int maxTestToneParam = geqBandCount - 1;

  // ─── Linking (master/slave), copying, whole states ───
  //
  // Links work like in the original editor: a master is linked to one or
  // more slaves (In A → In B). The master's link mask holds its own bit and
  // the bits of its slaves; a slave keeps only its own bit and cannot be
  // linked to anything else. A change of any member applies to the whole
  // group (master and all slaves).

  /// Ops that change another channel with the same value when it is linked.
  /// Matrix routing and crossing gains are not mirrored: linked channels are
  /// usually a left/right pair fed from different inputs.
  static const _linkableOps = {
    'gain', 'mute', 'phase', 'delay', 'peqBand', 'hiPass', 'loPass',
    'gate', 'compressor', 'limiter', 'geqBand', 'geqReset',
  };

  static int _mask(DeviceState state, String ch) => state.links[ch] ?? state.model.linkBit(ch);

  /// Master whose link mask contains [ch] (not [ch] itself), or null.
  ///
  /// Symmetric masks (every member lists every other, as stored by older
  /// panel versions) are read as a group whose first channel is the master.
  static String? masterOf(DeviceState state, String ch) {
    final model = state.model;
    final group = model.groupOf(ch);
    final bit = model.linkBit(ch);
    final own = _mask(state, ch);
    for (final other in group) {
      if (other == ch || _mask(state, other) & bit == 0) continue;
      final mutual = own & model.linkBit(other) != 0;
      if (mutual && group.indexOf(ch) < group.indexOf(other)) continue;
      return other;
    }
    return null;
  }

  /// Slaves of [ch] (empty unless [ch] is a master).
  static List<String> slavesOf(DeviceState state, String ch) {
    final model = state.model;
    if (masterOf(state, ch) != null) return const [];
    final mask = _mask(state, ch);
    return [
      for (final other in model.groupOf(ch))
        if (other != ch && mask & model.linkBit(other) != 0 && masterOf(state, other) == ch) other,
    ];
  }

  /// All members of [ch]'s link group, master first; just [ch] if unlinked.
  static List<String> linkGroupOf(DeviceState state, String ch) {
    final master = masterOf(state, ch) ?? ch;
    return [master, ...slavesOf(state, master)];
  }

  /// Channels [ch] is linked with (not including itself).
  static List<String> linkedWith(DeviceState state, String ch) =>
      [for (final c in linkGroupOf(state, ch)) if (c != ch) c];

  /// [op] plus a copy for every other member of the link group of the
  /// channel it changes.
  static List<Map<String, Object?>> expandLinked(DeviceState state, Map<String, Object?> op) {
    if (!_linkableOps.contains(op['op'])) return [op];
    final ch = op['ch'] as String;
    return [
      op,
      for (final other in linkedWith(state, ch)) {...op, 'ch': other},
    ];
  }

  /// Channels that can become slaves of [master]: same group, not [master],
  /// not a slave of another master and not a master themselves.
  static List<String> linkCandidates(DeviceState state, String master) {
    if (masterOf(state, master) != null) return const [];
    return [
      for (final other in state.model.groupOf(master))
        if (other != master &&
            (masterOf(state, other) ?? master) == master &&
            slavesOf(state, other).isEmpty)
          other,
    ];
  }

  /// Ops that make [slaves] the slaves of [master] (replacing its previous
  /// slaves); an empty list unlinks [master]. If [copySettings] is set, the
  /// settings of the master are copied to newly linked slaves.
  static List<Map<String, Object?>> linkSlaves(DeviceState state, String master, List<String> slaves,
      {bool copySettings = true}) {
    final model = state.model;
    if (!model.hasChannel(master)) throw ArgumentError('Unknown channel $master');
    final group = model.groupOf(master);
    if (slaves.any((c) => !group.contains(c) || c == master)) {
      throw ArgumentError('Only inputs or only outputs can be linked');
    }
    if (masterOf(state, master) != null) {
      throw ArgumentError('$master is linked to ${masterOf(state, master)} and cannot be a master');
    }
    final candidates = linkCandidates(state, master);
    final invalid = slaves.where((s) => !candidates.contains(s)).toList();
    if (invalid.isNotEmpty) throw ArgumentError('${invalid.join(', ')} cannot be linked to $master');

    final previous = slavesOf(state, master);
    final mask = slaves.fold<int>(model.linkBit(master), (m, c) => m | model.linkBit(c));
    return [
      link(master, mask),
      for (final s in {...previous, ...slaves})
        if (_mask(state, s) != model.linkBit(s)) link(s, model.linkBit(s)),
      if (copySettings)
        for (final s in slaves)
          if (!previous.contains(s)) ...copyChannel(state, master, s),
    ];
  }

  /// Ops that remove [ch] from its link group (a slave leaves its master,
  /// a master releases all its slaves).
  static List<Map<String, Object?>> unlink(DeviceState state, String ch) {
    final master = masterOf(state, ch);
    if (master == null) return linkSlaves(state, ch, const [], copySettings: false);
    final model = state.model;
    return [
      link(master, _mask(state, master) & ~model.linkBit(ch)),
      // All slaves get their own bit only (also normalises symmetric masks
      // of older versions, which would otherwise still point to [ch]).
      for (final s in slavesOf(state, master))
        if (_mask(state, s) != model.linkBit(s)) link(s, model.linkBit(s)),
    ];
  }

  /// Ops that copy the settings of [from] to [to] (same group only):
  /// gain, phase, delay, PEQ, filters, GEQ/gate (inputs) and dynamics
  /// (outputs). Name, mute, link and matrix routing are not copied (copying
  /// left to right must not route the right channel from the left input).
  static List<Map<String, Object?>> copyChannel(DeviceState state, String from, String to) {
    final model = state.model;
    if (from == to || !model.hasChannel(from) || !model.hasChannel(to) ||
        model.isInput(from) != model.isInput(to)) {
      throw ArgumentError('Settings can only be copied between two inputs or two outputs');
    }
    return [
      gain(to, state.gains[from]!),
      phase(to, state.phase[from]!),
      delay(to, state.delays[from]!),
      for (var b = 0; b < state.peqBands[from]!.length; b++) peqBand(to, b, state.peqBands[from]![b]),
      hiPass(to, state.hiPass[from]!),
      loPass(to, state.loPass[from]!),
      if (model.isInput(from)) ...[
        for (var b = 0; b < geqBandCount; b++) geqBand(to, b, state.geqBands[from]![b]),
        gate(to, state.gates[from]!),
      ] else ...[
        compressor(to, state.compressors[from]!),
        limiter(to, state.limiters[from]!),
      ],
    ];
  }

  /// Ops that bring a device to [state] completely (e.g. when a preset
  /// stored on the server is loaded onto a DSP).
  static List<Map<String, Object?>> fullState(DeviceState state) {
    final model = state.model;
    return [
      for (final ch in model.channels) ...[
        channelName(ch, state.channelNames[ch]),
        link(ch, state.links[ch] ?? model.linkBit(ch)),
        gain(ch, state.gains[ch]!),
        mute(ch, state.mutes[ch]!),
        phase(ch, state.phase[ch]!),
        delay(ch, state.delays[ch]!),
        hiPass(ch, state.hiPass[ch]!),
        loPass(ch, state.loPass[ch]!),
        for (var b = 0; b < state.peqBands[ch]!.length; b++) peqBand(ch, b, state.peqBands[ch]![b]),
      ],
      for (final ch in model.inputs) ...[
        for (var b = 0; b < geqBandCount; b++) geqBand(ch, b, state.geqBands[ch]![b]),
        gate(ch, state.gates[ch]!),
      ],
      for (final ch in model.outputs) ...[
        matrixRoute(ch, state.matrixRouting[ch]!),
        for (final input in model.inputs) matrixGain(ch, input, state.matrixGains['$ch:$input'] ?? 0),
        compressor(ch, state.compressors[ch]!),
        limiter(ch, state.limiters[ch]!),
      ],
    ];
  }

  // ─── Validation ───

  /// Value ranges of raw dynamics parameters (see TODO.md: not yet verified
  /// against the original editor).
  static const int maxGateTime = 2998;      // 1 … 2999 ms
  static const int maxDynTime = 4999;       // 1 … 5000 ms
  static const int minThresholdRaw = 0;     // -90 dB
  static const int maxGateThresholdRaw = 180; // 0 dB
  static const int maxThresholdRaw = 220;   // +20 dB
  static const int maxKnee = 12;

  /// Returns an error message, or null if [op] is well-formed for [model].
  static String? validate(Object? op, DeviceModel model) {
    if (op is! Map) return 'op must be an object';
    String? str(String k) => op[k] is String ? op[k] as String : null;
    bool isNum(String k) => op[k] is num && (op[k] as num).isFinite;
    bool isBool(String k) => op[k] is bool;
    bool intIn(String k, int min, int max) =>
        op[k] is int && (op[k] as int) >= min && (op[k] as int) <= max;

    final ch = str('ch');
    final hasChannel = ch != null && model.hasChannel(ch);
    final isInput = ch != null && model.isInput(ch);
    final isOutput = ch != null && model.isOutput(ch);

    switch (op['op']) {
      case 'gain':
        if (!hasChannel) return 'invalid channel';
        if (!isNum('db')) return 'db must be a number';
      case 'mute':
      case 'phase':
        if (!hasChannel) return 'invalid channel';
        if (!isBool('on')) return 'on must be a bool';
      case 'delay':
        if (!hasChannel) return 'invalid channel';
        if (!intIn('raw', 0, ProtocolService.delayRawMax)) return 'delay out of range';
      case 'matrixRoute':
        final out = str('out');
        if (out == null || !model.isOutput(out)) return 'invalid output';
        if (!intIn('mask', 0, model.matrixMask)) return 'invalid input mask';
      case 'matrixGain':
        final out = str('out');
        final input = str('in');
        if (out == null || !model.isOutput(out)) return 'invalid output';
        if (input == null || !model.isInput(input)) return 'invalid input';
        if (!isNum('db')) return 'db must be a number';
      case 'geqBand':
        if (!isInput) return 'invalid GEQ channel';
        if (!intIn('band', 0, geqBandCount - 1)) return 'invalid band';
        if (!isNum('db')) return 'db must be a number';
      case 'geqReset':
        if (!isInput) return 'invalid GEQ channel';
      case 'peqBand':
        if (!hasChannel) return 'invalid channel';
        if (!intIn('band', 0, peqBandCountFor(model, ch) - 1)) return 'invalid band';
        if (!intIn('freqRaw', 0, ProtocolService.freqRawMax) ||
            !intIn('qRaw', 0, ProtocolService.qRawMax) ||
            !intIn('type', 0, ProtocolService.peqTypeNames.length - 1) ||
            !isNum('gainDb') ||
            !isBool('bypass')) {
          return 'invalid PEQ band';
        }
      case 'hiPass':
      case 'loPass':
        if (!hasChannel) return 'invalid channel';
        if (!intIn('freqRaw', 0, ProtocolService.freqRawMax) ||
            !intIn('slope', 0, ProtocolService.crossoverSlopeNames.length - 1)) {
          return 'invalid filter';
        }
      case 'gate':
        if (!isInput) return 'gate is only available on inputs';
        if (!intIn('attack', 0, maxGateTime) ||
            !intIn('release', 0, maxGateTime) ||
            !intIn('hold', 0, maxGateTime) ||
            !intIn('threshold', minThresholdRaw, maxGateThresholdRaw)) {
          return 'invalid gate';
        }
      case 'compressor':
        if (!isOutput) return 'compressor is only available on outputs';
        if (!intIn('ratio', 0, ProtocolService.compressorRatioNames.length - 1) ||
            !intIn('attack', 0, maxDynTime) ||
            !intIn('release', 0, maxDynTime) ||
            !intIn('knee', 0, maxKnee) ||
            !intIn('threshold', minThresholdRaw, maxThresholdRaw)) {
          return 'invalid compressor';
        }
      case 'limiter':
        if (!isOutput) return 'limiter is only available on outputs';
        if (!intIn('attack', 0, maxDynTime) ||
            !intIn('release', 0, maxDynTime) ||
            !intIn('threshold', minThresholdRaw, maxThresholdRaw)) {
          return 'invalid limiter';
        }
      case 'channelName':
        if (!hasChannel) return 'invalid channel';
        final name = op['name'];
        if (name != null &&
            (name is! String || name.length > 8 || name.codeUnits.any((c) => c < 0x20 || c > 0x7E))) {
          return 'name must be at most 8 ASCII characters';
        }
      case 'link':
        if (!hasChannel) return 'invalid channel';
        // A slave may also be stored with an empty mask.
        if (!intIn('mask', 0, model.groupMask(ch)) ||
            (op['mask'] != 0 && (op['mask'] as int) & model.linkBit(ch) == 0)) {
          return 'invalid link mask';
        }
      case 'testTone':
        if (!intIn('type', 0, ProtocolService.testToneTypeNames.length - 1) ||
            !intIn('param', 0, maxTestToneParam)) {
          return 'invalid test tone';
        }
      default:
        return 'unknown op ${op['op']}';
    }
    return null;
  }

  // ─── Apply ───

  /// Apply an op that passed [validate] to [state], clamping and quantizing
  /// values the same way on client and server.
  static void apply(DeviceState state, Map<String, Object?> op) {
    final ch = op['ch'] as String?;
    int i(String k) => op[k] as int;
    switch (op['op']) {
      case 'gain':
        state.gains[ch!] = _proto.quantizeGain((op['db'] as num).toDouble().clamp(-60.0, 12.0));
      case 'mute':
        state.mutes[ch!] = op['on'] as bool;
      case 'phase':
        state.phase[ch!] = op['on'] as bool;
      case 'delay':
        state.delays[ch!] = i('raw');
      case 'matrixRoute':
        state.matrixRouting[op['out'] as String] = i('mask');
      case 'matrixGain':
        state.matrixGains['${op['out']}:${op['in']}'] = _proto
            .quantizeGain((op['db'] as num).toDouble().clamp(-60.0, ProtocolService.matrixGainMaxDb));
      case 'geqBand':
        state.geqBands[ch!]![i('band')] =
            _proto.quantizeGeqDb((op['db'] as num).toDouble().clamp(-12.0, 12.0));
      case 'geqReset':
        state.geqBands[ch!] = List.filled(geqBandCount, 0.0);
      case 'peqBand':
        state.peqBands[ch!]![i('band')] = PeqBand(
          freqRaw: i('freqRaw'),
          qRaw: i('qRaw'),
          gainDb: _proto.quantizeGeqDb((op['gainDb'] as num).toDouble().clamp(-12.0, 12.0)),
          type: i('type'),
          bypass: op['bypass'] as bool,
        );
      case 'hiPass':
        state.hiPass[ch!] = FilterState(freqRaw: i('freqRaw'), slope: i('slope'));
      case 'loPass':
        state.loPass[ch!] = FilterState(freqRaw: i('freqRaw'), slope: i('slope'));
      case 'gate':
        state.gates[ch!] = GateState(
            attack: i('attack'), release: i('release'), hold: i('hold'), threshold: i('threshold'));
      case 'compressor':
        state.compressors[ch!] = CompressorState(
          ratio: i('ratio'),
          attack: i('attack'),
          release: i('release'),
          knee: i('knee'),
          threshold: i('threshold'),
        );
      case 'limiter':
        final unknown = state.limiters[ch!]?.unknown ?? 0;
        state.limiters[ch] = LimiterState(
            attack: i('attack'), release: i('release'), unknown: unknown, threshold: i('threshold'));
      case 'channelName':
        final name = (op['name'] as String?)?.trim();
        state.channelNames[ch!] =
            (name == null || name.isEmpty) ? state.model.defaultDeviceName(ch) : name;
      case 'link':
        state.links[ch!] = i('mask');
      case 'testTone':
        state.testToneType = i('type');
        state.testToneParam = i('param');
    }
  }

  // ─── Device commands (server) ───

  /// Device frames for an op that has already been applied to [state],
  /// keyed by a coalescing key: a newer value for the same key replaces an
  /// older one that has not been sent yet.
  static Map<String, List<List<int>>> commands(DeviceState state, Map<String, Object?> op) {
    final proto = ProtocolService(state.model);
    final ch = op['ch'] as String?;
    switch (op['op']) {
      case 'gain':
        // The original app always sends gain twice; keep that until it is
        // known to be unnecessary.
        final frame = proto.buildGainCommand(ch!, state.gains[ch]!);
        return {'gain:$ch': [frame, frame]};
      case 'mute':
        return {'mute:$ch': [proto.buildMuteCommand(ch!, state.mutes[ch]!)]};
      case 'phase':
        return {'phase:$ch': [proto.buildPhaseCommand(ch!, state.phase[ch]!)]};
      case 'delay':
        return {'delay:$ch': [proto.buildDelayCommand(ch!, state.delays[ch]!)]};
      case 'matrixRoute':
        final out = op['out'] as String;
        return {'matrix:$out': [proto.buildMatrixRoutingCommand(out, state.matrixRouting[out]!)]};
      case 'matrixGain':
        final out = op['out'] as String;
        final input = op['in'] as String;
        return {
          'mixgain:$out:$input': [
            proto.buildMatrixGainCommand(out, input, state.matrixGains['$out:$input'] ?? 0),
          ],
        };
      case 'geqBand':
        final band = op['band'] as int;
        return {'geq:$ch:$band': [proto.buildGeqBandCommand(ch!, band, state.geqBands[ch]![band])]};
      case 'geqReset':
        return {
          for (int band = 0; band < geqBandCount; band++)
            'geq:$ch:$band': [proto.buildGeqBandCommand(ch!, band, 0.0)],
        };
      case 'peqBand':
        final band = op['band'] as int;
        final b = state.peqBands[ch]![band];
        return {
          'peq:$ch:$band': [
            proto.buildPeqBandCommand(ch!, band,
                gainDb: b.gainDb, freqRaw: b.freqRaw, qRaw: b.qRaw, type: b.type, bypass: b.bypass),
          ],
        };
      case 'hiPass':
        final f = state.hiPass[ch]!;
        return {'hpf:$ch': [proto.buildHiPassCommand(ch!, f.freqRaw, f.slope)]};
      case 'loPass':
        final f = state.loPass[ch]!;
        return {'lpf:$ch': [proto.buildLoPassCommand(ch!, f.freqRaw, f.slope)]};
      case 'gate':
        final g = state.gates[ch]!;
        return {
          'gate:$ch': [
            proto.buildGateCommand(ch!,
                attack: g.attack, release: g.release, hold: g.hold, threshold: g.threshold),
          ],
        };
      case 'compressor':
        final c = state.compressors[ch]!;
        return {
          'comp:$ch': [
            proto.buildCompressorCommand(ch!,
                ratio: c.ratio, attack: c.attack, release: c.release, knee: c.knee, threshold: c.threshold),
          ],
        };
      case 'limiter':
        final l = state.limiters[ch]!;
        return {
          'limit:$ch': [
            proto.buildLimiterCommand(ch!,
                attack: l.attack, release: l.release, unknown: l.unknown, threshold: l.threshold),
          ],
        };
      case 'channelName':
        return {'name:$ch': [proto.buildChannelNameCommand(ch!, state.channelNames[ch]!)]};
      case 'link':
        return {'link:$ch': [proto.buildLinkCommand(ch!, state.links[ch]!)]};
      case 'testTone':
        return {'testTone': [proto.buildTestToneCommand(state.testToneType, state.testToneParam)]};
      default:
        return const {};
    }
  }
}
