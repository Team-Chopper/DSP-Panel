# TODO – open protocol questions

The panel supports the DSP 206 and DSP 408 with the protocol knowledge that is
available today. These points still need captures from real hardware. Each
item says what is assumed now, how it shows up in the panel, and how to
capture what is missing.

## How to capture

1. Run the original *t.racks DSP Processor Editor* (408 or 206 version) on a
   PC in the same network as the DSP.
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

- [ ] **Config dump layout.** Assumed: same record format as the DSP 408,
      with 2 input records, 6 output records and 2 crossing gains per output
      (896 bytes, 18 chunks).
      *Panel:* if the assumption is wrong, the DSP tab shows a warning banner
      and default values; controls still work.
      *Capture:* connect the 206 editor to the 206 (it reads the whole
      configuration on connect) and run `inspect_capture.dart --model dsp206`.
      If it reports implausible values, the dump layout differs.
- [ ] **Number of config chunks** the 206 answers (`GET CONFIG CHUNK` frames
      in the same capture).
- [ ] **Startup commands** 0x22, 0x14 and the preset count 0x2c: does the 206
      answer them like the 408? (The panel tolerates missing answers.)

## Both models

- [ ] **Mute, phase, delay in the config dump.** Every channel record ends
      with `[gain, ?a, ?b, link]`; ?a and ?b were 0 in every capture.
      Probably mute, phase and/or delay.
      *Panel:* these values (and PEQ bypass) are not read from the DSP; the
      server remembers what was last set in the panel.
      *Capture:* in the editor mute **In A**, invert phase on **Out 2**, set
      the delay of **Out 3** to **10 ms**, then reconnect (or store and
      recall a preset) so the editor reads the configuration again.
- [ ] **PEQ bypass in the config dump.** A PEQ band in the dump is 6 bytes
      (gain, freq, Q, type) without bypass.
      *Capture:* bypass PEQ band 1 of **In A**, then reconnect.
- [ ] **Matrix crossing gain command.** The dump has one gain per output and
      input (0 dB = raw 280), but the command to set it is unknown.
      *Panel:* shown read-only on the Matrix page.
      *Capture:* change the level of one crossing point in the editor's matrix.
- [ ] **Limiter: unknown third parameter** (`attack release ? threshold`,
      always 0 so far). *Capture:* change every limiter control in the editor
      once.
- [ ] **Frequency, Q and slope encodings on the DSP 408.** The panel uses
      the encodings verified on a DSP 206 by the FlexiFlizz project
      (frequency 0–300 = 20 Hz–20 kHz, Q = 10^((raw−16)/40), slope table with
      0 = bypass). The 408 config dumps use the same frequency and slope
      scales, but no 408 *command* capture confirms Q yet.
      *Capture:* in the 408 editor move a PEQ band's Q to 1.0 and 10.0, and
      switch the HPF slope through a few values.
- [ ] **Compressor ratio table** (16 entries from the FlexiFlizz README).
      *Capture:* step through all ratios in the editor.
- [ ] **Value ranges** of gate/compressor/limiter attack, release, hold,
      threshold and knee, and the maximum delay (the panel allows
      680 ms). *Capture:* move each control to its minimum and maximum.
- [ ] **Channel link (0x3B `ch mask`).** Taken from the FlexiFlizz capture,
      not tested on a device. The mask holds one bit per channel of the same
      group (inputs or outputs, like the link byte in the dump) and includes
      the channel itself.
      *Panel:* sends 0x3B for every channel of the group and additionally
      sends each change to all linked channels itself, so linking works even
      if the DSP does not mirror changes. Matrix routing is deliberately not
      shared (left/right pairs are fed from different inputs).
      *Capture:* in the editor link **Out 1** and **Out 2**, then change the
      gain and a PEQ band of **Out 1**. Check (a) the 0x3B frames, (b) whether
      the editor also sends the change for **Out 2** or the DSP mirrors it,
      (c) the link byte in the dump after reconnecting and (d) whether
      changing the routing of **Out 1** also changes **Out 2** on the device
      itself – the panel could not prevent that. Repeat for two inputs.
- [ ] **Commands seen but not implemented:** test tone (0x39), copy channel
      (0x2D/0x2F), lock (0x2A). The panel copies channels by sending the
      single values instead of 0x2D/0x2F.
- [ ] **Preset count response** (`2c 00 27 0f 00 00 00` on the 408): meaning
      of the bytes.

## Out of scope for now

- [ ] USB HID (VID 0168, PID 0821) and RS232/485 transports. The panel only
      talks TCP port 9761.

## Sources

- FlexiFlizz/t.racks-DSP-control (GPL-3.0), DSP 206 protocol verified on
  hardware: https://github.com/FlexiFlizz/t.racks-DSP-control
- Captures of a DSP 408 in this repository: `analyze_pcap.py`
  ("Default Preset" dump), `pcap_error.txt` (full session with the "Test"
  preset), `analyze2.py`–`analyze5.py` ("Forest" preset, partially
  mis-transcribed). The usable ones are in
  `packages/dsp_core/lib/src/testing/captured_config.dart`.
