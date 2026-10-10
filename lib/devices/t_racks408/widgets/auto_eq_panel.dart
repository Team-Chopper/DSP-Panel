// Auto-EQ: measure pink noise with the RTA microphone and propose a
// correction for one channel, like the auto-EQ of system processors.
//
// - Inputs on the GEQ page: the correction goes into the 31-band GEQ.
// - Any channel on the EQ page: it goes into the free PEQ bands (peaks at
//   0 dB), e.g. per output for subs and tops.
//
// During the measurement the other channels of the group are muted (and
// restored afterwards). A linked channel is measured together with its
// partners (left/right) or alone; for "alone" the link is lifted during the
// measurement – a mute on a partner would otherwise mute the channel itself.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../../../services/rta_service.dart';
import '../../../services/rta_settings_provider.dart';
import '../../../theme/app_colors.dart';
import '../providers/device_provider.dart';
import 'eq_math.dart';
import 'tone_generator_panel.dart';

enum AutoEqTarget { geq, peq }

class AutoEqPanel extends StatefulWidget {
  final DeviceProvider device;
  final String channel;
  final AutoEqTarget target;

  /// Running RTA of the page (GEQ page); null: the panel starts its own.
  final RtaService? rta;

  /// Offer the DSP's own generator (not in the offline editor).
  final bool dspGenerator;

  /// New GEQ proposal (all 31 bands) or null.
  final ValueChanged<List<double>?>? onGeqProposal;

  /// New PEQ proposal (all bands) or null.
  final ValueChanged<List<PeqBand>?>? onPeqProposal;

  final VoidCallback onClose;

  const AutoEqPanel({
    super.key,
    required this.device,
    required this.channel,
    required this.target,
    required this.onClose,
    this.rta,
    this.dspGenerator = true,
    this.onGeqProposal,
    this.onPeqProposal,
  });

  @override
  State<AutoEqPanel> createState() => _AutoEqPanelState();
}

class _AutoEqPanelState extends State<AutoEqPanel> {
  static const _measureSeconds = 8.0;

  RtaService? _ownRta;
  String? _rtaError;
  Timer? _timer;
  double _progress = 0;
  double _maxBoost = 6;
  bool _isolate = true;

  List<double>? _geqProposal;
  List<PeqBand>? _peqProposal;
  String? _note;

  /// Mutes and link masks to restore after the measurement.
  Map<String, bool>? _savedMutes;
  Map<String, int>? _savedLinks;

  DeviceProvider get _device => widget.device;
  RtaService? get _rta => widget.rta ?? _ownRta;
  bool get _measuring => _timer != null;
  bool get _hasProposal => _geqProposal != null || _peqProposal != null;

  @override
  void initState() {
    super.initState();
    if (widget.rta == null) _startOwnRta();
  }

  Future<void> _startOwnRta() async {
    final settings = context.read<RtaSettingsProvider>();
    if (!settings.isMobile) settings.enumerateDevices();
    final rta = RtaService();
    try {
      await rta.requestPermission();
      await rta.startCapture(deviceId: settings.deviceId);
      if (!mounted) {
        rta.release();
        return;
      }
      setState(() => _ownRta = rta);
    } catch (e) {
      rta.release();
      if (mounted) setState(() => _rtaError = 'Microphone could not be started: $e');
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    _rta?.cancelMeasurement();
    // Not while the widget tree is being torn down: the mute changes notify
    // the pages.
    final device = _device, mutes = _savedMutes, links = _savedLinks;
    if (mutes != null || links != null) scheduleMicrotask(() => _restoreValues(device, mutes, links));
    _ownRta?.release();
    super.dispose();
  }

  // ─── Isolation ───

  List<String> get _group => _device.model.groupOf(widget.channel);

  /// Mute the other channels of the group; [alone]: lift the link first.
  void _isolateChannel({required bool alone}) {
    final ch = widget.channel;
    final linkGroup = [ch, ..._device.linkedWith(ch)];
    if (alone && linkGroup.length > 1) {
      _savedLinks = {for (final c in linkGroup) c: _device.linkMask(c)};
      for (final c in linkGroup) {
        _device.setLinkMask(c, _device.model.linkBit(c));
      }
    }
    final playing = alone ? {ch} : linkGroup.toSet();
    _savedMutes = {for (final c in _group) c: _device.getInputMute(c)};
    for (final c in _group) {
      final mute = !playing.contains(c);
      if (_device.getInputMute(c) != mute) _device.setMute(c, mute);
    }
  }

  void _restore() {
    final mutes = _savedMutes, links = _savedLinks;
    _savedMutes = null;
    _savedLinks = null;
    _restoreValues(_device, mutes, links);
  }

  /// Mutes first (while the link is still lifted), then the link masks.
  static void _restoreValues(DeviceProvider device, Map<String, bool>? mutes, Map<String, int>? links) {
    mutes?.forEach((c, on) {
      if (device.getInputMute(c) != on) device.setMute(c, on);
    });
    links?.forEach(device.setLinkMask);
  }

  /// null: cancelled; otherwise whether the channel is measured alone.
  Future<bool?> _askLinked() async {
    final partners = _device.linkedWith(widget.channel);
    if (partners.isEmpty) return true;
    final names = partners.map(_device.getAlias).join(', ');
    return showDialog<bool>(
      context: context,
      builder: (dialog) => AlertDialog(
        title: Text('${_device.getAlias(widget.channel)} is linked'),
        content: SizedBox(
          width: 400,
          child: Text(
            '${_device.getAlias(widget.channel)} is linked with $names – usually a left/right pair.\n\n'
            'Together: both play, the other channels are muted, the result applies to all linked channels.\n'
            'Alone: the link is lifted during the measurement (and restored afterwards), only '
            '${_device.getAlias(widget.channel)} plays. Applying the result changes the linked channels too.',
          ),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.of(dialog).pop(), child: const Text('Cancel')),
          TextButton(onPressed: () => Navigator.of(dialog).pop(true), child: const Text('Alone')),
          FilledButton(onPressed: () => Navigator.of(dialog).pop(false), child: const Text('Together')),
        ],
      ),
    );
  }

