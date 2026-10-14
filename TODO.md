# TODO – open protocol questions

The panel supports the DSP 206 and DSP 408. Since version 0.9.0 it follows the
original editor in everything the DSP sees, as far as static analysis of the
editor shows it: see [ORIGINAL-SOFTWARE.md](ORIGINAL-SOFTWARE.md) (how the
original software works, section 15 for the status in this project) and
[PROTOCOL-RE.md](PROTOCOL-RE.md). What is left are checks on real hardware for
the points that are only *inferred* there. Each item says what the panel does
now and how to capture what is missing.

## How to capture

1. Run the original editor (408 or 206 version) on a PC in the same network as
   the DSP.
2. Start Wireshark on that PC with the capture filter `tcp port 9761`.
3. Do exactly the steps of the item below, one thing at a time.
4. In Wireshark: select a packet → *Follow → TCP Stream* → show data as
   *Raw*, or export the packet bytes as hex, one frame per line.
5. Inspect the capture with the decoder of this repository:

   ```
   cd packages/dsp_core
   dart run tool/inspect_capture.dart <capture.txt> [--model dsp206]
   ```

   It prints every frame, decodes a config dump if the capture contains one,
   lists implausible values and shows the unknown words of every channel
   record.

## DSP 206

- [x] **Config dump layout and length** (928 bytes, 19 chunks), from the 206
      editor. *Still worth a capture:* connect the 206 editor and run
      `inspect_capture.dart --model dsp206` to see it on hardware.
- [ ] **Startup answers** 0x2C (lock/PTS status), 0x22 (slot table) and 0x14
      (current preset): does the 206 answer them like the 408? The panel
      tolerates missing answers.

## Both models

- [ ] **Linked channels (0x3B).** Like the original editor, the panel sends a
      change only for the edited channel and expects the DSP to apply it to
      the linked channels. *Capture/check:* link **Out 1** with **Out 2** in
      the panel, change the gain of **Out 1**, then reconnect and check that
      **Out 2** has the same gain on the device. Also note the link bytes of
      both channels in the dump (does the slave keep its own bit or 0?).
- [ ] **Current preset number (0x14):** 0-based or 1-based? A capture showed 20
      while U20 was active (1-based); the panel reads it that way.
- [ ] **Limiter knee:** the editor hides it; does the device use the value?
      *Check:* set a limiter knee of 12 dB in the panel and compare the limiter
      with knee 0.
- [ ] **GEQ bypass flag 0 (0x49)** is assumed to mean "active" like 0x3C.
- [ ] **Meter calibration:** feed a 1 kHz sine at −40, −20, −10 and 0 dBFS into
      **In A** to confirm that 1.0 really is full scale.
- [ ] **Test tone (0x39):** the commands are those of the editor but have not
      been tested on a device. *Capture:* switch the generator through pink,
      white and sine and note which inputs/outputs carry the signal.
- [ ] **Lock and PTS:** the panel unlocks a locked DSP with the password stored
      in its settings (0x2D). Not tested on a device; what the PTS counter
      counts is unknown.

## Not implemented

- Whole-device backup (0x19, 0x1A, 0x24, 0x25, 0x28, 0x04), factory reset
  (0x23), lock and PTS setup (0x2F, 0x2E): left to the original editor.
- USB HID (VID 0x0168, PID 0x0821), serial and UDP discovery. The panel only
  talks TCP port 9761.

## Sources

- The original editors (408 and 206 versions), static analysis:
  [ORIGINAL-SOFTWARE.md](ORIGINAL-SOFTWARE.md), [PROTOCOL-RE.md](PROTOCOL-RE.md).
- FlexiFlizz/t.racks-DSP-control (GPL-3.0), DSP 206 protocol verified on
  hardware: https://github.com/FlexiFlizz/t.racks-DSP-control
- Captures of a DSP 408 in this repository: `analyze_pcap.py`
  ("Default Preset" dump), `pcap_error.txt` (full session with the "Test"
  preset), `analyze2.py`–`analyze5.py` ("Forest" preset, partially
  mis-transcribed). The usable ones are in
  `packages/dsp_core/lib/src/testing/captured_config.dart`.
