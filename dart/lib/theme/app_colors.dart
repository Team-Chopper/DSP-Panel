// Colour palette of the panel (dark theme).
//
// Inputs and outputs have their own accent so they are easy to tell apart
// everywhere (faders, EQ curves, matrix).

import 'package:flutter/material.dart';

class AppColors {
  static const background = Color(0xFF14161A);
  static const backgroundDeep = Color(0xFF101215);
  static const surface = Color(0xFF1C1F24);
  static const surfaceHigh = Color(0xFF22262C);
  static const border = Color(0xFF2E333A);
  static const muted = Color(0xFF7D8590);
  static const text = Color(0xFFE6E8EB);

  static const accent = Color(0xFF4ADE80);
  static const danger = Color(0xFFF87171);
  static const warning = Color(0xFFFBBF24);
  static const info = Color(0xFF38BDF8);

  /// Accent of input channels
  static const input = Color(0xFF38BDF8);

  /// Accent of output channels
  static const output = Color(0xFFA78BFA);

  static Color forChannel(bool isInput) => isInput ? input : output;

  /// Distinct colours for curves of several channels of one group.
  static const curves = [
    Color(0xFF38BDF8), Color(0xFF4ADE80), Color(0xFFFBBF24), Color(0xFFF472B6),
    Color(0xFFA78BFA), Color(0xFFF87171), Color(0xFF2DD4BF), Color(0xFFFB923C),
  ];
}