  // ─── Measurement ───

  Future<void> _measure() async {
    final rta = _rta;
    if (rta == null || _measuring) return;
    if (_isolate) {
      final alone = await _askLinked();
      if (alone == null || !mounted) return;
      _isolateChannel(alone: alone);
    }
    _setProposal(null, null, note: null);
    rta.startMeasurement();
    setState(() {
      _progress = 0;
      _timer = Timer.periodic(const Duration(milliseconds: 200), (timer) {
        if (!mounted) {
          timer.cancel();
          return;
        }
        final progress = rta.measuredSeconds / _measureSeconds;
        if (progress < 1) {
          setState(() => _progress = progress);
          return;
        }
        timer.cancel();
        _timer = null;
        _restore();
        final measured = rta.stopMeasurement(geqCenterFrequencies);
        if (measured == null) {
          setState(() {});
          return;
        }
        _propose(measured);
      });
    });
  }

  void _cancelMeasurement() {
    _timer?.cancel();
    _timer = null;
    _rta?.cancelMeasurement();
    _restore();
    setState(() {});
  }

  void _propose(List<double> measured) {
    final ch = widget.channel;
    if (widget.target == AutoEqTarget.geq) {
      _setProposal(
        autoEqCorrection(
          measuredDb: measured,
          centers: geqCenterFrequencies,
          current: _device.getGeqBands(ch),
          maxBoost: _maxBoost,
        ),
        null,
      );
      return;
    }
    final bands = _device.getPeqBands(ch);
    final free = freePeqBands(bands);
    if (free.isEmpty) {
      _setProposal(null, null,
          note: 'No free PEQ band: Auto-EQ uses peak bands at 0 dB. Set a band to Peak with 0 dB and measure again.');
      return;
    }
    final target = autoEqCorrection(
      measuredDb: measured,
      centers: geqCenterFrequencies,
      current: List.filled(geqCenterFrequencies.length, 0.0),
      maxBoost: _maxBoost,
    );
    final proposal = fitPeqCorrection(target: target, centers: geqCenterFrequencies, bands: bands, maxBoost: _maxBoost);
    final used = [for (final i in free) if (proposal[i].gainDb.abs() >= 0.05) i];
    _setProposal(null, proposal,
        note: used.isEmpty
            ? 'The response is already within ±1 dB – nothing to correct.'
            : 'Uses band${used.length > 1 ? 's' : ''} ${used.map((i) => i + 1).join(', ')} of ${free.length} free.');
  }

  void _setProposal(List<double>? geq, List<PeqBand>? peq, {String? note}) {
    setState(() {
      _geqProposal = geq;
      _peqProposal = peq;
      _note = note;
    });
    widget.onGeqProposal?.call(geq);
    widget.onPeqProposal?.call(peq);
  }

