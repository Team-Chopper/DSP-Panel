// Channel menu: rename, copy settings, link channels, enter a gain value.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../../theme/app_colors.dart';
import '../providers/device_provider.dart';

/// Bottom sheet with everything that can be done with one channel.
Future<void> showChannelMenu(BuildContext context, DeviceProvider device, String channel) {
  final color = AppColors.forChannel(device.inputs.contains(channel));
  final linked = device.linkedWith(channel);
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: AppColors.surface,
    showDragHandle: true,
    shape: const RoundedRectangleBorder(borderRadius: BorderRadius.vertical(top: Radius.circular(16))),
    builder: (sheet) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: Icon(Icons.graphic_eq, color: color),
            title: Text(device.getAlias(channel)),
            subtitle: Text([
              if (device.getAlias(channel) != channel) channel,
              linked.isEmpty ? 'not linked' : 'linked with ${linked.join(', ')}',
            ].join(' · ')),
          ),
          const Divider(height: 1),
          ListTile(
            leading: const Icon(Icons.edit_outlined),
            title: const Text('Rename'),
            subtitle: const Text('Stored on the DSP, max. 8 characters'),
            onTap: () {
              Navigator.pop(sheet);
              showRenameDialog(context, device, channel);
            },
          ),
          ListTile(
            leading: const Icon(Icons.copy_all_outlined),
            title: const Text('Copy settings to …'),
            subtitle: const Text('Gain, phase, delay, EQ, filters, dynamics'),
            onTap: () {
              Navigator.pop(sheet);
              showCopyDialog(context, device, channel);
            },
          ),
          ListTile(
            leading: const Icon(Icons.link),
            title: const Text('Link with …'),
            subtitle: const Text('Linked channels change together'),
            onTap: () {
              Navigator.pop(sheet);
              showLinkDialog(context, device, channel);
            },
          ),
          const SizedBox(height: 8),
        ],
      ),
    ),
  );
}

Future<void> showRenameDialog(BuildContext context, DeviceProvider device, String channel) {
  final current = device.getAlias(channel);
  final controller = TextEditingController(text: current == channel ? '' : current);
  void save(BuildContext dialog) {
    final value = controller.text.trim();
    if (value.isEmpty) {
      device.resetAlias(channel);
    } else {
      device.setAlias(channel, value);
    }
    Navigator.of(dialog).pop();
  }

  return showDialog<void>(
    context: context,
    builder: (dialog) => AlertDialog(
      title: Text('Rename $channel'),
      content: TextField(
        controller: controller,
        maxLength: 8,
        autofocus: true,
        inputFormatters: [FilteringTextInputFormatter.allow(RegExp(r'[ -~]'))],
        decoration: const InputDecoration(
          labelText: 'Name',
          helperText: 'Stored on the DSP (max. 8 characters, no umlauts). Empty = default name.',
        ),
        onSubmitted: (_) => save(dialog),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.of(dialog).pop(), child: const Text('Cancel')),
        FilledButton(onPressed: () => save(dialog), child: const Text('Save')),
      ],
    ),
  );
}

Future<void> showCopyDialog(BuildContext context, DeviceProvider device, String from) {
  final targets = <String>{};
  final candidates = device.model.groupOf(from).where((c) => c != from).toList();
  return showDialog<void>(
    context: context,
    builder: (dialog) => StatefulBuilder(
      builder: (dialog, setState) => AlertDialog(
        title: Text('Copy ${device.getAlias(from)} to …'),
        content: SizedBox(
          width: 320,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text('Overwrites gain, phase, delay, EQ, filters and dynamics of the selected '
                  'channels. Names, mute, links and matrix routing stay.',
                  style: TextStyle(fontSize: 12, color: AppColors.muted)),
              const SizedBox(height: 8),
              for (final ch in candidates)
                CheckboxListTile(
                  dense: true,
                  value: targets.contains(ch),
                  title: Text(device.getAlias(ch)),
                  subtitle: device.getAlias(ch) == ch ? null : Text(ch),
                  onChanged: (v) => setState(() => v == true ? targets.add(ch) : targets.remove(ch)),
                ),
            ],
          ),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.of(dialog).pop(), child: const Text('Cancel')),
          FilledButton(
            onPressed: targets.isEmpty
                ? null
                : () {
                    device.copyChannel(from, targets.toList());
                    Navigator.of(dialog).pop();
                    ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                        content: Text('Copied ${device.getAlias(from)} to ${targets.length} channel(s)')));
                  },
            child: const Text('Copy'),
          ),
        ],
      ),
    ),
  );
}

Future<void> showLinkDialog(BuildContext context, DeviceProvider device, String channel) {
  final group = device.model.groupOf(channel);
  final selection = {channel, ...device.linkedWith(channel)};
  return showDialog<void>(
    context: context,
    builder: (dialog) => StatefulBuilder(
      builder: (dialog, setState) => AlertDialog(
        title: Text('Link ${device.getAlias(channel)}'),
        content: SizedBox(
          width: 320,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text('Linked channels change together. The link is stored on the DSP.',
                  style: TextStyle(fontSize: 12, color: AppColors.muted)),
              const SizedBox(height: 8),
              for (final ch in group)
                CheckboxListTile(
                  dense: true,
                  value: selection.contains(ch),
                  title: Text(device.getAlias(ch)),
                  enabled: ch != channel,
                  onChanged: (v) => setState(() => v == true ? selection.add(ch) : selection.remove(ch)),
                ),
            ],
          ),
        ),
        actions: [
          if (selection.length > 1 || device.linkedWith(channel).isNotEmpty)
            TextButton(
              onPressed: () {
                device.setLinkGroup([channel]);
                Navigator.of(dialog).pop();
              },
              child: const Text('Unlink'),
            ),
          TextButton(onPressed: () => Navigator.of(dialog).pop(), child: const Text('Cancel')),
          FilledButton(
            onPressed: () {
              device.setLinkGroup([for (final ch in group) if (selection.contains(ch)) ch]);
              Navigator.of(dialog).pop();
            },
            child: const Text('Apply'),
          ),
        ],
      ),
    ),
  );
}

Future<void> showGainDialog(BuildContext context, DeviceProvider device, String channel) {
  final controller = TextEditingController(text: device.getInputGain(channel).toStringAsFixed(1));
  void apply(BuildContext dialog) {
    final db = double.tryParse(controller.text.replaceAll(',', '.'));
    if (db != null && db >= -60.0 && db <= 12.0) {
      device.setGain(channel, db);
      Navigator.of(dialog).pop();
    }
  }

  return showDialog<void>(
    context: context,
    builder: (dialog) => AlertDialog(
      title: Text('${device.getAlias(channel)} gain'),
      content: TextField(
        controller: controller,
        autofocus: true,
        keyboardType: const TextInputType.numberWithOptions(decimal: true, signed: true),
        decoration: const InputDecoration(labelText: 'dB (-60.0 to +12.0)'),
        onSubmitted: (_) => apply(dialog),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.of(dialog).pop(), child: const Text('Cancel')),
        FilledButton(onPressed: () => apply(dialog), child: const Text('Set')),
      ],
    ),
  );
}
