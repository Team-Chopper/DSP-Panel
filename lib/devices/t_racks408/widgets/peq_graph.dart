// Parametric EQ graph of one channel: band curves, HPF/LPF and the total
// response. Drag a band point to change frequency and gain, scroll over it
// to change Q, tap to select it.

import 'dart:math' as math;

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

class PeqGraph extends StatefulWidget {
  final List<PeqBand> bands;
  final FilterState hiPass;
  final FilterState loPass;
  final int selectedBand;
  final ValueChanged<int> onSelect;
  final void Function(int band, int freqRaw, double gainDb) onMove;
  final void Function(int band, int qRaw) onQ;

  const PeqGraph({
    super.key,
    required this.bands,
    required this.hiPass,
    required this.loPass,
    required this.selectedBand,
    required this.onSelect,
    required this.onMove,
    required this.onQ,
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
            widget.onQ(band, (bands[band].qRaw + step).clamp(0, ProtocolService.qRawMax));
          });
        },
        child: GestureDetector(
          onTapDown: (d) => widget.onSelect(nearestBand(d.localPosition)),
          onPanStart: (d) {
            _drag = nearestBand(d.localPosition);
            widget.onSelect(_drag);
          },
          onPanUpdate: (d) => move(d.localPosition),
          child: CustomPaint(
            size: g.size,
            painter: _PeqPainter(g, bands, widget.hiPass, widget.loPass, widget.selectedBand),
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

  _PeqPainter(this.g, this.bands, this.hiPass, this.loPass, this.selected);

  double _bandDb(PeqBand b, double hz) =>
      b.bypass ? 0 : peqResponseDb(hz, _proto.peqFreqToHz(b.freqRaw), _proto.peqRawToQ(b.qRaw), b.gainDb, b.type);

  double _filterDb(FilterState f, bool high, double hz) => f.enabled
      ? filterResponseDb(hz, _proto.peqFreqToHz(f.freqRaw), ProtocolService.slopeDbPerOctave(f.slope).toDouble(), high)
      : 0;

  Path _curve(double Function(double hz) response) {
    final path = Path();
    const steps = 240;
    for (var s = 0; s <= steps; s++) {
      final x = g.padLeft + g.w * s / steps;
      final y = g.dbToY(response(g.xToHz(x)));
      s == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
    }
    return path;
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

    canvas.save();
    canvas.clipRect(area);
    for (var i = 0; i < bands.length; i++) {
      final b = bands[i];
      if (b.bypass || (b.gainDb.abs() < 0.05 && b.type == 0)) continue;
      final color = peqBandColors[i % peqBandColors.length];
      final path = _curve((hz) => _bandDb(b, hz));
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
    for (final (f, high) in [(hiPass, true), (loPass, false)]) {
      if (!f.enabled) continue;
      canvas.drawPath(
          _curve((hz) => _filterDb(f, high, hz)),
          Paint()
            ..color = AppColors.muted
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1.4);
    }
    canvas.drawPath(
      _curve((hz) =>
          bands.fold<double>(0, (sum, b) => sum + _bandDb(b, hz)) + _filterDb(hiPass, true, hz) + _filterDb(loPass, false, hz)),
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
      final color = b.bypass ? AppColors.muted : peqBandColors[i % peqBandColors.length];
      final r = isSelected ? 11.0 : 8.0;
      if (isSelected) canvas.drawCircle(c, r + 4, Paint()..color = color.withAlpha(60));
      canvas.drawCircle(c, r, Paint()..color = color);
      label
        ..text = TextSpan(
            text: '${i + 1}',
            style: interWeight(TextStyle(fontSize: isSelected ? 11 : 9, color: AppColors.background), FontWeight.w700))
        ..layout();
      label.paint(canvas, c - Offset(label.width / 2, label.height / 2));
    }
  }

  @override
  bool shouldRepaint(covariant _PeqPainter oldDelegate) => true;
}
