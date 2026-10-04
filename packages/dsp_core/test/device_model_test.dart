import 'package:dsp_core/dsp_core.dart';
import 'package:test/test.dart';

void main() {
  test('detects the model from the device info', () {
    expect(DeviceModel.detect('DSP408 V0104P'), DeviceModel.dsp408);
    expect(DeviceModel.detect('DSP 206 V0104P'), DeviceModel.dsp206);
    expect(DeviceModel.detect('dsp-206'), DeviceModel.dsp206);
    expect(DeviceModel.detect('Something else'), isNull);
  });

  test('channel bytes', () {
    const m206 = DeviceModel.dsp206;
    expect(m206.channels, ['In A', 'In B', 'Out 1', 'Out 2', 'Out 3', 'Out 4', 'Out 5', 'Out 6']);
    expect(m206.channelByte('Out 1'), 0x02);
    expect(m206.channelByte('In C'), isNull);
    expect(m206.channelForByte(0x07), 'Out 6');
    expect(m206.channelForByte(0x08), isNull);
    expect(m206.meterCount, 8);
    expect(m206.matrixMask, 0x03);

    const m408 = DeviceModel.dsp408;
    expect(m408.channelByte('Out 1'), 0x04);
    expect(m408.channelForByte(0x0B), 'Out 8');
    expect(m408.meterCount, 12);
    expect(m408.matrixBit('In D'), 0x08);
  });

  test('lookup by id', () {
    expect(DeviceModel.byId('dsp206'), DeviceModel.dsp206);
    expect(DeviceModel.byId('unknown'), DeviceModel.dsp408);
    expect(DeviceModel.isKnownId('dsp206'), isTrue);
    expect(DeviceModel.isKnownId('auto'), isFalse);
  });

  test('default state follows the topology', () {
    final s = DeviceState(DeviceModel.dsp206);
    expect(s.gains.keys, DeviceModel.dsp206.channels);
    expect(s.geqBands.keys, ['In A', 'In B']);
    expect(s.compressors.keys.length, 6);
    expect(s.matrixGains.length, 12);
    expect(s.matrixRouting, {'Out 1': 1, 'Out 2': 2, 'Out 3': 1, 'Out 4': 2, 'Out 5': 1, 'Out 6': 2});
    expect(s.peqBands['In B']!.length, 8);
    expect(s.peqBands['Out 6']!.length, 9);
  });
}
