// Delay: on top one window for the inputs (left) and one for the outputs
// (right) with a speaker per channel that is dragged to the right to delay
// it; below vertical faders to fine-tune. Values are shown in ms, m or ft;
// like in the original editor the unit is stored on the DSP (0x15) and the
// distance uses 344 m/s. The range of each side can be zoomed; the faders
// use the same range, so a small range gives a fine adjustment.

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';
import '../widgets/console_fader.dart';
import '../widgets/ui_common.dart';
import '../widgets/value_field.dart';

final _maxMs = ProtocolService.delayRawToMs(ProtocolService.delayRawMax);

/// Selectable ranges in ms (the last one is the device maximum).
final _ranges = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, _maxMs];

class DelayPage extends StatefulWidget {
  final DeviceProvider device;

  const DelayPage({super.key, required this.device});

  @override
  State<DelayPage> createState() => _DelayPageState();
}

class _DelayPageState extends State<DelayPage> {
  /// Chosen range per side in ms; null = automatic.
  double? _inputRange;
  double? _outputRange;

  @override
  Widget build(BuildContext context) {
    final device = widget.device;
    return ListenableBuilder(
      listenable: device,
      builder: (context, _) {
        final inputs = device.inputs;
        final outputs = device.outputs;
        final inFlex = math.max(inputs.length, 3);
        final outFlex = math.max(outputs.length, 3);
        final unit = _DelayUnit(device.delayUnit);

        Widget side(List<String> channels, bool isInput, Widget Function(double rangeMs, Color color) child) {
          final chosen = isInput ? _inputRange : _outputRange;
          final longest = channels.map(device.getDelayMs).fold<double>(0, math.max);
          final auto = _ranges.firstWhere((r) => r >= longest * 1.2 && r >= 5, orElse: () => _maxMs);
          return child(chosen ?? auto, AppColors.forChannel(isInput));
        }

        return Column(children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
            child: Row(children: [
              const Text('Unit', style: TextStyle(fontSize: 12, color: AppColors.muted)),
              const SizedBox(width: 8),
              SegmentedButton<int>(
                segments: [
                  for (var u = 0; u < ProtocolService.delayUnitNames.length; u++)
                    ButtonSegment(value: u, label: Text(ProtocolService.delayUnitNames[u])),
                ],
                selected: {unit.unit},
                showSelectedIcon: false,
                style: SegmentedButton.styleFrom(
                  visualDensity: VisualDensity.compact,
                  selectedBackgroundColor: AppColors.accent.withAlpha(40),
                  selectedForegroundColor: AppColors.accent,
                  side: const BorderSide(color: AppColors.border),
                ),
                onSelectionChanged: (s) => device.setDelayUnit(s.first),
              ),
              const SizedBox(width: 16),
              const Expanded(
                child: Text('Drag a speaker to the right to delay the channel; fine-tune with the faders below.',
                    overflow: TextOverflow.ellipsis, style: TextStyle(fontSize: 11, color: AppColors.muted)),
              ),
            ]),
          ),
          Expanded(
            child: LayoutBuilder(builder: (context, constraints) {
              final stageHeight = math.max(200.0, constraints.maxHeight * 0.42);
              final pageHeight = math.max(constraints.maxHeight, 520.0);
              final content = SizedBox(
                height: pageHeight - 12,
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(12, 4, 12, 12),
                  child: Column(children: [
                    SizedBox(
                      height: stageHeight,
                      child: Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                        Expanded(
                          flex: inFlex,
                          child: side(inputs, true, (range, color) => _Stage(
                                device: device,
                                title: 'Inputs',
                                channels: inputs,
                                color: color,
                                rangeMs: range,
                                chosenRange: _inputRange,
                                unit: unit,
                                onRange: (r) => setState(() => _inputRange = r),
                              )),
                        ),
                        const SizedBox(width: 12),
                        Expanded(
                          flex: outFlex,
                          child: side(outputs, false, (range, color) => _Stage(
                                device: device,
                                title: 'Outputs',
                                channels: outputs,
                                color: color,
                                rangeMs: range,
                                chosenRange: _outputRange,
                                unit: unit,
                                onRange: (r) => setState(() => _outputRange = r),
                              )),
                        ),
                      ]),
                    ),
                    const SizedBox(height: 12),
                    Expanded(
                      child: Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                        Expanded(
                          flex: inFlex,
                          child: side(inputs, true,
                              (range, color) => _Faders(device: device, channels: inputs, color: color, rangeMs: range, unit: unit)),
                        ),
                        const SizedBox(width: 12),
                        Expanded(
                          flex: outFlex,
                          child: side(outputs, false,
                              (range, color) => _Faders(device: device, channels: outputs, color: color, rangeMs: range, unit: unit)),
                        ),
                      ]),
                    ),
                  ]),
                ),
              );
              return constraints.maxHeight >= 520 ? content : SingleChildScrollView(child: content);
            }),
          ),
        ]);
      },
    );
  }
}

