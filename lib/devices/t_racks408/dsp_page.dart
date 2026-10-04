// DSP Page
//
// Everything for one DSP (or a server preset in the offline editor):
// section bar (Gain · EQ · GEQ · Gate · Comp · Limit · Matrix · Delay ·
// RTA) with presets,
// connection status and connect/disconnect on the right, the section
// content and the optional debug log. While the DSP is not connected the
// content is blurred and a connect button is shown on top.

import 'dart:ui' as ui;

import 'package:dsp_core/dsp_core.dart' show Msg;
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../../providers/panel_provider.dart';
import '../../theme/app_colors.dart';
import '../../theme/app_theme.dart';
import '../../widgets/dsp_dialog.dart';
import 'overlays/settings_overlay.dart';
import 'pages/delay_page.dart';
import 'pages/dynamics_pages.dart';
import 'pages/eq_page.dart';
import 'pages/home_page.dart';
import 'pages/matrix_page.dart';
import 'providers/connection_provider.dart';
import 'providers/device_provider.dart';
import 'widgets/geq_tab.dart';
import 'widgets/presets_dialog.dart';
import 'widgets/rta_tab.dart';

enum DspSection {
  home('Gain', Icons.tune),
  eq('EQ', Icons.show_chart),
  geq('GEQ', Icons.equalizer),
  gate('Gate', Icons.sensor_door_outlined),
  compressor('Comp', Icons.compress),
  limiter('Limit', Icons.vertical_align_top),
  matrix('Matrix', Icons.grid_view),
  delay('Delay', Icons.timer_outlined),
  rta('RTA', Icons.graphic_eq);

  final String label;
  final IconData icon;

  const DspSection(this.label, this.icon);
}

class DspPage extends StatefulWidget {
  final DspHandle handle;

  const DspPage({super.key, required this.handle});

  @override
  State<DspPage> createState() => _DspPageState();
}

class _DspPageState extends State<DspPage> {
  DspSection _section = DspSection.home;

  /// Channel to show when the EQ page is opened from the overview.
  String? _eqChannel;
  bool _showDebug = false;

  ConnectionProvider get _connection => widget.handle.connection;
  DeviceProvider get _device => widget.handle.device;

  void _openEq(String channel) => setState(() {
        _eqChannel = channel;
        _section = DspSection.eq;
      });

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: _connection,
      builder: (context, _) {
        final connection = _connection;
        final offline = connection.offline;
        final locked = !offline && !connection.isConnected;
        return Column(
          children: [
            _SectionBar(
              handle: widget.handle,
              section: _section,
              onSection: (s) => setState(() {
                _section = s;
                if (s != DspSection.eq && s != DspSection.geq) _eqChannel = null;
              }),
              showDebug: _showDebug,
              onToggleDebug: () => setState(() => _showDebug = !_showDebug),
            ),
            if (connection.isConnected && !connection.configReadable)
              _Banner(
                'The configuration of this DSP could not be read (unexpected layout for '
                '${connection.model.name}). The panel shows default values – controls still work. '
                'Check the model in "Edit DSP" or see TODO.md.',
              ),
            Expanded(
              child: Stack(
                fit: StackFit.expand,
                children: [
                  AbsorbPointer(
                    absorbing: locked,
                    child: ImageFiltered(
                      enabled: locked,
                      imageFilter: ui.ImageFilter.blur(sigmaX: 7, sigmaY: 7),
                      child: _content(),
                    ),
                  ),
                  if (locked) _ConnectOverlay(connection: connection),
                ],
              ),
            ),
            if (_showDebug && !offline) SizedBox(height: 220, child: _DebugPanel(connection: connection)),
          ],
        );
      },
    );
  }

  Widget _content() {
    final device = _device;
    return switch (_section) {
      DspSection.home => HomePage(device: device, onOpenEq: _openEq),
      DspSection.eq => EqPage(key: ValueKey('eq-$_eqChannel'), device: device, channel: _eqChannel),
      DspSection.geq => GeqTab(key: ValueKey('geq-$_eqChannel'), deviceProvider: device, initialChannel: _eqChannel),
      DspSection.gate => GatePage(device: device),
      DspSection.compressor => CompressorPage(device: device),
      DspSection.limiter => LimiterPage(device: device),
      DspSection.matrix => MatrixPage(device: device),
      DspSection.delay => DelayPage(device: device),
      DspSection.rta => const RtaTab(),
    };
  }
}

