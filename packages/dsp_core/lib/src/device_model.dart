// Device Models
//
// The t.racks DSP 206 and DSP 408 speak the same protocol; they differ in
// the number of channels and in the channel bytes of the outputs:
//
//   DSP 408: In A–D = 0x00–0x03, Out 1–8 = 0x04–0x0B
//   DSP 206: In A–B = 0x00–0x01, Out 1–6 = 0x02–0x07
//
// The DSP 206 mapping was verified on hardware by the FlexiFlizz project
// (https://github.com/FlexiFlizz/t.racks-DSP-control). Channel labels
// ('In A' … 'Out 8') are used as keys in the device state, the web-panel API
// and the UI.

class DeviceModel {
  /// Stable identifier used in configuration and API ('dsp408', 'dsp206').
  final String id;

  /// Display name ('DSP 408').
  final String name;

  final List<String> inputs;
  final List<String> outputs;

  /// Protocol channel byte of the first output.
  final int _firstOutputByte;

  const DeviceModel._(this.id, this.name, this.inputs, this.outputs, this._firstOutputByte);

  static const dsp408 = DeviceModel._(
    'dsp408',
    'DSP 408',
    ['In A', 'In B', 'In C', 'In D'],
    ['Out 1', 'Out 2', 'Out 3', 'Out 4', 'Out 5', 'Out 6', 'Out 7', 'Out 8'],
    0x04,
  );

  static const dsp206 = DeviceModel._(
    'dsp206',
    'DSP 206',
    ['In A', 'In B'],
    ['Out 1', 'Out 2', 'Out 3', 'Out 4', 'Out 5', 'Out 6'],
    0x02,
  );

  static const all = [dsp408, dsp206];

  /// Model by id; unknown ids fall back to the DSP 408.
  static DeviceModel byId(String? id) =>
      all.firstWhere((m) => m.id == id, orElse: () => dsp408);

  static bool isKnownId(String id) => all.any((m) => m.id == id);

  /// Recognise the model from the device info string (e.g. "DSP408 V0104P").
  static DeviceModel? detect(String deviceInfo) {
    final compact = deviceInfo.toUpperCase().replaceAll(RegExp(r'[^A-Z0-9]'), '');
    if (compact.contains('206')) return dsp206;
    if (compact.contains('408')) return dsp408;
    return null;
  }

  /// All channels in protocol and meter order: inputs, then outputs.
  List<String> get channels => [...inputs, ...outputs];

  /// Number of meter levels in a 0x40 response.
  int get meterCount => inputs.length + outputs.length;

  bool isInput(String channel) => inputs.contains(channel);
  bool isOutput(String channel) => outputs.contains(channel);
  bool hasChannel(String channel) => isInput(channel) || isOutput(channel);

  /// Protocol channel byte, or null if the model has no such channel.
  int? channelByte(String channel) {
    final i = inputs.indexOf(channel);
    if (i >= 0) return i;
    final o = outputs.indexOf(channel);
    if (o >= 0) return _firstOutputByte + o;
    return null;
  }

  /// Channel label for a protocol channel byte, or null.
  String? channelForByte(int byte) {
    if (byte < inputs.length) return inputs[byte];
    final o = byte - _firstOutputByte;
    return (o >= 0 && o < outputs.length) ? outputs[o] : null;
  }

  /// Matrix routing bit of an input (In A = 0x01, In B = 0x02, …).
  int matrixBit(String input) => 1 << inputs.indexOf(input);

  /// Bitmask with one bit per input.
  int get matrixMask => (1 << inputs.length) - 1;

  /// Channel link bit: position of the channel within its group (inputs or
  /// outputs), as stored in the config dump (In A = 0x01, Out 1 = 0x01).
  int linkBit(String channel) {
    final i = inputs.indexOf(channel);
    return 1 << (i >= 0 ? i : outputs.indexOf(channel));
  }

  /// The channels of the same group (inputs or outputs) as [channel].
  List<String> groupOf(String channel) => isInput(channel) ? inputs : outputs;

  /// Bitmask with one bit per channel of [channel]'s group.
  int groupMask(String channel) => (1 << groupOf(channel).length) - 1;

  /// Name the device uses for a channel by default ("InA", "Out1").
  String defaultDeviceName(String channel) => channel.replaceAll(' ', '');

  @override
  String toString() => name;
}

/// Number of GEQ bands per input channel (31-band ISO 1/3 octave).
const int geqBandCount = 31;

/// Inputs have 8 PEQ bands, outputs have 9 (both models).
int peqBandCountFor(DeviceModel model, String channel) => model.isInput(channel) ? 8 : 9;