  void _apply() {
    final ch = widget.channel;
    final geq = _geqProposal, peq = _peqProposal;
    if (geq != null) {
      final current = _device.getGeqBands(ch);
      for (var i = 0; i < geq.length; i++) {
        if (geq[i] != current[i]) _device.setGeqBand(ch, i, geq[i]);
      }
    }
    if (peq != null) {
      final current = _device.getPeqBands(ch);
      for (var i = 0; i < peq.length; i++) {
        final b = peq[i], c = current[i];
        if (b.freqRaw != c.freqRaw || b.qRaw != c.qRaw || b.gainDb != c.gainDb || b.type != c.type || b.bypass != c.bypass) {
          _device.setPeqBand(ch, i, freqRaw: b.freqRaw, qRaw: b.qRaw, gainDb: b.gainDb, type: b.type, bypass: b.bypass);
        }
      }
    }
    _setProposal(null, null);
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(
        content: Text('Auto-EQ applied to the ${geq != null ? 'GEQ' : 'PEQ'} of ${_device.getAlias(ch)}')));
  }

  // ─── UI ───

  @override
  Widget build(BuildContext context) {
    final alias = _device.getAlias(widget.channel);
    final isOutput = _device.outputs.contains(widget.channel);
    final String step;
    if (_rtaError != null) {
      step = _rtaError!;
    } else if (_rta == null) {
      step = 'Starting the microphone …';
    } else if (_measuring) {
      step = 'Measuring $alias … ${(_progress * _measureSeconds).toStringAsFixed(0)} / ${_measureSeconds.round()} s';
    } else if (_hasProposal) {
      step = 'Proposal for $alias shown as orange curve. ${_note ?? ''}';
    } else {
      step = _note ??
          '1. Pink noise into the system (generator below or external)${isOutput ? ', routed to $alias' : ' at $alias'}.  '
              '2. Measurement mic at the listening position.  3. Measure.';
    }
    final groupName = _device.inputs.contains(widget.channel) ? 'inputs' : 'outputs';

    return Container(
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(
        color: AppColors.surface,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: proposalBorder),
      ),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
        Row(children: [
          const Icon(Icons.auto_fix_high, size: 16, color: AppColors.warning),
          const SizedBox(width: 8),
          Text('Auto-EQ · $alias → ${widget.target == AutoEqTarget.geq ? 'GEQ' : 'PEQ'}',
              style: const TextStyle(fontSize: 13, color: AppColors.text)),
          const SizedBox(width: 12),
          Expanded(child: Text(step, style: const TextStyle(fontSize: 12, color: AppColors.muted))),
          IconButton(
            tooltip: 'Close Auto-EQ',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.close, size: 16),
            onPressed: widget.onClose,
          ),
        ]),
        if (_measuring)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: LinearProgressIndicator(value: _progress, color: AppColors.warning),
          ),
        const SizedBox(height: 6),
        Wrap(spacing: 12, runSpacing: 8, crossAxisAlignment: WrapCrossAlignment.center, children: [
          if (_measuring)
            OutlinedButton.icon(
              style: OutlinedButton.styleFrom(visualDensity: VisualDensity.compact),
              icon: const Icon(Icons.stop, size: 18),
              label: const Text('Cancel'),
              onPressed: _cancelMeasurement,
            )
          else
            FilledButton.icon(
              style: FilledButton.styleFrom(visualDensity: VisualDensity.compact),
              icon: const Icon(Icons.mic, size: 18),
              label: Text(_hasProposal ? 'Measure again' : 'Measure'),
              onPressed: _rta == null ? null : _measure,
            ),
          Row(mainAxisSize: MainAxisSize.min, children: [
            Checkbox(
              value: _isolate,
              visualDensity: VisualDensity.compact,
              onChanged: _measuring ? null : (v) => setState(() => _isolate = v ?? true),
            ),
            Text('Mute the other $groupName while measuring', style: const TextStyle(fontSize: 12)),
          ]),
          Row(mainAxisSize: MainAxisSize.min, children: [
            const Text('Max boost', style: TextStyle(fontSize: 12, color: AppColors.muted)),
            const SizedBox(width: 6),
            DropdownButton<double>(
              value: _maxBoost,
              isDense: true,
              dropdownColor: AppColors.surfaceHigh,
              style: const TextStyle(fontFamily: 'Inter', fontSize: 12, color: AppColors.text),
              items: [
                for (final v in const [0.0, 3.0, 6.0, 9.0]) DropdownMenuItem(value: v, child: Text('${v.round()} dB')),
              ],
              onChanged: (v) => setState(() => _maxBoost = v ?? 6),
            ),
          ]),
          if (_hasProposal) ...[
            FilledButton.icon(
              style: FilledButton.styleFrom(
                backgroundColor: AppColors.warning,
                foregroundColor: AppColors.backgroundDeep,
                visualDensity: VisualDensity.compact,
              ),
              icon: const Icon(Icons.check, size: 18),
              label: Text('Apply to $alias'),
              onPressed: _apply,
            ),
            TextButton(onPressed: () => _setProposal(null, null), child: const Text('Discard')),
          ],
        ]),
        const SizedBox(height: 8),
        const Row(children: [
          SizedBox(width: 120, child: Text('Browser noise', style: TextStyle(fontSize: 12, color: AppColors.muted))),
          Expanded(child: ToneGeneratorPanel(showFrequency: false)),
        ]),
        if (widget.dspGenerator) ...[
          const SizedBox(height: 6),
          Row(children: [
            const SizedBox(width: 120, child: Text('DSP generator', style: TextStyle(fontSize: 12, color: AppColors.muted))),
            Expanded(child: DspGeneratorPanel(device: _device)),
          ]),
        ],
        const SizedBox(height: 4),
        Text(
          'Corrects 40 Hz – 16 kHz towards the median level, smoothed, max. 12 dB cut'
          '${widget.target == AutoEqTarget.peq ? ', with the free PEQ bands' : ''}. The microphone response is '
          'not compensated – use a measurement mic. Whether the DSP generator passes the input EQ is not known '
          'yet (see TODO.md).',
          style: const TextStyle(fontSize: 11, color: AppColors.muted),
        ),
      ]),
    );
  }
}

/// Border colour of the Auto-EQ panel.
final Color proposalBorder = AppColors.warning.withAlpha(140);
