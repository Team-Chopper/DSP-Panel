// Presets dialog: the 20 presets stored on the DSP (U01–U20) and the preset
// library on the server. Server presets can be loaded onto a DSP of the same
// model, saved from the current settings and opened in an offline editor.

import 'package:dsp_core/dsp_core.dart';
import 'package:flutter/material.dart';

import '../../../providers/panel_provider.dart';
import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';

String presetSlot(int index) => 'U${(index + 1).toString().padLeft(2, '0')}';

/// Opens the presets dialog for the DSP (or offline editor) [handle].
Future<void> showPresetsDialog(BuildContext context, PanelProvider panel, DspHandle handle) {
  return showDialog<void>(
    context: context,
    builder: (context) => _PresetsDialog(panel: panel, handle: handle),
  );
}

/// Asks for a connected DSP of the preset's model and loads the preset onto it.
Future<void> showApplyPresetDialog(BuildContext context, PanelProvider panel, ServerPresetInfo preset) async {
  final targets = [
    for (final d in panel.dsps)
      if (d.info.model == preset.model) d,
  ];
  final target = await showDialog<DspHandle>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text('Load "${preset.name}" onto …'),
      content: SizedBox(
        width: 360,
        child: targets.isEmpty
            ? Text('No ${DeviceModel.byId(preset.model).name} is configured.')
            : Column(mainAxisSize: MainAxisSize.min, children: [
                for (final d in targets)
                  ListTile(
                    enabled: d.connection.isConnected,
                    leading: Icon(Icons.circle,
                        size: 12, color: d.connection.isConnected ? AppColors.accent : AppColors.danger),
                    title: Text(d.info.name),
                    subtitle: Text(d.connection.isConnected ? '${d.info.host}:${d.info.port}' : 'not connected'),
                    onTap: () => Navigator.pop(context, d),
                  ),
              ]),
      ),
      actions: [TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel'))],
    ),
  );
  if (target == null || !context.mounted) return;
  if (await _confirmApply(context, preset, target)) panel.applyServerPreset(preset.id, target.id);
}

Future<bool> _confirmApply(BuildContext context, ServerPresetInfo preset, DspHandle target) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('Load server preset'),
      content: Text('Send all values of "${preset.name}" to ${target.info.name}?\n\n'
          'The current settings of the DSP are replaced. The presets U01–U20 stored on '
          'the DSP are not changed.'),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
        FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Load')),
      ],
    ),
  );
  return ok ?? false;
}

Future<String?> _askName(BuildContext context,
    {required String title, String initial = '', int maxLength = 40, String action = 'Save', String? hint}) async {
  final controller = TextEditingController(text: initial);
  final name = await showDialog<String>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(title),
      content: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
        if (hint != null) ...[
          Text(hint, style: const TextStyle(fontSize: 12, color: AppColors.muted)),
          const SizedBox(height: 12),
        ],
        TextField(
          controller: controller,
          autofocus: true,
          maxLength: maxLength,
          decoration: const InputDecoration(labelText: 'Name'),
          onSubmitted: (v) => Navigator.pop(context, v.trim()),
        ),
      ]),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
        FilledButton(onPressed: () => Navigator.pop(context, controller.text.trim()), child: Text(action)),
      ],
    ),
  );
  controller.dispose();
  return (name == null || name.isEmpty) ? null : name;
}

class _PresetsDialog extends StatelessWidget {
  final PanelProvider panel;
  final DspHandle handle;

  const _PresetsDialog({required this.panel, required this.handle});

  bool get _offline => handle.connection.offline;

