// Mixing-console style fader: a slot with scale lines across it (labels
// beside), a ridged fader cap and an optional level meter. Used vertically
// for gain, EQ, filter, GEQ and delay, horizontally for the dynamics.
//
// Drag the cap (or anywhere on the track) to move it, scroll to step,
// double-tap to reset. A clicked fader takes the keyboard focus: the arrow
// keys step it (Shift: 10 steps), Page Up/Down move 10 % of the travel.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';

/// Mapping between a value and the fader position (0 = bottom/left,
/// 1 = top/right).
abstract class FaderTaper {
  const FaderTaper();

  double toPosition(double value);
  double fromPosition(double position);
}

class LinearTaper extends FaderTaper {
  final double min;
  final double max;

  const LinearTaper(this.min, this.max);

  @override
  double toPosition(double value) => ((value - min) / (max - min)).clamp(0.0, 1.0);

  @override
  double fromPosition(double position) => min + position.clamp(0.0, 1.0) * (max - min);
}

/// Logarithmic taper for values > 0 (times, frequencies).
class LogTaper extends FaderTaper {
  final double min;
  final double max;

  const LogTaper(this.min, this.max);

  @override
  double toPosition(double value) =>
      (math.log(value.clamp(min, max) / min) / math.log(max / min)).clamp(0.0, 1.0);

  @override
  double fromPosition(double position) => min * math.pow(max / min, position.clamp(0.0, 1.0));
}

/// Piecewise linear taper through (value, position) points, ascending.
class PiecewiseTaper extends FaderTaper {
  final List<(double, double)> points;

  const PiecewiseTaper(this.points);

  @override
  double toPosition(double value) {
    if (value <= points.first.$1) return points.first.$2;
    for (var i = 1; i < points.length; i++) {
      final (v1, p1) = points[i];
      if (value <= v1) {
        final (v0, p0) = points[i - 1];
        return p0 + (value - v0) / (v1 - v0) * (p1 - p0);
      }
    }
    return points.last.$2;
  }

  @override
  double fromPosition(double position) {
    if (position <= points.first.$2) return points.first.$1;
    for (var i = 1; i < points.length; i++) {
      final (v1, p1) = points[i];
      if (position <= p1) {
        final (v0, p0) = points[i - 1];
        return v0 + (position - p0) / (p1 - p0) * (v1 - v0);
      }
    }
    return points.last.$1;
  }
}

/// Console taper for channel gain (-60 … +12 dB): most of the travel is
/// spent around 0 dB.
const gainTaper = PiecewiseTaper([
  (-60, 0), (-40, 0.14), (-30, 0.25), (-20, 0.39), (-12, 0.53), (-6, 0.65), (0, 0.78), (6, 0.89), (12, 1),
]);

const gainMarks = [12.0, 6.0, 0.0, -6.0, -12.0, -20.0, -30.0, -40.0, -60.0];

String gainMarkLabel(double db) => db > 0 ? '+${db.round()}' : '${db.round()}';

class ConsoleFader extends StatefulWidget {
  final double value;
  final FaderTaper taper;
  final ValueChanged<double> onChanged;

  /// Value set by a double-tap, or null.
  final double? resetValue;

  /// Values with a scale line; [majorMarks] are drawn brighter.
  final List<double> marks;
  final List<double> majorMarks;

  /// Scale label of a mark, or null for no labels.
  final String Function(double value)? markLabel;

  final Color color;
  final Axis axis;

  /// Level meter beside the slot (0..1), vertical faders only.
  final double? meterLevel;

  /// Peak-hold position of the meter (0..1).
  final double? meterPeak;

  /// Clip indicator on top of the meter (null: none).
  final bool? clip;

  /// Limiter indicator on top of the meter (null: none).
  final bool? limit;

  /// Scroll wheel and arrow key step in value units (default: 1 % of the
  /// travel).
  final double? wheelStep;

  /// Cap size along the travel.
  final double capLength;

