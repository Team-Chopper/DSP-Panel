//! Reassembles protocol frames from a TCP byte stream. TCP gives no message
//! boundaries: one read can hold half a frame or several frames at once.
//!
//! Frames are delimited by their length byte (see `protocol`). A frame is
//! accepted when the footer sits where the length says and the checksum
//! matches. If that fails, the decoder falls back to the first `10 03` footer
//! with a valid checksum, and otherwise drops one byte and resynchronises on
//! the next `10 02`.

use crate::protocol::{Frame, calculate_checksum};

#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    /// Bytes discarded while resynchronising.
    pub dropped_bytes: usize,
    /// Frames whose length byte did not match the footer position.
    pub length_mismatches: usize,
}

impl FrameDecoder {
    /// Upper bound for buffered bytes before the buffer is discarded.
    pub const MAX_BUFFER: usize = 64 * 1024;

    pub fn new() -> Self {
        Self::default()
    }

    /// Feed received bytes; returns all frames completed by them.
    pub fn add(&mut self, data: &[u8]) -> Vec<Frame> {
        self.buffer.extend_from_slice(data);
        let mut frames = Vec::new();

        loop {
            self.skip_to_start();
            if self.buffer.len() < 5 {
                break;
            }
            let total = self.buffer[4] as usize + 8;
            if self.buffer.len() < total {
                break; // wait for more data
            }
            if self.is_valid_frame(total) {
                frames.push(self.take(total));
                continue;
            }
            // Length byte did not lead to a valid frame — look for a valid footer.
            if let Some(scanned) = self.scan_for_footer() {
                self.length_mismatches += 1;
                frames.push(self.take(scanned));
                continue;
            }
            // Neither rule matched: this start marker is garbage. Drop it and
            // resynchronise on the next 10 02.
            self.buffer.remove(0);
            self.dropped_bytes += 1;
        }

        if self.buffer.len() > Self::MAX_BUFFER {
            self.dropped_bytes += self.buffer.len();
            self.buffer.clear();
        }
        frames
    }

    /// Discard everything buffered (e.g. after reconnecting).
    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    fn skip_to_start(&mut self) {
        let b = &self.buffer;
        let mut i = 0;
        while i < b.len() {
            if b[i] == 0x10 && (i + 1 >= b.len() || b[i + 1] == 0x02) {
                break;
            }
            i += 1;
        }
        if i > 0 {
            self.buffer.drain(..i);
            self.dropped_bytes += i;
        }
    }

    fn is_valid_frame(&self, total: usize) -> bool {
        let b = &self.buffer;
        b[total - 3] == 0x10 && b[total - 2] == 0x03 && calculate_checksum(&b[2..total - 3]) == b[total - 1]
    }

    /// Length of the first frame that ends in `10 03 <valid checksum>`.
    fn scan_for_footer(&self) -> Option<usize> {
        let b = &self.buffer;
        let mut i = 6;
        while i + 2 < b.len() {
            if b[i] == 0x10 && b[i + 1] == 0x03 && calculate_checksum(&b[2..i]) == b[i + 2] {
                return Some(i + 3);
            }
            i += 1;
        }
        None
    }

    fn take(&mut self, length: usize) -> Frame {
        self.buffer.drain(..length).collect()
    }
}
