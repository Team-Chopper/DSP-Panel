// Theme of the panel: dark, flat surfaces, Inter for UI text.
//
// Inter is bundled as a variable font (assets/fonts/Inter-Variable.ttf);
// font weights are applied through the 'wght' axis.

import 'package:flutter/material.dart';

import 'app_colors.dart';

/// Text style with an Inter weight (works for the variable font).
TextStyle interWeight(TextStyle base, FontWeight weight) =>
    base.copyWith(fontWeight: weight, fontVariations: [FontVariation('wght', weight.value.toDouble())]);

/// Monospaced style for values (dB, Hz, ms), so numbers do not jump.
const monoValue = TextStyle(fontFamily: 'RobotoMono', fontFeatures: [FontFeature.tabularFigures()]);

ThemeData buildAppTheme() {
  final scheme = const ColorScheme.dark(
    surface: AppColors.background,
    primary: AppColors.accent,
    secondary: AppColors.output,
    tertiary: AppColors.warning,
    error: AppColors.danger,
    onSurface: AppColors.text,
    onPrimary: AppColors.background,
    onSecondary: AppColors.background,
    onError: AppColors.background,
    surfaceContainerHighest: AppColors.surfaceHigh,
    surfaceContainer: AppColors.surface,
    outline: AppColors.border,
    outlineVariant: AppColors.border,
  );

  final base = ThemeData.dark().textTheme.apply(
        fontFamily: 'Inter',
        bodyColor: AppColors.text,
        displayColor: AppColors.text,
      );
  final textTheme = base.copyWith(
    titleLarge: interWeight(base.titleLarge!, FontWeight.w600),
    titleMedium: interWeight(base.titleMedium!, FontWeight.w600),
    titleSmall: interWeight(base.titleSmall!, FontWeight.w600),
    labelLarge: interWeight(base.labelLarge!, FontWeight.w500),
  );

  final shape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(10));

  return ThemeData(
    colorScheme: scheme,
    useMaterial3: true,
    fontFamily: 'Inter',
    scaffoldBackgroundColor: AppColors.background,
    textTheme: textTheme,
    dividerColor: AppColors.border,
    appBarTheme: AppBarTheme(
      backgroundColor: AppColors.backgroundDeep,
      foregroundColor: AppColors.text,
      elevation: 0,
      scrolledUnderElevation: 0,
      titleTextStyle: interWeight(
          const TextStyle(fontFamily: 'Inter', fontSize: 18, color: AppColors.text), FontWeight.w700),
    ),
    tabBarTheme: const TabBarThemeData(
      labelColor: AppColors.text,
      unselectedLabelColor: AppColors.muted,
      indicatorColor: AppColors.accent,
      dividerColor: Colors.transparent,
    ),
    cardTheme: CardThemeData(
      color: AppColors.surface,
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: const BorderSide(color: AppColors.border),
      ),
    ),
    elevatedButtonTheme: ElevatedButtonThemeData(
      style: ElevatedButton.styleFrom(
        backgroundColor: AppColors.surfaceHigh,
        foregroundColor: AppColors.text,
        elevation: 0,
        shape: shape,
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(style: FilledButton.styleFrom(shape: shape)),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: AppColors.text,
        side: const BorderSide(color: AppColors.border),
        shape: shape,
      ),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: AppColors.backgroundDeep,
      border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: AppColors.border),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: AppColors.accent),
      ),
      labelStyle: const TextStyle(color: AppColors.muted),
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: AppColors.surface,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: const BorderSide(color: AppColors.border),
      ),
      titleTextStyle: interWeight(
          const TextStyle(fontFamily: 'Inter', fontSize: 18, color: AppColors.text), FontWeight.w600),
    ),
    popupMenuTheme: PopupMenuThemeData(
      color: AppColors.surfaceHigh,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: const BorderSide(color: AppColors.border),
      ),
    ),
    dropdownMenuTheme: const DropdownMenuThemeData(textStyle: TextStyle(color: AppColors.text)),
    checkboxTheme: CheckboxThemeData(
      fillColor: WidgetStateProperty.resolveWith(
          (states) => states.contains(WidgetState.selected) ? AppColors.accent : Colors.transparent),
      checkColor: WidgetStateProperty.all(AppColors.background),
      side: const BorderSide(color: AppColors.muted),
    ),
    sliderTheme: const SliderThemeData(
      activeTrackColor: AppColors.accent,
      thumbColor: AppColors.text,
      inactiveTrackColor: AppColors.border,
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: AppColors.surfaceHigh,
      contentTextStyle: const TextStyle(color: AppColors.text),
      behavior: SnackBarBehavior.floating,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
    ),
    tooltipTheme: TooltipThemeData(
      decoration: BoxDecoration(color: AppColors.surfaceHigh, borderRadius: BorderRadius.circular(8)),
      textStyle: const TextStyle(color: AppColors.text, fontSize: 12),
    ),
  );
}