// ─── Section bar ───

class _SectionBar extends StatelessWidget {
  final DspHandle handle;
  final DspSection section;
  final ValueChanged<DspSection> onSection;
  final bool showDebug;
  final VoidCallback onToggleDebug;

  const _SectionBar({
    required this.handle,
    required this.section,
    required this.onSection,
    required this.showDebug,
    required this.onToggleDebug,
  });

  @override
  Widget build(BuildContext context) {
    final width = MediaQuery.sizeOf(context).width;
    // Sections: icon and text from 1500 px, text only from 1000 px, icons
    // only below. Compact status below 800 px.
    final sectionStyle = width >= 1500 ? _SectionStyle.full : (width >= 1000 ? _SectionStyle.text : _SectionStyle.icon);
    final compact = width < 800;
    final connection = handle.connection;
    return Container(
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(bottom: BorderSide(color: AppColors.border)),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
      child: Row(
        children: [
          Expanded(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(children: [
                for (final s in DspSection.values)
                  _SectionButton(section: s, selected: s == section, style: sectionStyle, onTap: () => onSection(s)),
              ]),
            ),
          ),
          const SizedBox(width: 8),
          _PresetsButton(handle: handle, compact: compact),
          const SizedBox(width: 8),
          if (connection.offline)
            ..._editorActions(context, compact)
          else
            ..._dspActions(context, compact),
        ],
      ),
    );
  }

  List<Widget> _dspActions(BuildContext context, bool compact) {
    final connection = handle.connection;
    final info = connection.info;
    return [
      _StatusPill(connection: connection, compact: compact),
      const SizedBox(width: 8),
      if (info.enabled)
        FilledButton.icon(
          style: FilledButton.styleFrom(
            backgroundColor: AppColors.danger,
            foregroundColor: AppColors.backgroundDeep,
            visualDensity: VisualDensity.compact,
          ),
          icon: const Icon(Icons.power_settings_new, size: 18),
          label: Text(compact ? 'Off' : 'Disconnect'),
          onPressed: connection.disconnect,
        )
      else
        FilledButton.icon(
          style: FilledButton.styleFrom(visualDensity: VisualDensity.compact),
          icon: const Icon(Icons.power_settings_new, size: 18),
          label: const Text('Connect'),
          onPressed: connection.connect,
        ),
      PopupMenuButton<String>(
        tooltip: 'More',
        icon: const Icon(Icons.more_vert),
        onSelected: (action) {
          switch (action) {
            case 'edit':
              showEditDspDialog(context, info);
            case 'settings':
              SettingsOverlay.showSettingsOverlay(context, connection);
            case 'debug':
              onToggleDebug();
          }
        },
        itemBuilder: (context) => [
          const PopupMenuItem(
              value: 'edit', child: ListTile(leading: Icon(Icons.edit_outlined), title: Text('Edit DSP'))),
          const PopupMenuItem(
              value: 'settings', child: ListTile(leading: Icon(Icons.settings_outlined), title: Text('Settings'))),
          PopupMenuItem(
            value: 'debug',
            child: ListTile(
              leading: const Icon(Icons.bug_report_outlined),
              title: Text(showDebug ? 'Hide debug log' : 'Show debug log'),
            ),
          ),
        ],
      ),
    ];
  }

  List<Widget> _editorActions(BuildContext context, bool compact) {
    final panel = context.read<PanelProvider>();
    final presetId = handle.id.substring(Msg.presetPrefix.length);
    final preset = panel.serverPresets.where((p) => p.id == presetId).firstOrNull;
    return [
      Tooltip(
        message: 'Server preset for the ${handle.connection.model.name}, edited without a DSP.\n'
            'Every change is saved on the server.',
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
          decoration: BoxDecoration(
            color: AppColors.info.withAlpha(30),
            borderRadius: BorderRadius.circular(20),
            border: Border.all(color: AppColors.info.withAlpha(120)),
          ),
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            const Icon(Icons.cloud_outlined, size: 14, color: AppColors.info),
            if (!compact) ...[
              const SizedBox(width: 6),
              Text('Offline · ${handle.connection.model.name}',
                  style: const TextStyle(fontSize: 12, color: AppColors.info)),
            ],
          ]),
        ),
      ),
      const SizedBox(width: 8),
      FilledButton.icon(
        style: FilledButton.styleFrom(visualDensity: VisualDensity.compact),
        icon: const Icon(Icons.send_outlined, size: 16),
        label: Text(compact ? 'To DSP' : 'Load onto DSP …'),
        onPressed: preset == null ? null : () => showApplyPresetDialog(context, panel, preset),
      ),
      IconButton(
        tooltip: 'Close editor',
        icon: const Icon(Icons.close),
        onPressed: () => panel.closeEditor(presetId),
      ),
    ];
  }
}

