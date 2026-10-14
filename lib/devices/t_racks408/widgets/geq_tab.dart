// Graphic EQ (GEQ, inputs): curve on top, below it one fader per band,
// each exactly under its point of the curve (like the original editor).

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../../../services/rta_service.dart';
import '../../../services/rta_settings_provider.dart';
import '../../../theme/app_colors.dart';
import '../providers/device_provider.dart';
import 'console_fader.dart';
import 'auto_eq_panel.dart';
import 'ui_common.dart';
import 'value_field.dart';

// 31-band ISO 1/3-octave center frequencies
const List<double> _bandFrequencies = [
  20, 25, 31.5, 40, 50, 63, 80, 100, 125, 160,
  200, 250, 315, 400, 500, 630, 800, 1000, 1250, 1600,
  2000, 2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000,
];

String _formatFreq(double freq) {
  if (freq >= 1000) {
    final k = freq / 1000;
    return k == k.truncateToDouble() ? '${k.toInt()}K' : '${k}K';
  }
  return freq == freq.truncateToDouble()
      ? '${freq.toInt()}'
      : '$freq';
}

const _geqMarks = [12.0, 9.0, 6.0, 3.0, 0.0, -3.0, -6.0, -9.0, -12.0];
const _geqTaper = LinearTaper(-12, 12);
const _geqCapLength = 22.0;

String _geqLabel(double db) => db % 6 == 0 ? (db > 0 ? '+${db.round()}' : '${db.round()}') : '';

class GeqTab extends StatefulWidget {
  final DeviceProvider deviceProvider;

  /// Input shown first (e.g. when opened from the EQ overview).
  final String? initialChannel;

  /// Offer the DSP's own generator in the Auto-EQ (not in the offline editor).
  final bool dspGenerator;

  const GeqTab({super.key, required this.deviceProvider, this.initialChannel, this.dspGenerator = true});

  @override
  State<GeqTab> createState() => _GeqTabState();
}

class _GeqTabState extends State<GeqTab> {
  DeviceProvider get deviceProvider => widget.deviceProvider;

  late String _channel = (widget.initialChannel != null && deviceProvider.inputs.contains(widget.initialChannel))
      ? widget.initialChannel!
      : deviceProvider.inputs.first;
  int? _activeBandIndex;

  bool _rtaEnabled = false;
  RtaService? _rtaService;
  bool _disposed = false;

  // ─── Auto-EQ ───
  bool _autoEqOpen = false;
  List<double>? _proposal;

  Future<void> _openAutoEq() async {
    if (_autoEqOpen) {
      _closeAutoEq();
      return;
    }
    if (!_rtaEnabled) await _toggleRta();
    if (!mounted || !_rtaEnabled) return;
    setState(() => _autoEqOpen = true);
  }

  void _closeAutoEq() => setState(() {
        _autoEqOpen = false;
        _proposal = null;
      });

  /// Below this width the curve and the faders scroll sideways together.
  static const _minWidth = 1180.0;
  static const _faderBankHeight = 250.0;

