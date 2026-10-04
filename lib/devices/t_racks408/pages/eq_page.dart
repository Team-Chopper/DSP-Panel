// EQ page (parametric EQ of every channel):
//   channel selection In 1 … Out 8
//   PEQ graph over the full width
//   band table + Freq/Q/Gain faders of the selected band | HPF and LPF |
//   channel gain

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';
import '../widgets/console_fader.dart';
import '../widgets/fader_strip.dart';
import '../widgets/peq_graph.dart';
import '../widgets/ui_common.dart';
import '../widgets/value_field.dart';

const _proto = ProtocolService();

// Scale marks of the frequency and Q faders (raw protocol values).
const _freqMarkHz = [20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0];
final _freqMarks = [for (final hz in _freqMarkHz) _proto.peqHzToFreq(hz).toDouble()];
final _qMarks = [for (final q in [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0]) _proto.peqQToRaw(q).toDouble()];
const _peqGainMarks = [12.0, 9.0, 6.0, 3.0, 0.0, -3.0, -6.0, -9.0, -12.0];

/// Nominal frequency of a scale mark (20, 50, … 20k).
String _freqLabel(double raw) {
  final i = _freqMarks.indexOf(raw);
  final hz = i >= 0 ? _freqMarkHz[i] : _proto.peqFreqToHz(raw.round());
  return hz >= 1000 ? '${(hz / 1000).round()}k' : '${hz.round()}';
}
String _qText(int raw) {
  final q = _proto.peqRawToQ(raw);
  return q >= 10 ? q.toStringAsFixed(1) : q.toStringAsFixed(2);
}

String _qLabel(double raw) {
  final q = _proto.peqRawToQ(raw.round());
  return q >= 1 ? q.round().toString() : q.toStringAsFixed(1);
}

String _gainLabel(double db) => db % 6 == 0 ? gainMarkLabel(db) : '';

class EqPage extends StatefulWidget {
  final DeviceProvider device;

  /// Channel to show first (from the EQ overview), or null.
  final String? channel;

  const EqPage({super.key, required this.device, this.channel});

  @override
  State<EqPage> createState() => _EqPageState();
}

class _EqPageState extends State<EqPage> {
  late String _channel = widget.channel ?? widget.device.inputs.first;
  int _band = 0;

  static const _bottomHeight = 362.0;
  static const _minGraphHeight = 220.0;
  static const _minWidth = 1060.0;

  @override
  Widget build(BuildContext context) {
    final device = widget.device;
    return ListenableBuilder(
      listenable: device,
      builder: (context, _) {
        if (!device.model.hasChannel(_channel)) _channel = device.inputs.first;
        final bands = device.getPeqBands(_channel);
        if (_band >= bands.length) _band = 0;
        final color = AppColors.forChannel(device.inputs.contains(_channel));

        return LayoutBuilder(builder: (context, constraints) {
          const selectorHeight = 46.0;
          final graphHeight =
              math.max(_minGraphHeight, constraints.maxHeight - selectorHeight - _bottomHeight - 24);
          final tooLow = graphHeight + selectorHeight + _bottomHeight + 24 > constraints.maxHeight;

          final bottom = SizedBox(
            height: _bottomHeight,
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Expanded(
                  child: PanelCard(
                    title: 'PEQ · ${device.getAlias(_channel)}',
                    accent: color,
                    trailing: Text('${bands.length} bands',
                        style: const TextStyle(fontSize: 11, color: AppColors.muted)),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Expanded(
                          child: _BandTable(
                            device: device,
                            channel: _channel,
                            bands: bands,
                            selected: _band,
                            onSelect: (b) => setState(() => _band = b),
                          ),
                        ),
                        const SizedBox(width: 12),
                        _BandFaders(device: device, channel: _channel, band: _band, bandData: bands[_band]),
                      ],
                    ),
                  ),
                ),
                const SizedBox(width: 12),
                SizedBox(
                  width: 250,
                  child: PanelCard(
                    title: 'Filters',
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Expanded(child: _FilterColumn(device: device, channel: _channel, highPass: true)),
                        Container(width: 1, color: AppColors.border, margin: const EdgeInsets.symmetric(horizontal: 6)),
                        Expanded(child: _FilterColumn(device: device, channel: _channel, highPass: false)),
                      ],
                    ),
                  ),
                ),
                const SizedBox(width: 12),
                SizedBox(
                  width: 112,
                  child: PanelCard(
                    title: 'Gain',
                    child: ChannelFader(device: device, channel: _channel, color: color, showName: false),
                  ),
                ),
              ],
            ),
          );

          final page = Column(
            children: [
              SizedBox(
                height: selectorHeight,
                child: ChannelSelector(
                  device: device,
                  selected: _channel,
                  onSelected: (c) => setState(() => _channel = c),
                ),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
                child: SizedBox(
                  height: graphHeight,
                  child: PeqGraph(
                    bands: bands,
                    hiPass: device.getHiPass(_channel),
                    loPass: device.getLoPass(_channel),
                    selectedBand: _band,
                    onSelect: (b) => setState(() => _band = b),
                    onMove: (b, freqRaw, gainDb) => device.setPeqBand(_channel, b, freqRaw: freqRaw, gainDb: gainDb),
                    onQ: (b, qRaw) => device.setPeqBand(_channel, b, qRaw: qRaw),
                  ),
                ),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
                child: constraints.maxWidth - 24 >= _minWidth
                    ? bottom
                    : SingleChildScrollView(
                        scrollDirection: Axis.horizontal,
                        child: SizedBox(width: _minWidth, child: bottom),
                      ),
              ),
            ],
          );
          return tooLow ? SingleChildScrollView(child: page) : page;
        });
      },
    );
  }
}

