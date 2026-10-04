// Frame Decoder
//
// Reassembles protocol frames from a TCP byte stream. TCP gives no message
// boundaries: one read can hold half a frame or several frames at once.
//
// Frames are delimited by their length byte (see protocol_service.dart).
// A frame is accepted when the footer sits where the length says and the
// checksum matches. If that fails, the decoder falls back to the first
// 10 03 footer with a valid checksum, and otherwise drops one byte and
// resynchronises on the next 10 02.

import 'protocol_service.dart';

class FrameDecoder {
  /// Upper bound for buffered bytes before the buffer is discarded.
  static const int maxBuffer = 64 * 1024;

  final List<int> _buffer = [];

  /// Bytes discarded while resynchronising.
  int droppedBytes = 0;

  /// Frames whose length byte did not match the footer position.
  int lengthMismatches = 0;

  /// Feed received bytes; returns all frames completed by them.
  List<List<int>> add(List<int> data) {
    _buffer.addAll(data);
    final frames = <List<int>>[];

    while (true) {
      _skipToStart();
      if (_buffer.length < 5) break;

      final total = _buffer[4] + 8;
      if (_buffer.length < total) break; // wait for more data

      if (_isValidFrame(total)) {
        frames.add(_take(total));
        continue;
      }

      // Length byte did not lead to a valid frame — look for a valid footer.
      final scanned = _scanForFooter();
      if (scanned != null) {
        lengthMismatches++;
        frames.add(_take(scanned));
        continue;
      }

      // Neither rule matched: this start marker is garbage. Drop it and
      // resynchronise on the next 10 02.
      _buffer.removeAt(0);
      droppedBytes++;
    }

    if (_buffer.length > maxBuffer) {
      droppedBytes += _buffer.length;
      _buffer.clear();
    }
    return frames;
  }

  /// Discard everything buffered (e.g. after reconnecting).
  void reset() => _buffer.clear();

  void _skipToStart() {
    var i = 0;
    while (i < _buffer.length) {
      if (_buffer[i] == 0x10 && (i + 1 >= _buffer.length || _buffer[i + 1] == 0x02)) {
        break;
      }
      i++;
    }
    if (i > 0) {
      _buffer.removeRange(0, i);
      droppedBytes += i;
    }
  }

  bool _isValidFrame(int total) =>
      _buffer[total - 3] == 0x10 &&
      _buffer[total - 2] == 0x03 &&
      ProtocolService.calculateChecksum(_buffer.getRange(2, total - 3)) == _buffer[total - 1];

  /// Length of the first frame that ends in `10 03 <valid checksum>`.
  int? _scanForFooter() {
    for (var i = 6; i + 2 < _buffer.length; i++) {
      if (_buffer[i] == 0x10 && _buffer[i + 1] == 0x03 &&
          ProtocolService.calculateChecksum(_buffer.getRange(2, i)) == _buffer[i + 2]) {
        return i + 3;
      }
    }
    return null;
  }

  List<int> _take(int length) {
    final frame = _buffer.sublist(0, length);
    _buffer.removeRange(0, length);
    return frame;
  }
}