  const ConsoleFader({
    super.key,
    required this.value,
    required this.taper,
    required this.onChanged,
    this.resetValue,
    this.marks = const [],
    this.majorMarks = const [],
    this.markLabel,
    this.color = AppColors.accent,
    this.axis = Axis.vertical,
    this.meterLevel,
    this.meterPeak,
    this.clip,
    this.limit,
    this.wheelStep,
    this.capLength = 30,
  });

  @override
  State<ConsoleFader> createState() => _ConsoleFaderState();
}

class _ConsoleFaderState extends State<ConsoleFader> {
  /// Pointer offset from the cap centre while dragging.
  double _grab = 0;
  bool _dragging = false;
  final FocusNode _focus = FocusNode(debugLabel: 'fader');
  bool _focused = false;

  bool get _vertical => widget.axis == Axis.vertical;

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  _FaderGeometry _geometry(Size size) => _FaderGeometry(
        size: size,
        vertical: _vertical,
        capLength: widget.capLength,
        hasLabels: widget.markLabel != null,
        hasMeter: widget.meterLevel != null,
        hasIndicators: widget.clip != null || widget.limit != null,
      );

  /// One step up (+1) or down (-1), [count] times.
  void _step(int direction, {int count = 1}) {
    final step = widget.wheelStep;
    if (step != null) {
      widget.onChanged(widget.value + direction * step * count);
    } else {
      _emit(widget.taper.toPosition(widget.value) + direction * 0.01 * count);
    }
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) return KeyEventResult.ignored;
    final count = HardwareKeyboard.instance.isShiftPressed ? 10 : 1;
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.arrowUp || key == LogicalKeyboardKey.arrowRight) {
      _step(1, count: count);
    } else if (key == LogicalKeyboardKey.arrowDown || key == LogicalKeyboardKey.arrowLeft) {
      _step(-1, count: count);
    } else if (key == LogicalKeyboardKey.pageUp) {
      _emit(widget.taper.toPosition(widget.value) + 0.1);
    } else if (key == LogicalKeyboardKey.pageDown) {
      _emit(widget.taper.toPosition(widget.value) - 0.1);
    } else if (key == LogicalKeyboardKey.escape) {
      _focus.unfocus();
    } else {
      return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  double _along(Offset p) => _vertical ? p.dy : p.dx;

  void _start(Offset local, Size size) {
    final g = _geometry(size);
    final cap = g.positionToPixel(widget.taper.toPosition(widget.value));
    final pointer = _along(local);
    if ((pointer - cap).abs() <= widget.capLength / 2 + 4) {
      _grab = pointer - cap;
    } else {
      _grab = 0;
      _emit(g.pixelToPosition(pointer));
    }
    setState(() => _dragging = true);
  }

  void _update(Offset local, Size size) => _emit(_geometry(size).pixelToPosition(_along(local) - _grab));

  void _end() => setState(() => _dragging = false);

  void _emit(double position) => widget.onChanged(widget.taper.fromPosition(position));

  void _wheel(PointerScrollEvent event) {
    final delta = _vertical || event.scrollDelta.dx == 0 ? -event.scrollDelta.dy : event.scrollDelta.dx;
    if (delta == 0) return;
    _step(delta > 0 ? 1 : -1);
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final size = Size(constraints.maxWidth, constraints.maxHeight);
      return Focus(
        focusNode: _focus,
        onKeyEvent: _onKey,
        onFocusChange: (f) => setState(() => _focused = f),
        child: Listener(
          onPointerDown: (_) => _focus.requestFocus(),
          onPointerSignal: (e) {
            if (e is PointerScrollEvent) {
              GestureBinding.instance.pointerSignalResolver.register(e, (e) => _wheel(e as PointerScrollEvent));
            }
          },
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onDoubleTap: widget.resetValue == null ? null : () => widget.onChanged(widget.resetValue!),
            onVerticalDragStart: _vertical ? (d) => _start(d.localPosition, size) : null,
            onVerticalDragUpdate: _vertical ? (d) => _update(d.localPosition, size) : null,
            onVerticalDragEnd: _vertical ? (_) => _end() : null,
            onHorizontalDragStart: !_vertical ? (d) => _start(d.localPosition, size) : null,
            onHorizontalDragUpdate: !_vertical ? (d) => _update(d.localPosition, size) : null,
            onHorizontalDragEnd: !_vertical ? (_) => _end() : null,
            child: CustomPaint(
              size: size,
              painter: _FaderPainter(
                geometry: _geometry(size),
                position: widget.taper.toPosition(widget.value),
                marks: [for (final m in widget.marks) (widget.taper.toPosition(m), m)],
                majorMarks: widget.majorMarks,
                markLabel: widget.markLabel,
                color: widget.color,
                meterLevel: widget.meterLevel,
                meterPeak: widget.meterPeak,
                clip: widget.clip,
                limit: widget.limit,
                active: _dragging,
                focused: _focused,
              ),
            ),
          ),
        ),
      );
    });
  }
}