  Future<void> _toggleRta() async {
    if (_rtaEnabled) {
      if (_autoEqOpen) _closeAutoEq();
      _rtaService?.release();
      _rtaService = null;
      setState(() => _rtaEnabled = false);
    } else {
      final rtaSettings = context.read<RtaSettingsProvider>();
      if (!rtaSettings.isMobile) {
        rtaSettings.enumerateDevices();
      }
      final svc = RtaService();
      svc.onUpdate = () {
        if (!_disposed) setState(() {});
      };
      try {
        await svc.requestPermission();
        await svc.startCapture(deviceId: rtaSettings.deviceId);
        if (!mounted) {
          svc.release();
          return;
        }
        _rtaService = svc;
        setState(() => _rtaEnabled = true);
      } catch (e) {
        svc.release();
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('Failed to start RTA: $e')),
          );
        }
      }
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _rtaService?.release();
    super.dispose();
  }

  void _setBand(int index, double dB) {
    setState(() => _activeBandIndex = index);
    deviceProvider.setGeqBand(_channel, index, ((dB * 10).round() / 10).clamp(-12.0, 12.0));
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: deviceProvider,
      builder: (context, _) {
        // The DSP 206 has fewer inputs.
        if (!deviceProvider.inputs.contains(_channel)) _channel = deviceProvider.inputs.first;
        final bands = deviceProvider.getGeqBands(_channel);
        return Column(
          children: [
            Row(children: [
              Expanded(
                child: ChannelSelector(
                  device: deviceProvider,
                  selected: _channel,
                  showOutputs: false,
                  onSelected: (c) => setState(() {
                    // A proposal belongs to one input.
                    if (c != _channel) _proposal = null;
                    _channel = c;
                  }),
                ),
              ),
              _ActionButton(label: 'Flat', active: false, onPressed: () => deviceProvider.resetGeqBands(_channel)),
              const SizedBox(width: 8),
              _ActionButton(
                label: 'Bypass',
                active: deviceProvider.getGeqBypass(_channel),
                onPressed: () => deviceProvider.setGeqBypass(_channel, !deviceProvider.getGeqBypass(_channel)),
              ),
              const SizedBox(width: 8),
              _ActionButton(label: 'RTA', active: _rtaEnabled, onPressed: _toggleRta),
              const SizedBox(width: 8),
              _ActionButton(label: 'Auto-EQ', active: _autoEqOpen, onPressed: _openAutoEq),
              const SizedBox(width: 12),
            ]),
            if (_autoEqOpen && _rtaService != null)
              Padding(
                padding: const EdgeInsets.fromLTRB(12, 0, 12, 8),
                child: AutoEqPanel(
                  // A new panel per input (restores mutes of the previous one).
                  key: ValueKey('autoeq-$_channel'),
                  device: deviceProvider,
                  channel: _channel,
                  target: AutoEqTarget.geq,
                  rta: _rtaService,
                  dspGenerator: widget.dspGenerator,
                  onGeqProposal: (p) => setState(() => _proposal = p),
                  onClose: _closeAutoEq,
                ),
              ),
            Expanded(
              child: LayoutBuilder(builder: (context, constraints) {
                final width = math.max(constraints.maxWidth - 24, _minWidth);
                final content = SizedBox(
                  width: width,
                  child: Column(children: [
                    Expanded(
                      child: LayoutBuilder(
                        builder: (context, inner) => _GeqGraph(
                          bands: bands,
                          drawMode: false,
                          activeBandIndex: _activeBandIndex,
                          rtaMagnitudes: _rtaEnabled ? _rtaService?.magnitudes : null,
                          rtaPeaks: _rtaEnabled ? _rtaService?.peaks : null,
                          proposal: _autoEqOpen ? _proposal : null,
                          width: width,
                          height: inner.maxHeight,
                          narrow: false,
                          onBandChanged: _setBand,
                          onBandReset: (index) => _setBand(index, 0),
                          onDragEnd: () => setState(() => _activeBandIndex = null),
                        ),
                      ),
                    ),
                    const SizedBox(height: 8),
                    SizedBox(height: _faderBankHeight, child: _faderBank(bands, width)),
                  ]),
                );
                return Padding(
                  padding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
                  child: constraints.maxWidth - 24 >= _minWidth
                      ? content
                      : Scrollbar(child: SingleChildScrollView(scrollDirection: Axis.horizontal, child: content)),
                );
              }),
            ),
          ],
        );
      },
    );
  }

  /// One fader per band, centred under the band's point of the curve.
  Widget _faderBank(List<double> bands, double width) {
    const padLeft = _GeqGraph.padLeftWide;
    const padRight = _GeqGraph.padRightWide;
    final graphW = width - padLeft - padRight;
    final xs = [for (final f in _bandFrequencies) padLeft + _freqToNorm(f) * graphW];
    var spacing = double.infinity;
    for (var i = 1; i < xs.length; i++) {
      spacing = math.min(spacing, xs[i] - xs[i - 1]);
    }
    final faderWidth = spacing - 2;
    const fieldHeight = 20.0;
    const labelHeight = 16.0;

    return Container(
      decoration: BoxDecoration(
        color: AppColors.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: AppColors.border),
      ),
      child: Stack(children: [
        // dB scale left of the first fader
        Positioned(
          left: 0,
          width: padLeft - 2,
          top: 6,
          bottom: 6 + fieldHeight + labelHeight + 4,
          child: FaderScaleLabels(taper: _geqTaper, marks: _geqMarks, markLabel: _geqLabel, capLength: _geqCapLength),
        ),
        for (var i = 0; i < xs.length; i++)
          Positioned(
            left: xs[i] - faderWidth / 2,
            width: faderWidth,
            top: 6,
            bottom: 4,
            child: Column(children: [
              Expanded(
                child: ConsoleFader(
                  value: bands[i],
                  taper: _geqTaper,
                  marks: _geqMarks,
                  majorMarks: const [0.0],
                  capLength: _geqCapLength,
                  color: _activeBandIndex == i ? AppColors.accent : AppColors.input,
                  wheelStep: 0.5,
                  resetValue: 0,
                  onChanged: (v) => _setBand(i, v),
                ),
              ),
              const SizedBox(height: 4),
              ValueField(
                text: bands[i].toStringAsFixed(1),
                height: fieldHeight,
                fontSize: 9,
                tooltip: '${_formatFreq(_bandFrequencies[i])} Hz',
                onSubmitted: (input) {
                  final v = parseNumber(input);
                  if (v == null) return false;
                  _setBand(i, v);
                  return true;
                },
              ),
              SizedBox(
                height: labelHeight,
                child: Center(
                  child: Text(_formatFreq(_bandFrequencies[i]),
                      style: const TextStyle(fontSize: 9, color: AppColors.muted)),
                ),
              ),
            ]),
          ),
      ]),
    );
  }
}