enum _SectionStyle { full, text, icon }

class _SectionButton extends StatelessWidget {
  final DspSection section;
  final bool selected;
  final _SectionStyle style;
  final VoidCallback onTap;

  const _SectionButton({required this.section, required this.selected, required this.style, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final color = selected ? AppColors.accent : AppColors.muted;
    final showIcon = style != _SectionStyle.text;
    final showText = style != _SectionStyle.icon;
    return Padding(
      padding: const EdgeInsets.only(right: 4),
      child: Tooltip(
        message: showText ? '' : section.label,
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: onTap,
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 120),
            padding: EdgeInsets.symmetric(horizontal: showText ? 12 : 10, vertical: 8),
            decoration: BoxDecoration(
              color: selected ? AppColors.accent.withAlpha(30) : Colors.transparent,
              borderRadius: BorderRadius.circular(8),
            ),
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              if (showIcon) Icon(section.icon, size: 18, color: color),
              if (showIcon && showText) const SizedBox(width: 6),
              if (showText) ...[
                Text(section.label,
                    style: interWeight(TextStyle(fontSize: 13, color: selected ? AppColors.text : AppColors.muted),
                        selected ? FontWeight.w600 : FontWeight.w500)),
              ],
            ]),
          ),
        ),
      ),
    );
  }
}

class _PresetsButton extends StatelessWidget {
  final DspHandle handle;
  final bool compact;

  const _PresetsButton({required this.handle, required this.compact});

  String _label() {
    final device = handle.device;
    if (handle.connection.offline) return handle.info.name;
    if (!handle.connection.isConnected) return 'Presets';
    final name = device.currentPreset;
    for (final e in device.presets.entries) {
      if (e.value == name) return '${presetSlot(e.key)} · $name';
    }
    return name;
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: handle.device,
      builder: (context, _) => Tooltip(
        message: 'Presets on the DSP and on the server',
        child: OutlinedButton.icon(
          style: OutlinedButton.styleFrom(
            visualDensity: VisualDensity.compact,
            side: const BorderSide(color: AppColors.border),
            foregroundColor: AppColors.text,
          ),
          icon: const Icon(Icons.library_music_outlined, size: 18, color: AppColors.accent),
          label: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: compact ? 90 : 200),
            child: Text(_label(), overflow: TextOverflow.ellipsis),
          ),
          onPressed: () => showPresetsDialog(context, context.read<PanelProvider>(), handle),
        ),
      ),
    );
  }
}

