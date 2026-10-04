// Gate (inputs), compressor and limiter (outputs), each on its own page:
// channel selection, transfer curve (input level → output level) and the
// parameters as horizontal faders below.
//
// The value ranges are sensible guesses and not yet verified against the
// original editor (see TODO.md).

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';
import '../widgets/console_fader.dart';
import '../widgets/ui_common.dart';
import '../widgets/value_field.dart';

String _formatMs(int ms) => ms >= 1000 ? '${(ms / 1000).toStringAsFixed(2)} s' : '$ms ms';

String _thresholdLabel(double db) => db % 20 == 0 || db == 0 ? '${db.round()}' : '';

List<double> _thresholdMarks(double max) => [for (var db = -90.0; db <= max; db += 10) db];

String _timeLabel(double ms) => ms >= 1000 ? '${(ms / 1000).round()}s' : '${ms.round()}';

List<double> _timeMarks(int max) => [
      for (final ms in [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0])
        if (ms <= max) ms,
    ];

// ─── Pages ───

class GatePage extends StatelessWidget {
  final DeviceProvider device;

  const GatePage({super.key, required this.device});

  @override
  Widget build(BuildContext context) => _DynamicsLayout(
        device: device,
        inputs: true,
        builder: (context, ch, color) {
          final g = device.getGate(ch);
          final defaults = GateState();
          final thresholdDb = ProtocolService.thresholdRawToDb(g.threshold);
          void set(void Function(GateState g) change) => device.setGate(ch, g..let(change));
          return _DynamicsContent(
            title: 'Gate · ${device.getAlias(ch)}',
            color: color,
            curve: (x) => x >= thresholdDb ? x : x - 60,
            thresholdDb: thresholdDb,
            side: [
              _StateBadge(active: thresholdDb > -90, color: color),
              const SizedBox(height: 12),
              const _Hint('Below the threshold the gate closes. -90 dB = always open.'),
            ],
            params: [
              _thresholdParam(g.threshold, 0, defaults.threshold, color, (v) => set((g) => g.threshold = v)),
              _timeParam('Attack', g.attack, 2999, defaults.attack, color, (v) => set((g) => g.attack = v)),
              _timeParam('Hold', g.hold, 2999, defaults.hold, color, (v) => set((g) => g.hold = v)),
              _timeParam('Release', g.release, 2999, defaults.release, color, (v) => set((g) => g.release = v)),
            ],
          );
        },
      );
}

class CompressorPage extends StatelessWidget {
  final DeviceProvider device;

  const CompressorPage({super.key, required this.device});

  /// Ratio (n in 1:n) of a ratio index; "Limit" is infinite.
  static double ratioValue(int index) {
    final name = ProtocolService.compressorRatioNames[index];
    if (name == 'Limit') return double.infinity;
    return double.tryParse(name.split(':').last) ?? 1;
  }

