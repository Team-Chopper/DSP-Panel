// Shared RTA audio capture + FFT service
// Used by both the standalone RTA tab and the GEQ overlay (and its Auto-EQ).
//
// 48 kHz capture (44.1 kHz if the device does not offer it), 16384-point
// FFT (≈ 2.9 Hz per bin) with 75 % overlap (≈ 12 spectra per second),
// displayed in 1/24-octave bands with selectable averaging. For the Auto-EQ
// a measurement accumulates the power spectrum over several seconds and
// reduces it to the 31 ISO 1/3-octave bands of the GEQ.

import 'dart:async';
import 'dart:math' as math;
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_recorder/flutter_recorder.dart';
// ignore: implementation_imports
import 'package:flutter_recorder/src/enums.dart' show CaptureDevice;
import 'package:fftea/fftea.dart';

// ─── Constants ───

const int rtaFftSize = 16384;
const int rtaHop = rtaFftSize ~/ 4; // 75 % overlap
const int rtaPreferredSampleRate = 48000;
const int rtaBandsPerOctave = 24;
const double rtaDisplayMinFreq = 20.0;
const double rtaDisplayMaxFreq = 20000.0;
final int rtaBandCount = (math.log(rtaDisplayMaxFreq / rtaDisplayMinFreq) / math.ln2 * rtaBandsPerOctave).round();
const double rtaMinDb = -140.0;
const double rtaMaxDb = 10.0;
const double rtaDbRange = rtaMaxDb - rtaMinDb;
const double rtaPeakDecayDbPerSec = 6.0;

/// Pre-computed log-spaced band edge frequencies.
/// Band i spans from rtaBandEdges[i] to rtaBandEdges[i+1].
final List<double> rtaBandEdges = () {
  final logMin = math.log(rtaDisplayMinFreq);
  final logMax = math.log(rtaDisplayMaxFreq);
  return List<double>.generate(
    rtaBandCount + 1,
    (i) => math.exp(logMin + (logMax - logMin) * i / rtaBandCount),
  );
}();

/// Averaging of the displayed spectrum (time constant in seconds, 0 = none).
enum RtaAveraging {
  none('Off', 0),
  fast('Fast', 0.125),
  slow('Slow', 1.0),
  long('Long', 4.0);

  final String label;
  final double seconds;

  const RtaAveraging(this.label, this.seconds);
}

/// Manages microphone capture, FFT processing, and peak hold for RTA display.
class RtaService {
  List<double> magnitudes = List.filled(rtaBandCount, rtaMinDb);
  List<double> peaks = List.filled(rtaBandCount, rtaMinDb);
  bool isRunning = false;
  bool isInitialized = false;

  /// Sample rate the capture actually runs at.
  int sampleRate = rtaPreferredSampleRate;

  RtaAveraging averaging = RtaAveraging.fast;

  /// Called when new FFT data is ready — set this to trigger UI updates.
  VoidCallback? onUpdate;

  final FFT _fft = FFT(rtaFftSize);
  final Float64List _window = Float64List.fromList([
    for (var i = 0; i < rtaFftSize; i++) 0.5 * (1 - math.cos(2 * math.pi * i / (rtaFftSize - 1))),
  ]);
  final List<double> _sampleBuffer = [];
  StreamSubscription? _audioSubscription;
  Timer? _uiTimer;
  bool _hasPendingUpdate = false;
  DateTime _lastProcessTime = DateTime.now();

  /// Smoothed band power (linear) for the display.
  List<double>? _bandPower;

  /// Accumulated bin power of a running measurement.
  Float64List? _measureSum;
  int _measureCount = 0;
  bool get isMeasuring => _measureSum != null;

  /// Seconds of audio in the running measurement.
  double get measuredSeconds => _measureCount * rtaHop / sampleRate;

  List<CaptureDevice> enumerateDevices() {
    return Recorder.instance.listCaptureDevices();
  }

  Future<bool> requestPermission() async {
    // In the browser, flutter_recorder triggers the permission prompt itself.
    if (!kIsWeb &&
        (defaultTargetPlatform == TargetPlatform.android ||
            defaultTargetPlatform == TargetPlatform.iOS)) {
      try {
        const channel = MethodChannel('dsp/permissions');
        return await channel.invokeMethod<bool>('requestMicrophone') ?? false;
      } catch (_) {
        return true;
      }
    }
    return true;
  }