/// Scale labels for a row of narrow vertical faders without own labels
/// (same height and cap length as the faders).
class FaderScaleLabels extends StatelessWidget {
  final FaderTaper taper;
  final List<double> marks;
  final String Function(double value) markLabel;
  final double capLength;

  const FaderScaleLabels({
    super.key,
    required this.taper,
    required this.marks,
    required this.markLabel,
    this.capLength = 30,
  });

  @override
  Widget build(BuildContext context) => CustomPaint(
        painter: _ScaleLabelPainter(this),
        child: const SizedBox.expand(),
      );
}

class _ScaleLabelPainter extends CustomPainter {
  final FaderScaleLabels scale;

  _ScaleLabelPainter(this.scale);

  @override
  void paint(Canvas canvas, Size size) {
    final g = _FaderGeometry(size: size, vertical: true, capLength: scale.capLength, hasLabels: false, hasMeter: false);
    final label = TextPainter(textDirection: TextDirection.ltr);
    for (final m in scale.marks) {
      final text = scale.markLabel(m);
      if (text.isEmpty) continue;
      label
        ..text = TextSpan(text: text, style: monoValue.copyWith(fontSize: 9, color: m == 0 ? AppColors.text : AppColors.muted))
        ..layout();
      final y = g.positionToPixel(scale.taper.toPosition(m));
      label.paint(canvas, Offset(size.width - label.width - 4, y - label.height / 2));
    }
  }

  @override
  bool shouldRepaint(covariant _ScaleLabelPainter oldDelegate) => true;
}

/// Where things are inside the fader box.
class _FaderGeometry {
  final Size size;
  final bool vertical;
  final double capLength;
  final bool hasLabels;
  final bool hasMeter;

  /// Clip/limit indicators above the scale (space for two is reserved, so
  /// faders with one or two indicators line up).
  final bool hasIndicators;

  _FaderGeometry({
    required this.size,
    required this.vertical,
    required this.capLength,
    required this.hasLabels,
    required this.hasMeter,
    this.hasIndicators = false,
  });

  static const labelSpace = 24.0;
  static const meterWidth = 22.0;

  /// Space above the meter for the clip/limit indicators.
  static const indicatorHeight = 20.0;

  double get _cross => vertical ? size.width : size.height;
  double get _travel => vertical ? size.height : size.width;

  /// Cross-axis range of the slot area (between labels and meter).
  double get trackStart => hasLabels && vertical ? labelSpace : 0;
  double get trackEnd => _cross - (hasMeter && vertical ? meterWidth + 4 : 0) - (hasLabels && !vertical ? 14 : 0);
  double get trackCenter => (trackStart + trackEnd) / 2;
  double get capWidth => math.min(trackEnd - trackStart - 2, vertical ? 40.0 : 26.0).clamp(10.0, 60.0);

