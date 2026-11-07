// Gain page: EQ overview on top, the faders below. The input and output EQ
// blocks have exactly the width of the fader blocks below them; both scroll
// together if the window is too narrow.

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../../services/local_settings.dart';
import '../../../theme/app_colors.dart';
import '../providers/device_provider.dart';
import '../widgets/eq_overview.dart';
import '../widgets/fader_strip.dart';

class HomePage extends StatefulWidget {
  final DeviceProvider device;
  final ValueChanged<String> onOpenEq;

  const HomePage({super.key, required this.device, required this.onOpenEq});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  /// Gap between the four blocks (EQ and faders of inputs and outputs),
  /// the same horizontally and vertically.
  static const _groupGap = 12.0;
  static const _pagePadding = 12.0;
  static const _minFaderWidth = 84.0;
  static const _maxFaderWidth = 200.0;
  static const _minContentHeight = 440.0;

  /// Hidden EQ curves, remembered per browser and DSP.
  late Set<String> _hidden = LocalSettings.getStringSet(_settingsKey);

  String get _settingsKey => 'hiddenEqCurves.${widget.device.dspId}';

  void _toggle(String channel) {
    setState(() {
      _hidden = {..._hidden};
      if (!_hidden.remove(channel)) _hidden.add(channel);
    });
    LocalSettings.setStringSet(_settingsKey, _hidden);
  }

  @override
  Widget build(BuildContext context) {
    final device = widget.device;
    return ListenableBuilder(
      listenable: device,
      builder: (context, _) => LayoutBuilder(builder: (context, constraints) {
        final inputs = device.inputs;
        final outputs = device.outputs;
        const groupExtra = 2 * faderGroupPadding;
        final available = constraints.maxWidth - 2 * _pagePadding - _groupGap - 2 * groupExtra;
        final faderWidth = (available / (inputs.length + outputs.length)).clamp(_minFaderWidth, _maxFaderWidth);
        final inWidth = inputs.length * faderWidth + groupExtra;
        final outWidth = outputs.length * faderWidth + groupExtra;
        final height = constraints.maxHeight;

        Widget pair(Widget left, Widget right) => Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(width: inWidth, child: left),
                const SizedBox(width: _groupGap),
                SizedBox(width: outWidth, child: right),
              ],
            );

        // Only scroll vertically when the window is too low: a vertical
        // scroll view would compete with the faders for drags.
        final contentHeight = height - 2 * _pagePadding;
        final tooLow = contentHeight < _minContentHeight;
        final eqHeight = math.max(170.0, math.max(contentHeight, _minContentHeight) * 0.38);
        final page = Scrollbar(
          child: SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.all(_pagePadding),
            child: SizedBox(
              width: inWidth + _groupGap + outWidth,
              height: tooLow ? _minContentHeight : contentHeight,
              child: Column(
                children: [
                  SizedBox(
                    height: eqHeight,
                    child: pair(
                      EqGroupGraph(
                        device: device,
                        title: 'Inputs',
                        accent: AppColors.input,
                        channels: inputs,
                        hidden: _hidden,
                        onToggle: _toggle,
                        onOpenEq: widget.onOpenEq,
                      ),
                      EqGroupGraph(
                        device: device,
                        title: 'Outputs',
                        accent: AppColors.output,
                        channels: outputs,
                        hidden: _hidden,
                        onToggle: _toggle,
                        onOpenEq: widget.onOpenEq,
                      ),
                    ),
                  ),
                  const SizedBox(height: _groupGap),
                  Expanded(
                    child: pair(
                      FaderGroup(device: device, title: 'Inputs', color: AppColors.input, channels: inputs),
                      FaderGroup(device: device, title: 'Outputs', color: AppColors.output, channels: outputs),
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
        return tooLow ? SingleChildScrollView(child: page) : page;
      }),
    );
  }
}