  /// Start audio capture and FFT processing.
  /// [deviceId] — capture device ID (0 = default on mobile).
  Future<void> startCapture({required int deviceId}) async {
    try {
      try {
        await _init(deviceId, rtaPreferredSampleRate);
      } catch (_) {
        await _init(deviceId, 44100);
      }
      Recorder.instance.start();
      Recorder.instance.startStreamingData();
      isInitialized = true;
      isRunning = true;
      _lastProcessTime = DateTime.now();

      _sampleBuffer.clear();
      _audioSubscription =
          Recorder.instance.uint8ListStream.listen((container) {
        final f32 = container.toF32List(from: PCMFormat.f32le);
        _sampleBuffer.addAll(f32);

        while (_sampleBuffer.length >= rtaFftSize) {
          _processFFT(_sampleBuffer);
          _sampleBuffer.removeRange(0, rtaHop);
        }
      });

      // Throttle UI updates to ~30fps
      _uiTimer = Timer.periodic(const Duration(milliseconds: 33), (_) {
        if (_hasPendingUpdate) {
          _hasPendingUpdate = false;
          onUpdate?.call();
        }
      });
    } catch (e) {
      debugPrint('RtaService: Failed to start capture: $e');
      rethrow;
    }
  }

  Future<void> _init(int deviceId, int rate) async {
    await Recorder.instance.init(
      deviceID: deviceId,
      format: PCMFormat.f32le,
      sampleRate: rate,
      channels: RecorderChannels.mono,
    );
    sampleRate = rate;
  }

  void _processFFT(List<double> samples) {
    final windowed = Float64List(rtaFftSize);
    for (int i = 0; i < rtaFftSize; i++) {
      windowed[i] = samples[i] * _window[i];
    }

    // Power per bin, normalised so that a full-scale sine reads ~0 dB
    // (Hann window coherent gain 0.5).
    final mags = _fft.realFft(windowed).discardConjugates().magnitudes();
    final power = Float64List(mags.length);
    const norm = 2.0 / (rtaFftSize * 0.5);
    for (var i = 0; i < mags.length; i++) {
      final m = mags[i] * norm;
      power[i] = m * m;
    }

    final measure = _measureSum;
    if (measure != null) {
      for (var i = 0; i < power.length; i++) {
        measure[i] += power[i];
      }
      _measureCount++;
    }

    // Aggregate bins into log-spaced bands (power sum; bands narrower than
    // a bin take the power density of the nearest bin).
    final binHz = sampleRate / rtaFftSize;
    final bands = Float64List(rtaBandCount);
    for (int b = 0; b < rtaBandCount; b++) {
      final lo = rtaBandEdges[b] / binHz;
      final hi = rtaBandEdges[b + 1] / binHz;
      final loBin = lo.ceil().clamp(1, power.length - 1);
      final hiBin = hi.floor().clamp(1, power.length - 1);
      if (hiBin < loBin) {
        bands[b] = power[((lo + hi) / 2).round().clamp(1, power.length - 1)] * (hi - lo);
      } else {
        var sum = 0.0;
        for (int i = loBin; i <= hiBin; i++) {
          sum += power[i];
        }
        bands[b] = sum;
      }
    }

    final now = DateTime.now();
    final dt = now.difference(_lastProcessTime).inMicroseconds / 1e6;
    _lastProcessTime = now;

    // Exponential averaging of the band power
    final previous = _bandPower;
    final tau = averaging.seconds;
    if (previous == null || previous.length != rtaBandCount || tau <= 0) {
      _bandPower = bands;
    } else {
      final alpha = 1 - math.exp(-rtaHop / sampleRate / tau);
      for (var b = 0; b < rtaBandCount; b++) {
        previous[b] += alpha * (bands[b] - previous[b]);
      }
    }

    final bandDb = [for (final p in _bandPower!) _db(p)];
    final decay = rtaPeakDecayDbPerSec * dt;
    peaks = [
      for (int b = 0; b < rtaBandCount; b++) math.max(bandDb[b], peaks[b] - decay).clamp(rtaMinDb, rtaMaxDb),
    ];
    magnitudes = bandDb;
    _hasPendingUpdate = true;
  }

