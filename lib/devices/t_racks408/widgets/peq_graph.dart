// Parametric EQ graph of one channel: band curves, HPF/LPF and the total
// response. Drag a band point to change frequency and gain, scroll over it
// to change Q, tap to select it. Bypassed bands are shown as a dashed
// preview of what they would do; an Auto-EQ proposal is drawn in orange.

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';
import 'eq_math.dart';

const _proto = ProtocolService();

/// One colour per PEQ band (9 bands on outputs).
const peqBandColors = [
  Color(0xFFF87171), Color(0xFF4ADE80), Color(0xFF38BDF8), Color(0xFFFBBF24), Color(0xFFA78BFA),
  Color(0xFFFDE68A), Color(0xFFF472B6), Color(0xFF2DD4BF), Color(0xFFFB923C),
];

const _minHz = 20.0;
const _maxHz = 20000.0;
const _rangeDb = 15.0;
const peqGainMax = 12.0;

/// Colour of Auto-EQ proposals.
const proposalColor = Color(0xFFFBBF24);

class PeqGraph extends StatefulWidget {
  final List<PeqBand> bands;
  final FilterState hiPass;
  final FilterState loPass;
  final int selectedBand;
  final ValueChanged<int> onSelect;
  final void Function(int band, int freqRaw, double gainDb) onMove;
  final void Function(int band, int qRaw) onQ;

  /// Auto-EQ proposal: the bands as they would be after applying it.
  final List<PeqBand>? proposal;

  /// Slopes a bypassed HPF/LPF gets when switched on (for its preview).
  final int hiPassPreviewSlope;
  final int loPassPreviewSlope;

  const PeqGraph({
    super.key,
    required this.bands,
    required this.hiPass,
    required this.loPass,
    required this.selectedBand,
    required this.onSelect,
    required this.onMove,
    required this.onQ,
    this.proposal,
    this.hiPassPreviewSlope = ProtocolService.defaultSlope,
    this.loPassPreviewSlope = ProtocolService.defaultSlope,
  });

  @override
  State<PeqGraph> createState() => _PeqGraphState();
}

class _PeqGraphState extends State<PeqGraph> {
  /// Band being dragged.
  int _drag = 0;

  @override
  Widget build(BuildContext context) {
    final bands = widget.bands;
    return LayoutBuilder(builder: (context, constraints) {
      final g = _Geometry(Size(constraints.maxWidth, constraints.maxHeight));

      int nearestBand(Offset p) {
        var best = 0;
        var bestDistance = double.infinity;
        for (var i = 0; i < bands.length; i++) {
          final c = g.bandPoint(bands[i]);
          final d = (c - p).distance;
          if (d < bestDistance) {
            bestDistance = d;
            best = i;
          }
        }
        return best;
      }

      void move(Offset p) {
        final gain = (g.yToDb(p.dy) * 10).round() / 10;
        widget.onMove(_drag, _proto.peqHzToFreq(g.xToHz(p.dx)), gain.clamp(-peqGainMax, peqGainMax));
      }

      return Listener(
        onPointerSignal: (e) {
          if (e is! PointerScrollEvent) return;
          GestureBinding.instance.pointerSignalResolver.register(e, (e) {
            final band = nearestBand(e.localPosition);
            final step = (e as PointerScrollEvent).scrollDelta.dy < 0 ? 1 : -1;
            widget.onSelect(band);
            widget.onQ(band, (bands[band].qRaw + step).clamp(0, ProtocolService.maxQRawForType(bands[band].type)));
          });
        },
        child: GestureDetector(
          onTapDown: (d) => widget.onSelect(nearestBand(d.localPosition)),
          onPanStart: (d) {
            _drag = nearestBand(d.localPosition);
            widget.onSelect(_drag);
          },
          onPanUpdate: (d) => move(d.localPosition),
          // Own layer: other parts of the page (e.g. the gain meter) must not
          // make this graph repaint.
          child: RepaintBoundary(
            child: CustomPaint(
              size: g.size,
              painter: _PeqPainter(g, bands, widget.hiPass, widget.loPass, widget.selectedBand, widget.proposal,
                  widget.hiPassPreviewSlope, widget.loPassPreviewSlope),
            ),
          ),
        ),
      );
    });
  }
}

class _Geometry {
  final Size size;

  _Geometry(this.size);

  final padLeft = 34.0;
  final padRight = 10.0;
  final padTop = 10.0;
  final padBottom = 22.0;

  double get w => size.width - padLeft - padRight;
  double get h => size.height - padTop - padBottom;

  double hzToX(double hz) => padLeft + math.log(hz / _minHz) / math.log(_maxHz / _minHz) * w;
  double xToHz(double x) => _minHz * math.pow(_maxHz / _minHz, ((x - padLeft) / w).clamp(0.0, 1.0));
  double dbToY(double db) => padTop + h / 2 - db.clamp(-_rangeDb, _rangeDb) / _rangeDb * h / 2;
  double yToDb(double y) => (padTop + h / 2 - y) / (h / 2) * _rangeDb;