  @override
  Widget build(BuildContext context) => _DynamicsLayout(
        device: device,
        inputs: false,
        builder: (context, ch, color) {
          final c = device.getCompressor(ch);
          final defaults = CompressorState();
          final thresholdDb = ProtocolService.thresholdRawToDb(c.threshold);
          final ratio = ratioValue(c.ratio);
          final knee = c.knee.toDouble();
          void set(void Function(CompressorState c) change) => device.setCompressor(ch, c..let(change));
          double response(double x) {
            if (ratio <= 1) return x;
            final over = x - thresholdDb;
            if (knee > 0 && over.abs() <= knee / 2) {
              // Soft knee: quadratic transition
              return x + (1 / ratio - 1) * math.pow(over + knee / 2, 2) / (2 * knee);
            }
            return over <= 0 ? x : thresholdDb + over / ratio;
          }

          return _DynamicsContent(
            title: 'Compressor · ${device.getAlias(ch)}',
            color: color,
            curve: response,
            thresholdDb: thresholdDb,
            side: [
              _StateBadge(active: c.ratio != 0 && thresholdDb < 20, color: color),
              const SizedBox(height: 12),
              const Text('Ratio', style: TextStyle(fontSize: 11, color: AppColors.muted)),
              const SizedBox(height: 4),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8),
                decoration: BoxDecoration(
                  color: AppColors.backgroundDeep,
                  borderRadius: BorderRadius.circular(6),
                  border: Border.all(color: AppColors.border),
                ),
                child: DropdownButtonHideUnderline(
                  child: DropdownButton<int>(
                    value: c.ratio,
                    isExpanded: true,
                    dropdownColor: AppColors.surfaceHigh,
                    style: monoValue.copyWith(fontSize: 13, color: c.ratio == 0 ? AppColors.muted : color),
                    items: [
                      for (var i = 0; i < ProtocolService.compressorRatioNames.length; i++)
                        DropdownMenuItem(value: i, child: Text(ProtocolService.compressorRatioNames[i])),
                    ],
                    onChanged: (v) {
                      if (v != null) set((c) => c.ratio = v);
                    },
                  ),
                ),
              ),
            ],
            params: [
              _thresholdParam(c.threshold, 20, defaults.threshold, color, (v) => set((c) => c.threshold = v)),
              _timeParam('Attack', c.attack, 5000, defaults.attack, color, (v) => set((c) => c.attack = v)),
              _timeParam('Release', c.release, 5000, defaults.release, color, (v) => set((c) => c.release = v)),
              _Param(
                label: 'Knee',
                fader: ConsoleFader(
                  axis: Axis.horizontal,
                  value: knee,
                  taper: const LinearTaper(0, 12),
                  marks: [for (var k = 0.0; k <= 12; k++) k],
                  markLabel: (k) => k % 3 == 0 ? '${k.round()}' : '',
                  color: color,
                  wheelStep: 1,
                  resetValue: defaults.knee.toDouble(),
                  onChanged: (v) => set((c) => c.knee = v.round().clamp(0, 12)),
                ),
                text: '${c.knee} dB',
                onSubmitted: (input) {
                  final v = parseNumber(input);
                  if (v == null) return false;
                  set((c) => c.knee = v.round().clamp(0, 12));
                  return true;
                },
              ),
            ],
          );
        },
      );
}

class LimiterPage extends StatelessWidget {
  final DeviceProvider device;

  const LimiterPage({super.key, required this.device});

  @override
  Widget build(BuildContext context) => _DynamicsLayout(
        device: device,
        inputs: false,
        builder: (context, ch, color) {
          final l = device.getLimiter(ch);
          final defaults = LimiterState();
          final thresholdDb = ProtocolService.thresholdRawToDb(l.threshold);
          void set(void Function(LimiterState l) change) => device.setLimiter(ch, l..let(change));
          return _DynamicsContent(
            title: 'Limiter · ${device.getAlias(ch)}',
            color: color,
            curve: (x) => math.min(x, thresholdDb),
            thresholdDb: thresholdDb,
            side: [
              _StateBadge(active: thresholdDb < 20, color: color),
              const SizedBox(height: 12),
              const _Hint('+20 dB means the limiter never engages.'),
            ],
            params: [
              _thresholdParam(l.threshold, 20, defaults.threshold, color, (v) => set((l) => l.threshold = v)),
              _timeParam('Attack', l.attack, 5000, defaults.attack, color, (v) => set((l) => l.attack = v)),
              _timeParam('Release', l.release, 5000, defaults.release, color, (v) => set((l) => l.release = v)),
            ],
          );
        },
      );
}

extension _Let<T> on T {
  /// Apply [change] to this object (used to edit a state copy in place).
  void let(void Function(T it) change) => change(this);
}

// ─── Parameters ───

_Param _thresholdParam(int raw, double maxDb, int defaultRaw, Color color, ValueChanged<int> onChanged) {
  final db = ProtocolService.thresholdRawToDb(raw);
  return _Param(
    label: 'Threshold',
    fader: ConsoleFader(
      axis: Axis.horizontal,
      value: db,
      taper: LinearTaper(-90, maxDb),
      marks: _thresholdMarks(maxDb),
      majorMarks: const [0.0],
      markLabel: _thresholdLabel,
      color: color,
      wheelStep: 0.5,
      resetValue: ProtocolService.thresholdRawToDb(defaultRaw),
      onChanged: (v) => onChanged(ProtocolService.thresholdDbToRaw(v.clamp(-90.0, maxDb))),
    ),
    text: '${formatDbValue(db)} dB',
    onSubmitted: (input) {
      final v = parseNumber(input);
      if (v == null) return false;
      onChanged(ProtocolService.thresholdDbToRaw(v.clamp(-90.0, maxDb)));
      return true;
    },
  );
}

