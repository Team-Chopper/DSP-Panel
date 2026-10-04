// Panel page: one tab per DSP managed by the server, plus a tab for every
// server preset open in the offline editor.

import 'dart:async';

import 'package:dsp_core/dsp_core.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../devices/t_racks408/dsp_page.dart';
import '../providers/panel_provider.dart';
import '../services/server_connection.dart';
import '../theme/app_colors.dart';
import '../theme/app_theme.dart';
import '../widgets/dsp_dialog.dart';

class PanelPage extends StatefulWidget {
  const PanelPage({super.key});

  @override
  State<PanelPage> createState() => _PanelPageState();
}

class _PanelPageState extends State<PanelPage> {
  StreamSubscription<String>? _errors;

  @override
  void initState() {
    super.initState();
    _errors = context.read<PanelProvider>().errors.listen((message) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(message), backgroundColor: AppColors.danger),
      );
    });
  }

  @override
  void dispose() {
    _errors?.cancel();
    super.dispose();
  }

  static Color statusColor(DspStatus status) => switch (status) {
        DspStatus.ready => AppColors.accent,
        DspStatus.connecting || DspStatus.initializing || DspStatus.busy => AppColors.warning,
        DspStatus.error || DspStatus.disconnected => AppColors.danger,
      };

  static String _modelShort(String model) => DeviceModel.byId(model).name.replaceAll('DSP ', '');

  @override
  Widget build(BuildContext context) {
    final panel = context.watch<PanelProvider>();
    final server = context.watch<ServerConnection>();
    final dsps = panel.dsps;
    final editors = panel.editors;
    final all = [...dsps, ...editors];

    // Keep the selection valid when DSPs are added or removed.
    var selectedId = panel.selectedId;
    if (all.isNotEmpty && !all.any((d) => d.id == selectedId)) {
      selectedId = all.first.id;
      WidgetsBinding.instance.addPostFrameCallback((_) => panel.select(selectedId));
    } else if (all.isEmpty && selectedId != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => panel.select(null));
    }
    final selected = all.where((d) => d.id == selectedId).firstOrNull;

    return Scaffold(
      appBar: AppBar(
        titleSpacing: 16,
        title: Row(mainAxisSize: MainAxisSize.min, children: [
          Container(
            width: 28,
            height: 28,
            decoration: BoxDecoration(
              color: AppColors.accent.withAlpha(36),
              borderRadius: BorderRadius.circular(8),
            ),
            child: const Icon(Icons.graphic_eq, size: 18, color: AppColors.accent),
          ),
          const SizedBox(width: 10),
          Text('DSP Panel', style: interWeight(const TextStyle(fontSize: 18), FontWeight.w700)),
        ]),
        actions: [
          IconButton(
            icon: const Icon(Icons.add),
            tooltip: 'Add DSP',
            onPressed: () => showAddDspDialog(context),
          ),
          PopupMenuButton<String>(
            icon: const Icon(Icons.menu),
            tooltip: 'Menu',
            onSelected: (action) {
              switch (action) {
                case 'add':
                  showAddDspDialog(context);
                case 'logout':
                  server.logout();
              }
            },
            itemBuilder: (context) => [
              const PopupMenuItem(value: 'add', child: ListTile(leading: Icon(Icons.add), title: Text('Add DSP'))),
              if (server.authEnabled) ...[
                const PopupMenuDivider(),
                const PopupMenuItem(
                  value: 'logout',
                  child: ListTile(leading: Icon(Icons.logout), title: Text('Log out of the panel')),
                ),
              ],
            ],
          ),
          const SizedBox(width: 8),
        ],
        bottom: all.isEmpty
            ? null
            : PreferredSize(
                preferredSize: const Size.fromHeight(42),
                child: SizedBox(
                  height: 42,
                  child: ListView(
                    scrollDirection: Axis.horizontal,
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    children: [
                      for (final dsp in dsps)
                        ListenableBuilder(
                          listenable: dsp.connection,
                          builder: (context, _) => _DspTab(
                            name: '${dsp.info.name} · ${_modelShort(dsp.info.model)}',
                            leading: Icon(Icons.circle, size: 9, color: statusColor(dsp.info.status)),
                            selected: dsp.id == selectedId,
                            onTap: () => panel.select(dsp.id),
                          ),
                        ),
                      if (editors.isNotEmpty)
                        Container(
                          width: 1,
                          margin: const EdgeInsets.symmetric(horizontal: 8, vertical: 10),
                          color: AppColors.border,
                        ),
                      for (final editor in editors)
                        _DspTab(
                          name: '${editor.info.name} · ${_modelShort(editor.info.model)}',
                          leading: const Icon(Icons.cloud_outlined, size: 14, color: AppColors.info),
                          subtitle: 'offline',
                          selected: editor.id == selectedId,
                          onTap: () => panel.select(editor.id),
                          onClose: () => panel.closeEditor(editor.id.substring(Msg.presetPrefix.length)),
                        ),
                    ],
                  ),
                ),
              ),
      ),
      body: Column(
        children: [
          if (server.state != ServerState.connected)
            MaterialBanner(
              content: Text(server.error ?? 'Connecting to server …'),
              leading: const SizedBox(width: 20, height: 20, child: CircularProgressIndicator(strokeWidth: 2)),
              actions: const [SizedBox.shrink()],
            ),
          Expanded(
            child: selected == null
                ? _EmptyState(loaded: panel.loaded)
                : DspPage(key: ValueKey(selected.id), handle: selected),
          ),
        ],
      ),
    );
  }
}

class _DspTab extends StatelessWidget {
  final String name;
  final Widget leading;
  final String? subtitle;
  final bool selected;
  final VoidCallback onTap;
  final VoidCallback? onClose;

  const _DspTab({
    required this.name,
    required this.leading,
    required this.selected,
    required this.onTap,
    this.subtitle,
    this.onClose,
  });

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: const BorderRadius.vertical(top: Radius.circular(8)),
      child: Container(
        padding: EdgeInsets.only(left: 14, right: onClose == null ? 14 : 4),
        decoration: BoxDecoration(
          border: Border(
            bottom: BorderSide(color: selected ? AppColors.accent : Colors.transparent, width: 2),
          ),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            leading,
            const SizedBox(width: 8),
            Text(
              name,
              style: interWeight(
                TextStyle(fontSize: 13, color: selected ? AppColors.text : AppColors.muted),
                selected ? FontWeight.w600 : FontWeight.w500,
              ),
            ),
            if (subtitle != null) ...[
              const SizedBox(width: 6),
              Text(subtitle!, style: const TextStyle(fontSize: 11, color: AppColors.info)),
            ],
            if (onClose != null)
              IconButton(
                icon: const Icon(Icons.close, size: 14),
                tooltip: 'Close editor',
                visualDensity: VisualDensity.compact,
                onPressed: onClose,
              ),
          ],
        ),
      ),
    );
  }
}

class _EmptyState extends StatelessWidget {
  final bool loaded;

  const _EmptyState({required this.loaded});

  @override
  Widget build(BuildContext context) {
    if (!loaded) return const Center(child: CircularProgressIndicator());
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Icon(Icons.speaker_group_outlined, size: 48, color: AppColors.muted),
          const SizedBox(height: 12),
          const Text('No DSPs configured yet.'),
          const SizedBox(height: 16),
          FilledButton.icon(
            onPressed: () => showAddDspDialog(context),
            icon: const Icon(Icons.add),
            label: const Text('Add DSP'),
          ),
        ],
      ),
    );
  }
}