// ─── Band table ───

class _BandTable extends StatelessWidget {
  final DeviceProvider device;
  final String channel;
  final List<PeqBand> bands;
  final int selected;
  final ValueChanged<int> onSelect;

  const _BandTable({
    required this.device,
    required this.channel,
    required this.bands,
    required this.selected,
    required this.onSelect,
  });

  static const _rowHeight = 28.0;

  Widget _header(String text, {double? width, int? flex}) {
    final t = Text(text, style: const TextStyle(fontSize: 10, color: AppColors.muted, letterSpacing: 0.5));
    return flex != null ? Expanded(flex: flex, child: t) : SizedBox(width: width, child: t);
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: 6, bottom: 4),
          child: Row(children: [
            _header('#', width: 30),
            _header('TYPE', flex: 3),
            _header('FREQ (Hz)', flex: 2),
            _header('GAIN (dB)', flex: 2),
            _header('Q', flex: 2),
            _header('', width: 62),
          ]),
        ),
        for (var i = 0; i < bands.length; i++) _row(i, bands[i]),
      ],
    );
  }

  Widget _row(int i, PeqBand b) {
    final active = i == selected;
    final color = peqBandColors[i % peqBandColors.length];
    final gap = const SizedBox(width: 6);
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () => onSelect(i),
      child: Container(
        height: _rowHeight,
        margin: const EdgeInsets.only(bottom: 2),
        padding: const EdgeInsets.symmetric(horizontal: 6),
        decoration: BoxDecoration(
          color: active ? color.withAlpha(28) : null,
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: active ? color.withAlpha(140) : Colors.transparent),
        ),
        child: Row(children: [
          SizedBox(
            width: 30,
            child: Row(children: [
              Container(width: 8, height: 8, decoration: BoxDecoration(color: b.bypass ? AppColors.muted : color, shape: BoxShape.circle)),
              const SizedBox(width: 5),
              Text('${i + 1}', style: monoValue.copyWith(fontSize: 11, color: AppColors.text)),
            ]),
          ),
          Expanded(
            flex: 3,
            child: DropdownButtonHideUnderline(
              child: DropdownButton<int>(
                value: b.type,
                isDense: true,
                isExpanded: true,
                dropdownColor: AppColors.surfaceHigh,
                style: const TextStyle(fontFamily: 'Inter', fontSize: 11, color: AppColors.text),
                items: [
                  for (var t = 0; t < ProtocolService.peqTypeNames.length; t++)
                    DropdownMenuItem(value: t, child: Text(ProtocolService.peqTypeNames[t])),
                ],
                onChanged: (t) {
                  onSelect(i);
                  if (t != null) device.setPeqBand(channel, i, type: t);
                },
              ),
            ),
          ),
          gap,
          Expanded(
            flex: 2,
            child: ValueField(
              text: formatHz(_proto.peqFreqToHz(b.freqRaw)),
              height: 24,
              onSubmitted: (input) {
                final hz = parseFrequency(input);
                if (hz == null || hz <= 0) return false;
                onSelect(i);
                device.setPeqBand(channel, i, freqRaw: _proto.peqHzToFreq(hz.clamp(20.0, 20000.0)));
                return true;
              },
            ),
          ),
          gap,
          Expanded(
            flex: 2,
            child: ValueField(
              text: formatDbValue(b.gainDb),
              height: 24,
              onSubmitted: (input) {
                final db = parseNumber(input);
                if (db == null) return false;
                onSelect(i);
                device.setPeqBand(channel, i, gainDb: ((db * 10).round() / 10).clamp(-peqGainMax, peqGainMax));
                return true;
              },
            ),
          ),
          gap,
          Expanded(
            flex: 2,
            child: ValueField(
              text: _qText(b.qRaw),
              height: 24,
              onSubmitted: (input) {
                final q = parseNumber(input);
                if (q == null || q <= 0) return false;
                onSelect(i);
                device.setPeqBand(channel, i, qRaw: _proto.peqQToRaw(q));
                return true;
              },
            ),
          ),
          gap,
          SizedBox(
            width: 56,
            child: _SmallToggle(
              label: 'Bypass',
              active: b.bypass,
              color: AppColors.danger,
              onTap: () {
                onSelect(i);
                device.setPeqBand(channel, i, bypass: !b.bypass);
              },
            ),
          ),
        ]),
      ),
    );
  }
}

