# Protocol findings from the original editor (reverse engineering)

Static analysis (2026-10-10) of the vendor editors *the t.racks DSP 408 Processor
Editor V1.05* and *DSP 206 Processor Editor V1.05* with REA + Ghidra 12.1.4. Both
are 32-bit C++Builder/VCL programs; function names come from their export table
and the VCL method tables, UI ranges from the embedded form resources. Nothing
here was observed on hardware: it is what the editor sends and how it interprets
answers. The binaries and the decompiled output stay outside the repository
(`C:\RE\dsp-panel\analysis`).

Legend: **observed** = read directly from code/data of the editor;
*inferred* = conclusion from the observed code that a hardware test should confirm.

## Config dump layout (answers "mute/phase/delay/PEQ bypass in the dump")

The editor keeps the device state as one byte array that *is* the dump
(0x27 requests, 50-byte 0x24 chunks). Every setter writes to the same offsets the
panel already decodes, so the whole layout is confirmed (observed):

| Part | DSP 408 | DSP 206 |
|---|---|---|
| Header (flags, preset name 14 B) | 0x000 | 0x000 |
| Input records (140 B) | 0x010, 4 × | 0x010, 2 × |
| Output records | 0x240, 8 × 104 B | 0x128, 6 × 100 B |
| Mute bits, inputs (u16, bit = input) | 0x580 | 0x380 |
| Mute bits, outputs (u16, bit = output) | 0x582 | 0x382 |
| PEQ bypass, one u16 per channel byte c, bit = band | 0x584 + 2·c | 0x384 + 2·c |
| unknown (4 B) | 0x59C | 0x394 |
| Delay unit (u16, see 0x15) | 0x5A0 | 0x398 |
| unknown | 0x5A2–0x5A7 | 0x39A–0x39F |
| **Total length** | **0x5A8 = 1448 B → 29 chunks (0x00–0x1C)** | **0x3A0 = 928 B → 19 chunks (0x00–0x12)** |

- Channel tail `[gain, ?a, ?b, link]` is **`[gain, phase, delay, link]`** (observed:
  the 0x36 setter writes `?a`, the 0x38 setter writes `?b`). Phase is 0/1.
- The editor requests 29 chunks on the 408 and **19** on the 206 (observed loop
  bounds). The panel currently reads only 18 chunks on the 206, so it never gets
  the mute/bypass block there.
- The limiter block is `[attack, release, knee, threshold]` (see below).

## Commands

Frame format unchanged (`10 02 00 <addr> <len> <cmd> <data…> 10 03 <xor>`).
Channel byte: inputs 0…, outputs from 4 (408) / 2 (206) — confirmed for the 206.