/// Display unit: 0 = ms, 1 = m, 2 = ft (as stored on the DSP).
class _DelayUnit {
  final int unit;

  const _DelayUnit(int unit) : unit = unit < 0 || unit > 2 ? 0 : unit;

  String get suffix => ProtocolService.delayUnitNames[unit];

  /// Length of one ms of delay in this unit (1 for ms).
  double get _perMs => switch (unit) {
        1 => ProtocolService.metersPerMs,
        2 => ProtocolService.metersPerMs * ProtocolService.feetPerMeter,
        _ => 1.0,
      };

  double fromMs(double ms) => ms * _perMs;
  double toMs(double value) => value / _perMs;

  String format(double ms) {
    final v = fromMs(ms);
    return v < 10 ? v.toStringAsFixed(2) : v.toStringAsFixed(1);
  }

  String formatRange(double ms) {
    final v = fromMs(ms);
    final text = v < 1 ? v.toStringAsFixed(2) : (v < 10 ? v.toStringAsFixed(1) : v.toStringAsFixed(0));
    return '$text $suffix';
  }
}

// ─── Speakers ───

class _Stage extends StatefulWidget {
  final DeviceProvider device;
  final String title;
  final List<String> channels;
  final Color color;
  final double rangeMs;
  final double? chosenRange;
  final _DelayUnit unit;
  final ValueChanged<double?> onRange;

  const _Stage({
    required this.device,
    required this.title,
    required this.channels,
    required this.color,
    required this.rangeMs,
    required this.chosenRange,
    required this.unit,
    required this.onRange,
  });

  @override
  State<_Stage> createState() => _StageState();
}

class _StageState extends State<_Stage> {
  /// Range kept while a speaker is dragged (the automatic range would
  /// otherwise change under the pointer).
  double? _dragRange;

  /// Delay at the start of the current drag and the distance dragged since.
  double _dragStartMs = 0;
  double _dragDx = 0;

  static const _labelWidth = 76.0;
  static const _speakerSize = 26.0;

  double get _range => _dragRange ?? widget.rangeMs;

  @override
  Widget build(BuildContext context) {
    final unit = widget.unit;
    return PanelCard(
      title: widget.title,
      accent: widget.color,
      trailing: _RangeMenu(chosen: widget.chosenRange, current: widget.rangeMs, unit: unit, onSelected: widget.onRange),
      child: LayoutBuilder(builder: (context, constraints) {
        final laneWidth = constraints.maxWidth - _labelWidth - _speakerSize;
        const rulerHeight = 16.0;
        final rowHeight = math.max(24.0, (constraints.maxHeight - rulerHeight) / widget.channels.length);
        double xFor(double ms) => _labelWidth + (ms / _range).clamp(0.0, 1.0) * laneWidth;

        return Stack(children: [
          // Distance grid
          Positioned.fill(
            child: CustomPaint(
              painter: _StageGridPainter(
                left: _labelWidth + _speakerSize / 2,
                width: laneWidth,
                rangeMs: _range,
                unit: unit,
                rulerHeight: rulerHeight,
              ),
            ),
          ),
          for (var i = 0; i < widget.channels.length; i++)
            Positioned(
              left: 0,
              right: 0,
              top: i * rowHeight,
              height: rowHeight,
              child: _speakerRow(widget.channels[i], xFor, laneWidth),
            ),
        ]);
      }),
    );
  }