_Param _timeParam(String label, int raw, int maxMs, int defaultRaw, Color color, ValueChanged<int> onChanged) {
  final ms = ProtocolService.timeRawToMs(raw);
  return _Param(
    label: label,
    fader: ConsoleFader(
      axis: Axis.horizontal,
      value: ms.toDouble(),
      taper: LogTaper(1, maxMs.toDouble()),
      marks: _timeMarks(maxMs),
      markLabel: _timeLabel,
      color: color,
      resetValue: ProtocolService.timeRawToMs(defaultRaw).toDouble(),
      onChanged: (v) => onChanged(ProtocolService.timeMsToRaw(v.round().clamp(1, maxMs))),
    ),
    text: _formatMs(ms),
    onSubmitted: (input) {
      final v = parseNumber(input);
      if (v == null) return false;
      final inSeconds = RegExp(r'\d\s*s\b').hasMatch(input) && !input.contains('ms');
      onChanged(ProtocolService.timeMsToRaw((inSeconds ? v * 1000 : v).round().clamp(1, maxMs)));
      return true;
    },
  );
}

class _Param {
  final String label;
  final Widget fader;
  final String text;
  final bool Function(String input) onSubmitted;

  _Param({required this.label, required this.fader, required this.text, required this.onSubmitted});
}

// ─── Layout ───

class _DynamicsLayout extends StatefulWidget {
  final DeviceProvider device;

  /// Inputs (gate) or outputs (compressor, limiter).
  final bool inputs;
  final Widget Function(BuildContext context, String channel, Color color) builder;

  const _DynamicsLayout({required this.device, required this.inputs, required this.builder});

  @override
  State<_DynamicsLayout> createState() => _DynamicsLayoutState();
}

class _DynamicsLayoutState extends State<_DynamicsLayout> {
  String? _channel;

  @override
  Widget build(BuildContext context) {
    final device = widget.device;
    return ListenableBuilder(
      listenable: device,
      builder: (context, _) {
        final group = widget.inputs ? device.inputs : device.outputs;
        final channel = (_channel != null && group.contains(_channel)) ? _channel! : group.first;
        return Column(children: [
          ChannelSelector(
            device: device,
            selected: channel,
            showInputs: widget.inputs,
            showOutputs: !widget.inputs,
            onSelected: (c) => setState(() => _channel = c),
          ),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.fromLTRB(12, 4, 12, 12),
              child: widget.builder(context, channel, AppColors.forChannel(widget.inputs)),
            ),
          ),
        ]);
      },
    );
  }
}

class _DynamicsContent extends StatelessWidget {
  final String title;
  final Color color;
  final double Function(double inputDb) curve;
  final double thresholdDb;
  final List<Widget> side;
  final List<_Param> params;

  const _DynamicsContent({
    required this.title,
    required this.color,
    required this.curve,
    required this.thresholdDb,
    required this.side,
    required this.params,
  });

  static const _paramHeight = 58.0;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final paramsHeight = params.length * _paramHeight + 50;
      final curveHeight = math.max(240.0, constraints.maxHeight - paramsHeight - 12);
      final tooLow = curveHeight + paramsHeight + 12 > constraints.maxHeight;
      final content = Column(children: [
        SizedBox(
          height: curveHeight,
          child: PanelCard(
            title: title,
            accent: color,
            child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
              const Spacer(),
              AspectRatio(aspectRatio: 1, child: TransferCurve(color: color, response: curve, thresholdDb: thresholdDb)),
              const SizedBox(width: 24),
              SizedBox(
                width: 220,
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: side),
              ),
              const Spacer(),
            ]),
          ),
        ),
        const SizedBox(height: 12),
        SizedBox(
          height: paramsHeight,
          child: PanelCard(
            child: Column(children: [
              for (final p in params)
                SizedBox(
                  height: _paramHeight,
                  child: Row(children: [
                    SizedBox(
                      width: 90,
                      child: Text(p.label,
                          style: interWeight(const TextStyle(fontSize: 12, color: AppColors.muted), FontWeight.w600)),
                    ),
                    Expanded(child: SizedBox(height: 48, child: p.fader)),
                    const SizedBox(width: 16),
                    ValueField(text: p.text, width: 100, height: 30, fontSize: 12, onSubmitted: p.onSubmitted),
                  ]),
                ),
            ]),
          ),
        ),
      ]);
      return tooLow ? SingleChildScrollView(child: content) : content;
    });
  }
}