  /// Height reserved above the travel for the indicators.
  double get indicatorSpace => hasIndicators && vertical ? 2 * (indicatorHeight + 3) : 0;

  double get _min => capLength / 2 + 2 + indicatorSpace;
  double get _max => _travel - capLength / 2 - 2;

  /// Pixel along the travel axis for a position (vertical: 1 = top).
  double positionToPixel(double p) => vertical ? _max - p * (_max - _min) : _min + p * (_max - _min);

  double pixelToPosition(double px) =>
      (vertical ? (_max - px) / (_max - _min) : (px - _min) / (_max - _min)).clamp(0.0, 1.0);
}

class _FaderPainter extends CustomPainter {
  final _FaderGeometry geometry;
  final double position;
  final List<(double position, double value)> marks;
  final List<double> majorMarks;
  final String Function(double value)? markLabel;
  final Color color;
  final double? meterLevel;
  final double? meterPeak;
  final bool? clip;
  final bool? limit;
  final bool active;
  final bool focused;

  _FaderPainter({
    required this.geometry,
    required this.position,
    required this.marks,
    required this.majorMarks,
    required this.markLabel,
    required this.color,
    required this.meterLevel,
    required this.meterPeak,
    required this.clip,
    required this.limit,
    required this.active,
    required this.focused,
  });

  /// Laid-out scale labels, shared by all faders (a page repaints its
  /// faders often; text layout is the expensive part).
  static final Map<String, TextPainter> _labels = {};

  static TextPainter _labelPainter(String text, bool major) => _labels.putIfAbsent(
        '$text|$major',
        () => TextPainter(
          text: TextSpan(text: text, style: monoValue.copyWith(fontSize: 9, color: major ? AppColors.text : AppColors.muted)),
          textDirection: TextDirection.ltr,
        )..layout(),
      );

  static final Map<String, TextPainter> _indicatorLabels = {};

  /// Path with the 1 px gaps between 3 px segments, cached per meter size.
  static final Map<Rect, Path> _gapPaths = {};

  static Path _segmentGaps(Rect meter) {
    if (_gapPaths.length > 64) _gapPaths.clear();
    return _gapPaths.putIfAbsent(meter, () {
      const segment = 3.0, gap = 1.0;
      final path = Path();
      for (var y = meter.bottom - segment; y > meter.top; y -= segment + gap) {
        path.addRect(Rect.fromLTRB(meter.left, y - gap, meter.right, y));
      }
      return path;
    });
  }

  /// [_meterColors] dimmed (unlit segments).
  static const _meterColorsDim = [Color(0xFF173A26), Color(0xFF173A26), Color(0xFF3F3A24), Color(0xFF3E1F22)];

  static const _meterColors = [AppColors.accent, AppColors.accent, Color(0xFFFDE68A), AppColors.danger];
  /// Colour stops in fader positions of [gainTaper]: green up to about -8 dB,
  /// yellow towards 0 dB, red from 0 dB.
  static const _meterStops = [0.0, 0.6, 0.72, 0.78];

