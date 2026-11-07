// EQ overview of the gain page: the total response of every channel of one
// group (PEQ, filters and, for inputs, the GEQ). The legend has a checkbox
// per channel to hide its curve; tapping a curve opens the EQ page for it.

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../providers/device_provider.dart';
import 'eq_math.dart';
import 'ui_common.dart';

const _minHz = 20.0;
const _maxHz = 20000.0;
const _rangeDb = 15.0;

double _xToHz(double x, double width) => _minHz * math.pow(_maxHz / _minHz, (x / width).clamp(0.0, 1.0));
double _hzToX(double hz, double width) => math.log(hz / _minHz) / math.log(_maxHz / _minHz) * width;
double _dbToY(double db, double height) => height / 2 - db.clamp(-_rangeDb, _rangeDb) / _rangeDb * height / 2;

/// Colour of the curve of the [index]-th channel of a group.
Color curveColor(int index) => AppColors.curves[index % AppColors.curves.length];

class EqGroupGraph extends StatelessWidget {
  final DeviceProvider device;
  final String title;
  final Color accent;
  final List<String> channels;

  /// Channels whose curve is hidden.
  final Set<String> hidden;
  final ValueChanged<String> onToggle;
  final ValueChanged<String> onOpenEq;

  const EqGroupGraph({
    super.key,
    required this.device,
    required this.title,
    required this.accent,
    required this.channels,
    required this.hidden,
    required this.onToggle,
    required this.onOpenEq,
  });

  @override
  Widget build(BuildContext context) {
    final visible = [
      for (var i = 0; i < channels.length; i++)
        if (!hidden.contains(channels[i])) i,
    ];
    // Settings read once per build, not for every point of every curve.
    final eqs = {for (final i in visible) i: ChannelEq.of(device, channels[i])};
    return PanelCard(
      title: '$title · EQ',
      accent: accent,
      child: Column(
        children: [
          Expanded(
            child: LayoutBuilder(builder: (context, constraints) {
              final size = Size(constraints.maxWidth, constraints.maxHeight);
              return GestureDetector(
                onTapUp: (d) {
                  // Open the visible channel whose curve is closest to the tap.
                  final hz = _xToHz(d.localPosition.dx, size.width);
                  String? best;
                  var bestDistance = double.infinity;
                  for (final i in visible) {
                    final y = _dbToY(eqs[i]!.db(hz), size.height);
                    final distance = (y - d.localPosition.dy).abs();
                    if (distance < bestDistance) {
                      bestDistance = distance;
                      best = channels[i];
                    }
                  }
                  if (best != null) onOpenEq(best);
                },
                child: CustomPaint(
                  size: size,
                  painter: _OverviewPainter(
                    curves: [
                      for (final i in visible) (curveColor(i), eqs[i]!.db),
                    ],
                  ),
                ),
              );
            }),
          ),
          const SizedBox(height: 6),
          Wrap(
            spacing: 2,
            runSpacing: 0,
            children: [
              for (var i = 0; i < channels.length; i++)
                _LegendEntry(
                  label: device.getAlias(channels[i]),
                  color: curveColor(i),
                  shown: !hidden.contains(channels[i]),
                  onToggle: () => onToggle(channels[i]),
                ),
            ],
          ),
        ],
      ),
    );
  }
}

class _LegendEntry extends StatelessWidget {
  final String label;
  final Color color;
  final bool shown;
  final VoidCallback onToggle;

  const _LegendEntry({required this.label, required this.color, required this.shown, required this.onToggle});

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: shown ? 'Hide curve' : 'Show curve',
      waitDuration: const Duration(milliseconds: 700),
      child: InkWell(
        borderRadius: BorderRadius.circular(6),
        onTap: onToggle,
        child: Padding(
          padding: const EdgeInsets.only(right: 6),
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            SizedBox(
              width: 26,
              height: 26,
              child: Checkbox(
                value: shown,
                onChanged: (_) => onToggle(),
                activeColor: color,
                checkColor: AppColors.background,
                side: BorderSide(color: color.withAlpha(180), width: 1.5),
                visualDensity: VisualDensity.compact,
                materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
              ),
            ),
            Text(label, style: TextStyle(fontSize: 11, color: shown ? AppColors.text : AppColors.muted)),
          ]),
        ),
      ),
    );
  }
}

class _OverviewPainter extends CustomPainter {
  final List<(Color, double Function(double hz))> curves;

  _OverviewPainter({required this.curves});

  @override
  void paint(Canvas canvas, Size size) {
    final grid = Paint()
      ..color = AppColors.border
      ..strokeWidth = 1;
    // Decade lines and dB lines
    for (final hz in [100.0, 1000.0, 10000.0]) {
      final x = _hzToX(hz, size.width);
      canvas.drawLine(Offset(x, 0), Offset(x, size.height), grid);
    }
    for (final db in [-12.0, -6.0, 6.0, 12.0]) {
      final y = _dbToY(db, size.height);
      canvas.drawLine(Offset(0, y), Offset(size.width, y), grid..color = AppColors.border.withAlpha(120));
    }
    canvas.drawLine(Offset(0, size.height / 2), Offset(size.width, size.height / 2),
        Paint()
          ..color = AppColors.muted.withAlpha(140)
          ..strokeWidth = 1);

    final labels = TextPainter(textDirection: TextDirection.ltr);
    for (final (hz, text) in [(100.0, '100'), (1000.0, '1k'), (10000.0, '10k')]) {
      labels
        ..text = TextSpan(text: text, style: const TextStyle(fontFamily: 'Inter', fontSize: 9, color: AppColors.muted))
        ..layout();
      labels.paint(canvas, Offset(_hzToX(hz, size.width) + 3, size.height - labels.height - 1));
    }

    const steps = 160;
    for (final (color, response) in curves) {
      final path = Path();
      for (var s = 0; s <= steps; s++) {
        final x = s / steps * size.width;
        final y = _dbToY(response(_xToHz(x, size.width)), size.height);
        s == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
      }
      canvas.drawPath(
        path,
        Paint()
          ..color = color
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.8
          ..strokeJoin = StrokeJoin.round,
      );
    }
  }

  @override
  bool shouldRepaint(covariant _OverviewPainter oldDelegate) => true;
}