  Widget _speakerRow(String ch, double Function(double ms) xFor, double laneWidth) {
    final device = widget.device;
    final ms = device.getDelayMs(ch);
    final x = xFor(ms);
    return Stack(children: [
      Positioned(
        left: 0,
        width: _labelWidth - 6,
        top: 0,
        bottom: 0,
        child: Align(
          alignment: Alignment.centerLeft,
          child: Text(device.getAlias(ch),
              overflow: TextOverflow.ellipsis, style: TextStyle(fontSize: 12, color: widget.color)),
        ),
      ),
      // Lane
      Positioned(
        left: _labelWidth + _speakerSize / 2,
        width: laneWidth,
        top: 0,
        bottom: 0,
        child: Center(child: Container(height: 1, color: AppColors.border)),
      ),
      // Always present (zero width at 0 ms): inserting it would rebuild the
      // speaker below and cancel a running drag.
      Positioned(
          left: _labelWidth + _speakerSize / 2,
          width: math.max(0.0, x - _labelWidth),
          top: 0,
          bottom: 0,
          child: Center(child: Container(height: 2, color: widget.color.withAlpha(120))),
        ),
      Positioned(
        left: x,
        width: _speakerSize,
        top: 0,
        bottom: 0,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onHorizontalDragStart: (_) => setState(() {
            _dragRange = widget.rangeMs;
            _dragStartMs = device.getDelayMs(ch);
            _dragDx = 0;
          }),
          onHorizontalDragUpdate: (d) {
            // Relative to the start of the drag, so small steps are not lost
            // to the 1/96 ms resolution.
            _dragDx += d.delta.dx;
            final next = (_dragStartMs + _dragDx / laneWidth * _range).clamp(0.0, math.min(_range, _maxMs)).toDouble();
            device.setDelayMs(ch, next);
          },
          onHorizontalDragEnd: (_) => setState(() => _dragRange = null),
          onDoubleTap: () => device.setDelayMs(ch, 0),
          child: MouseRegion(
            cursor: SystemMouseCursors.resizeLeftRight,
            child: Tooltip(
              message: '${device.getAlias(ch)}: ${widget.unit.format(ms)} ${widget.unit.suffix}',
              waitDuration: const Duration(milliseconds: 500),
              child: Center(
                child: Container(
                  width: _speakerSize,
                  height: _speakerSize,
                  decoration: BoxDecoration(
                    color: ms > 0 ? widget.color : AppColors.surfaceHigh,
                    borderRadius: BorderRadius.circular(6),
                    border: Border.all(color: widget.color),
                    boxShadow: const [BoxShadow(color: Colors.black45, blurRadius: 4, offset: Offset(0, 2))],
                  ),
                  child: Icon(Icons.speaker, size: 18, color: ms > 0 ? AppColors.background : widget.color),
                ),
              ),
            ),
          ),
        ),
      ),
    ]);
  }
}

class _StageGridPainter extends CustomPainter {
  final double left;
  final double width;
  final double rangeMs;
  final _DelayUnit unit;
  final double rulerHeight;

  _StageGridPainter({
    required this.left,
    required this.width,
    required this.rangeMs,
    required this.unit,
    required this.rulerHeight,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final label = TextPainter(textDirection: TextDirection.ltr);
    final bottom = size.height - rulerHeight;
    for (var i = 0; i <= 10; i++) {
      final x = left + width * i / 10;
      canvas.drawLine(Offset(x, 0), Offset(x, bottom),
          Paint()
            ..color = i % 5 == 0 ? AppColors.border : AppColors.border.withAlpha(110)
            ..strokeWidth = 1);
      if (i % 2 != 0) continue;
      final v = unit.fromMs(rangeMs * i / 10);
      label
        ..text = TextSpan(
            text: v < 10 ? v.toStringAsFixed(v == 0 ? 0 : (v < 1 ? 2 : 1)) : v.toStringAsFixed(0),
            style: monoValue.copyWith(fontSize: 9, color: AppColors.muted))
        ..layout();
      label.paint(canvas, Offset((x - label.width / 2).clamp(0.0, size.width - label.width), bottom + 3));
    }
  }

  @override
  bool shouldRepaint(covariant _StageGridPainter old) =>
      old.rangeMs != rangeMs || old.unit.unit != unit.unit || old.width != width;
}

class _RangeMenu extends StatelessWidget {
  final double? chosen;
  final double current;
  final _DelayUnit unit;
  final ValueChanged<double?> onSelected;

