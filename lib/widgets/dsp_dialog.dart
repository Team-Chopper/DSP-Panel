// Add / edit DSP dialog

import 'package:dsp_core/dsp_core.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../providers/panel_provider.dart';

/// Ask for name, host and port and add a new DSP on the server.
Future<void> showAddDspDialog(BuildContext context) async {
  final result = await showDialog<_DspFields>(
    context: context,
    builder: (_) => const _DspDialog(title: 'Add DSP'),
  );
  if (result != null && context.mounted) {
    context
        .read<PanelProvider>()
        .addDsp(name: result.name, host: result.host, port: result.port, model: result.model);
  }
}

/// Edit or delete an existing DSP.
Future<void> showEditDspDialog(BuildContext context, DspInfo info) async {
  final panel = context.read<PanelProvider>();
  final result = await showDialog<_DspFields>(
    context: context,
    builder: (_) => _DspDialog(title: 'Edit DSP', initial: info),
  );
  if (result == null || !context.mounted) return;
  if (result.delete) {
    panel.removeDsp(info.id);
  } else {
    panel.updateDsp(info.id,
        name: result.name, host: result.host, port: result.port, model: result.model);
  }
}

class _DspFields {
  final String name;
  final String host;
  final int port;
  final String model;
  final bool delete;

  const _DspFields(this.name, this.host, this.port, this.model, {this.delete = false});
}

class _DspDialog extends StatefulWidget {
  final String title;
  final DspInfo? initial;

  const _DspDialog({required this.title, this.initial});

  @override
  State<_DspDialog> createState() => _DspDialogState();
}

class _DspDialogState extends State<_DspDialog> {
  final _formKey = GlobalKey<FormState>();
  late final _name = TextEditingController(text: widget.initial?.name ?? '');
  late final _host = TextEditingController(text: widget.initial?.host ?? '192.168.3.100');
  late final _port = TextEditingController(text: '${widget.initial?.port ?? 9761}');
  late String _model = widget.initial?.modelSetting ?? 'auto';

  @override
  void dispose() {
    _name.dispose();
    _host.dispose();
    _port.dispose();
    super.dispose();
  }

  void _submit() {
    if (!_formKey.currentState!.validate()) return;
    Navigator.pop(
        context, _DspFields(_name.text.trim(), _host.text.trim(), int.parse(_port.text.trim()), _model));
  }

  Future<void> _delete() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Delete DSP'),
        content: Text('Remove "${widget.initial!.name}" from the panel? '
            'The settings on the device itself are not changed.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text('Cancel')),
          TextButton(onPressed: () => Navigator.pop(ctx, true), child: const Text('Delete')),
        ],
      ),
    );
    if (confirmed == true && mounted) {
      Navigator.pop(context, const _DspFields('', '', 0, 'auto', delete: true));
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.title),
      content: Form(
        key: _formKey,
        child: SizedBox(
          width: 360,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextFormField(
                controller: _name,
                autofocus: true,
                maxLength: 40,
                decoration: const InputDecoration(labelText: 'Name', hintText: 'e.g. FOH, Monitors'),
                validator: (v) => (v == null || v.trim().isEmpty) ? 'Please enter a name' : null,
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  Expanded(
                    child: TextFormField(
                      controller: _host,
                      decoration: const InputDecoration(labelText: 'IP Address'),
                      validator: (v) => (v == null ||
                              !RegExp(r'^[A-Za-z0-9.\-:\[\]]{1,253}$').hasMatch(v.trim()))
                          ? 'Invalid address'
                          : null,
                    ),
                  ),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 100,
                    child: TextFormField(
                      controller: _port,
                      keyboardType: TextInputType.number,
                      decoration: const InputDecoration(labelText: 'Port'),
                      validator: (v) {
                        final port = int.tryParse(v?.trim() ?? '');
                        return (port == null || port < 1 || port > 65535) ? '1–65535' : null;
                      },
                      onFieldSubmitted: (_) => _submit(),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              DropdownButtonFormField<String>(
                initialValue: _model,
                decoration: const InputDecoration(
                  labelText: 'Model',
                  helperText: 'Auto detects the model when connecting',
                ),
                items: [
                  DropdownMenuItem(
                    value: 'auto',
                    child: Text(widget.initial == null
                        ? 'Auto'
                        : 'Auto (detected: ${DeviceModel.byId(widget.initial!.model).name})'),
                  ),
                  for (final m in DeviceModel.all) DropdownMenuItem(value: m.id, child: Text(m.name)),
                ],
                onChanged: (v) => setState(() => _model = v ?? 'auto'),
              ),
            ],
          ),
        ),
      ),
      actions: [
        if (widget.initial != null)
          TextButton(
            onPressed: _delete,
            style: TextButton.styleFrom(foregroundColor: Theme.of(context).colorScheme.error),
            child: const Text('Delete'),
          ),
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
        TextButton(onPressed: _submit, child: const Text('Save')),
      ],
    );
  }
}
