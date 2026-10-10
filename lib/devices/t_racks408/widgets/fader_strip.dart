// Fader groups of the gain page: one console fader per channel with dB
// scale and meter, a field to type the gain, mute and phase.

import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';
import 'channel_menu.dart';
import 'console_fader.dart';
import 'value_field.dart';

/// Inner horizontal padding of a fader group (each side).
const faderGroupPadding = 6.0;

/// Inputs or outputs: a titled box with one fader per channel, separated
/// by thin lines.
class FaderGroup extends StatelessWidget {
  final DeviceProvider device;
  final String title;
  final Color color;
  final List<String> channels;

  const FaderGroup({
    super.key,
    required this.device,
    required this.title,
    required this.color,
    required this.channels,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: AppColors.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border(top: BorderSide(color: color, width: 2)),
      ),
      padding: const EdgeInsets.fromLTRB(faderGroupPadding, 8, faderGroupPadding, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(left: 8, bottom: 6),
            child: Text(title.toUpperCase(),
                style: interWeight(TextStyle(fontSize: 10, letterSpacing: 1.2, color: color), FontWeight.w600)),
          ),
          Expanded(
            child: Row(
              children: [
                for (var i = 0; i < channels.length; i++) ...[
                  if (i > 0)
                    Container(
                      width: 1,
                      margin: const EdgeInsets.symmetric(vertical: 6),
                      color: AppColors.border,
                    ),
                  Expanded(child: ChannelFader(device: device, channel: channels[i], color: color)),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class ChannelFader extends StatelessWidget {
  final DeviceProvider device;
  final String channel;
  final Color color;

  /// Show the channel name (opens the channel menu).
  final bool showName;

  const ChannelFader({
    super.key,
    required this.device,
    required this.channel,
    required this.color,
    this.showName = true,
  });

  @override
  Widget build(BuildContext context) {
    final gain = device.getInputGain(channel);
    final muted = device.getInputMute(channel);
    final inverted = device.getInputPhase(channel);
    final linked = device.linkedWith(channel).isNotEmpty;
    final master = device.linkMaster(channel);

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 3),
      child: Column(
        children: [
          if (showName) ...[
            InkWell(
              borderRadius: BorderRadius.circular(6),
              onTap: () => showChannelMenu(context, device, channel),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    Flexible(
                      child: Text(device.getAlias(channel),
                          overflow: TextOverflow.ellipsis,
                          style: interWeight(const TextStyle(fontSize: 12, color: AppColors.text), FontWeight.w600)),
                    ),
                    if (linked)
                      Tooltip(
                        message: master != null
                            ? 'Linked to ${device.getAlias(master)}'
                            : 'Master of ${device.linkedWith(channel).map(device.getAlias).join(', ')}',
                        child: Icon(Icons.link, size: 13, color: color),
                      ),
                    const Icon(Icons.expand_more, size: 14, color: AppColors.muted),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 2),
          ],
          Expanded(
            // Only the fader listens to the meters (updated every 25–40 ms), on
            // its own layer so the rest of the page is not repainted.
            child: RepaintBoundary(
              child: ListenableBuilder(
              listenable: device.meters,
              builder: (context, _) => ConsoleFader(
                value: gain,
                taper: gainTaper,
                marks: gainMarks,
                majorMarks: const [0.0],
                markLabel: gainMarkLabel,
                color: color,
                // Meter in dBFS on the fader's dB scale (0 dBFS at the 0 mark).
                meterLevel: gainTaper.toPosition(device.getMeterDb(channel)),
                meterPeak: gainTaper.toPosition(device.getMeterPeakDb(channel)),
                clip: device.isClipping(channel),
                // Limiter status as reported by the DSP.
                limit: device.outputs.contains(channel) ? device.isLimiting(channel) : null,
                wheelStep: gain < -20 ? 0.5 : 0.1,
                resetValue: 0,
                onChanged: (v) => device.setGain(channel, device.quantizeGain(v.clamp(-60.0, 12.0))),
              ),
            ),
            ),
          ),
          const SizedBox(height: 6),
          ValueField(
            text: formatDbValue(gain),
            tooltip: 'Gain in dB (-60 … +12)',
            onSubmitted: (input) {
              final v = parseNumber(input);
              if (v == null) return false;
              device.setGain(channel, device.quantizeGain(v.clamp(-60.0, 12.0)));
              return true;
            },
          ),
          const SizedBox(height: 6),
          Row(
            children: [
              Expanded(
                child: _ToggleButton(
                  label: 'M',
                  tooltip: muted ? 'Unmute' : 'Mute',
                  active: muted,
                  activeColor: AppColors.danger,
                  onTap: () => device.toggleMute(channel),
                ),
              ),
              const SizedBox(width: 4),
              Expanded(
                child: _ToggleButton(
                  label: 'Ø',
                  tooltip: inverted ? 'Phase inverted' : 'Invert phase',
                  active: inverted,
                  activeColor: AppColors.warning,
                  onTap: () => device.togglePhase(channel),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _ToggleButton extends StatelessWidget {
  final String label;
  final String tooltip;
  final bool active;
  final Color activeColor;
  final VoidCallback onTap;

  const _ToggleButton({
    required this.label,
    required this.tooltip,
    required this.active,
    required this.activeColor,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: tooltip,
      child: InkWell(
        borderRadius: BorderRadius.circular(6),
        onTap: onTap,
        child: Container(
          height: 28,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: active ? activeColor : AppColors.surfaceHigh,
            borderRadius: BorderRadius.circular(6),
            border: Border.all(color: active ? activeColor : AppColors.border),
          ),
          child: Text(label,
              style: interWeight(
                  TextStyle(fontSize: 12, color: active ? AppColors.background : AppColors.text), FontWeight.w700)),
        ),
      ),
    );
  }
}
