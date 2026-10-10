// Web-panel API
//
// Messages between browser and server are JSON objects with a "t" (type)
// field, sent over a single WebSocket at /ws.
//
// Client → server
//   {t: subscribe,   dsp}                 full state + log, then live updates
//   {t: unsubscribe, dsp}
//   {t: addDsp,      name, host, port, model}     model: auto|dsp408|dsp206
//   {t: updateDsp,   id, name?, host?, port?, enabled?, model?}
//   {t: removeDsp,   id}
//   {t: op,          dsp, op}             see device_ops.dart
//   {t: loadPreset,  dsp, index}
//   {t: savePreset,  dsp, index, name}
//   {t: setRefresh,  dsp, ms}
//   {t: debug,       dsp, rx, tx}         packet logging on/off
//   {t: clearLog,    dsp}
//
// Presets stored on the server (edited offline, loaded onto a DSP)
//   {t: presetCreate, name, model?, fromDsp?}   empty preset or snapshot of a DSP
//   {t: presetRename, preset, name}
//   {t: presetDelete, preset}
//   {t: presetApply,  preset, dsp}              send all values to the DSP
//   subscribe / unsubscribe / op with dsp: "preset:<id>" edit a preset offline
//
// Server → client
//   {t: hello,      dsps: [DspInfo]}
//   {t: dspInfo,    info: DspInfo}        added or changed
//   {t: dspRemoved, id}
//   {t: state,      dsp, state}           full DeviceState
//   {t: op,         dsp, op}              change made by another client
//   {t: meters,     dsp, v: [dBFS per channel], dyn, tx, rx}
//                   dyn: bit n = compressor, bit 8+n = limiter of output n active
//   {t: log,        dsp, entries: [LogEntry], reset?}
//   {t: presets,    presets: [ServerPresetInfo]}  on connect and on every change
//   {t: error,      dsp?, message}

class Msg {
  static const subscribe = 'subscribe';
  static const unsubscribe = 'unsubscribe';
  static const addDsp = 'addDsp';
  static const updateDsp = 'updateDsp';
  static const removeDsp = 'removeDsp';
  static const op = 'op';
  static const loadPreset = 'loadPreset';
  static const savePreset = 'savePreset';
  static const setRefresh = 'setRefresh';
  static const debug = 'debug';
  static const clearLog = 'clearLog';
  static const presetCreate = 'presetCreate';
  static const presetRename = 'presetRename';
  static const presetDelete = 'presetDelete';
  static const presetApply = 'presetApply';

  /// Prefix of the "dsp" id that addresses a server preset (offline editor).
  static const presetPrefix = 'preset:';

  static const hello = 'hello';
  static const dspInfo = 'dspInfo';
  static const dspRemoved = 'dspRemoved';
  static const state = 'state';
  static const meters = 'meters';
  static const log = 'log';
  static const presets = 'presets';
  static const error = 'error';
}

/// A preset stored on the server.
class ServerPresetInfo {
  final String id;
  final String name;

  /// DeviceModel id the preset belongs to.
  final String model;

  /// Last change, milliseconds since epoch.
  final int updated;

  const ServerPresetInfo({required this.id, required this.name, required this.model, required this.updated});

  Map<String, Object?> toJson() => {'id': id, 'name': name, 'model': model, 'updated': updated};

  factory ServerPresetInfo.fromJson(Map<String, Object?> json) => ServerPresetInfo(
    id: json['id'] as String,
    name: json['name'] as String,
    model: json['model'] as String,
    updated: (json['updated'] as num).toInt(),
  );
}

/// Connection status of a DSP as seen by the server.
enum DspStatus {
  /// Not connected and not trying (disabled).
  disconnected,

  /// Opening the TCP connection.
  connecting,

  /// Running the init sequence.
  initializing,

  /// Ready for commands.
  ready,

  /// Loading or saving a preset.
  busy,

  /// Connection failed; the server retries automatically.
  error,
}

/// Default keepalive/meter polling interval in ms (allowed: 5–300).
const int defaultRefreshMs = 40;

/// Public description of one configured DSP and its connection.
class DspInfo {
  final String id;
  final String name;
  final String host;
  final int port;
  final bool enabled;
  final DspStatus status;

  /// Progress of init or preset load/save, 0.0 – 1.0.
  final double progress;
  final String? message;
  final int refreshMs;
  final bool logRx;
  final bool logTx;

  /// Model in use ('dsp408', 'dsp206'): detected from the device, or the
  /// configured one.
  final String model;

  /// Configured model: 'auto' (detect) or a model id.
  final String modelSetting;

  /// False if the config dump could not be decoded plausibly; the panel
  /// then shows default values and a warning.
  final bool configReadable;

  const DspInfo({
    required this.id,
    required this.name,
    required this.host,
    required this.port,
    required this.enabled,
    this.status = DspStatus.disconnected,
    this.progress = 0.0,
    this.message,
    this.refreshMs = defaultRefreshMs,
    this.logRx = false,
    this.logTx = false,
    this.model = 'dsp408',
    this.modelSetting = 'auto',
    this.configReadable = true,
  });

  Map<String, Object?> toJson() => {
    'id': id,
    'name': name,
    'host': host,
    'port': port,
    'enabled': enabled,
    'status': status.name,
    'progress': progress,
    'message': message,
    'refreshMs': refreshMs,
    'logRx': logRx,
    'logTx': logTx,
    'model': model,
    'modelSetting': modelSetting,
    'configReadable': configReadable,
  };

  factory DspInfo.fromJson(Map<String, Object?> json) => DspInfo(
    id: json['id'] as String,
    name: json['name'] as String,
    host: json['host'] as String,
    port: (json['port'] as num).toInt(),
    enabled: json['enabled'] as bool,
    status: DspStatus.values.firstWhere(
        (s) => s.name == json['status'], orElse: () => DspStatus.disconnected),
    progress: (json['progress'] as num? ?? 0).toDouble(),
    message: json['message'] as String?,
    refreshMs: (json['refreshMs'] as num? ?? defaultRefreshMs).toInt(),
    logRx: json['logRx'] as bool? ?? false,
    logTx: json['logTx'] as bool? ?? false,
    model: json['model'] as String? ?? 'dsp408',
    modelSetting: json['modelSetting'] as String? ?? 'auto',
    configReadable: json['configReadable'] as bool? ?? true,
  );
}

/// One line in a DSP's debug log. Packet entries carry both a raw hex and a
/// decoded representation so the client can toggle between them.
class LogEntry {
  /// Seconds since the connection was opened.
  final double timestamp;
  final String text;
  final String? rawHex;
  final String? decoded;

  const LogEntry.status(this.text, this.timestamp)
      : rawHex = null,
        decoded = null;

  const LogEntry.packet({
    required this.rawHex,
    required this.decoded,
    required this.timestamp,
  }) : text = '';

  /// The string to display, given the current decode toggle.
  String display(bool decodeOn) {
    if (rawHex != null) return decodeOn ? decoded! : rawHex!;
    return text;
  }

  Map<String, Object?> toJson() => {
    'ts': timestamp,
    if (rawHex == null) 'text': text,
    if (rawHex != null) 'hex': rawHex,
    if (decoded != null) 'dec': decoded,
  };

  factory LogEntry.fromJson(Map<String, Object?> json) {
    final ts = (json['ts'] as num).toDouble();
    if (json['hex'] != null) {
      return LogEntry.packet(
          rawHex: json['hex'] as String, decoded: json['dec'] as String, timestamp: ts);
    }
    return LogEntry.status(json['text'] as String, ts);
  }
}