// ─── Action button ───

class _ActionButton extends StatelessWidget {
  final String label;
  final bool active;
  final VoidCallback onPressed;

  const _ActionButton({required this.label, required this.active, required this.onPressed});

  @override
  Widget build(BuildContext context) {
    return InkWell(
      borderRadius: BorderRadius.circular(8),
      onTap: onPressed,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 7),
        decoration: BoxDecoration(
          color: active ? AppColors.input.withAlpha(40) : AppColors.surfaceHigh,
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: active ? AppColors.input : AppColors.border),
        ),
        child: Text(label, style: TextStyle(fontSize: 12, color: active ? AppColors.input : AppColors.text)),
      ),
    );
  }
}

// ─── Log-frequency helpers ───

const double _minFreq = 10; // log-floor (graph visually represents ~0 Hz)
const double _maxFreq = 30000;
final double _logMin = math.log(_minFreq);
final double _logMax = math.log(_maxFreq);
final double _logRange = _logMax - _logMin;

/// Map a frequency to a normalized 0..1 position on the log axis.
double _freqToNorm(double freq) =>
    (math.log(freq) - _logMin) / _logRange;

/// Map a normalized 0..1 position back to frequency.
double _normToFreq(double norm) =>
    math.exp(_logMin + norm * _logRange);

// ─── Graph + band interaction ───

class _GeqGraph extends StatelessWidget {
  final List<double> bands;
  final bool drawMode;
  final int? activeBandIndex;
  final List<double>? rtaMagnitudes;
  final List<double>? rtaPeaks;
  final List<double>? proposal;
  final double width;
  final double height;
  final bool narrow;
  final void Function(int index, double dB) onBandChanged;
  final void Function(int index) onBandReset;
  final VoidCallback onDragEnd;

  const _GeqGraph({
    required this.bands,
    required this.drawMode,
    required this.activeBandIndex,
    this.rtaMagnitudes,
    this.rtaPeaks,
    this.proposal,
    required this.width,
    required this.height,
    required this.narrow,
    required this.onBandChanged,
    required this.onBandReset,
    required this.onDragEnd,
  });

