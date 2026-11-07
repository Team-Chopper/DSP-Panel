// Inline number field: shows the current value and can be typed into
// directly (Enter or leaving the field applies, Escape reverts). Replaces
// the old input pop-ups.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../../theme/app_colors.dart';
import '../../../theme/app_theme.dart';

class ValueField extends StatefulWidget {
  /// Formatted current value.
  final String text;

  /// Called with the typed text; return false if it was not accepted.
  final bool Function(String input) onSubmitted;

  final double? width;
  final double height;
  final double fontSize;
  final Color color;
  final TextAlign textAlign;
  final String? tooltip;

  const ValueField({
    super.key,
    required this.text,
    required this.onSubmitted,
    this.width,
    this.height = 26,
    this.fontSize = 11,
    this.color = AppColors.text,
    this.textAlign = TextAlign.center,
    this.tooltip,
  });

  @override
  State<ValueField> createState() => _ValueFieldState();
}

class _ValueFieldState extends State<ValueField> {
  late final _controller = TextEditingController(text: widget.text);
  late final _focus = FocusNode(onKeyEvent: _onKey);
  bool _invalid = false;

  @override
  void initState() {
    super.initState();
    _focus.addListener(_onFocus);
  }

  @override
  void didUpdateWidget(ValueField oldWidget) {
    super.didUpdateWidget(oldWidget);
    // Follow the device while the user is not typing.
    if (!_focus.hasFocus && _controller.text != widget.text) _controller.text = widget.text;
  }

  @override
  void dispose() {
    _focus.dispose();
    _controller.dispose();
    super.dispose();
  }

  void _onFocus() {
    if (_focus.hasFocus) {
      _controller.selection = TextSelection(baseOffset: 0, extentOffset: _controller.text.length);
    } else {
      _apply();
    }
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is KeyDownEvent && event.logicalKey == LogicalKeyboardKey.escape) {
      _controller.text = widget.text;
      setState(() => _invalid = false);
      node.unfocus();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _apply() {
    final input = _controller.text.trim();
    if (input == widget.text || input.isEmpty) {
      _controller.text = widget.text;
      setState(() => _invalid = false);
      return;
    }
    final ok = widget.onSubmitted(input);
    setState(() => _invalid = !ok);
    if (!ok) _controller.text = widget.text;
  }

  @override
  Widget build(BuildContext context) {
    final field = SizedBox(
      width: widget.width,
      height: widget.height,
      child: TextField(
        controller: _controller,
        focusNode: _focus,
        textAlign: widget.textAlign,
        style: monoValue.copyWith(fontSize: widget.fontSize, color: widget.color),
        cursorColor: AppColors.accent,
        keyboardType: const TextInputType.numberWithOptions(decimal: true, signed: true),
        decoration: InputDecoration(
          isDense: true,
          filled: true,
          fillColor: AppColors.backgroundDeep,
          contentPadding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
          enabledBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(6),
            borderSide: BorderSide(color: _invalid ? AppColors.danger : AppColors.border),
          ),
          focusedBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(6),
            borderSide: const BorderSide(color: AppColors.accent),
          ),
        ),
        onSubmitted: (_) => _focus.unfocus(),
      ),
    );
    final tooltip = widget.tooltip;
    return tooltip == null ? field : Tooltip(message: tooltip, waitDuration: const Duration(milliseconds: 700), child: field);
  }
}

// ─── Parsing typed values ───

/// Number from user input: accepts a comma as decimal separator and ignores
/// a unit after the number ("−3,5 dB", "12ms").
double? parseNumber(String input) {
  final match = RegExp(r'^\s*([+\-−]?\d*[.,]?\d+)').firstMatch(input);
  if (match == null) return null;
  return double.tryParse(match.group(1)!.replaceAll(',', '.').replaceAll('−', '-'));
}

/// Frequency in Hz from "1200", "1.2k", "1,2 kHz".
double? parseFrequency(String input) {
  final value = parseNumber(input);
  if (value == null) return null;
  return RegExp(r'\d\s*k', caseSensitive: false).hasMatch(input) ? value * 1000 : value;
}

String formatHz(double hz) {
  if (hz >= 1000) {
    final k = hz / 1000;
    return '${k >= 10 ? k.toStringAsFixed(1) : k.toStringAsFixed(2)}k';
  }
  return hz >= 100 ? hz.toStringAsFixed(0) : hz.toStringAsFixed(1);
}

String formatDbValue(double db) => '${db > 0 ? '+' : ''}${db.toStringAsFixed(1)}';