class _StatusPill extends StatelessWidget {
  final ConnectionProvider connection;
  final bool compact;

  const _StatusPill({required this.connection, required this.compact});

  @override
  Widget build(BuildContext context) {
    final info = connection.info;
    final (Color color, String text) = switch (info.status) {
      DspStatus.ready => (AppColors.accent, 'Connected'),
      DspStatus.connecting => (AppColors.warning, 'Connecting'),
      DspStatus.initializing => (AppColors.warning, 'Loading ${(info.progress * 100).round()} %'),
      DspStatus.busy => (AppColors.warning, '${info.message ?? 'Busy'} ${(info.progress * 100).round()} %'),
      DspStatus.error => (AppColors.danger, 'Disconnected'),
      DspStatus.disconnected => (AppColors.danger, 'Disconnected'),
    };
    final tooltip = [
      connection.connectionStatus,
      '${connection.model.name} · ${info.host}:${info.port}',
      if (connection.isConnected) ...[
        'TX ${connection.packetsSent} · RX ${connection.packetsReceived}',
        'Mute, phase, delay and PEQ bypass cannot be read from the DSP yet –\n'
            'the panel shows the last values set here.',
      ],
    ].join('\n');
    return Tooltip(
      message: tooltip,
      child: Container(
        padding: EdgeInsets.symmetric(horizontal: compact ? 8 : 10, vertical: 5),
        decoration: BoxDecoration(
          color: color.withAlpha(30),
          borderRadius: BorderRadius.circular(20),
          border: Border.all(color: color.withAlpha(140)),
        ),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          Container(width: 8, height: 8, decoration: BoxDecoration(color: color, shape: BoxShape.circle)),
          if (!compact) ...[
            const SizedBox(width: 7),
            Text(text, style: interWeight(TextStyle(fontSize: 12, color: color), FontWeight.w600)),
          ],
        ]),
      ),
    );
  }
}

class _Banner extends StatelessWidget {
  final String text;

  const _Banner(this.text);

  @override
  Widget build(BuildContext context) => Container(
        width: double.infinity,
        color: AppColors.warning.withAlpha(30),
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        child: Row(children: [
          const Icon(Icons.warning_amber, color: AppColors.warning, size: 18),
          const SizedBox(width: 10),
          Expanded(child: Text(text, style: const TextStyle(fontSize: 12))),
        ]),
      );
}

// ─── Not connected ───

/// Shown on top of the blurred content while the DSP is not ready.
class _ConnectOverlay extends StatelessWidget {
  final ConnectionProvider connection;

  const _ConnectOverlay({required this.connection});