// ─── Faders of the selected band ───

class _BandFaders extends StatelessWidget {
  final DeviceProvider device;
  final String channel;
  final int band;
  final PeqBand bandData;

  const _BandFaders({required this.device, required this.channel, required this.band, required this.bandData});

  @override
  Widget build(BuildContext context) {
    final color = peqBandColors[band % peqBandColors.length];
    final b = bandData;
    return SizedBox(
      width: 3 * 74,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _FaderColumn(
            title: 'Freq',
            fader: ConsoleFader(
              value: b.freqRaw.toDouble(),
              taper: LinearTaper(0, ProtocolService.freqRawMax.toDouble()),
              marks: _freqMarks,
              markLabel: _freqLabel,
              color: color,
              wheelStep: 1,
              onChanged: (v) => device.setPeqBand(channel, band, freqRaw: v.round()),
            ),
            field: ValueField(
              text: formatHz(_proto.peqFreqToHz(b.freqRaw)),
              onSubmitted: (input) {
                final hz = parseFrequency(input);
                if (hz == null || hz <= 0) return false;
                device.setPeqBand(channel, band, freqRaw: _proto.peqHzToFreq(hz.clamp(20.0, 20000.0)));
                return true;
              },
            ),
          ),
          _FaderColumn(
            title: 'Q',
            fader: ConsoleFader(
              value: b.qRaw.toDouble(),
              taper: LinearTaper(0, ProtocolService.qRawMax.toDouble()),
              marks: _qMarks,
              markLabel: _qLabel,
              color: color,
              wheelStep: 1,
              resetValue: _proto.peqQToRaw(3.0).toDouble(),
              onChanged: (v) => device.setPeqBand(channel, band, qRaw: v.round()),
            ),
            field: ValueField(
              text: _qText(b.qRaw),
              onSubmitted: (input) {
                final q = parseNumber(input);
                if (q == null || q <= 0) return false;
                device.setPeqBand(channel, band, qRaw: _proto.peqQToRaw(q));
                return true;
              },
            ),
          ),
          _FaderColumn(
            title: 'Gain',
            fader: ConsoleFader(
              value: b.gainDb,
              taper: const LinearTaper(-peqGainMax, peqGainMax),
              marks: _peqGainMarks,
              majorMarks: const [0.0],
              markLabel: _gainLabel,
              color: color,
              wheelStep: 0.5,
              resetValue: 0,
              onChanged: (v) => device.setPeqBand(channel, band, gainDb: ((v * 10).round() / 10).clamp(-peqGainMax, peqGainMax)),
            ),
            field: ValueField(
              text: formatDbValue(b.gainDb),
              onSubmitted: (input) {
                final db = parseNumber(input);
                if (db == null) return false;
                device.setPeqBand(channel, band, gainDb: ((db * 10).round() / 10).clamp(-peqGainMax, peqGainMax));
                return true;
              },
            ),
          ),
        ],
      ),
    );
  }
}