  @override
  Widget build(BuildContext context) {
    return Dialog(
      backgroundColor: AppColors.surface,
      insetPadding: const EdgeInsets.all(16),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 600, maxHeight: 640),
        child: DefaultTabController(
          length: _offline ? 1 : 2,
          initialIndex: 0,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(20, 14, 8, 0),
                child: Row(children: [
                  const Icon(Icons.library_music_outlined, color: AppColors.accent, size: 20),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text('Presets · ${handle.info.name}',
                        style: interWeight(const TextStyle(fontSize: 16), FontWeight.w600)),
                  ),
                  IconButton(icon: const Icon(Icons.close), onPressed: () => Navigator.pop(context)),
                ]),
              ),
              TabBar(
                tabs: [
                  if (!_offline) const Tab(text: 'On the DSP'),
                  const Tab(text: 'On the server'),
                ],
              ),
              Flexible(
                child: ListenableBuilder(
                  listenable: Listenable.merge([panel, handle.connection, handle.device]),
                  builder: (context, _) => TabBarView(children: [
                    if (!_offline) _DevicePresets(handle: handle),
                    _ServerPresets(panel: panel, handle: handle),
                  ]),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ─── Presets stored on the DSP ───

class _DevicePresets extends StatelessWidget {
  final DspHandle handle;

  const _DevicePresets({required this.handle});

  @override
  Widget build(BuildContext context) {
    final connection = handle.connection;
    final device = handle.device;
    if (!connection.isConnected) {
      return const _Empty(icon: Icons.link_off, text: 'Connect the DSP to load or store its presets.');
    }
    final entries = device.presets.entries.toList()..sort((a, b) => a.key.compareTo(b.key));
    if (entries.isEmpty) return const _Empty(icon: Icons.hourglass_empty, text: 'No presets read from the DSP.');
    return ListView(
      padding: const EdgeInsets.symmetric(vertical: 8),
      children: [
        for (final e in entries)
          _PresetRow(
            leading: presetSlot(e.key),
            title: e.value,
            // The slot table (0x22) tells which slots hold a stored preset.
            subtitle: device.presetSlots.isNotEmpty && !device.presetSlots.contains(e.key) ? 'empty' : null,
            // The DSP reports the current preset number (0x14); older
            // states only have the name from the dump.
            current: device.currentPresetNumber > 0
                ? device.currentPresetNumber == e.key + 1
                : e.value == device.currentPreset,
            actions: [
              TextButton.icon(
                icon: const Icon(Icons.download_outlined, size: 16),
                label: const Text('Load'),
                onPressed: () {
                  connection.loadPreset(e.key);
                  Navigator.pop(context);
                },
              ),
              IconButton(
                icon: const Icon(Icons.save_outlined, size: 18),
                tooltip: 'Store current settings in ${presetSlot(e.key)}',
                onPressed: () async {
                  final name = await _askName(context,
                      title: 'Store in ${presetSlot(e.key)}',
                      initial: e.value,
                      maxLength: 14,
                      hint: 'The current settings replace preset ${presetSlot(e.key)} on the DSP.');
                  if (name != null) connection.savePreset(e.key, name);
                },
              ),
            ],
          ),
      ],
    );
  }
}

// ─── Presets stored on the server ───

class _ServerPresets extends StatelessWidget {
  final PanelProvider panel;
  final DspHandle handle;

  const _ServerPresets({required this.panel, required this.handle});

  @override
  Widget build(BuildContext context) {
    final model = handle.connection.model;
    final offline = handle.connection.offline;
    final connected = handle.connection.isConnected;
    final presets = panel.serverPresetsFor(model)..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
    final canSave = !offline && connected && handle.connection.configReadable;

    return Column(children: [
      Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
        child: Wrap(spacing: 8, runSpacing: 8, crossAxisAlignment: WrapCrossAlignment.center, children: [
          if (!offline)
            FilledButton.icon(
              icon: const Icon(Icons.cloud_upload_outlined, size: 18),
              label: const Text('Save current settings'),
              onPressed: canSave
                  ? () async {
                      final name = await _askName(context,
                          title: 'Save to server',
                          initial: handle.device.currentPreset,
                          hint: 'Saves all settings of ${handle.info.name} as a new server preset.');
                      if (name != null) panel.savePresetFromDsp(handle.id, name);
                    }
                  : null,
            ),
          OutlinedButton.icon(
            icon: const Icon(Icons.add, size: 18),
            label: const Text('New empty preset'),
            onPressed: () async {
              final name = await _askName(context,
                  title: 'New ${model.name} preset', action: 'Create', hint: 'Starts with the default settings.');
              if (name != null) panel.createServerPreset(name, model);
            },
          ),
        ]),
      ),
      Padding(
        padding: const EdgeInsets.fromLTRB(16, 4, 16, 4),
        child: Text(
          'Only presets for the ${model.name} are listed. '
          '${offline ? '' : canSave || !connected ? '' : 'The configuration of this DSP could not be read, so it cannot be saved. '}'
          'Open a preset to edit it without a DSP.',
          style: const TextStyle(fontSize: 11, color: AppColors.muted),
        ),
      ),
      Expanded(
        child: presets.isEmpty
            ? const _Empty(icon: Icons.cloud_off_outlined, text: 'No server presets for this model yet.')
            : ListView(
                padding: const EdgeInsets.only(bottom: 8),
                children: [
                  for (final p in presets)
                    _PresetRow(
                      leading: null,
                      title: p.name,
                      subtitle: 'changed ${_formatDate(p.updated)}',
                      current: offline && handle.id == '${Msg.presetPrefix}${p.id}',
                      actions: [
                        if (!offline)
                          TextButton.icon(
                            icon: const Icon(Icons.download_outlined, size: 16),
                            label: const Text('Load'),
                            onPressed: connected
                                ? () async {
                                    if (await _confirmApply(context, p, handle) && context.mounted) {
                                      panel.applyServerPreset(p.id, handle.id);
                                      Navigator.pop(context);
                                    }
                                  }
                                : null,
                          )
                        else
                          TextButton.icon(
                            icon: const Icon(Icons.send_outlined, size: 16),
                            label: const Text('To DSP …'),
                            onPressed: () => showApplyPresetDialog(context, panel, p),
                          ),
                        IconButton(
                          icon: const Icon(Icons.edit_note, size: 20),
                          tooltip: 'Edit offline',
                          onPressed: () {
                            Navigator.pop(context);
                            panel.openEditor(p.id);
                          },
                        ),
                        PopupMenuButton<String>(
                          icon: const Icon(Icons.more_vert, size: 18),
                          onSelected: (action) async {
                            if (action == 'rename') {
                              final name = await _askName(context, title: 'Rename preset', initial: p.name, action: 'Rename');
                              if (name != null) panel.renameServerPreset(p.id, name);
                            } else if (action == 'delete') {
                              final ok = await showDialog<bool>(
                                context: context,
                                builder: (context) => AlertDialog(
                                  title: const Text('Delete preset'),
                                  content: Text('Delete "${p.name}" from the server? This cannot be undone.'),
                                  actions: [
                                    TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
                                    FilledButton(
                                      style: FilledButton.styleFrom(backgroundColor: AppColors.danger),
                                      onPressed: () => Navigator.pop(context, true),
                                      child: const Text('Delete'),
                                    ),
                                  ],
                                ),
                              );
                              if (ok == true) panel.deleteServerPreset(p.id);
                            }
                          },
                          itemBuilder: (context) => const [
                            PopupMenuItem(value: 'rename', child: Text('Rename')),
                            PopupMenuItem(value: 'delete', child: Text('Delete', style: TextStyle(color: AppColors.danger))),
                          ],
                        ),
                      ],
                    ),
                ],
              ),
      ),
    ]);
  }

  static String _formatDate(int ms) {
    final d = DateTime.fromMillisecondsSinceEpoch(ms);
    String two(int v) => v.toString().padLeft(2, '0');
    return '${two(d.day)}.${two(d.month)}.${d.year} ${two(d.hour)}:${two(d.minute)}';
  }
}

class _PresetRow extends StatelessWidget {
  final String? leading;
  final String title;
  final String? subtitle;
  final bool current;
  final List<Widget> actions;

  const _PresetRow({
    required this.leading,
    required this.title,
    required this.current,
    required this.actions,
    this.subtitle,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 2),
      padding: const EdgeInsets.only(left: 12, right: 4),
      decoration: BoxDecoration(
        color: current ? AppColors.accent.withAlpha(24) : null,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: current ? AppColors.accent.withAlpha(120) : Colors.transparent),
      ),
      child: Row(children: [
        if (leading != null)
          SizedBox(width: 40, child: Text(leading!, style: monoValue.copyWith(fontSize: 12, color: AppColors.muted))),
        Expanded(
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 8),
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text(title.isEmpty ? '—' : title, overflow: TextOverflow.ellipsis),
              if (subtitle != null) Text(subtitle!, style: const TextStyle(fontSize: 11, color: AppColors.muted)),
            ]),
          ),
        ),
        if (current)
          const Padding(
            padding: EdgeInsets.only(right: 4),
            child: Text('current', style: TextStyle(fontSize: 11, color: AppColors.accent)),
          ),
        ...actions,
      ]),
    );
  }
}

class _Empty extends StatelessWidget {
  final IconData icon;
  final String text;

  const _Empty({required this.icon, required this.text});

  @override
  Widget build(BuildContext context) => Center(
        child: Padding(
          padding: const EdgeInsets.all(32),
          child: Column(mainAxisSize: MainAxisSize.min, children: [
            Icon(icon, size: 32, color: AppColors.muted),
            const SizedBox(height: 12),
            Text(text, textAlign: TextAlign.center, style: const TextStyle(color: AppColors.muted)),
          ]),
        ),
      );
}