| Cmd | Data | Meaning | Status |
|---|---|---|---|
| 0x10 | – | handshake | observed |
| 0x11 | – | **disconnect** (exit, Disconnect button, wrong device/version) | observed |
| 0x12 | – | **idle heartbeat**: sent by the transport timer only when no request is pending and the send queue is empty (5 s timeout); also after a finished dump/store | observed |
| 0x13 | – | device info; editor requires the answer to start with `DSP408`/`DSP408 V010` (`DSP206`/`DSP206 V010`), otherwise sends 0x11 and shows "Wrong device (version)" | observed |
| 0x14 | – | current preset number → answer byte 0 | observed (numbering *inferred*: 0 = none/factory, n = Un) |
| 0x15 | unit | **delay display unit** 0 = ms, 1 = m, 2 = ft (buttons `rzbUnit0–2`), stored in the dump | observed |
| 0x20 | n | recall preset | observed |
| 0x21 | slot | store preset (after 0x26 name, then 0x12) | observed |
| 0x22 | – | preset slot table: answer = 20 bytes, the editor marks a slot as used when its byte is 0xFF | observed |
| 0x26 | 14 B | preset name for the next store | observed |
| 0x27 | chunk | get config chunk → answer 0x24 `[chunk, 50 B]` | observed |
| 0x29 | i | get preset name → answer `[i, name 14 B]`, 20 slots | observed |
| **0x2A** | src dst | **copy channel** (the editor then re-reads the whole configuration) | observed |
| 0x2C | – | **PTS/lock status**, *not* the preset count — see below | observed |
| 0x2D | 1 + 4 B | lock password check (answer 0x2D) | observed |
| 0x2E | 4 + 3 B | PTS setup (password, on/off, time) | observed |
| 0x2F | 4 B | lock setup (password) | observed |
| 0x30 | ch ratio attack release knee threshold (u16 each) | compressor | observed |
| 0x31 / 0x32 | ch freq(u16) slope | LPF / HPF; slope 0 = bypass, else combo index + 1 (BW-6 … LK-48) | observed, matches panel |
| 0x33 | ch band gain freq Q type bypass | PEQ band | observed |
| 0x34 | ch gain(u16) | gain | observed |
| 0x35 | ch on | mute | observed |
| 0x36 | ch on | phase | observed |
| 0x38 | ch delay(u16) | delay (raw/96 = ms, max 65280 = 680 ms) | observed |
| 0x39 | type param | test tone: type = radio Tag (0 analog in, 1 pink, 2 white, 3 sine); sine param = combo index 0…30 = ISO 20 Hz…20 kHz. Dialog title "Input Signal Select" | observed |
| 0x3A | ch mask | matrix routing | observed |
| 0x3B | ch mask | link | observed |
| **0x3C** | ch flag | sent by the "EQ Bypass" button (whole PEQ of a channel) | observed; flag meaning *inferred* (1 = bypass) |
| 0x3D | ch 8 B | channel name | observed |
| 0x3E | ch attack release hold threshold | gate | observed |
| 0x3F | ch attack release **knee** threshold | limiter | observed |
| 0x40 | – | meters (timer `tmInLevelDetTimer`) | observed |
| **0x41** | out-ch in-index gain(u16) | **matrix crossing gain** (raw 0…280, 280 = 0 dB, same scale as gain → −60…0 dB) | observed |
| 0x48 | ch band value(u16) | GEQ band | observed |
| **0x49** | ch flag | sent by the GEQ "Bypass" button | observed; meaning *inferred* |
| 0x04, 0x19, 0x1A, 0x23, 0x24 (sent), 0x25, 0x28 | various | used only by the menu functions Upload/Download (backup of all presets) | seen, not traced in detail |

## Connect sequence of the editor (observed)

`0x10` → `0x13` (model/version check) → `0x2C` (PTS/lock) → `0x22` (slot table)
→ `0x14` (current preset) → `0x29` × 20 → `0x27` × 29 (408) / 19 (206) → `0x12`.
Meters are polled with `0x40` by a timer; `0x12` keeps the link alive while idle.

## 0x2C answer (PTS / lock)

`[ptsOn, limit_hi, limit_lo, counter_hi, counter_lo, locked]` (u16 big-endian).
If `ptsOn` and `limit <= counter` the editor asks for the PTS password; if `locked`
is non-zero it asks for the unlock password; otherwise it continues with 0x22.
The 408 capture `2c 00 27 0f 00 00 00` = PTS off, limit 9999, counter 0, not locked.
(Which u16 is limit/counter is *inferred* from the comparison and the 9999 default.)

## Meters (0x40 answer)

12 slots of 3 bytes, then a u16 (LE) status word (observed, both models):

- A slot is the **upper 3 bytes of an IEEE float32**: `bits = d1<<24 | d0<<16 | d2<<8`.
  The editor shows `20·log10(value)` (0 → 1e-7). Reading `d0 d1` as float16 gives
  ≈ 0.5 … 1.875 — exactly the "uncalibrated 0.5…2.0" range seen so far; 1.0 = 0 dBFS
  (*inferred*).
- LED steps: inputs light segment k when `dB > {−50,−30,−20,−10,−5,0,5,12,15}[k] − 18`,
  outputs when `dB > {−50,−20,−10,−5,0,5,12,14}[k] − 24` (labels are probably dBu).
- DSP 206: slots 0–1 inputs, 2–3 unused, 4–9 outputs.
- Status word: **bit n = compressor of output n active, bit 8+n = limiter of output n
  active**. The editor shows the comp LED only if the ratio is not 1:1 and the limit
  LED only if the threshold is below the maximum (+20 dB).

## Encodings and ranges (from the editor's controls)

- Q: exact display table for raw 0…100: 0.40 0.42 0.45 0.47 0.50 0.53 0.56 0.59 0.63
  0.67 0.71 0.75 0.79 0.84 0.89 0.94 1.00 1.06 1.12 1.19 1.26 1.3 1.4 1.5 1.6 1.7 1.8
  1.9 2.0 2.1 2.2 2.4 2.5 2.7 2.8 3.0 3.2 3.4 3.6 3.8 4.0 4.2 4.5 4.8 5.0 5.3 5.7 6.0
  6.3 6.7 7.1 7.6 8.0 8.5 9.0 9.5 10.1 10.7 11.3 12 12.7 13.5 14 15 16 17 18 19 20 21
  23 24 25 27 29 30 32 34 36 38 40 43 45 48 51 54 57 60 64 68 72 76 81 85 91 96 102
  108 114 121 128 — the panel's `10^((raw−16)/40)` matches up to rounding.