class _FaderColumn extends StatelessWidget {
  final String title;
  final Widget fader;
  final Widget field;

  const _FaderColumn({required this.title, required this.fader, required this.field});

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 74,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 3),
        child: Column(children: [
          Text(title, style: interWeight(const TextStyle(fontSize: 11, color: AppColors.muted), FontWeight.w600)),
          const SizedBox(height: 4),
          Expanded(child: fader),
          const SizedBox(height: 6),
          field,
        ]),
      ),
    );
  }
}

// ─── HPF / LPF ───

class _FilterColumn extends StatelessWidget {
  final DeviceProvider device;
  final String channel;
  final bool highPass;

  const _FilterColumn({required this.device, required this.channel, required this.highPass});

  @override
  Widget build(BuildContext context) {
    final f = highPass ? device.getHiPass(channel) : device.getLoPass(channel);
    final slope = device.filterSlope(channel, highPass: highPass);
    final color = highPass ? AppColors.danger : AppColors.info;
    void setFreq(int raw) =>
        highPass ? device.setHiPass(channel, freqRaw: raw) : device.setLoPass(channel, freqRaw: raw);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(children: [
          Text(highPass ? 'HPF' : 'LPF',
              style: interWeight(TextStyle(fontSize: 12, color: f.enabled ? color : AppColors.muted), FontWeight.w700)),
          const Spacer(),
          SizedBox(
            width: 56,
            child: _SmallToggle(
              label: 'Bypass',
              active: !f.enabled,
              color: AppColors.danger,
              onTap: () => highPass
                  ? device.setHiPass(channel, enabled: !f.enabled)
                  : device.setLoPass(channel, enabled: !f.enabled),
            ),
          ),
        ]),
        const SizedBox(height: 6),
        Expanded(
          child: Opacity(
            opacity: f.enabled ? 1 : 0.55,
            child: ConsoleFader(
              value: f.freqRaw.toDouble(),
              taper: LinearTaper(0, ProtocolService.freqRawMax.toDouble()),
              marks: _freqMarks,
              markLabel: _freqLabel,
              color: color,
              wheelStep: 1,
              resetValue: highPass ? 0 : ProtocolService.freqRawMax.toDouble(),
              onChanged: (v) => setFreq(v.round()),
            ),
          ),
        ),
        const SizedBox(height: 6),
        ValueField(
          text: formatHz(_proto.peqFreqToHz(f.freqRaw)),
          tooltip: 'Cut-off frequency in Hz',
          onSubmitted: (input) {
            final hz = parseFrequency(input);
            if (hz == null || hz <= 0) return false;
            setFreq(_proto.peqHzToFreq(hz.clamp(20.0, 20000.0)));
            return true;
          },
        ),
        const SizedBox(height: 6),
        Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          decoration: BoxDecoration(
            color: AppColors.backgroundDeep,
            borderRadius: BorderRadius.circular(6),
            border: Border.all(color: AppColors.border),
          ),
          child: DropdownButtonHideUnderline(
            child: DropdownButton<int>(
              value: slope,
              isExpanded: true,
              isDense: true,
              dropdownColor: AppColors.surfaceHigh,
              style: const TextStyle(fontFamily: 'Inter', fontSize: 11, color: AppColors.text),
              items: [
                for (var s = 1; s < ProtocolService.crossoverSlopeNames.length; s++)
                  DropdownMenuItem(value: s, child: Text(ProtocolService.crossoverSlopeNames[s])),
              ],
              onChanged: (s) {
                if (s != null) device.setFilterSlope(channel, s, highPass: highPass);
              },
            ),
          ),
        ),
      ],
    );
  }
}

class _SmallToggle extends StatelessWidget {
  final String label;
  final bool active;
  final Color color;
  final VoidCallback onTap;

  const _SmallToggle({required this.label, required this.active, required this.color, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return InkWell(
      borderRadius: BorderRadius.circular(6),
      onTap: onTap,
      child: Container(
        height: 24,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: active ? color.withAlpha(50) : AppColors.surfaceHigh,
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: active ? color : AppColors.border),
        ),
        child: Text(label, style: TextStyle(fontSize: 10, color: active ? color : AppColors.muted)),
      ),
    );
  }
}
