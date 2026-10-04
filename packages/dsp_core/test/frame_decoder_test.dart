import 'package:dsp_core/dsp_core.dart';
import 'package:dsp_core/testing.dart';
import 'package:test/test.dart';

void main() {
  final validFrames = [
    for (final hex in configDumpFrames) hexToBytes(hex),
    for (final hex in capturedSession408) hexToBytes(hex),
  ];

  List<int> response(int cmd, List<int> payload) {
    final dataBytes = [0x01, 0x00, payload.length + 1, cmd, ...payload];
    return [0x10, 0x02, ...dataBytes, 0x10, 0x03, ProtocolService.calculateChecksum(dataBytes)];
  }

  test('captured frames satisfy the length and checksum rule', () {
    for (final frame in validFrames) {
      expect(frame.length, frame[4] + 8);
      expect(FrameDecoder().add(frame), [frame]);
    }
  });

  test('several frames in one read', () {
    final decoder = FrameDecoder();
    final frames = decoder.add([for (final f in validFrames) ...f]);
    expect(frames, validFrames);
    expect(decoder.droppedBytes, 0);
  });

  test('frames split into single bytes', () {
    final decoder = FrameDecoder();
    final frames = <List<int>>[];
    for (final b in [for (final f in validFrames.take(5)) ...f]) {
      frames.addAll(decoder.add([b]));
    }
    expect(frames, validFrames.take(5).toList());
  });

  test('payload containing 10 03 is not cut short', () {
    final frame = response(0x24, [0x01, 0x10, 0x03, 0x42, 0x10, 0x03, 0x00, 0x99]);
    final decoder = FrameDecoder();
    final frames = decoder.add(frame.sublist(0, 9));
    expect(frames, isEmpty);
    expect(decoder.add(frame.sublist(9)), [frame]);
  });

  test('payload containing 10 02 is not mistaken for a frame start', () {
    final frame = response(0x24, [0x02, 0x10, 0x02, 0x00, 0x01, 0x05]);
    expect(FrameDecoder().add(frame), [frame]);
  });

  test('garbage before a frame is skipped', () {
    final decoder = FrameDecoder();
    final frame = validFrames.first;
    expect(decoder.add([0x00, 0xff, 0x10, 0x55, ...frame]), [frame]);
    expect(decoder.droppedBytes, 4);
  });

  test('corrupted frame is dropped and the decoder resynchronises', () {
    final decoder = FrameDecoder();
    final broken = List.of(validFrames[0])..[20] ^= 0xff;
    final frames = decoder.add([...broken, ...validFrames[1]]);
    expect(frames, [validFrames[1]]);
    expect(decoder.droppedBytes, greaterThan(0));
  });

  test('frame with a wrong length byte is recovered by its footer', () {
    // Two payload bytes too many for its length byte (like the transcription
    // error that was in analyze_pcap.py), with a matching checksum.
    final original = hexToBytes(configDumpFrames[7]);
    final corrupted = [
      ...original.sublist(0, 9), 0x78, 0x00, ...original.sublist(9, original.length - 1),
    ];
    corrupted.add(ProtocolService.calculateChecksum(corrupted.getRange(2, corrupted.length - 2)));
    final decoder = FrameDecoder();
    expect(decoder.add([...corrupted, ...validFrames[0]]), [corrupted, validFrames[0]]);
    expect(decoder.lengthMismatches, 1);
  });
}