  Offset bandPoint(PeqBand b) => Offset(hzToX(_proto.peqFreqToHz(b.freqRaw)), dbToY(b.gainDb));
}

class _PeqPainter extends CustomPainter {
  final _Geometry g;
  final List<PeqBand> bands;
  final FilterState hiPass;
  final FilterState loPass;
  final int selected;
  final List<PeqBand>? proposal;
  final int hiPassPreviewSlope;
  final int loPassPreviewSlope;

  _PeqPainter(this.g, this.bands, this.hiPass, this.loPass, this.selected, this.proposal, this.hiPassPreviewSlope,
      this.loPassPreviewSlope);

  static const _hpColor = AppColors.danger;
  static const _lpColor = AppColors.info;

  static const _steps = 240;

  /// Response of a band at every curve point, as if it were active.
  List<double> _bandCurve(PeqBand b, List<double> hz) {
    final fc = _proto.peqFreqToHz(b.freqRaw), q = _proto.peqRawToQ(b.qRaw);
    return [for (final f in hz) peqResponseDb(f, fc, q, b.gainDb, b.type)];
  }

  List<double>? _filterCurve(FilterState f, bool high, List<double> hz) {
    if (!f.enabled) return null;
    return _filterCurveWith(f.freqRaw, f.slope, high, hz);
  }

  List<double> _filterCurveWith(int freqRaw, int slope, bool high, List<double> hz) {
    final fc = _proto.peqFreqToHz(freqRaw);
    return [for (final x in hz) filterResponseDb(x, fc, slope, high)];
  }

  static List<double> _plus(List<double> a, List<double> b) => [for (var i = 0; i < a.length; i++) a[i] + b[i]];

  Path _path(List<double> db) {
    final path = Path();
    for (var s = 0; s <= _steps; s++) {
      final x = g.padLeft + g.w * s / _steps;
      final y = g.dbToY(db[s]);
      s == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
    }
    return path;
  }

  /// Dashed version of [path].
  static Path _dashed(Path path, {double dash = 5, double gap = 4}) {
    final out = Path();
    for (final ui.PathMetric metric in path.computeMetrics()) {
      for (var d = 0.0; d < metric.length; d += dash + gap) {
        out.addPath(metric.extractPath(d, math.min(d + dash, metric.length)), Offset.zero);
      }
    }
    return out;
  }

  /// Total of the active bands plus the filters.
  List<double> _total(List<PeqBand> bands, List<List<double>> curves, List<double>? hp, List<double>? lp) {
    final total = List<double>.filled(_steps + 1, 0);
    for (var i = 0; i < bands.length; i++) {
      if (!peqBandAudible(bands[i])) continue;
      final c = curves[i];
      for (var s = 0; s <= _steps; s++) {
        total[s] += c[s];
      }
    }
    for (final f in [hp, lp]) {
      if (f == null) continue;
      for (var s = 0; s <= _steps; s++) {
        total[s] += f[s];
      }
    }
    return total;
  }