- Q rules in the editor: Peak and both all-passes use the table; shelves and the
  LP/HP PEQ types are limited to Q ≤ 3 and reset to 0.71 when the type changes.
- PEQ type order 0…8: Peak, Low Shelf, High Shelf, LP −6, LP −12, HP −6, HP −12,
  Allpass 1, Allpass 2 (English language table; matches the panel). An unused
  string list in the binary has HP/LP swapped — ignore it.
- Compressor ratios (16): 1.0 1.1 1.3 1.5 1.7 2.0 2.5 3.0 3.5 4.0 5.0 6.0 8.0 10 20 Limit.
- Ranges (raw): gain 0…400; PEQ/GEQ gain 0…240 (120 = 0 dB, 0.1 dB); freq 0…300;
  Q 0…100; gate/comp/limiter attack 0…998, release 9…2999, gate hold 9…998;
  comp/limiter threshold 0…220 (−90…+20 dB), gate threshold 0…180 (−90…0 dB);
  comp ratio 0…15; **comp and limiter knee 0…12**; delay 0…65280; matrix crossing
  gain 0…280.
- Delay/distance: 96 samples per ms, speed of sound 344 m/s.

## EQ curves drawn by the editor

The editor computes its graphs with bilinear biquads in Zölzer's K-form
(`K = tan(π·f/48000)`), not with the RBJ cookbook (observed):

- Peak: boost `(1 + V·K/Q + K²)/(1 + K/Q + K²)`, cut = exact inverse with `V = 10^(|G|/20)`.
  The RBJ peak used by the panel splits A over both polynomials, so the editor's
  boosts look narrower and its cuts wider at the same Q.
- Shelves: 2nd order, the band's Q replaces 1/√2 (`… + √V·K/Q + …` over `… + K/Q + …`),
  cut = inverse of boost. This equals an RBJ shelf whose corner frequency is shifted
  by `√A = 10^(|G|/80)` (*inferred* by comparing the transfer functions).
- LP/HP −6 are first order, LP/HP −12 second order with Q, Allpass 1 first order,
  Allpass 2 second order with Q.

## Linking in the original editor (observed)

- Link dialog: for every newly added slave the editor first **copies master → slave
  (0x2A)**, then sends **one 0x3B for the master** (mask = own bit + slaves), then
  re-reads the whole configuration (0x12 → 0x22 → … → 0x27).
- A normal parameter change is sent **once, for the edited channel only**; the
  editor updates its own model for all group members. The group of a channel is
  its own link word if that contains its own bit, else the word of the first lower
  channel that contains it. *Inferred:* the DSP mirrors changes to linked channels
  itself; the panel's extra per-member commands are redundant but harmless.

## Other notes

- RS232 "Extend Remote Control Protocol" (help page in the editor): separate frame
  `7B 7D addr cmd d1 d2 d3 7D 7B`, commands 0x41–0x4A (gain, mute, preset, volumes,
  read-back). Gain scale there: 0–80 = −60…−20 dB (0.5 dB), 80–280 = −20…0 dB,
  280–400 = 0…+12 dB (0.1 dB).
- Preset files `.prs` of the editor: header `DSP408V010`, 0x5AC bytes for the 408.
- Default IP in the editor: 192.168.1.101.

## Panel changes

Done (2026-10-10):

- DSP 206 reads 19 chunks (dump length 928), the 408 29.
- Phase/delay from the channel tail, mute and PEQ bypass from the switches block;
  the server no longer remembers these values (`DspConfig.remembered` removed).
- Matrix crossing gain editable via 0x41 (Matrix page, op `matrixGain`).
- Meters decoded as truncated float32 → dBFS; the status word drives the limit
  LED on the gain page and live LEDs on the Comp/Limit pages.

Open:

1. Limiter knee (0…12) instead of "unknown"; comp knee validation max 12, not 60.
2. Rename 0x2C to PTS/lock status and handle `locked`/PTS (at least show a message).
3. Optional: 0x2A copy, 0x3C/0x49 bypass-all, 0x15 delay unit, Zölzer curves.
4. Hardware checks still worth doing: 0x3C/0x49 flag meaning, meter dBFS
   calibration, that the DSP mirrors linked changes, PTS counter units.
