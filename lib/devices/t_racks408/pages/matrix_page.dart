// Matrix: which inputs feed which output.
//
// Left: routing diagram (inputs on the left, outputs on the right, one
// curve per active crossing, coloured by input). Right: the crossing grid,
// inputs are the columns, outputs the rows; tap a crossing to toggle it and
// type its gain (−60 … 0 dB, cmd 0x41) into the field of the cell.
// On narrow windows the diagram sits above the grid.
//
// Linked channels do not share their routing or crossing gains (left/right
// pairs are fed from different inputs).

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';
import '../widgets/ui_common.dart';
import '../widgets/value_field.dart';

class MatrixPage extends StatelessWidget {
  final DeviceProvider device;

  const MatrixPage({super.key, required this.device});

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: device,
      builder: (context, _) => LayoutBuilder(builder: (context, constraints) {
        final routing = PanelCard(title: 'Routing', child: _RoutingDiagram(device: device));
        final matrix = PanelCard(
          title: 'Matrix',
          trailing: const Text('tap to toggle · type the gain in dB',
              style: TextStyle(fontSize: 11, color: AppColors.muted)),
          child: _MatrixGrid(device: device),
        );
        if (constraints.maxWidth > 900) {
          return Padding(
            padding: const EdgeInsets.all(12),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Expanded(flex: 5, child: routing),
                const SizedBox(width: 12),
                Expanded(flex: 4, child: matrix),
              ],
            ),
          );
        }
        return ListView(
          padding: const EdgeInsets.all(12),
          children: [
            SizedBox(height: 360, child: routing),
            const SizedBox(height: 12),
            // Rows high enough for the gain field in every cell.
            SizedBox(height: math.max(320, 70 + device.outputs.length * 62.0), child: matrix),
          ],
        );
      }),
    );
  }
}

// ─── Routing diagram ───

class _RoutingDiagram extends StatelessWidget {
  final DeviceProvider device;

  const _RoutingDiagram({required this.device});

  static const _nodeWidth = 92.0;
  static const _nodeHeight = 30.0;

  @override
  Widget build(BuildContext context) {
    final inputs = device.inputs;
    final outputs = device.outputs;
    return LayoutBuilder(builder: (context, constraints) {
      final size = Size(constraints.maxWidth, constraints.maxHeight);
      double y(int i, int n) => size.height * (i + 0.5) / n;
      final inPoints = [for (var i = 0; i < inputs.length; i++) Offset(_nodeWidth, y(i, inputs.length))];
      final outPoints = [
        for (var o = 0; o < outputs.length; o++) Offset(size.width - _nodeWidth, y(o, outputs.length)),
      ];

      final routes = <_Route>[];
      for (var o = 0; o < outputs.length; o++) {
        for (var i = 0; i < inputs.length; i++) {
          if (!device.getMatrixEnabled(outputs[o], inputs[i])) continue;
          routes.add(_Route(inPoints[i], outPoints[o], AppColors.curves[i % AppColors.curves.length],
              device.getMatrixGain(outputs[o], inputs[i])));
        }
      }

      Widget node(String ch, Offset at, bool isInput) {
        final color = AppColors.forChannel(isInput);
        final connected = isInput
            ? outputs.any((o) => device.getMatrixEnabled(o, ch))
            : inputs.any((i) => device.getMatrixEnabled(ch, i));
        return Positioned(
          left: isInput ? 0 : at.dx,
          top: at.dy - _nodeHeight / 2,
          width: _nodeWidth,
          height: _nodeHeight,
          child: Container(
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: connected ? color.withAlpha(36) : AppColors.surfaceHigh,
              borderRadius: BorderRadius.circular(8),
              border: Border.all(color: connected ? color : AppColors.border),
            ),
            child: Text(device.getAlias(ch),
                overflow: TextOverflow.ellipsis,
                style: interWeight(
                    TextStyle(fontSize: 12, color: connected ? AppColors.text : AppColors.muted), FontWeight.w600)),
          ),
        );
      }

      return Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned.fill(child: CustomPaint(painter: _RoutingPainter(routes))),
          for (var i = 0; i < inputs.length; i++) node(inputs[i], inPoints[i], true),
          for (var o = 0; o < outputs.length; o++) node(outputs[o], outPoints[o], false),
        ],
      );
    });
  }
}

class _Route {
  final Offset from;
  final Offset to;
  final Color color;
  final double gainDb;

  _Route(this.from, this.to, this.color, this.gainDb);
}

class _RoutingPainter extends CustomPainter {
  final List<_Route> routes;

  _RoutingPainter(this.routes);

  @override
  void paint(Canvas canvas, Size size) {
    final label = TextPainter(textDirection: TextDirection.ltr);
    for (final r in routes) {
      final dx = (r.to.dx - r.from.dx) * 0.45;
      final path = Path()
        ..moveTo(r.from.dx, r.from.dy)
        ..cubicTo(r.from.dx + dx, r.from.dy, r.to.dx - dx, r.to.dy, r.to.dx, r.to.dy);
      canvas.drawPath(
        path,
        Paint()
          ..color = r.color.withAlpha(200)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2,
      );
      final dot = Paint()..color = r.color;
      canvas.drawCircle(r.to, 3.5, dot);
      canvas.drawCircle(r.from, 3.5, dot);
      if (r.gainDb != 0) {
        label
          ..text = TextSpan(
              text: '${r.gainDb > 0 ? '+' : ''}${r.gainDb.toStringAsFixed(1)}',
              style: monoValue.copyWith(fontSize: 10, color: AppColors.warning))
          ..layout();
        label.paint(canvas, Offset(r.to.dx - 34, r.to.dy - 14));
      }
    }
  }

  @override
  bool shouldRepaint(covariant _RoutingPainter oldDelegate) => true;
}

