import 'package:dsp/devices/t_racks408/providers/connection_provider.dart';
import 'package:dsp/devices/t_racks408/providers/device_provider.dart';
import 'package:dsp/devices/t_racks408/widgets/console_fader.dart';
import 'package:dsp/devices/t_racks408/widgets/value_field.dart';
import 'package:dsp_core/dsp_core.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('DeviceProvider (client proxy)', () {
    late List<Map<String, Object?>> sent;
    late bool ready;
    late DeviceProvider device;

    setUp(() {
      sent = [];
      ready = true;
      device = DeviceProvider('dsp1', sent.add, () => ready);
    });

    test('applies changes locally and coalesces what it sends', () {
      fakeAsync((async) {
        for (var i = 0; i <= 20; i++) {
          device.setGain('In A', -i.toDouble());
        }
        expect(device.getInputGain('In A'), -20.0);
        expect(sent, isEmpty);
        async.elapse(const Duration(milliseconds: 50));
        expect(sent, [
          {'t': Msg.op, 'dsp': 'dsp1', 'op': DeviceOps.gain('In A', -20.0)},
        ]);
      });
    });

    test('ignores device changes while the DSP is not ready', () {
      fakeAsync((async) {
        ready = false;
        device.setGain('Out 1', -6);
        device.toggleMute('Out 1');
        device.setAlias('Out 1', 'Sub');
        async.elapse(const Duration(milliseconds: 50));
        expect(device.getOutputGain('Out 1'), 0.0);
        expect(device.getOutputMute('Out 1'), isFalse);
        expect(device.getAlias('Out 1'), 'Out 1');
        expect(sent, isEmpty);
      });
    });

    test('channel names are stored on the device', () {
      fakeAsync((async) {
        device.setAlias('Out 1', 'Sub');
        expect(device.getAlias('Out 1'), 'Sub');
        device.resetAlias('In A');
        async.elapse(const Duration(milliseconds: 50));
        expect(sent.map((m) => m['op']), [
          DeviceOps.channelName('Out 1', 'Sub'),
          DeviceOps.channelName('In A', null),
        ]);
        expect(device.getAlias('In A'), 'In A');
      });
    });

    test('topology follows the model of the state from the server', () {
      expect(device.inputs.length, 4);
      device.applyState(DeviceState(DeviceModel.dsp206).toJson());
      expect(device.model, DeviceModel.dsp206);
      expect(device.inputs, ['In A', 'In B']);
      expect(device.outputs.length, 6);
      device.applyMeters(List.filled(8, -12.0));
      expect(device.getMeterDb('Out 6'), -12.0);
      expect(device.getMeterDb('Out 7'), ProtocolService.silenceDb);
    });

    test('meters: peak hold, clip and the compressor/limiter status from the DSP', () {
      final levels = List.filled(12, -30.0);
      levels[0] = -0.2; // In A at full scale
      // Out 2: compressor working, Out 3: limiter working
      device.applyMeters(levels, dynamics: 0x0402);
      expect(device.isClipping('In A'), isTrue);
      expect(device.isClipping('In B'), isFalse);
      expect(device.isCompressing('Out 2'), isTrue);
      expect(device.isLimiting('Out 2'), isFalse);
      expect(device.isLimiting('Out 3'), isTrue);
      expect(device.getMeterPeakDb('In A'), -0.2);
      device.applyMeters(List.filled(12, -40.0));
      expect(device.getMeterPeakDb('In A'), -0.2, reason: 'held');
      expect(device.isLimiting('Out 3'), isTrue, reason: 'indicators stay lit for a moment');
    });

    test('matrix crossing gain is sent per crossing point', () {
      fakeAsync((async) {
        device.setMatrixGain('Out 2', 'In C', -7.25);
        device.setMatrixGain('Out 2', 'In D', 3);
        expect(device.getMatrixGain('Out 2', 'In C'), -7.3);
        expect(device.getMatrixGain('Out 2', 'In D'), 0.0);
        async.elapse(const Duration(milliseconds: 50));
        expect(sent.map((m) => m['op']), [
          DeviceOps.matrixGain('Out 2', 'In C', -7.25),
          DeviceOps.matrixGain('Out 2', 'In D', 3),
        ]);
      });
    });

    test('filter on/off restores the last slope', () {
      fakeAsync((async) {
        device.setHiPass('Out 1', slope: 10);
        device.setHiPass('Out 1', enabled: false);
        expect(device.getHiPass('Out 1').slope, 0);
        device.setHiPass('Out 1', enabled: true);
        expect(device.getHiPass('Out 1').slope, 10);
        device.setLoPass('Out 2', enabled: true);
        expect(device.getLoPass('Out 2').slope, ProtocolService.defaultSlope);
        async.elapse(const Duration(milliseconds: 50));
      });
    });

    test('state from the server keeps unsent local changes', () {
      fakeAsync((async) {
        device.toggleMatrixInput('Out 2', 'In C');
        final server = DeviceState()..currentPreset = 'Show';
        device.applyState(server.toJson());
        expect(device.currentPreset, 'Show');
        expect(device.getMatrixEnabled('Out 2', 'In C'), isTrue);
        async.elapse(const Duration(milliseconds: 50));
      });
    });

    test('applies ops from other clients', () {
      device.applyRemoteOp(DeviceOps.peqBand('Out 3', 1, PeqBand(gainDb: 4, freqRaw: 250)));
      expect(device.getPeqBands('Out 3')[1].gainDb, 4.0);
      device.applyRemoteOp({'op': 'gain', 'ch': 'nope', 'db': 1}); // ignored
    });

    test('linked channels change together, only the original op is sent', () {
      fakeAsync((async) {
        device.setLinkSlaves('Out 1', ['Out 2'], copySettings: false);
        async.elapse(const Duration(milliseconds: 50));
        expect(sent.map((m) => m['op']), [DeviceOps.link('Out 1', 0x03)]);
        expect(device.linkedWith('Out 1'), ['Out 2']);

        sent.clear();
        device.setGain('Out 1', -6);
        device.toggleMute('Out 2');
        expect(device.getOutputGain('Out 2'), -6.0);
        expect(device.getOutputMute('Out 1'), isTrue);
        expect(device.getOutputGain('Out 3'), 0.0);
        async.elapse(const Duration(milliseconds: 50));
        // The server distributes the change to the linked channels.
        expect(sent.map((m) => m['op']), [DeviceOps.gain('Out 1', -6), DeviceOps.mute('Out 2', true)]);
      });
    });

    test('copies the settings of a channel', () {
      fakeAsync((async) {
        device.setGain('Out 1', -3);
        device.setPeqBand('Out 1', 2, gainDb: 5, freqRaw: 120);
        device.setHiPass('Out 1', enabled: true, freqRaw: 80);
        device.setAlias('Out 1', 'Top L');
        device.toggleMatrixInput('Out 1', 'In C');
        final routingOut3 = device.getMatrixEnabled('Out 3', 'In C');
        device.copyChannel('Out 1', ['Out 3', 'Out 4']);
        expect(device.getMatrixEnabled('Out 3', 'In C'), routingOut3, reason: 'routing is not copied');
        for (final ch in ['Out 3', 'Out 4']) {
          expect(device.getOutputGain(ch), -3.0);
          expect(device.getPeqBand(ch, 2).gainDb, 5.0);
          expect(device.getPeqBand(ch, 2).freqRaw, 120);
          expect(device.getHiPass(ch).freqRaw, 80);
          expect(device.getAlias(ch), ch, reason: 'names are not copied');
        }
        expect(() => device.copyChannel('Out 1', ['In A']), throwsArgumentError);
        async.elapse(const Duration(milliseconds: 50));
      });
    });
  });

  test('ConnectionProvider maps server status', () {
    final sent = <Map<String, Object?>>[];
    final connection = ConnectionProvider(
        'dsp1', sent.add, const DspInfo(id: 'dsp1', name: 'FOH', host: '10.0.0.5', port: 9761, enabled: true));
    expect(connection.isConnected, isFalse);

    connection.updateInfo(const DspInfo(
        id: 'dsp1', name: 'FOH', host: '10.0.0.5', port: 9761, enabled: true,
        status: DspStatus.initializing, progress: 0.5));
    expect(connection.isLoading, isTrue);
    expect(connection.isSocketConnected, isTrue);
    expect(connection.initProgress, 0.5);

    connection.updateInfo(const DspInfo(
        id: 'dsp1', name: 'FOH', host: '10.0.0.5', port: 9761, enabled: true, status: DspStatus.ready));
    expect(connection.isConnected, isTrue);
    connection.loadPreset(2);
    expect(sent.single, {'t': Msg.loadPreset, 'dsp': 'dsp1', 'index': 2});
  });

  test('typed values: comma decimals, units, kHz', () {
    expect(parseNumber('-7,5'), -7.5);
    expect(parseNumber('+3 dB'), 3.0);
    expect(parseNumber('12ms'), 12.0);
    expect(parseNumber('abc'), isNull);
    expect(parseFrequency('1.2k'), 1200.0);
    expect(parseFrequency('1,5 kHz'), 1500.0);
    expect(parseFrequency('80'), 80.0);
  });

  test('console fader tapers map values and positions both ways', () {
    for (final db in [-60.0, -33.0, -6.0, 0.0, 4.5, 12.0]) {
      expect(gainTaper.fromPosition(gainTaper.toPosition(db)), closeTo(db, 1e-9));
    }
    expect(gainTaper.toPosition(0), 0.78);
    const log = LogTaper(1, 1000);
    expect(log.toPosition(31.6227766), closeTo(0.5, 1e-6));
    expect(log.fromPosition(1), 1000);
  });
}
