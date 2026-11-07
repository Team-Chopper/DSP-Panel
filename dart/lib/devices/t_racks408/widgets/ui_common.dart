// Small building blocks shared by the DSP pages.

import 'package:flutter/material.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';
import '../providers/device_provider.dart';

/// Card with a small title row.
class PanelCard extends StatelessWidget {
  final String? title;
  final Color? accent;
  final Widget? trailing;
  final Widget child;
  final EdgeInsetsGeometry padding;

  const PanelCard({
    super.key,
    this.title,
    this.accent,
    this.trailing,
    required this.child,
    this.padding = const EdgeInsets.fromLTRB(12, 10, 12, 12),
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: AppColors.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: AppColors.border),
      ),
      padding: padding,
      // In a scroll view the height is unbounded and the child takes its
      // natural height; otherwise it fills the card.
      child: LayoutBuilder(builder: (context, constraints) => _content(constraints.hasBoundedHeight)),
    );
  }

  Widget _content(bool bounded) {
    return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (title != null)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Row(children: [
                if (accent != null) ...[
                  Container(width: 8, height: 8, decoration: BoxDecoration(color: accent, shape: BoxShape.circle)),
                  const SizedBox(width: 8),
                ],
                Expanded(
                  child: Text(title!,
                      style: interWeight(const TextStyle(fontSize: 13, color: AppColors.text), FontWeight.w600)),
                ),
                ?trailing,
              ]),
            ),
          if (bounded) Flexible(child: child) else child,
        ],
      );
  }
}

/// Single-select channel chips; inputs and outputs are visually separated.
class ChannelSelector extends StatelessWidget {
  final DeviceProvider device;
  final String selected;
  final ValueChanged<String> onSelected;
  final bool showInputs;
  final bool showOutputs;

  const ChannelSelector({
    super.key,
    required this.device,
    required this.selected,
    required this.onSelected,
    this.showInputs = true,
    this.showOutputs = true,
  });

  @override
  Widget build(BuildContext context) {
    // Channels linked with the selected one are highlighted as well: a
    // change on this page applies to all of them.
    final partners = device.linkedWith(selected).toSet();
    Widget chip(String ch) {
      final isInput = device.inputs.contains(ch);
      final color = AppColors.forChannel(isInput);
      final active = ch == selected;
      final partner = partners.contains(ch);
      final linked = device.linkedWith(ch).isNotEmpty;
      final master = device.linkMaster(ch);
      final chipWidget = Padding(
        padding: const EdgeInsets.only(right: 6),
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: () => onSelected(ch),
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 120),
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 7),
            decoration: BoxDecoration(
              color: active
                  ? color.withAlpha(40)
                  : partner
                      ? color.withAlpha(18)
                      : AppColors.surfaceHigh,
              borderRadius: BorderRadius.circular(8),
              border: Border.all(
                color: active ? color : (partner ? color.withAlpha(150) : AppColors.border),
              ),
            ),
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              Text(device.getAlias(ch),
                  style: interWeight(
                      TextStyle(fontSize: 12, color: active || partner ? color : AppColors.text),
                      active ? FontWeight.w600 : FontWeight.w400)),
              if (linked) ...[
                const SizedBox(width: 4),
                Icon(Icons.link, size: 12, color: active || partner ? color : AppColors.muted),
              ],
            ]),
          ),
        ),
      );
      if (!linked) return chipWidget;
      return Tooltip(
        message: master != null
            ? 'Linked to ${device.getAlias(master)}'
            : 'Master of ${device.linkedWith(ch).map(device.getAlias).join(', ')}',
        waitDuration: const Duration(milliseconds: 600),
        child: chipWidget,
      );
    }

    return Align(
      alignment: Alignment.centerLeft,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
        child: Row(children: [
          if (showInputs) ...device.inputs.map(chip),
          if (showInputs && showOutputs)
            Container(
              width: 1,
              height: 24,
              margin: const EdgeInsets.only(left: 4, right: 10),
              color: AppColors.border,
            ),
          if (showOutputs) ...device.outputs.map(chip),
        ]),
      ),
    );
  }
}

/// Text in the value font (tabular figures).
class ValueText extends StatelessWidget {
  final String text;
  final double size;
  final Color color;

  const ValueText(this.text, {super.key, this.size = 11, this.color = AppColors.text});

  @override
  Widget build(BuildContext context) =>
      Text(text, style: monoValue.copyWith(fontSize: size, color: color));
}