  /// Segmented level meter: every segment is pre-coloured but dim and
  /// lights up when the level reaches it. Clip and limit indicators sit on
  /// top.
  void _paintMeter(Canvas canvas, Size size, double top, double bottom) {
    final level = meterLevel!.clamp(0.0, 1.0);
    final x0 = size.width - _FaderGeometry.meterWidth;
    final indicators = [
      // Limit on top, clip right at the end of the scale.
      if (limit != null) (limit!, AppColors.warning, 'L'),
      if (clip != null) (clip!, AppColors.danger, 'C'),
    ];
    const ind = _FaderGeometry.indicatorHeight;
    // The meter spans exactly the scale, so its level reads against the dB
    // labels; the indicators sit above the scale.
    final meter = Rect.fromLTRB(x0, top, size.width, bottom);
    canvas.drawRRect(RRect.fromRectAndRadius(meter.inflate(1), const Radius.circular(2)), Paint()..color = Colors.black);

    // Few draw calls: the browser renders the whole page again on every
    // frame of any animation (dialogs, tab ripples), so a meter must be cheap.
    // 1. the whole scale pre-coloured but dim, 2. the lit part, 3. one path
    // with all segment gaps on top.
    LinearGradient gradient(List<Color> colors) =>
        LinearGradient(begin: Alignment.bottomCenter, end: Alignment.topCenter, colors: colors, stops: _meterStops);
    canvas.drawRect(meter, Paint()..shader = gradient(_meterColorsDim).createShader(meter));
    final litTop = meter.bottom - meter.height * level;
    if (level > 0) {
      canvas.drawRect(Rect.fromLTRB(x0, litTop, size.width, meter.bottom),
          Paint()..shader = gradient(_meterColors).createShader(meter));
    }
    canvas.drawPath(_segmentGaps(meter), Paint()..color = Colors.black);
    final peak = meterPeak;
    if (peak != null && peak > 0.01) {
      final py = meter.bottom - meter.height * peak.clamp(0.0, 1.0);
      canvas.drawRect(
          Rect.fromLTRB(x0, py - 1, size.width, py + 1), Paint()..shader = gradient(_meterColors).createShader(meter));
    }

    // Indicators above the meter
    for (var i = 0; i < indicators.length; i++) {
      final (on, c, letter) = indicators[i];
      // Bottom slot right above the scale: clip; top slot: limit.
      final slot = indicators.length == 2 ? i : 1;
      final r = Rect.fromLTWH(x0, 2 + slot * (ind + 3), _FaderGeometry.meterWidth, ind);
      final rr = RRect.fromRectAndRadius(r, const Radius.circular(2));
      if (on) {
        // Glow as a soft ring (no blur filter: blurs are expensive in the browser)
        canvas.drawRRect(rr.inflate(2), Paint()..color = c.withAlpha(70));
      }
      canvas.drawRRect(rr, Paint()..color = on ? c : c.withAlpha(45));
      final t = _indicatorLabels.putIfAbsent(
        '$letter|$on',
        () => TextPainter(
          text: TextSpan(
              text: letter,
              style: interWeight(
                  TextStyle(fontSize: 12, height: 1, color: on ? AppColors.backgroundDeep : c.withAlpha(150)),
                  FontWeight.w800)),
          textDirection: TextDirection.ltr,
        )..layout(),
      );
      t.paint(canvas, r.center - Offset(t.width / 2, t.height / 2));
    }
  }

  bool get _v => geometry.vertical;

  /// Point from (along travel, across) coordinates.
  Offset _pt(double along, double across) => _v ? Offset(across, along) : Offset(along, across);

  Rect _rect(double alongStart, double alongEnd, double acrossStart, double acrossEnd) => _v
      ? Rect.fromLTRB(acrossStart, alongStart, acrossEnd, alongEnd)
      : Rect.fromLTRB(alongStart, acrossStart, alongEnd, acrossEnd);

