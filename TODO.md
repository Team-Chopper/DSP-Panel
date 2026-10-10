# TODO – open protocol questions

The panel supports the DSP 206 and DSP 408 with the protocol knowledge that is
available today. These points still need captures from real hardware. Each
item says what is assumed now, how it shows up in the panel, and how to
capture what is missing.

> **2026-10-10:** most of these questions are now answered by static analysis of
> the original editors (mute/phase/delay/PEQ bypass in the dump, 0x41 matrix
> crossing gain, limiter knee, 206 dump length, meters, 0x2C, link behaviour …),
> see [PROTOCOL-RE.md](PROTOCOL-RE.md). The panel uses them for the items ticked
> below; the other findings are not used yet. Hardware checks are still useful
> for the points marked *inferred* there.

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

- [x] **Config dump layout.** Confirmed by the 206 editor: same record format
      as the DSP 408 with 2 input and 6 output records (896 bytes), then mute
      and PEQ bypass bits; 928 bytes in total.
      *Panel:* decodes it; if a device ever answers differently, the DSP tab
      shows a warning banner and default values.
      *Still worth a capture:* connect the 206 editor and run
      `inspect_capture.dart --model dsp206` to see it on hardware.
- [x] **Number of config chunks:** the 206 editor reads 19 chunks
      (0x00–0x12), the 408 editor 29; the panel does the same.
- [ ] **Startup commands** 0x22, 0x14 and the preset count 0x2c: does the 206
      answer them like the 408? (The panel tolerates missing answers.)

## Both models

- [x] **Mute, phase, delay in the config dump.** The channel record ends
      with `[gain, phase, delay, link]`; mute is a bit mask per group right
      after the records (see PROTOCOL-RE.md). *Panel:* read from the device
      on connect and preset load (the server no longer remembers them).
      *Capture to confirm on hardware:* mute **In A**, invert phase on
      **Out 2**, set the delay of **Out 3** to **10 ms**, reconnect.
- [x] **PEQ bypass in the config dump:** one u16 per channel after the mute
      bits, bit = band. *Panel:* read from the device.
- [x] **Matrix crossing gain command:** 0x41 `output input-index gain(2B)`,
      gain like the channel gain (−60 … 0 dB). *Panel:* editable on the
      Matrix page. *Capture to confirm:* change one crossing point in the
      editor's matrix.
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
- [ ] **Channel link (0x3B `ch mask`), master/slave.** The original editor
      links in one direction: linking Out 1 with Out 2 makes Out 1 the master
      (mask = Out 1 + Out 2) and Out 2 a slave that cannot be linked any
      further; more channels are linked to the master. A DSP 206 dump showed
      exactly that (Out 1 lists Out 2, Out 2 only itself).
      *Panel:* reads the masks like that (older symmetric masks are read as a
      group with the first channel as master), sends 0x3B for the master
      (mask with its slaves) and for released slaves (own bit only), and sends
      every change of any member to the whole group itself, so linking works
      even if the DSP does not mirror changes. Matrix routing is deliberately
      not shared (left/right pairs are fed from different inputs).
      *Capture:* in the editor link **Out 1** with **Out 2**, then change the
      gain and a PEQ band of **Out 2** (the slave). Check (a) the 0x3B frames
      (is a frame sent for the slave, and with which mask?), (b) whether the
      editor also sends the change for **Out 1** or the DSP mirrors it, (c)
      the link bytes in the dump after reconnecting and (d) whether changing
      the routing of **Out 1** also changes **Out 2** on the device itself.
      Repeat for two inputs.
- [ ] **Test tone (0x39 `type param`).** From the FlexiFlizz capture:
      type 0 = off, 1 = pink noise, 2 = white noise, 3 = sine with param =
      frequency index. Not tested on a device.
      *Panel:* RTA page → DSP generator. The sine frequency index is
      *assumed* to count the 31 ISO 1/3-octave frequencies (0 = 20 Hz …
      30 = 20 kHz); the generator state is not read back (the panel shows
      what it last set).
      *Capture:* in the editor switch the generator through pink, white and
      sine at 20 Hz, 1 kHz and 20 kHz; note which inputs/outputs carry the
      signal and whether a level is sent.
- [x] **Meter values.** 0x40 answers with 12 slots, each the upper three
      bytes of a float32 (the old "float16 0.5 … 2.0" reading), then a status
      word: bit n = compressor, bit 8+n = limiter of output n working. On the
      DSP 206 the outputs start at slot 4. *Panel:* meters in dBFS
      (20·log10, 1.0 = 0 dBFS), clip at −0.5 dBFS, compressor/limiter
      indicators from the status word.
      *Still worth a capture:* feed a 1 kHz sine at −40, −20, −10 and 0 dBFS
      into **In A** to confirm that 1.0 really is full scale.
- [ ] **Shelf filters.** The EQ graphs draw the PEQ low/high shelf as an
      RBJ cookbook shelf with the band's Q. Whether the DSP uses Q, a slope
      or a fixed shape for shelves is unknown.
      *Capture/compare:* set a shelf with Q 0.7 and Q 3 in the editor and
      compare the curve the editor draws.
- [ ] **Commands seen but not implemented:** copy channel (0x2D/0x2F), lock
      (0x2A). The panel copies channels by sending the single values instead
      of 0x2D/0x2F.
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