  @override
  void paint(Canvas canvas, Size size) {
    final area = Rect.fromLTWH(g.padLeft, g.padTop, g.w, g.h);
    canvas.drawRRect(RRect.fromRectAndRadius(area, const Radius.circular(6)), Paint()..color = AppColors.backgroundDeep);

    final label = TextPainter(textDirection: TextDirection.ltr);
    void text(String s, Offset at, {bool right = false, bool center = false}) {
      label
        ..text = TextSpan(text: s, style: monoValue.copyWith(fontSize: 9, color: AppColors.muted))
        ..layout();
      final dx = right ? -label.width : center ? -label.width / 2 : 0.0;
      label.paint(canvas, at + Offset(dx, -label.height / 2));
    }

    final grid = Paint()
      ..color = AppColors.border
      ..strokeWidth = 1;
    for (var db = -12.0; db <= 12; db += 3) {
      final y = g.dbToY(db);
      canvas.drawLine(Offset(area.left, y), Offset(area.right, y),
          db == 0 ? (Paint()..color = AppColors.muted.withAlpha(160)) : grid);
      if (db % 6 == 0) text(db > 0 ? '+${db.round()}' : '${db.round()}', Offset(area.left - 4, y), right: true);
    }
    for (final decade in [20.0, 100.0, 1000.0, 10000.0]) {
      for (var m = 1; m < 10; m++) {
        final hz = decade * m;
        if (hz < _minHz || hz > _maxHz || (decade == 20 && m > 4)) continue;
        final x = g.hzToX(hz);
        final major = m == 1;
        canvas.drawLine(Offset(x, area.top), Offset(x, area.bottom),
            Paint()
              ..color = major ? AppColors.muted.withAlpha(110) : AppColors.border.withAlpha(150)
              ..strokeWidth = 1);
      }
    }
    for (final hz in [20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0]) {
      text(hz >= 1000 ? '${(hz / 1000).round()}k' : '${hz.round()}', Offset(g.hzToX(hz), area.bottom + 11), center: true);
    }

    // Every curve is computed once at the same frequencies.
    final hz = [for (var s = 0; s <= _steps; s++) g.xToHz(g.padLeft + g.w * s / _steps)];
    final curves = [for (final b in bands) _bandCurve(b, hz)];
    final hp = _filterCurve(hiPass, true, hz);
    final lp = _filterCurve(loPass, false, hz);
    final total = _total(bands, curves, hp, lp);
    Paint preview(Color color, bool emphasised) => Paint()
      ..color = color.withAlpha(emphasised ? 210 : 130)
      ..style = PaintingStyle.stroke
      ..strokeWidth = emphasised ? 1.8 : 1.3;

    canvas.save();
    canvas.clipRect(area);
    for (var i = 0; i < bands.length; i++) {
      final b = bands[i];
      final color = peqBandColors[i % peqBandColors.length];
      // Peak/shelf at 0 dB and all-pass bands draw no curve.
      final flat = (b.type <= 2 && b.gainDb.abs() < 0.05) || b.type >= 7;
      if (flat) continue;
      if (b.bypass) {
        // Preview of a bypassed band: the total response as it would be with
        // this band switched on (active bands and filters included).
        canvas.drawPath(_dashed(_path(_plus(total, curves[i]))), preview(color, i == selected));
        continue;
      }
      final path = _path(curves[i]);
      if (i == selected) {
        final fill = Path.from(path)
          ..lineTo(area.right, g.dbToY(0))
          ..lineTo(area.left, g.dbToY(0))
          ..close();
        canvas.drawPath(fill, Paint()..color = color.withAlpha(36));
      }
      canvas.drawPath(
          path,
          Paint()
            ..color = color.withAlpha(i == selected ? 220 : 110)
            ..style = PaintingStyle.stroke
            ..strokeWidth = i == selected ? 2 : 1.2);
    }
    // HPF/LPF: active ones in their colour, bypassed ones as a dashed preview
    // of the total with the filter switched on.
    for (final (f, high, previewSlope, color) in [
      (hiPass, true, hiPassPreviewSlope, _hpColor),
      (loPass, false, loPassPreviewSlope, _lpColor),
    ]) {
      final curve = _filterCurveWith(f.freqRaw, f.enabled ? f.slope : previewSlope, high, hz);
      if (f.enabled) {
        canvas.drawPath(
            _path(curve),
            Paint()
              ..color = color.withAlpha(170)
              ..style = PaintingStyle.stroke
              ..strokeWidth = 1.5);
      } else if (curve.any((db) => db < -0.5)) {
        canvas.drawPath(_dashed(_path(_plus(total, curve))), preview(color, false));
      }
    }
    final proposed = proposal;
    if (proposed != null) {
      canvas.drawPath(
        _path(_total(proposed, [for (final b in proposed) _bandCurve(b, hz)], hp, lp)),
        Paint()
          ..color = proposalColor
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2.2
          ..strokeJoin = StrokeJoin.round,
      );
    }
    canvas.drawPath(
      _path(total),
      Paint()
        ..color = AppColors.text
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2.4
        ..strokeJoin = StrokeJoin.round,
    );
    canvas.restore();

    // Band points, selected one on top
    final order = [for (var i = 0; i < bands.length; i++) if (i != selected) i, selected];
    for (final i in order) {
      final b = bands[i];
      final c = g.bandPoint(b);
      final isSelected = i == selected;
      final color = peqBandColors[i % peqBandColors.length];
      final r = isSelected ? 11.0 : 8.0;
      if (isSelected) canvas.drawCircle(c, r + 4, Paint()..color = color.withAlpha(60));
      if (b.bypass) {
        // Hollow point: band is bypassed
        canvas.drawCircle(c, r, Paint()..color = AppColors.backgroundDeep);
        canvas.drawCircle(
            c,
            r - 1,
            Paint()
              ..color = color
              ..style = PaintingStyle.stroke
              ..strokeWidth = 2);
      } else {
        canvas.drawCircle(c, r, Paint()..color = color);
      }
      label
        ..text = TextSpan(
            text: '${i + 1}',
            style: interWeight(
                TextStyle(fontSize: isSelected ? 11 : 9, color: b.bypass ? color : AppColors.background),
                FontWeight.w700))
        ..layout();
      label.paint(canvas, c - Offset(label.width / 2, label.height / 2));
    }
  }

  @override
  bool shouldRepaint(covariant _PeqPainter oldDelegate) => true;
}