  static double _db(double power) =>
      power <= 0 ? rtaMinDb : (10 * math.log(power) / math.ln10).clamp(rtaMinDb, rtaMaxDb);

  /// Clear the peak hold and the averaging.
  void resetPeaks() {
    peaks = List.filled(rtaBandCount, rtaMinDb);
    _bandPower = null;
  }

  // ─── Measurement (Auto-EQ) ───

  /// Start accumulating the spectrum.
  void startMeasurement() {
    _measureSum = Float64List(rtaFftSize ~/ 2 + 1);
    _measureCount = 0;
  }

  /// Stop the measurement; returns the level in dB of each 1/3-octave band
  /// around [centers], or null if nothing was measured.
  List<double>? stopMeasurement(List<double> centers) {
    final sum = _measureSum;
    final count = _measureCount;
    _measureSum = null;
    if (sum == null || count == 0) return null;
    final binHz = sampleRate / rtaFftSize;
    final factor = math.pow(2, 1 / 6).toDouble();
    return [
      for (final fc in centers)
        () {
          final lo = (fc / factor / binHz).ceil().clamp(1, sum.length - 1);
          final hi = (fc * factor / binHz).floor().clamp(lo, sum.length - 1);
          var p = 0.0;
          for (var i = lo; i <= hi; i++) {
            p += sum[i];
          }
          return 10 * math.log(math.max(p / count, 1e-20)) / math.ln10;
        }(),
    ];
  }

  void cancelMeasurement() => _measureSum = null;

  /// Stop capture and reset display state.
  void stopCapture() {
    release();
    magnitudes = List.filled(rtaBandCount, rtaMinDb);
    peaks = List.filled(rtaBandCount, rtaMinDb);
    _bandPower = null;
  }

  /// Release native resources — safe to call from dispose().
  void release() {
    _uiTimer?.cancel();
    _uiTimer = null;
    _hasPendingUpdate = false;
    _audioSubscription?.cancel();
    _audioSubscription = null;
    _sampleBuffer.clear();
    _measureSum = null;

    if (isInitialized) {
      try {
        Recorder.instance.stopStreamingData();
        Recorder.instance.deinit();
      } catch (e) {
        debugPrint('RtaService: Failed to stop capture: $e');
      }
      isInitialized = false;
    }
    isRunning = false;
  }
}

/// Auto-EQ: GEQ correction for a measured 1/3-octave response (pink noise
/// through the system, measured with a microphone), like the auto-EQ of
/// system processors.
///
/// The bands between [loHz] and [hiHz] are corrected towards their average
/// level; the correction is smoothed over neighbouring bands, limited to
/// [maxBoost] / [maxCut] and added to the [current] GEQ (the measurement
/// already includes it). Bands outside the range keep their value.
List<double> autoEqCorrection({
  required List<double> measuredDb,
  required List<double> centers,
  required List<double> current,
  double loHz = 40,
  double hiHz = 16000,
  double maxBoost = 6,
  double maxCut = 12,
}) {
  final inRange = [for (final f in centers) f >= loHz && f <= hiHz];
  final used = [for (var i = 0; i < centers.length; i++) if (inRange[i]) measuredDb[i]];
  if (used.isEmpty) return List.of(current);
  // Median as reference: robust against single deep notches or peaks.
  final sorted = [...used]..sort();
  final reference = sorted[sorted.length ~/ 2];
  final raw = [for (var i = 0; i < centers.length; i++) inRange[i] ? reference - measuredDb[i] : 0.0];
  return [
    for (var i = 0; i < centers.length; i++)
      if (!inRange[i])
        current[i]
      else
        () {
          final prev = i > 0 && inRange[i - 1] ? raw[i - 1] : raw[i];
          final next = i < centers.length - 1 && inRange[i + 1] ? raw[i + 1] : raw[i];
          final smooth = 0.25 * prev + 0.5 * raw[i] + 0.25 * next;
          final target = current[i] + smooth.clamp(-maxCut, maxBoost);
          return ((target.clamp(-12.0, 12.0)) * 10).round() / 10;
        }(),
  ];
}
