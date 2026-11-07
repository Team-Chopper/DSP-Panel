// Channel menu: rename, copy settings, link channels, enter a gain value.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../../theme/app_colors.dart';
import '../providers/device_provider.dart';

/// Bottom sheet with everything that can be done with one channel.
Future<void> showChannelMenu(BuildContext context, DeviceProvider device, String channel) {
  final color = AppColors.forChannel(device.inputs.contains(channel));
  final master = device.linkMaster(channel);
  final slaves = device.linkSlaves(channel);
  final linkText = master != null
      ? 'linked to ${device.getAlias(master)}'
      : slaves.isNotEmpty
          ? 'master of ${slaves.map(device.getAlias).join(', ')}'
          : 'not linked';
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
              linkText,
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
            subtitle: const Text('Gain, mute, phase, delay, EQ, filters, dynamics'),
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
              Text(
                  'Overwrites gain, mute, phase, delay, EQ, filters and dynamics of the selected channels. '
                  'Names, links and matrix routing stay.'
                  '${device.offline ? '' : ' The DSP copies the settings itself; the panel reads them back.'}',
                  style: const TextStyle(fontSize: 12, color: AppColors.muted)),
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
                    device.copyChannel(from, [for (final ch in candidates) if (targets.contains(ch)) ch]);
                    Navigator.of(dialog).pop();
                    ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                        content: Text('Copying ${device.getAlias(from)} to ${targets.length} channel(s)')));
                  },
            child: const Text('Copy'),
          ),
        ],
      ),
    ),
  );
}

/// Link dialog, master/slave like the original editor: [channel] becomes
/// the master of the selected channels. A slave cannot be linked further;
/// its dialog only offers to leave its master.
Future<void> showLinkDialog(BuildContext context, DeviceProvider device, String channel) {
  final master = device.linkMaster(channel);
  if (master != null) {
    return showDialog<void>(
      context: context,
      builder: (dialog) => AlertDialog(
        title: Text('Link ${device.getAlias(channel)}'),
        content: SizedBox(
          width: 340,
          child: Text(
            '${device.getAlias(channel)} is linked to ${device.getAlias(master)} (master): changes of the master '
            'apply to it. A linked channel cannot be linked to other channels – to add more channels, link them '
            'to ${device.getAlias(master)}.',
            style: const TextStyle(fontSize: 13),
          ),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.of(dialog).pop(), child: const Text('Close')),
          TextButton(
            onPressed: () {
              Navigator.of(dialog).pop();
              showLinkDialog(context, device, master);
            },
            child: Text('Edit ${device.getAlias(master)} …'),
          ),
          FilledButton(
            onPressed: () {
              device.unlink(channel);
              Navigator.of(dialog).pop();
            },
            child: const Text('Unlink'),
          ),
        ],
      ),
    );
  }

  final group = device.model.groupOf(channel);
  final current = device.linkSlaves(channel);
  final candidates = device.linkCandidates(channel);
  final selection = {...current};
  return showDialog<void>(
    context: context,
    builder: (dialog) => StatefulBuilder(
      builder: (dialog, setState) {
        return AlertDialog(
          title: Text('Link ${device.getAlias(channel)} with …'),
          content: SizedBox(
            width: 340,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '${device.getAlias(channel)} becomes the master, the selected channels follow it. Like the '
                  'original editor, newly linked channels first get the settings of ${device.getAlias(channel)} '
                  '(gain, mute, phase, delay, EQ, filters, dynamics – not names or routing). The link is stored on '
                  'the DSP; a change is sent once and the DSP applies it to the linked channels.',
                  style: const TextStyle(fontSize: 12, color: AppColors.muted),
                ),
                const SizedBox(height: 8),
                for (final ch in group)
                  if (ch != channel)
                    CheckboxListTile(
                      dense: true,
                      value: selection.contains(ch),
                      title: Text(device.getAlias(ch)),
                      subtitle: candidates.contains(ch) ? null : Text(_linkReason(device, ch)),
                      enabled: candidates.contains(ch),
                      onChanged: (v) => setState(() => v == true ? selection.add(ch) : selection.remove(ch)),
                    ),
              ],
            ),
          ),
          actions: [
            if (current.isNotEmpty)
              TextButton(
                onPressed: () {
                  device.unlink(channel);
                  Navigator.of(dialog).pop();
                },
                child: const Text('Unlink all'),
              ),
            TextButton(onPressed: () => Navigator.of(dialog).pop(), child: const Text('Cancel')),
            FilledButton(
              onPressed: () {
                device.setLinkSlaves(channel, [for (final ch in group) if (selection.contains(ch)) ch]);
                Navigator.of(dialog).pop();
              },
              child: const Text('Apply'),
            ),
          ],
        );
      },
    ),
  );
}

/// Why [ch] cannot be linked as a slave right now.
String _linkReason(DeviceProvider device, String ch) {
  final master = device.linkMaster(ch);
  if (master != null) return 'linked to ${device.getAlias(master)}';
  return 'master of ${device.linkSlaves(ch).map(device.getAlias).join(', ')}';
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
