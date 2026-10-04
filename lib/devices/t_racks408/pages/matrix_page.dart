// Matrix: which inputs feed which output, filling the whole page. Inputs
// are the columns (top), outputs the rows (side). Tap a crossing to toggle
// it. The crossing gain is read from the device and only shown: the
// command to set it is not known yet (see TODO.md).
//
// Linked channels do not share their routing (left/right pairs are fed
// from different inputs).

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';

class MatrixPage extends StatelessWidget {
  final DeviceProvider device;

  const MatrixPage({super.key, required this.device});

  static const _labelWidth = 110.0;
  static const _headerHeight = 40.0;
  static const _gap = 6.0;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: device,
      builder: (context, _) => LayoutBuilder(builder: (context, constraints) {
        final inputs = device.inputs;
        final outputs = device.outputs;
        // Fill the page, but keep the cells usable on small windows.
        final width = math.max(constraints.maxWidth - 24, _labelWidth + inputs.length * 90.0);
        final height = math.max(constraints.maxHeight - 24, _headerHeight + outputs.length * 44.0);
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

        return Scrollbar(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(12),
            child: SingleChildScrollView(scrollDirection: Axis.horizontal, child: grid),
          ),
        );
      }),
    );
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

  @override
  Widget build(BuildContext context) {
    final enabled = device.getMatrixEnabled(output, input);
    final gain = device.getMatrixGain(output, input);
    final gainText = '${gain > 0 ? '+' : ''}${gain.toStringAsFixed(1)} dB';
    return Tooltip(
      message: '${device.getAlias(input)} → ${device.getAlias(output)}\n'
          'Crossing gain $gainText (read-only, see TODO.md)',
      waitDuration: const Duration(milliseconds: 700),
      child: InkWell(
        borderRadius: BorderRadius.circular(10),
        onTap: () => device.toggleMatrixInput(output, input),
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 120),
          width: width,
          height: height,
          decoration: BoxDecoration(
            color: enabled ? AppColors.accent.withAlpha(40) : AppColors.surface,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: enabled ? AppColors.accent : AppColors.border, width: enabled ? 1.6 : 1),
          ),
          child: Stack(children: [
            Center(
              child: Icon(enabled ? Icons.check_circle : Icons.circle_outlined,
                  size: math.min(28, height * 0.5), color: enabled ? AppColors.accent : AppColors.border),
            ),
            if (height >= 40)
              Positioned(
                right: 8,
                bottom: 4,
                child: Text(gainText,
                    style: monoValue.copyWith(
                        fontSize: 10, color: gain == 0 ? AppColors.muted.withAlpha(140) : AppColors.warning)),
              ),
          ]),
        ),
      ),
    );
  }
}
