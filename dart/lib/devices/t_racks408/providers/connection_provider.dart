// Connection State Provider
//
// Connection status, preset load/save, refresh interval and debug log of
// one DSP. The panel server holds the actual TCP connection; this provider
// shows its status and forwards the user's actions.

import 'package:dsp_core/dsp_core.dart';
import 'package:flutter/foundation.dart';

export 'package:dsp_core/dsp_core.dart' show LogEntry, DspInfo, DspStatus, DeviceModel;

class ConnectionProvider extends ChangeNotifier {
  static const maxDebugEntries = 500;

  final String dspId;
  final void Function(Map<String, Object?> message) _send;

  ConnectionProvider(this.dspId, this._send, this._info, {this.offline = false});

  /// True for a server preset opened in the offline editor (no device).
  final bool offline;

  DspInfo _info;
  final List<LogEntry> _debugEntries = [];
  int _packetsSent = 0;
  int _packetsReceived = 0;
  bool _showDecoded = false;

  DspInfo get info => _info;
  DspStatus get status => _info.status;

  /// Model in use (detected or configured).
  DeviceModel get model => DeviceModel.byId(_info.model);

  /// False if the device's configuration could not be decoded.
  bool get configReadable => _info.configReadable;

  String get connectionStatus {
    if (offline) return 'Offline preset – changes are saved on the server';
    final address = '${_info.host}:${_info.port}';
    switch (_info.status) {
      case DspStatus.disconnected:
        return _info.enabled ? 'Disconnected' : 'Disabled';
      case DspStatus.connecting:
        return 'Connecting to $address …';
      case DspStatus.initializing:
        return 'Initializing $address …';
      case DspStatus.ready:
        return 'Connected to $address';
      case DspStatus.busy:
        return _info.message ?? 'Busy …';
      case DspStatus.error:
        return _info.message ?? 'Error';
    }
  }

  /// Ready for commands (socket connected and not loading).
  bool get isConnected => _info.status == DspStatus.ready;
  bool get isSocketConnected => const {
        DspStatus.initializing,
        DspStatus.ready,
        DspStatus.busy,
      }.contains(_info.status);
  bool get isLoading => const {
        DspStatus.connecting,
        DspStatus.initializing,
        DspStatus.busy,
      }.contains(_info.status);
  double get initProgress => _info.progress;

  int get packetsSent => _packetsSent;
  int get packetsReceived => _packetsReceived;
  int get refreshIntervalMs => _info.refreshMs;

  List<LogEntry> get debugEntries => List.unmodifiable(_debugEntries);
  bool get showRx => _info.logRx;
  bool get showTx => _info.logTx;
  bool get showDecoded => _showDecoded;

  // ─── Actions ───

  /// Enable the DSP; the server connects and keeps reconnecting.
  void connect() => _send({'t': Msg.updateDsp, 'id': dspId, 'enabled': true});

  /// Disable the DSP; the server disconnects.
  void disconnect() => _send({'t': Msg.updateDsp, 'id': dspId, 'enabled': false});

  /// Load a preset by index (0-based)
  void loadPreset(int presetIndex) {
    if (!isConnected) return;
    _send({'t': Msg.loadPreset, 'dsp': dspId, 'index': presetIndex});
  }

  /// Save (store) the current settings to a preset slot (0-based index).
  void savePreset(int presetIndex, String presetName) {
    if (!isConnected) return;
    _send({'t': Msg.savePreset, 'dsp': dspId, 'index': presetIndex, 'name': presetName});
  }

  /// Update the refresh interval (keepalive polling rate) in milliseconds.
  void setRefreshInterval(int ms) => _send({'t': Msg.setRefresh, 'dsp': dspId, 'ms': ms});

  /// Toggle Rx packet log (on the server, for everyone viewing this DSP)
  void toggleShowRx() => _send({'t': Msg.debug, 'dsp': dspId, 'rx': !showRx});

  /// Toggle Tx packet log
  void toggleShowTx() => _send({'t': Msg.debug, 'dsp': dspId, 'tx': !showTx});

  /// Toggle decoded packet descriptions (local display only)
  void toggleShowDecoded() {
    _showDecoded = !_showDecoded;
    notifyListeners();
  }

  /// Clear debug messages
  void clearMessages() => _send({'t': Msg.clearLog, 'dsp': dspId});

  // ─── Updates from the server ───

  void updateInfo(DspInfo info) {
    _info = info;
    notifyListeners();
  }

  void updateCounters(int sent, int received) {
    _packetsSent = sent;
    _packetsReceived = received;
  }

  void applyLog(List<LogEntry> entries, {required bool reset}) {
    if (reset) _debugEntries.clear();
    _debugEntries.addAll(entries);
    if (_debugEntries.length > maxDebugEntries) {
      _debugEntries.removeRange(0, _debugEntries.length - maxDebugEntries);
    }
    notifyListeners();
  }
}