  static const padLeftWide = 36.0;
  static const padRightWide = 8.0;

  double get _padLeft => narrow ? 40.0 : padLeftWide;
  static const double _padRight = padRightWide;
  double get _padTop => narrow ? 14.0 : 10.0;
  double get _padBottom => narrow ? 36.0 : 28.0;

  double get _graphWidth => width - _padLeft - _padRight;
  double get _graphHeight => height - _padTop - _padBottom;

  /// Convert pixel X to the nearest band index using log-frequency mapping.
  int _bandIndexFromX(double x) {
    final norm = ((x - _padLeft) / _graphWidth).clamp(0.0, 1.0);
    final freq = _normToFreq(norm);
    // Find nearest band by log-distance
    int best = 0;
    double bestDist = double.infinity;
    for (int i = 0; i < 31; i++) {
      final d = (math.log(_bandFrequencies[i]) - math.log(freq)).abs();
      if (d < bestDist) {
        bestDist = d;
        best = i;
      }
    }
    return best;
  }

  double _dBFromY(double y) {
    final local = y - _padTop;
    final ratio = 1.0 - (local / _graphHeight);
    return (ratio * 24 - 12).clamp(-12.0, 12.0);
  }

  void _handleDrag(Offset position) {
    final index = _bandIndexFromX(position.dx);
    final dB = _dBFromY(position.dy);
    onBandChanged(index, dB);
  }

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerSignal: (event) {
        if (event is PointerScrollEvent) {
          final index = _bandIndexFromX(event.localPosition.dx);
          final current = bands[index.clamp(0, 30)];
          final delta = -event.scrollDelta.dy / 50;
          onBandChanged(index, (current + delta).clamp(-12.0, 12.0));
        }
      },
      child: GestureDetector(
        onPanStart: drawMode ? (d) => _handleDrag(d.localPosition) : null,
        onPanUpdate: drawMode ? (d) => _handleDrag(d.localPosition) : null,
        onPanEnd: drawMode ? (_) => onDragEnd() : null,
        onVerticalDragStart:
            !drawMode ? (d) => _handleDrag(d.localPosition) : null,
        onVerticalDragUpdate:
            !drawMode ? (d) => _handleDrag(d.localPosition) : null,
        onVerticalDragEnd: !drawMode ? (_) => onDragEnd() : null,
        onDoubleTapDown: (d) {
          final index = _bandIndexFromX(d.localPosition.dx);
          onBandReset(index);
        },
        child: CustomPaint(
          size: Size(width, height),
          painter: _GeqPainter(
            bands: bands,
            activeBandIndex: activeBandIndex,
            rtaMagnitudes: rtaMagnitudes,
            rtaPeaks: rtaPeaks,
            proposal: proposal,
            padLeft: _padLeft,
            padRight: _padRight,
            padTop: _padTop,
            padBottom: _padBottom,
            narrow: narrow,
          ),
        ),
      ),
    );
  }
}

// ─── Custom painter ───

class _GeqPainter extends CustomPainter {
  final List<double> bands;
  final int? activeBandIndex;
  final List<double>? rtaMagnitudes;
  final List<double>? rtaPeaks;
  final List<double>? proposal;
  final double padLeft;
  final double padRight;
  final double padTop;
  final double padBottom;
  final bool narrow;
  final Color curveColor = const Color(0xFF38BDF8);

  _GeqPainter({
    required this.bands,
    this.activeBandIndex,
    this.rtaMagnitudes,
    this.rtaPeaks,
    this.proposal,
    required this.padLeft,
    required this.padRight,
    required this.padTop,
    required this.padBottom,
    required this.narrow,
  });

  /// Map frequency to pixel X using log scale.
  double _freqToX(double freq, double graphW) =>
      padLeft + _freqToNorm(freq) * graphW;