  const _RangeMenu({required this.chosen, required this.current, required this.unit, required this.onSelected});

  @override
  Widget build(BuildContext context) {
    return PopupMenuButton<double>(
      tooltip: 'Range of the speakers and faders',
      onSelected: (r) => onSelected(r < 0 ? null : r),
      itemBuilder: (context) => [
        PopupMenuItem(value: -1, child: Text('Auto', style: TextStyle(color: chosen == null ? AppColors.accent : null))),
        for (final r in _ranges)
          PopupMenuItem(
            value: r,
            child: Text(unit.formatRange(r), style: TextStyle(color: chosen == r ? AppColors.accent : null)),
          ),
      ],
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        decoration: BoxDecoration(
          color: AppColors.backgroundDeep,
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: AppColors.border),
        ),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          const Icon(Icons.zoom_in, size: 14, color: AppColors.muted),
          const SizedBox(width: 4),
          Text('${chosen == null ? 'Auto · ' : ''}${unit.formatRange(current)}',
              style: monoValue.copyWith(fontSize: 11, color: AppColors.text)),
          const Icon(Icons.arrow_drop_down, size: 16, color: AppColors.muted),
        ]),
      ),
    );
  }
}

// ─── Faders ───

class _Faders extends StatelessWidget {
  final DeviceProvider device;
  final List<String> channels;
  final Color color;
  final double rangeMs;
  final _DelayUnit unit;

  const _Faders({
    required this.device,
    required this.channels,
    required this.color,
    required this.rangeMs,
    required this.unit,
  });

  @override
  Widget build(BuildContext context) {
    final marks = [for (var i = 0; i <= 10; i++) rangeMs * i / 10];
    return PanelCard(
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final ch in channels)
            Expanded(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 3),
                child: Column(children: [
                  Text(device.getAlias(ch),
                      overflow: TextOverflow.ellipsis,
                      style: interWeight(TextStyle(fontSize: 12, color: color), FontWeight.w600)),
                  const SizedBox(height: 4),
                  Expanded(
                    child: ConsoleFader(
                      value: device.getDelayMs(ch).clamp(0.0, rangeMs),
                      taper: LinearTaper(0, rangeMs),
                      marks: marks,
                      majorMarks: const [0.0],
                      markLabel: (ms) {
                        final i = (ms / rangeMs * 10).round();
                        return i % 5 == 0 ? unit.format(ms) : '';
                      },
                      color: color,
                      wheelStep: rangeMs / 200,
                      resetValue: 0,
                      onChanged: (ms) => device.setDelayMs(ch, ms.clamp(0.0, _maxMs)),
                    ),
                  ),
                  const SizedBox(height: 6),
                  ValueField(
                    text: unit.format(device.getDelayMs(ch)),
                    tooltip: 'Delay in ${unit.suffix} (max. ${unit.formatRange(_maxMs)})',
                    onSubmitted: (input) {
                      final v = parseNumber(input);
                      if (v == null || v < 0) return false;
                      // A typed unit wins over the shown one ("3 m", "10 ms", "8 ft").
                      final typed = RegExp(r'\d\s*ms').hasMatch(input)
                          ? 0
                          : RegExp(r'\d\s*ft').hasMatch(input)
                              ? 2
                              : RegExp(r'\d\s*m').hasMatch(input)
                                  ? 1
                                  : unit.unit;
                      device.setDelayMs(ch, _DelayUnit(typed).toMs(v).clamp(0.0, _maxMs));
                      return true;
                    },
                  ),
                  const SizedBox(height: 2),
                  Text(unit.suffix, style: const TextStyle(fontSize: 10, color: AppColors.muted)),
                ]),
              ),
            ),
        ],
      ),
    );
  }
}