// ─── Crossing grid ───

class _MatrixGrid extends StatelessWidget {
  final DeviceProvider device;

  const _MatrixGrid({required this.device});

  static const _labelWidth = 96.0;
  static const _headerHeight = 32.0;
  static const _gap = 6.0;

  @override
  Widget build(BuildContext context) {
    final inputs = device.inputs;
    final outputs = device.outputs;
    return LayoutBuilder(builder: (context, constraints) {
      // Fill the card, but keep the cells usable on small windows.
      final width = math.max(constraints.maxWidth, _labelWidth + inputs.length * 64.0);
      final height = math.max(constraints.maxHeight, _headerHeight + outputs.length * 36.0);
      final cellWidth = (width - _labelWidth) / inputs.length - _gap;
      final cellHeight = (height - _headerHeight - _gap) / outputs.length - _gap;

      final grid = SizedBox(
        width: width,
        height: height,
        child: Column(children: [
          SizedBox(
            height: _headerHeight,
            child: Row(children: [
              const SizedBox(
                width: _labelWidth,
                child: Align(
                  alignment: Alignment.bottomLeft,
                  child: Text('OUT \\ IN', style: TextStyle(fontSize: 10, letterSpacing: 1, color: AppColors.muted)),
                ),
              ),
              for (final i in inputs)
                Container(
                  width: cellWidth,
                  margin: const EdgeInsets.only(right: _gap),
                  alignment: Alignment.center,
                  decoration: const BoxDecoration(border: Border(bottom: BorderSide(color: AppColors.input, width: 2))),
                  child: Text(device.getAlias(i),
                      overflow: TextOverflow.ellipsis,
                      style: interWeight(const TextStyle(fontSize: 13, color: AppColors.input), FontWeight.w600)),
                ),
            ]),
          ),
          const SizedBox(height: _gap),
          for (final o in outputs)
            Padding(
              padding: const EdgeInsets.only(bottom: _gap),
              child: SizedBox(
                height: cellHeight,
                child: Row(children: [
                  Container(
                    width: _labelWidth - _gap,
                    margin: const EdgeInsets.only(right: _gap),
                    padding: const EdgeInsets.only(left: 10),
                    alignment: Alignment.centerLeft,
                    decoration: const BoxDecoration(border: Border(left: BorderSide(color: AppColors.output, width: 2))),
                    child: Row(children: [
                      Flexible(
                        child: Text(device.getAlias(o),
                            overflow: TextOverflow.ellipsis,
                            style: interWeight(const TextStyle(fontSize: 13, color: AppColors.output), FontWeight.w600)),
                      ),
                      if (device.linkedWith(o).isNotEmpty) ...[
                        const SizedBox(width: 4),
                        const Icon(Icons.link, size: 13, color: AppColors.output),
                      ],
                    ]),
                  ),
                  for (final i in inputs)
                    Padding(
                      padding: const EdgeInsets.only(right: _gap),
                      child: _Cell(device: device, input: i, output: o, width: cellWidth, height: cellHeight),
                    ),
                ]),
              ),
            ),
        ]),
      );

      if (width <= constraints.maxWidth && height <= constraints.maxHeight) return grid;
      return Scrollbar(
        child: SingleChildScrollView(
          child: SingleChildScrollView(scrollDirection: Axis.horizontal, child: grid),
        ),
      );
    });
  }
}

class _Cell extends StatelessWidget {
  final DeviceProvider device;
  final String input;
  final String output;
  final double width;
  final double height;

  const _Cell({
    required this.device,
    required this.input,
    required this.output,
    required this.width,
    required this.height,
  });

  /// Smallest cell that has room for the gain field.
  static const _fieldMinHeight = 46.0;
  static const _fieldMinWidth = 58.0;

  @override
  Widget build(BuildContext context) {
    final enabled = device.getMatrixEnabled(output, input);
    final gain = device.getMatrixGain(output, input);
    final color = AppColors.curves[device.inputs.indexOf(input) % AppColors.curves.length];
    final editable = height >= _fieldMinHeight && width >= _fieldMinWidth;
    Widget icon(double size) => Icon(enabled ? Icons.check_circle : Icons.circle_outlined,
        size: size, color: enabled ? color : AppColors.border);
    return Tooltip(
      message: '${device.getAlias(input)} → ${device.getAlias(output)}\n'
          'Crossing gain ${formatDbValue(gain)} dB',
      waitDuration: const Duration(milliseconds: 700),
      child: InkWell(
        borderRadius: BorderRadius.circular(10),
        onTap: () => device.toggleMatrixInput(output, input),
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 120),
          width: width,
          height: height,
          decoration: BoxDecoration(
            color: enabled ? color.withAlpha(40) : AppColors.surfaceHigh,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: enabled ? color : AppColors.border, width: enabled ? 1.6 : 1),
          ),
          child: editable
              ? Column(children: [
                  Expanded(child: Center(child: icon(math.min(24, (height - 28) * 0.8)))),
                  Padding(
                    padding: const EdgeInsets.fromLTRB(4, 0, 4, 4),
                    child: ValueField(
                      text: formatDbValue(gain),
                      width: math.min(width - 8, 72),
                      height: 22,
                      fontSize: 10,
                      color: gain == 0 ? AppColors.muted : AppColors.warning,
                      tooltip: 'Crossing gain in dB (-60 … 0)',
                      onSubmitted: (text) {
                        final v = parseNumber(text);
                        if (v == null) return false;
                        device.setMatrixGain(output, input, v.clamp(-60.0, 0.0));
                        return true;
                      },
                    ),
                  ),
                ])
              : Center(child: icon(math.min(26, height * 0.5))),
        ),
      ),
    );
  }
}