  @override
  Widget build(BuildContext context) {
    final info = connection.info;
    final working = const {DspStatus.connecting, DspStatus.initializing, DspStatus.busy}.contains(info.status);
    final String title;
    String? detail;
    switch (info.status) {
      case DspStatus.connecting:
        title = 'Connecting …';
        detail = '${info.host}:${info.port}';
      case DspStatus.initializing:
        title = 'Reading the configuration … ${(info.progress * 100).round()} %';
      case DspStatus.busy:
        title = '${info.message ?? 'Working …'} ${(info.progress * 100).round()} %';
      case DspStatus.error:
        title = 'Connection failed';
        detail = '${info.message ?? 'Error'}\nRetrying automatically.';
      case DspStatus.disconnected:
      case DspStatus.ready:
        title = 'Disconnected';
        detail = '${info.name} · ${info.host}:${info.port}';
    }

    return Container(
      color: AppColors.background.withAlpha(90),
      alignment: Alignment.center,
      padding: const EdgeInsets.all(24),
      child: Container(
        constraints: const BoxConstraints(maxWidth: 380),
        padding: const EdgeInsets.all(24),
        decoration: BoxDecoration(
          color: AppColors.surface.withAlpha(235),
          borderRadius: BorderRadius.circular(16),
          border: Border.all(color: AppColors.border),
          boxShadow: const [BoxShadow(color: Colors.black54, blurRadius: 30)],
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (working)
              SizedBox(
                width: 44,
                height: 44,
                child: CircularProgressIndicator(
                  value: info.status == DspStatus.connecting ? null : info.progress,
                  color: AppColors.accent,
                  backgroundColor: AppColors.border,
                  strokeWidth: 4,
                ),
              )
            else
              Icon(info.status == DspStatus.error ? Icons.error_outline : Icons.link_off,
                  size: 40, color: AppColors.danger),
            const SizedBox(height: 16),
            Text(title,
                textAlign: TextAlign.center,
                style: interWeight(const TextStyle(fontSize: 17, color: AppColors.text), FontWeight.w600)),
            if (detail != null) ...[
              const SizedBox(height: 6),
              Text(detail, textAlign: TextAlign.center, style: const TextStyle(fontSize: 12, color: AppColors.muted)),
            ],
            if (!info.enabled) ...[
              const SizedBox(height: 20),
              FilledButton.icon(
                style: FilledButton.styleFrom(padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 16)),
                icon: const Icon(Icons.power_settings_new),
                label: const Text('Connect'),
                onPressed: connection.connect,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

// ─── Debug log ───

class _DebugPanel extends StatefulWidget {
  final ConnectionProvider connection;

  const _DebugPanel({required this.connection});

  @override
  State<_DebugPanel> createState() => _DebugPanelState();
}

class _DebugPanelState extends State<_DebugPanel> {
  final ScrollController _scroll = ScrollController();
  bool _atNewest = true;
  bool _showTimestamps = false;
  int _lastCount = 0;

  @override
  void initState() {
    super.initState();
    _scroll.addListener(() {
      final pos = _scroll.position;
      // reverse: true – the newest messages are at maxScrollExtent (top)
      _atNewest = pos.pixels >= pos.maxScrollExtent - 10.0;
    });
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final connection = widget.connection;
    final entries = connection.debugEntries;
    if (entries.length != _lastCount) {
      _lastCount = entries.length;
      if (_atNewest && _scroll.hasClients) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
        });
      }
    }
    return Container(
      margin: const EdgeInsets.fromLTRB(8, 4, 8, 8),
      decoration: BoxDecoration(
        color: AppColors.backgroundDeep,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: AppColors.border),
      ),
      child: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 4, 4, 0),
            child: Row(children: [
              Text('Debug log', style: interWeight(const TextStyle(fontSize: 12), FontWeight.w600)),
              const Spacer(),
              _check('Rx', connection.showRx, connection.toggleShowRx),
              _check('Tx', connection.showTx, connection.toggleShowTx),
              _check('Decode', connection.showDecoded, connection.toggleShowDecoded),
              _check('Time', _showTimestamps, () => setState(() => _showTimestamps = !_showTimestamps)),
              IconButton(
                icon: const Icon(Icons.delete_sweep_outlined, size: 18),
                tooltip: 'Clear',
                onPressed: connection.clearMessages,
              ),
            ]),
          ),
          Expanded(
            child: ListView.builder(
              controller: _scroll,
              reverse: true,
              itemCount: entries.length,
              itemBuilder: (context, index) {
                final entry = entries[index];
                final msg = entry.display(connection.showDecoded);
                return Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 1),
                  child: Text(
                    _showTimestamps ? '[${entry.timestamp.toStringAsFixed(4)}] $msg' : msg,
                    style: monoValue.copyWith(fontSize: 10, color: AppColors.muted),
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }

  Widget _check(String label, bool value, VoidCallback onChanged) => Row(mainAxisSize: MainAxisSize.min, children: [
        Checkbox(
          value: value,
          onChanged: (_) => onChanged(),
          materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
          visualDensity: VisualDensity.compact,
        ),
        Text(label, style: const TextStyle(fontSize: 11)),
        const SizedBox(width: 4),
      ]);
}
