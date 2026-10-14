// Controls of the test signal generator (RTA page and Auto-EQ).

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../../services/tone_generator.dart';
import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';
import 'eq_math.dart';
import 'value_field.dart';

/// The DSP's own signal generator (cmd 0x39 like the original editor; not
/// tested on a device). "Analog input" switches it off.
class DspGeneratorPanel extends StatelessWidget {
  final DeviceProvider device;

  const DspGeneratorPanel({super.key, required this.device});

  static bool _confirmed = false;

  /// '20 Hz', '31.5 Hz', '1.25 kHz'
  static String _isoLabel(double hz) {
    String trim(double v) => v.toString().replaceAll(RegExp(r'\.0$'), '');
    return hz >= 1000 ? '${trim(hz / 1000)} kHz' : '${trim(hz)} Hz';
  }

  Future<void> _set(BuildContext context, int type) async {
    if (type != 0 && device.testToneType == 0 && !_confirmed) {
      final ok = await showDialog<bool>(
        context: context,
        builder: (dialog) => AlertDialog(
          title: const Text('Start the DSP generator?'),
          content: const SizedBox(
            width: 380,
            child: Text(
              'The DSP plays the test signal itself – depending on gains and routing it can reach the speakers '
              'at full level. Turn the outputs down first.\n\n'
              'The command is the one of the original editor but has not been tested on a device.',
            ),
          ),
          actions: [
            TextButton(onPressed: () => Navigator.of(dialog).pop(false), child: const Text('Cancel')),
            FilledButton(onPressed: () => Navigator.of(dialog).pop(true), child: const Text('Start')),
          ],
        ),
      );
      if (ok != true) return;
      _confirmed = true;
    }
    device.setTestTone(type);
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: device,
      builder: (context, _) {
        final type = device.testToneType;
        final param = device.testToneParam.clamp(0, geqCenterFrequencies.length - 1);
        return Wrap(
          spacing: 12,
          runSpacing: 8,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            SegmentedButton<int>(
              style: const ButtonStyle(visualDensity: VisualDensity.compact),
              showSelectedIcon: false,
              segments: [
                for (var t = 0; t < ProtocolService.testToneTypeNames.length; t++)
                  ButtonSegment(value: t, label: Text(ProtocolService.testToneTypeNames[t])),
              ],
              selected: {type},
              onSelectionChanged: (s) => _set(context, s.first),
            ),
            if (type == 3)
              Row(mainAxisSize: MainAxisSize.min, children: [
                const Text('Freq', style: TextStyle(fontSize: 12, color: AppColors.muted)),
                const SizedBox(width: 6),
                DropdownButton<int>(
                  value: param,
                  isDense: true,
                  dropdownColor: AppColors.surfaceHigh,
                  style: monoValue.copyWith(fontSize: 12, color: AppColors.text),
                  items: [
                    for (var i = 0; i < geqCenterFrequencies.length; i++)
                      DropdownMenuItem(value: i, child: Text(_isoLabel(geqCenterFrequencies[i]))),
                  ],
                  onChanged: (i) {
                    if (i != null) device.setTestTone(3, i);
                  },
                ),
                const SizedBox(width: 6),
                const Tooltip(
                  message: 'The 31 ISO 1/3-octave frequencies of the original editor',
                  child: Icon(Icons.info_outline, size: 14, color: AppColors.muted),
                ),
              ]),
            if (type != 0)
              const Text('generator running', style: TextStyle(fontSize: 12, color: AppColors.warning)),
          ],
        );
      },
    );
  }
}

class ToneGeneratorPanel extends StatelessWidget {
  /// Show the frequency control (hidden for Auto-EQ, which uses pink noise).
  final bool showFrequency;

  const ToneGeneratorPanel({super.key, this.showFrequency = true});

  @override
  Widget build(BuildContext context) {
    final gen = ToneGenerator.instance;
    return ListenableBuilder(
      listenable: gen,
      builder: (context, _) {
        if (!gen.isSupported) {
          return const Text('The signal generator is only available in the browser.',
              style: TextStyle(fontSize: 12, color: AppColors.muted));
        }
        return Wrap(
          spacing: 12,
          runSpacing: 8,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            FilledButton.icon(
              style: FilledButton.styleFrom(
                backgroundColor: gen.isPlaying ? AppColors.danger : AppColors.warning,
                foregroundColor: AppColors.backgroundDeep,
                visualDensity: VisualDensity.compact,
              ),
              icon: Icon(gen.isPlaying ? Icons.stop : Icons.volume_up, size: 18),
              label: Text(gen.isPlaying ? 'Stop signal' : 'Play signal'),
              onPressed: gen.toggle,
            ),
            SegmentedButton<ToneSignal>(
              style: const ButtonStyle(visualDensity: VisualDensity.compact),
              showSelectedIcon: false,
              segments: [
                for (final s in ToneSignal.values) ButtonSegment(value: s, label: Text(s.label)),
              ],
              selected: {gen.signal},
              onSelectionChanged: (s) => gen.setSignal(s.first),
            ),
            if (showFrequency && gen.signal == ToneSignal.sine)
              Row(mainAxisSize: MainAxisSize.min, children: [
                const Text('Freq', style: TextStyle(fontSize: 12, color: AppColors.muted)),
                SizedBox(
                  width: 180,
                  child: Slider(
                    value: math.log(gen.frequency / 20) / math.log(1000),
                    onChanged: (v) => gen.setFrequency(20 * math.pow(1000, v).toDouble()),
                  ),
                ),
                ValueField(
                  text: formatHz(gen.frequency),
                  width: 72,
                  onSubmitted: (input) {
                    final hz = parseFrequency(input);
                    if (hz == null || hz <= 0) return false;
                    gen.setFrequency(hz);
                    return true;
                  },
                ),
              ]),
            Row(mainAxisSize: MainAxisSize.min, children: [
              const Text('Level', style: TextStyle(fontSize: 12, color: AppColors.muted)),
              SizedBox(
                width: 160,
                child: Slider(
                  min: ToneGenerator.minLevelDb,
                  max: ToneGenerator.maxLevelDb,
                  value: gen.levelDb,
                  onChanged: gen.setLevel,
                ),
              ),
              Text('${gen.levelDb.round()} dBFS', style: monoValue.copyWith(fontSize: 12)),
            ]),
          ],
        );
      },
    );
  }
}
