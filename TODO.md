# TODO – open protocol questions

The panel supports the DSP 206 and DSP 408, and since 1.0.0-alpha.3 (Rust only)
the DSP 204. Since version 0.9.0 it follows the
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
   cargo run -p dsp-core --bin inspect-capture -- <capture.txt> [--model dsp206]
   ```

   (The old Dart decoder does the same: `cd dart/packages/dsp_core`,
   `dart run tool/inspect_capture.dart …`.)

   It prints every frame, decodes a config dump if the capture contains one,
   lists implausible values and shows the unknown words of every channel
   record.

## DSP 206

- [x] **Config dump layout and length** (928 bytes, 19 chunks), from the 206
      editor. *Still worth a capture:* connect the 206 editor and run
      `inspect-capture --model dsp206` to see it on hardware.
- [ ] **Startup answers** 0x2C (lock/PTS status), 0x22 (slot table) and 0x14
      (current preset): does the 206 answer them like the 408? The panel
      tolerates missing answers.

## DSP 204

- [ ] **Everything.** No editor of the 204 was analysed; the panel treats it
      like the 206 with four outputs (channel bytes Out 1–4 = 0x02–0x05,
      dump 724 bytes in 15 chunks, meters in slots 0–1 and 4–7), which matches
      the measurements of the other project. *Check:* connect, compare every
      page with the device, then one link and one preset store/recall.

## All models

- [ ] **Linked channels (0x3B).** Like the original editor, the panel sends a
      change only for the edited channel and expects the DSP to apply it to
      the linked channels. *Capture/check:* link **Out 1** with **Out 2** in
      the panel, change the gain of **Out 1**, then reconnect and check that
      **Out 2** has the same gain on the device. Also note the link bytes of
      both channels in the dump (does the slave keep its own bit or 0?).
      *Seen on a DSP 204 (V0104P) by another project:* the DSP applies a
      change on any member to the whole group (gain, delay, polarity, HP/LP,
      PEQ, GEQ, gate, compressor, limiter, mute), not names, routing and
      crossing gains; forming a group copies the first channel's settings.
      Slaves there had link word 0, and a channel with 0 that is no longer in
      a group ignores changes. Since 1.0.0-alpha.3 the panel therefore resets
      every released slave whose word is not just its own bit.
- [ ] **Link mask width (0x3B):** the editor sends one byte, the other project
      a u16 (little endian), which works on the 204. *Check:* after linking in
      the panel, the link word of the master in the dump must be exactly its
      own bit plus the slaves (no stray high byte).
- [ ] **PEQ type order:** the panel uses the editor's language table
      (3/4 = LP −6/−12, 5/6 = HP −6/−12); the other project has HP and LP the
      other way round. *Check with the RTA:* pink noise from In A to Out 1,
      PEQ band 1 of Out 1 at type 3, 1 kHz: the highs must drop, not the lows.
- [ ] **Current preset number (0x14):** 0-based or 1-based? A capture showed 20
      while U20 was active (1-based); the panel reads it that way.
- [ ] **Limiter knee:** the editor hides it; does the device use the value?
      *Check:* set a limiter knee of 12 dB in the panel and compare the limiter
      with knee 0.
- [ ] **GEQ bypass flag 0 (0x49)** is assumed to mean "active" like 0x3C.
- [ ] **Meter calibration:** feed a 1 kHz sine at −40, −20, −10 and 0 dBFS into
      **In A** to confirm that 1.0 really is full scale. *Seen on a DSP 204 by
      the other project:* the output meter is after the output gain (−6 dB
      gain moved it by −6.3 dB) and before the mute; when the limiter starts,
      its threshold is about 26 dB (25.1 … 27.7 in five rounds) above the
      loudest meter reading. That fits the editor's output LEDs, which sit
      24 dB lower than the meter (PROTOCOL-RE.md, Meters). The limiter
      calculator uses this as its default reference.
- [x] **Limiter bits of the status word** (bit 8 + n = limiter of output n):
      confirmed by the other project on a DSP 204 (`00 03` while Out 1 and 2
      limit). The bit can stay on for up to 4 s, in two of six cases even
      after the threshold was raised to +20 dB, so a limit LED may hang on.
- [ ] **Test tone (0x39):** the commands are those of the editor but have not
      been tested on a device. *Capture:* switch the generator through pink,
      white and sine and note which inputs/outputs carry the signal.
- [ ] **Lock and PTS:** the panel unlocks a locked DSP with the password stored
      in its settings (0x2D). Not tested on a device; what the PTS counter
      counts is unknown.

## Not implemented

- Whole-device backup (0x19, 0x1A, 0x24, 0x25, 0x28, 0x04), factory reset
  (0x23), lock and PTS setup (0x2F, 0x2E): left to the original editor.
- USB HID (VID 0x0168, PID 0x0821) and serial. The panel only talks TCP port
  9761; its network search (Rust, since 1.0.0-alpha.3) probes that port on
  the /24 networks of the server and of the configured DSPs (handshake and
  device info, then the connection is closed without 0x11), not the editor's
  UDP discovery. *Check on hardware:* what a DSP does with such a short
  connection while another controller is connected to it. *From the other
  project, tested on a DSP 204 (macOS):* every HID report carries one frame
  from byte 0, sent with report ID 0 and zero-padded to 64 bytes; the frames
  are the same as on TCP.

## Sources

- The original editors (408 and 206 versions), static analysis:
  [ORIGINAL-SOFTWARE.md](ORIGINAL-SOFTWARE.md), [PROTOCOL-RE.md](PROTOCOL-RE.md).
- FlexiFlizz/t.racks-DSP-control (GPL-3.0), DSP 206 protocol verified on
  hardware: https://github.com/FlexiFlizz/t.racks-DSP-control
- "The other project": an independent open-source control app for the DSP
  204/408 (MIT, November 2026). Its protocol notes include measurements on a
  real DSP 204 (firmware V0104P); its DSP 408 and 206 support is untested.
- Captures of a DSP 408 in this repository (in `dart/`): `analyze_pcap.py`
  ("Default Preset" dump), `pcap_error.txt` (full session with the "Test"
  preset), `analyze2.py`–`analyze5.py` ("Forest" preset, partially
  mis-transcribed). The usable ones are in `crates/dsp-core/src/testing.rs`
  (and `dart/packages/dsp_core/lib/src/testing/captured_config.dart`).