class _StateBadge extends StatelessWidget {
  final bool active;
  final Color color;

  const _StateBadge({required this.active, required this.color});

  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
        decoration: BoxDecoration(
          color: active ? color.withAlpha(40) : AppColors.surfaceHigh,
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: active ? color : AppColors.border),
        ),
        child: Text(active ? 'Active' : 'Off', style: TextStyle(fontSize: 12, color: active ? color : AppColors.muted)),
      );
}

class _Hint extends StatelessWidget {
  final String text;

  const _Hint(this.text);

  @override
  Widget build(BuildContext context) => Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        const Icon(Icons.info_outline, size: 13, color: AppColors.muted),
        const SizedBox(width: 6),
        Expanded(child: Text(text, style: const TextStyle(fontSize: 11, color: AppColors.muted))),
      ]);
}

/// Transfer curve: input level (x) against output level (y), -90…+20 dB.
class TransferCurve extends StatelessWidget {
  final Color color;
  final double Function(double inputDb) response;
  final double thresholdDb;

  const TransferCurve({super.key, required this.color, required this.response, required this.thresholdDb});

  @override
  Widget build(BuildContext context) => Container(
        decoration: BoxDecoration(
          color: AppColors.backgroundDeep,
          borderRadius: BorderRadius.circular(10),
          border: Border.all(color: AppColors.border),
        ),
        padding: const EdgeInsets.all(8),
        child: CustomPaint(painter: _TransferPainter(color, response, thresholdDb)),
      );
}

class _TransferPainter extends CustomPainter {
  static const _min = -90.0;
  static const _max = 20.0;

  final Color color;
  final double Function(double) response;
  final double thresholdDb;

  _TransferPainter(this.color, this.response, this.thresholdDb);

  @override
  void paint(Canvas canvas, Size size) {
    double x(double db) => (db - _min) / (_max - _min) * size.width;
    double y(double db) => size.height - (db.clamp(_min, _max) - _min) / (_max - _min) * size.height;

    final grid = Paint()
      ..color = AppColors.border
      ..strokeWidth = 1;
    final label = TextPainter(textDirection: TextDirection.ltr);
    for (var db = -80.0; db <= 20; db += 20) {
      canvas.drawLine(Offset(x(db), 0), Offset(x(db), size.height), grid);
      canvas.drawLine(Offset(0, y(db)), Offset(size.width, y(db)), grid);
      label
        ..text = TextSpan(text: '${db.round()}', style: monoValue.copyWith(fontSize: 8, color: AppColors.muted))
        ..layout();
      label.paint(canvas, Offset(x(db) + 2, size.height - label.height - 1));
    }
    // Unity line (no processing)
    canvas.drawLine(Offset(x(_min), y(_min)), Offset(x(_max), y(_max)),
        Paint()
          ..color = AppColors.muted.withAlpha(110)
          ..strokeWidth = 1);
    // Threshold marker
    final tx = x(thresholdDb.clamp(_min, _max));
    canvas.drawLine(Offset(tx, 0), Offset(tx, size.height),
        Paint()
          ..color = color.withAlpha(90)
          ..strokeWidth = 1);

    final path = Path();
    const steps = 220;
    for (var s = 0; s <= steps; s++) {
      final inDb = _min + (_max - _min) * s / steps;
      final px = x(inDb), py = y(response(inDb));
      s == 0 ? path.moveTo(px, py) : path.lineTo(px, py);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2.5
        ..strokeJoin = StrokeJoin.round,
    );

    label
      ..text = const TextSpan(text: 'in → out', style: TextStyle(fontFamily: 'Inter', fontSize: 9, color: AppColors.muted))
      ..layout();
    label.paint(canvas, const Offset(4, 2));
  }

  @override
  bool shouldRepaint(covariant _TransferPainter oldDelegate) => true;
}