  @override
  void paint(Canvas canvas, Size size) {
    final graphW = size.width - padLeft - padRight;
    final graphH = size.height - padTop - padBottom;

    // Font sizes scaled for mobile
    final axisLabelSize = narrow ? 12.0 : 9.0;
    final dbLabelSize = narrow ? 11.0 : 7.0;
    final bubbleFontSize = narrow ? 14.0 : 11.0;

    // Background
    final graphRect = Rect.fromLTWH(padLeft, padTop, graphW, graphH);
    canvas.drawRect(graphRect, Paint()..color = const Color(0xFF14161A));
    canvas.drawRect(
      graphRect,
      Paint()
        ..color = const Color(0xFF7D8590)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.0,
    );

    final gridPaint = Paint()
      ..color = const Color(0xFF7D8590)
      ..strokeWidth = 0.5;

    final zeroPaint = Paint()
      ..color = const Color(0xFFF87171)
      ..strokeWidth = 1.0;

    // Horizontal grid + labels
    const dBSteps = [-12.0, -6.0, 0.0, 6.0, 12.0];
    final labelStyle = TextStyle(fontFamily: 'Inter', 
      color: const Color(0xFF7D8590),
      fontSize: axisLabelSize,
    );

    for (final dB in dBSteps) {
      final y = padTop + graphH * (1.0 - (dB + 12) / 24);
      final paint = dB == 0 ? zeroPaint : gridPaint;
      canvas.drawLine(Offset(padLeft, y), Offset(padLeft + graphW, y), paint);

      final tp = TextPainter(
        text: TextSpan(
            text: '${dB > 0 ? '+' : ''}${dB.toInt()}',
            style: labelStyle),
        textDirection: TextDirection.ltr,
      )..layout();
      tp.paint(canvas, Offset(padLeft - tp.width - 4, y - tp.height / 2));
    }

    // Vertical grid lines — logarithmically positioned
    final thickGridPaint = Paint()
      ..color = const Color(0xFF7D8590)
      ..strokeWidth = 1.5;

    final gridFreqs = <double>[
      for (double f = 20; f < 100; f += 10) f,
      for (double f = 100; f < 1000; f += 100) f,
      for (double f = 1000; f < 10000; f += 1000) f,
      10000, 20000,
    ];

    for (final freq in gridFreqs) {
      final x = _freqToX(freq, graphW);
      final thick =
          freq == 100 || freq == 1000 || freq == 10000 || freq == 30000;
      canvas.drawLine(Offset(x, padTop), Offset(x, padTop + graphH),
          thick ? thickGridPaint : gridPaint);
    }

    // Frequency labels along bottom at key positions
    // On mobile, show more labels since we have the space
    final freqLabels = narrow
        ? [20.0, 31.5, 50.0, 80.0, 100.0, 160.0, 250.0, 500.0, 800.0,
           1000.0, 1600.0, 2500.0, 5000.0, 8000.0, 10000.0, 16000.0, 20000.0]
        : [20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0];
    for (final freq in freqLabels) {
      final label = _formatFreq(freq);
      final tp = TextPainter(
        text: TextSpan(text: label, style: labelStyle),
        textDirection: TextDirection.ltr,
      )..layout();
      final x = _freqToX(freq, graphW) - tp.width / 2;
      tp.paint(canvas, Offset(x, padTop + graphH + 4));
    }

    // ── RTA overlay bars (drawn behind EQ controls) ──
    if (rtaMagnitudes != null && rtaMagnitudes!.length == rtaBandCount) {
      final rtaBarPaint = Paint()..color = const Color(0x4F38BDF8); // cyan ~31%
      final rtaPeakPaint = Paint()
        ..color = const Color(0x78FFFFFF) // white ~47%
        ..strokeWidth = 1.0;

      for (int b = 0; b < rtaBandCount; b++) {
        final loFreq = rtaBandEdges[b];
        final hiFreq = rtaBandEdges[b + 1];

        // Map RTA band edges to pixel X via GEQ's log axis
        final xLeft = _freqToX(loFreq.clamp(_minFreq, _maxFreq), graphW);
        final xRight = _freqToX(hiFreq.clamp(_minFreq, _maxFreq), graphW);
        if (xRight <= padLeft || xLeft >= padLeft + graphW) continue;

        // Squash full RTA range (-140..+10 dB) into the graph height
        final norm = (rtaMagnitudes![b] - rtaMinDb) / rtaDbRange;
        final barY = padTop + graphH * (1.0 - norm.clamp(0.0, 1.0));

        final clampedLeft = xLeft.clamp(padLeft, padLeft + graphW);
        final clampedRight = xRight.clamp(padLeft, padLeft + graphW);

        canvas.drawRect(
          Rect.fromLTRB(clampedLeft, barY, clampedRight, padTop + graphH),
          rtaBarPaint,
        );

        // Peak hold marker
        if (rtaPeaks != null && rtaPeaks!.length == rtaBandCount) {
          final peakNorm = (rtaPeaks![b] - rtaMinDb) / rtaDbRange;
          final peakY = padTop + graphH * (1.0 - peakNorm.clamp(0.0, 1.0));
          canvas.drawLine(
            Offset(clampedLeft, peakY),
            Offset(clampedRight, peakY),
            rtaPeakPaint,
          );
        }
      }
    }

    // GEQ curve: smooth line through the band values, filled to 0 dB
    final zeroY = padTop + graphH * 0.5; // 0 dB line
    final points = [
      for (int i = 0; i < 31; i++)
        Offset(_freqToX(_bandFrequencies[i], graphW), padTop + graphH * (1.0 - (bands[i] + 12) / 24)),
    ];
    final line = _monotoneCurve(points);
    final fill = Path.from(line)
      ..lineTo(points.last.dx, zeroY)
      ..lineTo(points.first.dx, zeroY)
      ..close();
    canvas.drawPath(fill, Paint()..color = curveColor.withAlpha(45));
    canvas.drawPath(
      line,
      Paint()
        ..color = curveColor
        ..style = PaintingStyle.stroke
        ..strokeWidth = narrow ? 3 : 2.5
        ..strokeJoin = StrokeJoin.round,
    );

    // Auto-EQ proposal
    final proposed = proposal;
    if (proposed != null && proposed.length == 31) {
      final proposalPoints = [
        for (int i = 0; i < 31; i++)
          Offset(_freqToX(_bandFrequencies[i], graphW), padTop + graphH * (1.0 - (proposed[i] + 12) / 24)),
      ];
      canvas.drawPath(
        _monotoneCurve(proposalPoints),
        Paint()
          ..color = const Color(0xFFFBBF24)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2
          ..strokeJoin = StrokeJoin.round,
      );
      for (final p in proposalPoints) {
        canvas.drawCircle(p, 2.5, Paint()..color = const Color(0xFFFBBF24));
      }
    }

    for (int i = 0; i < 31; i++) {
      final dB = bands[i];
      final cx = points[i].dx;
      final bandY = points[i].dy;
      final active = activeBandIndex == i;

      // Handle on the line
      final radius = (narrow ? 6.0 : 4.5) * (active ? 1.6 : 1.0);
      canvas.drawCircle(Offset(cx, bandY), radius, Paint()..color = const Color(0xFF14161A));
      canvas.drawCircle(
        Offset(cx, bandY),
        radius,
        Paint()
          ..color = dB == 0 ? const Color(0xFF7D8590) : curveColor
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2,
      );

      // dB value above the handle
      if (dB != 0.0) {
        final dbLabel = '${dB >= 0 ? '+' : ''}${dB.toStringAsFixed(1)}';
        final dbTp = TextPainter(
          text: TextSpan(
            text: dbLabel,
            style: TextStyle(fontFamily: 'Inter', 
              color: const Color(0xFFE6E8EB),
              fontSize: dbLabelSize,
              fontWeight: FontWeight.w600,
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        // Position above handle for boost, below for cut
        final dbY = dB >= 0
            ? bandY - radius - dbTp.height - 1
            : bandY + radius + 1;
        final dbX = (cx - dbTp.width / 2)
            .clamp(padLeft, padLeft + graphW - dbTp.width);
        if (dbY >= padTop - 2 && dbY + dbTp.height <= padTop + graphH + 2) {
          dbTp.paint(canvas, Offset(dbX, dbY));
        }
      }
    }

    // dB popup bubble for active band
    if (activeBandIndex != null &&
        activeBandIndex! >= 0 &&
        activeBandIndex! < 31) {
      final i = activeBandIndex!;
      final dB = bands[i];
      final cx = _freqToX(_bandFrequencies[i], graphW);
      final bandY = padTop + graphH * (1.0 - (dB + 12) / 24);

      final label = '${dB >= 0 ? '+' : ''}${dB.toStringAsFixed(1)} dB';
      final tp = TextPainter(
        text: TextSpan(
          text: label,
          style: TextStyle(fontFamily: 'Inter', 
            color: const Color(0xFFE6E8EB),
            fontSize: bubbleFontSize,
            fontWeight: FontWeight.w600,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();

      final bubblePadH = narrow ? 10.0 : 8.0;
      final bubblePadV = narrow ? 6.0 : 4.0;
      final bubbleW = tp.width + bubblePadH * 2;
      final bubbleH = tp.height + bubblePadV * 2;

      // Position above the handle; flip below if too close to top
      final above = bandY - bubbleH - 8;
      final bubbleY = above >= padTop ? above : bandY + 12;
      // Center horizontally, clamp within graph
      final bubbleX = (cx - bubbleW / 2)
          .clamp(padLeft, padLeft + graphW - bubbleW);

      final bubbleRect = RRect.fromRectAndRadius(
        Rect.fromLTWH(bubbleX, bubbleY, bubbleW, bubbleH),
        const Radius.circular(4),
      );
      canvas.drawRRect(
          bubbleRect, Paint()..color = const Color(0xDD22262C));
      tp.paint(
        canvas,
        Offset(bubbleX + bubblePadH, bubbleY + bubblePadV),
      );
    }
  }

  @override
  bool shouldRepaint(covariant _GeqPainter oldDelegate) => true;

  /// Smooth curve through [points] (ascending x) that does not overshoot:
  /// monotone cubic interpolation (Fritsch–Carlson), so a flat band next
  /// to a boosted one stays flat.
  static Path _monotoneCurve(List<Offset> points) {
    final n = points.length;
    final path = Path()..moveTo(points.first.dx, points.first.dy);
    if (n < 2) return path;
    final slopes = [
      for (var k = 0; k < n - 1; k++)
        (points[k + 1].dy - points[k].dy) / (points[k + 1].dx - points[k].dx),
    ];
    final tangents = List<double>.filled(n, 0);
    tangents[0] = slopes.first;
    tangents[n - 1] = slopes.last;
    for (var k = 1; k < n - 1; k++) {
      tangents[k] = slopes[k - 1] * slopes[k] <= 0 ? 0 : (slopes[k - 1] + slopes[k]) / 2;
    }
    for (var k = 0; k < n - 1; k++) {
      if (slopes[k] == 0) {
        tangents[k] = 0;
        tangents[k + 1] = 0;
        continue;
      }
      final a = tangents[k] / slopes[k];
      final b = tangents[k + 1] / slopes[k];
      final s = a * a + b * b;
      if (s > 9) {
        final t = 3 / math.sqrt(s);
        tangents[k] = t * a * slopes[k];
        tangents[k + 1] = t * b * slopes[k];
      }
    }
    for (var k = 0; k < n - 1; k++) {
      final p1 = points[k];
      final p2 = points[k + 1];
      final h = (p2.dx - p1.dx) / 3;
      path.cubicTo(p1.dx + h, p1.dy + tangents[k] * h, p2.dx - h, p2.dy - tangents[k + 1] * h, p2.dx, p2.dy);
    }
    return path;
  }
}