  @override
  void paint(Canvas canvas, Size size) {
    final g = geometry;
    final c = g.trackCenter;
    final halfLine = g.capWidth / 2 + 5;
    final top = g.positionToPixel(_v ? 1 : 0);
    final bottom = g.positionToPixel(_v ? 0 : 1);

    // Scale lines across the slot, labels beside it
    double? lastLabelAt;
    for (final (pos, value) in marks) {
      final at = g.positionToPixel(pos);
      final major = majorMarks.contains(value);
      canvas.drawLine(
        _pt(at, c - halfLine),
        _pt(at, c + halfLine),
        Paint()
          ..color = major ? AppColors.muted : AppColors.border
          ..strokeWidth = major ? 1.6 : 1,
      );
      final text = markLabel?.call(value);
      if (text == null || text.isEmpty) continue;
      final label = _labelPainter(text, major);
      final extent = _v ? label.height : label.width;
      if (lastLabelAt != null && (at - lastLabelAt).abs() < extent + 1) continue;
      lastLabelAt = at;
      final offset = _v
          ? Offset(g.trackStart - 3 - label.width, at - label.height / 2)
          : Offset(at - label.width / 2, g.trackEnd + 2);
      label.paint(canvas, offset);
    }

    // Slot
    final slot = RRect.fromRectAndRadius(
        _rect(math.min(top, bottom) - 4, math.max(top, bottom) + 4, c - 3, c + 3), const Radius.circular(3));
    canvas.drawRRect(slot, Paint()..color = Colors.black);
    canvas.drawRRect(
        slot,
        Paint()
          ..color = AppColors.border
          ..style = PaintingStyle.stroke);
    final capAt = g.positionToPixel(position);
    final fillStart = _v ? bottom : top;
    canvas.drawRRect(
      RRect.fromRectAndRadius(
          _rect(math.min(capAt, fillStart), math.max(capAt, fillStart), c - 1.2, c + 1.2), const Radius.circular(1)),
      Paint()..color = color.withAlpha(170),
    );

    // Meter
    if (meterLevel != null && _v) _paintMeter(canvas, size, top, bottom);

    // Cap: shadow, body with gradient, grip ridges, centre line
    final half = g.capLength / 2;
    final capRect = _rect(capAt - half, capAt + half, c - g.capWidth / 2, c + g.capWidth / 2);
    final cap = RRect.fromRectAndRadius(capRect, const Radius.circular(4));
    if (focused) {
      // Keyboard focus: coloured glow around the cap
      canvas.drawRRect(
        cap.inflate(3),
        Paint()
          ..color = color.withAlpha(150)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.5,
      );
    }
    canvas.drawRRect(
      cap.shift(_v ? const Offset(0, 3) : const Offset(2, 2)),
      // Hard, slightly transparent shadow: a blur filter per fader is too
      // expensive when the browser repaints the page on every frame.
      Paint()..color = Colors.black.withAlpha(110),
    );
    canvas.drawRRect(
      cap,
      Paint()
        ..shader = LinearGradient(
          begin: _v ? Alignment.topCenter : Alignment.centerLeft,
          end: _v ? Alignment.bottomCenter : Alignment.centerRight,
          colors: active
              ? const [Color(0xFF7A818C), Color(0xFF4B515A), Color(0xFF353A41)]
              : const [Color(0xFF666C76), Color(0xFF3F444C), Color(0xFF2B2F35)],
        ).createShader(capRect),
    );
    canvas.drawRRect(
      cap,
      Paint()
        ..color = Colors.black.withAlpha(160)
        ..style = PaintingStyle.stroke,
    );
    // Grip ridges: dark groove with a light edge below
    final inset = g.capWidth * 0.14;
    final ridges = math.max(2, (g.capLength / 6).floor());
    for (var i = 0; i < ridges; i++) {
      final t = (i + 1) / (ridges + 1);
      final at = capAt - half + g.capLength * t;
      if ((at - capAt).abs() < 2.5) continue; // leave room for the centre line
      canvas.drawLine(_pt(at, c - g.capWidth / 2 + inset), _pt(at, c + g.capWidth / 2 - inset),
          Paint()
            ..color = Colors.black.withAlpha(150)
            ..strokeWidth = 1);
      canvas.drawLine(_pt(at + 1, c - g.capWidth / 2 + inset), _pt(at + 1, c + g.capWidth / 2 - inset),
          Paint()
            ..color = Colors.white.withAlpha(40)
            ..strokeWidth = 1);
    }
    canvas.drawLine(_pt(capAt, c - g.capWidth / 2 + 2), _pt(capAt, c + g.capWidth / 2 - 2),
        Paint()
          ..color = color
          ..strokeWidth = 2);
  }

  @override
  bool shouldRepaint(covariant _FaderPainter old) => true;
}
