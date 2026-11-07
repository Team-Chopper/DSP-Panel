# The original DSP 408 / DSP 206 software – how it works

This document describes the Windows control software that ships with the DSP 408
(4 inputs, 8 outputs) and the DSP 206 (2 inputs, 6 outputs). Both programs are
version 1.05. They are called the **408 original software** and the
**206 original software** here, or simply "the original software" when both are
meant. The two programs share one code base; section 14 lists the differences.

The goal is interoperability: everything a third-party controller (such as this
project) needs to talk to the devices the same way the original software does.

## 1. Scope and method

- The facts come from static analysis of the two Windows programs: their code,
  their embedded forms and string tables, and their help pages.
- Nothing in this document was measured on hardware. It describes what the
  original software sends, expects and displays.
- No code of the original software is reproduced here. Formulas and tables are
  given only as far as they are needed to encode and decode values.

**Legend:** every statement was read directly from the programs unless it is
marked *(inferred)*. An inferred statement is a conclusion from what was read
and should be confirmed on hardware. Section 16 lists the open points.

Numbers are hexadecimal where they are written `0x..`, or where they are bytes
of a frame (`10 02 …`). Multi-byte values in the main protocol are unsigned
16-bit little-endian (`u16`) unless noted otherwise.

## 2. Devices and signal flow

| | DSP 408 | DSP 206 |
|---|---|---|
| Inputs | In A – In D | In A – In B |
| Outputs | Out 1 – Out 8 | Out 1 – Out 6 |
| Channel byte, inputs | `0x00`–`0x03` | `0x00`–`0x01` |
| Channel byte, outputs | `0x04`–`0x0B` | `0x02`–`0x07` |
| PEQ bands | 8 per input, 9 per output | 8 per input, 9 per output |
| GEQ | 31 bands per input | 31 bands per input |

Commands address a channel by its absolute channel byte.

The block diagram on the Matrix page shows the processing order:

- **Input:** gain → noise gate → GEQ (31 bands) → PEQ (8 bands) → high-pass /
  low-pass → delay.
- **Output:** matrix (input routing and crossing gain) → crossover high-pass /
  low-pass → PEQ (9 bands) → gain → compressor → limiter → delay → mute.

Only inputs have a gate and a GEQ. Only outputs have a compressor, a limiter
and the matrix.

## 3. User interface

### 3.1 Main window

- **Top bar:**
  - one button per page: Gain, Gate, Comp, Limit, Delay, Matrix, GEQ, and one
    page per channel (In A … / Out 1 …);
  - an Online/Offline button;
  - a label that shows the active interface ("USB", "NET" or the COM port).
- **Each page:** a graph area on top and a parameter area below.
- **Bottom bar:**
  - the device ID ("ID : n") and the IP address;
  - the current preset (number and name);
  - Store and Recall buttons.
- **Startup:**
  - device ID 1, TCP port 9761, IP 192.168.1.101;
  - after a short delay the connect dialog opens and scans for a device by
    itself.

### 3.2 Pages

| Page | Graph area | Controls |
|---|---|---|
| Gain | Response of all inputs and of all outputs, per-channel show/hide | Per channel: fader, Mute, Phase (normal/inverse), level meter; outputs also have a limit LED |
| Gate (inputs) | Transfer curve, input meters | Threshold, Attack, Release, Hold |
| Comp (outputs) | Transfer curve, output meters, compressor LEDs | Threshold, Ratio, Attack, Release, Knee |
| Limit (outputs) | Transfer curve, meters, limiter LEDs | Threshold, Attack, Release. A Knee control exists but is hidden in both programs. |
| Delay | One bar per channel, draggable with the mouse | Unit buttons ms / m / ft, one delay fader per channel |
| Matrix | Clickable block diagram: a block jumps to its page, a channel name opens the rename dialog | Per output: routing buttons for every input, crossing gain per input (field with up/down) |
| GEQ | Response of the selected input; EQ Reset, EQ Bypass | Channel selector, 31 sliders |
| Channel (In x / Out n) | EQ graph with magnitude/phase view, overlay of other channels, EQ Reset, "show all EQ", EQ Bypass | PEQ table (band, frequency, Q, gain, type, bypass); faders for the selected band; high-pass and low-pass blocks (frequency, slope, bypass); gain strip |

**EQ graph handling:**

- Bands can be dragged with the mouse.
- Keyboard: ←/→ change the frequency, +/− the Q, ↑/↓ the gain.
- EQ Bypass bypasses the whole PEQ of a channel (command `0x3C`) and writes
  "BYPASS" over the graph.

### 3.3 Controls and their ranges

| Control | Raw range | Shown as | Command |
|---|---|---|---|
| Gain | 0 … 400 | −60 … +12 dB | `0x34` |
| Mute | 0 / 1 | off / on | `0x35` |
| Phase | 0 / 1 | normal / inverse | `0x36` |
| Delay | 0 … 65280 | 0 … 680 ms | `0x38` |
| Delay unit | 0 / 1 / 2 | ms / m / ft | `0x15` |
| HP/LP frequency | 0 … 300 | 19.7 Hz … 20.16 kHz | `0x32` / `0x31` |
| HP/LP slope | 0 … 20 | bypass, 20 filter types | in `0x32` / `0x31` |
| PEQ frequency | 0 … 300 | as above | `0x33` |
| PEQ Q | 0 … 100 | 0.40 … 128 | `0x33` |
| PEQ gain | 0 … 240 | −12 … +12 dB | `0x33` |
| PEQ type | 0 … 8 | 9 filter types | `0x33` |
| PEQ band bypass | 0 / 1 | | `0x33` |
| EQ Bypass (whole channel) | 0 / 1 | | `0x3C` |
| GEQ band | 0 … 240 | −12 … +12 dB | `0x48` |
| GEQ Bypass | 0 / 1 | | `0x49` |
| Gate threshold | 0 … 180 | −90 … 0 dB | `0x3E` |
| Gate attack / hold / release | 0 … 998 / 9 … 998 / 9 … 2999 | ms, see 10.4 | `0x3E` |
| Compressor threshold | 0 … 220 | −90 … +20 dB | `0x30` |
| Compressor ratio | 0 … 15 | 1:1 … Limit | `0x30` |
| Compressor attack / release | 0 … 998 / 9 … 2999 | ms | `0x30` |
| Compressor knee | 0 … 12 | 0 … 12 dB | `0x30` |
| Limiter threshold / attack / release | as compressor | | `0x3F` |
| Limiter knee | 0 … 12 | hidden in the UI | `0x3F` |
| Routing | bit per input | on/off | `0x3A` |
| Crossing gain | 0 … 280 | −60 … 0 dB | `0x41` |
| Channel name | max. 8 characters | | `0x3D` |
| Link | see 8.4 | | `0x3B` |

### 3.4 Meters

- **Polling:** meters are requested with `0x40` every 300 ms.
- **Inputs:** 9 LED segments, labelled −50 / −30 / −20 / −10 / −5 / 0 / +5 /
  +12 / Clip.
- **Outputs:** 8 segments and a separate limit LED.
- **Comp and Limit pages:** an additional compressor or limiter LED per output.
- The thresholds and LED rules are in section 10.8.

### 3.5 Menus

**Visible menu items:**

- **File:**
  - Open and Save of a single preset file (`.prs`, section 12.1);
  - Upload and Download of a whole-device backup (`.unt`, section 12.2);
  - Exit, which asks for confirmation.
- **Link:** link selection dialog.
- **Copy:** channel copy dialog.
- **Lock:** lock setup.
- **ID/IP setup:** see section 4.
- **Test Tone:** test signal dialog.
- **Channel Name:** channel name list.
- **Help:**
  - a help page for the RS232 remote-control protocol (section 13);
  - About.

**Hidden menu items:**

These exist in the programs but are switched off, so a user cannot reach them:

- Factory Default;
- Language selection;
- PTS setup;
- two debug windows: a frame log and a message list;
- some empty placeholders.

## 4. Dialogs

| Dialog | What it does | Commands |
|---|---|---|
| Connect / scan | Progress bar and status text.<br>Buttons: Rescan, "Demo" (work offline without a device), and Set (opens the ID/IP setup). | Scan, section 5.2 |
| ID/IP setup | Device ID 1 … 250 and IP address (4 octets, default 192.168.1.101). Only editable while offline. A manual/auto switch for the IP exists but is hidden. | **None.** It only sets the address byte the software uses in frames and the TCP host it connects to. The original software cannot change the device's own network settings. |
| Link selection | A triangular grid, separately for inputs and outputs: each channel picks its master among itself and the lower channels. A channel that is already a slave cannot be a master. | `0x2A`, `0x3B`, then re-read (8.4) |
| Channel copy | Source and destination for inputs (In → In) and for outputs (Out → Out). Copying a channel onto itself is ignored. | `0x2A`, then re-read (8.5) |
| Channel name | One name, max. 8 characters. | `0x3D` |
| Channel name list | All channel names (defaults "Input1" …, "Output1" …). | `0x3D` for every changed name |
| Store preset | List U01 – U20, then a name dialog (max. 14 characters). | `0x26`, `0x21`, `0x12` (8.2) |
| Recall preset | List F00 (factory) and U01 – U20. | `0x20`, then re-read |
| PEQ / GEQ reset | "Are you sure to reset EQ?". The PEQ of a channel goes back to its default bands; the GEQ goes to 0 dB. | `0x33` per band, or `0x48` × 31 |
| Test signal | Analog input (generator off) / pink noise / white noise / sine. The sine frequency is one of the 31 ISO frequencies from 20 Hz to 20 kHz. | `0x39` |
| Lock setup | A 4-character password (default text "1234"). | `0x2F`, then `0x11` and disconnect |
| Unlock | Asks for the 4-character password while connecting to a locked device. | `0x2D` (8.6) |
| PTS password / PTS setup | Password, then on/off, limit (max. 9999) and the current counter. | `0x2D`, `0x2E` (8.6) |
| Factory default (hidden) | "Return to factory default?" | `0x23`, then re-read |
| Progress | Upload/download/store/load progress. | – |
| Messages | Generic message box, e.g. "No device connected", "Wrong device", "Wrong device version", "Open file error", "Download error", "Password error", "Check time out". | – |
| About, Exit | Version information, and the confirmation before quitting. | Exit sends `0x11` |

## 5. Transports and timing

### 5.1 Interfaces

The same frames (section 6) are used on all three interfaces.

| Interface | Details |
|---|---|
| TCP/IP | The software is a TCP client to port **9761**. Default host 192.168.1.101. If a read returns fewer than 6 bytes, the last frame is sent again. |
| USB HID | VID **0x0168**, PID **0x0821** (same for both models). Each output report is report ID 0 followed by exactly one frame; frames are never split. Report length comes from the device capabilities; the frames fit into 64-byte reports *(inferred)*. |
| Serial (COM port) | 115200 baud, 8N1 *(8N1 inferred)*. COM ports are taken from the Windows serial port list. |
| UDP discovery | A broadcast "Discovery: Who is out there?" to port 30303 (the common embedded-module discovery scheme). It belongs to the hidden "auto" IP mode and cannot be reached in the UI. |

### 5.2 Connect scan

The connect dialog tries the interfaces in this order:

1. USB, if the HID device is present (watchdog 6 s).
2. TCP, if an IP address is set (2 s).
3. Every COM port in turn (10 s each).

The first interface that completes the handshake wins. Its name is then shown
in the top bar.

### 5.3 Request handling

- **One request at a time.** Only one request is in flight. Any complete
  received frame counts as its answer, and then the next frame is sent.
- **Queue replacement.** Parameter frames and meter polls replace a queued
  frame with the same command byte, even one for another channel. Only the
  newest frame per command byte waits in the queue.
- **Answer timeouts:**
  - Handshake: 100 ms on USB/COM and 4 s on TCP, with 3 attempts.
  - Normal operation: 1 s (TCP/COM) or 5 s (USB), with one retry. After that
    the link counts as broken: the software shows "Link broken" and
    disconnects.
  - Store and factory reset: 10 s. Recall: 50 s.
- **Heartbeat.** When no request is pending and the queue is empty, the
  transport sends `0x12` to keep the link alive. The idle time before this is
  about 2 s, and the answer timeout up to 5 s *(inferred from the timer
  values)*.
- **Disconnect.** `0x11` is sent:
  - on exit and on Disconnect;
  - after a wrong device or version answer;
  - when a password dialog is cancelled;
  - after a lock setup.

## 6. Frame format

```
10 02  <src> <dst>  <len>  <cmd> <data …>  10 03  <chk>
```

| Field | Meaning |
|---|---|
| `10 02` | start |
| `src`, `dst` | Addresses. The PC is `00`; the device uses its **device ID** (1 … 250, default 1). Requests carry `00 <ID>`, answers `<ID> 00`. Answers from another ID are ignored. |
| `len` | 1 + number of data bytes (it counts the command byte) |
| `cmd` | command |
| `10 03` | end |
| `chk` | XOR of all bytes from the first `10` up to and including `03` |

- **Length.** The frame length is `len + 8`.
- **No byte stuffing.** A `10` inside the data is sent as is, so the receiver
  must use `len` to find the end.
- **Checksum.** The software does not check the checksum of received frames.

**Example:** meter request `10 02 00 01 01 40 10 03 41`
(10⊕02⊕00⊕01⊕01⊕40⊕10⊕03 = 41).

**Checksum shortcut.** Because `10 02` and `10 03` together XOR to `01`, the
checksum equals 01 ⊕ src ⊕ dst ⊕ len ⊕ cmd ⊕ data.

**Acknowledge.** Most setters and preset operations are answered with an
acknowledge frame whose command byte is `0x01` and which has no data:
`10 02 01 00 01 01 10 03 00`.

## 7. Command reference

**Data layout notation:**

- "ch" is the absolute channel byte (section 2).
- Values are `u16` little-endian unless noted.
- "→" describes the answer.

| Cmd | Data | Meaning |
|---|---|---|
| `0x01` | – | Answer only: acknowledge (ACK). |
| `0x04` | – | Last step of a backup download. The software disconnects 300 ms later. Probably makes the device apply the restore or restart *(inferred)*. |
| `0x10` | – | Handshake → `0x10`. |
| `0x11` | – | Disconnect / end of session (no answer expected). |
| `0x12` | – | Idle heartbeat, also sent after a dump, a store or a preset-file load → ACK *(the ACK is seen in a network capture of the original software, not in the program)*. |
| `0x13` | – | Device info → ASCII text. Must start with `DSP408` and `DSP408 V010` (or `DSP206` / `DSP206 V010`). Otherwise "Wrong device (version)" and `0x11`. |
| `0x14` | – | Current preset number → 1 byte. See section 16 for the numbering. |
| `0x15` | unit | Delay display unit: 0 = ms, 1 = m, 2 = ft. Stored in the dump; display only. |
| `0x19` | offset (u16), length (u8) | Read a system block. The software sends `00 00 12` → `0x19` with 18 bytes. Content not interpreted. |
| `0x1A` | offset (u16), length (u8), bytes | Write a system block. The software sends `01 00 11` + 17 bytes (block bytes 1 … 17) → ACK. |
| `0x20` | n | Recall preset: 0 = F00 (factory), 1 … 20 = U01 … U20 → ACK (timeout 50 s). |
| `0x21` | slot | Store the current settings to U01 … U20 (slot 1 … 20) → ACK. |
| `0x22` | – | Preset slot table → 20 bytes, one per user slot; `0xFF` = slot used. |
| `0x23` | – | Factory reset → ACK (timeout 10 s). Also the first step of a backup download. |
| `0x24` | slot | Sent: select stored preset U*slot* (1 … 20) for read-out → ACK. |
| `0x24` | – | Received: answer to `0x27`, `[chunk, 50 bytes]`. |
| `0x25` | chunk, slot, 50 bytes | Write one 50-byte chunk of a stored preset (slot 0 … 19) → `0x25 [chunk]`. |
| `0x26` | name (14 bytes, space-padded) | Name for the next store → ACK. |
| `0x27` | chunk | Read a chunk of the current configuration → `0x24 [chunk, 50 bytes]`. |
| `0x28` | chunk | Read a chunk of the preset selected with `0x24` → `0x28 [chunk, 50 bytes]`. |
| `0x29` | i (0 … 19) | Preset name of U(i+1) → `[i, 14 bytes]`. |
| `0x2A` | src, dst | Copy channel *src* to *dst* (absolute channel bytes, same group) → ACK. |
| `0x2C` | – | PTS / lock status → `[ptsOn, limit_hi, limit_lo, count_hi, count_lo, locked]` (8.6). |
| `0x2D` | mode, password (4 bytes) | Password check: mode 0 = unlock, 1 = PTS → `[mode, result]`, result 1 = correct. |
| `0x2E` | password (4), limit_hi, limit_lo, on | PTS setup. |
| `0x2F` | password (4) | Lock setup. |
| `0x30` | ch, ratio, attack, release, knee, threshold | Compressor (outputs). |
| `0x31` | ch, freq, slope (u8) | Low-pass. |
| `0x32` | ch, freq, slope (u8) | High-pass. |
| `0x33` | ch, band, gain, freq, Q (u8), type (u8), bypass (u8) | PEQ band. |
| `0x34` | ch, gain | Channel gain. |
| `0x35` | ch, on | Mute. |
| `0x36` | ch, on | Phase invert. |
| `0x38` | ch, delay | Delay. |
| `0x39` | type, index | Test signal: type 0 = analog input (generator off), 1 = pink noise, 2 = white noise, 3 = sine; index = sine frequency 0 … 30. |
| `0x3A` | ch, mask (u8) | Routing of an output: bit *n* = input *n*. |
| `0x3B` | ch, mask (u8) | Link word. Bits are relative to the channel's group (In A = bit 0, Out 1 = bit 0). |
| `0x3C` | ch, flag | Bypass the whole PEQ of a channel: 1 = bypass, 0 = active. |
| `0x3D` | ch, name (8 bytes, NUL-padded) | Channel name. |
| `0x3E` | ch, attack, release, hold, threshold | Noise gate (inputs). |
| `0x3F` | ch, attack, release, knee, threshold | Limiter (outputs). |
| `0x40` | – | Meters → 12 × 3 bytes + status word (10.8). |
| `0x41` | ch (output), input index, gain | Matrix crossing gain. |
| `0x48` | ch, band (0 … 30), value | GEQ band (inputs). |
| `0x49` | ch, flag | GEQ bypass of an input: 1 = bypass *(0 = active, inferred)*. |

No other command bytes appear in either program. `0x37` and the other gaps are
not used.

## 8. Sequences

### 8.1 Connect

```
0x10 → 0x13 (model/version check) → 0x2C (PTS/lock, see 8.6)
     → 0x22 (slot table) → 0x14 (current preset)
     → 0x29 × 20 (preset names) → 0x27 × 29 (408) / × 19 (206) → 0x12
```

- Each step is sent when the answer to the previous one arrives.
- After the last chunk the software sends `0x12`.
- From then on the meter timer polls `0x40`.

### 8.2 Presets

- **Recall:** `0x20 n` → ACK → full re-read (`0x27` × chunk count) →
  `0x12`. Progress text "Loading preset".
- **Store:** `0x26 name` → ACK → `0x21 slot` → ACK → `0x12`.
- **Preset names:** `0x29` for 0 … 19. The bottom bar shows the current preset
  number and the name from the dump header (section 9.1).

### 8.3 Parameter changes

- **One frame per change.** A normal parameter change sends **one** frame, for
  the edited channel only.
- **Model update for linked channels.** If the channel is linked, the software
  updates its own model for all members of the group. It sends nothing for the
  other members. Presumably the device applies the change to the linked
  channels itself *(inferred)*.
- **How the software finds a channel's group:**
  - If the channel's link word contains the channel's own bit, the group is
    the set of bits in that word.
  - Otherwise the group is the word of the first lower channel that contains
    this channel's bit.

### 8.4 Linking

- **Default state.** Every channel's link word is its own bit, i.e. not linked.
- **The Set button of the link dialog:**
  1. For every newly added slave: copy master → slave with `0x2A`.
  2. One `0x3B` for the master. Mask = own bit + all slaves.
  3. Re-read of the configuration (`0x22` → … → `0x27` × chunk count).
- **Slave link words.** They are not sent. A DSP 206 dump shows the slave
  keeping only its own bit.

### 8.5 Channel copy

- **Online:** `0x2A src dst` → ACK → full re-read.
- **Offline**, the copy is done in the model only:
  - Inputs copy gate, GEQ, PEQ, high/low-pass, gain, phase, delay, mute, the
    PEQ band bypass word and the channel's EQ-bypass bit.
  - Outputs copy crossover, PEQ, compressor, limiter, gain, phase, delay, mute,
    the bypass word and the EQ-bypass bit.
  - Name, link word and routing are never copied.

### 8.6 Lock and PTS

The answer to `0x2C` is `[ptsOn, limit_hi, limit_lo, count_hi, count_lo, locked]`.
Limit and count are big-endian u16. While connecting, the software decides:

1. **PTS limit reached.** If `ptsOn` is set and `limit ≤ count`, the PTS
   password dialog opens.
2. **Locked device.** If `locked ≠ 0`, the unlock dialog opens.
3. **Otherwise** the connect sequence continues with `0x22`.

**Password check:**

- Passwords are 4 characters, converted to upper case.
- `0x2D [mode, password]` is answered with `[mode, result]`:
  - result 1: the software sends `0x22` and continues connecting. After a
    correct PTS password it also opens the PTS setup.
  - result 0: "Password error".
- No answer within about 3 s gives "Check time out" and a disconnect.
- Cancel sends `0x11` and disconnects.

**Setup commands:**

- `0x2E [password, limit_hi, limit_lo, on]` sets the PTS limit (0 … 9999) and
  switches it on or off.
- `0x2F [password]` sets the lock password. The software then sends `0x11` and
  disconnects.

What PTS counts is not visible in the software. The menu name suggests a
use/power-on counter *(inferred)*.

### 8.7 Factory reset (hidden menu item)

`0x23` → ACK → `0x22` → normal full read as in 8.1.

### 8.8 Loading a preset file

A `.prs` file (section 12.1) is not written as a block. While connected, the
software sends every value as a single command, 385 frames in total on the
DSP 408:

1. `0x26` preset name.
2. Per input: `0x3D`, `0x3E`, `0x48` × 31, `0x32`, `0x31`, `0x34`, `0x36`,
   `0x38`, `0x3B`.
3. Per output: `0x3D`, `0x3A`, `0x41` × inputs, `0x32`, `0x31`, `0x30`,
   `0x3F`, `0x34`, `0x36`, `0x38`, `0x3B`.
4. `0x35` for every channel.
5. `0x33` for every PEQ band of every channel.

**Completion:** the load counts as finished at the first meter answer after
more than 10 ACKs. Then `0x12` is sent (timeout 50 s).

Offline the file is only loaded into the editor *(inferred)*.

### 8.9 Whole-device backup

**Upload (device → file):**

1. `0x19 00 00 12` → 18-byte system block.
2. For every used user slot *p* (from the `0x22` table):
   1. `0x24 p` → ACK.
   2. `0x28 chunk` for chunks 0 … 28 (408) / 0 … 18 (206) → `0x28 [chunk, 50 bytes]`.
3. `0x12`. The file is written 300 ms later.

**Download (file → device):**

1. `0x23` → ACK.
2. `0x1A 01 00 11` + 17 bytes → ACK.
3. For every slot that is marked used in the file: `0x25 [chunk, slot, 50 bytes]`
   for every chunk, each answered with `0x25 [chunk]`.
4. `0x04`. The software disconnects 300 ms later.

### 8.10 Answer handling

| Answer | Reaction of the software |
|---|---|
| `0x01` | ACK. The next step depends on what was sent:<br>• after recall: `0x27` chunk 0<br>• after `0x26`: `0x21`<br>• after `0x21`: `0x12`<br>• after `0x23`: `0x22`, or `0x1A` during a backup download<br>• after `0x24`: `0x28` chunk 0<br>• after `0x1A`: `0x25`<br>• after copy/link: `0x22`<br>• during a file load: progress |
| `0x10` | Connected; send `0x13`. |
| `0x13` | Check the model and version string; then `0x2C`. |
| `0x2C` | Lock/PTS decision (8.6). |
| `0x2D` | Password result. |
| `0x22` | Store the slot table; then `0x14`. |
| `0x14` | Store the current preset number; then `0x29` index 0. |
| `0x29` | Store the name; then the next index, or `0x27` chunk 0. |
| `0x24` | Store the chunk; then the next `0x27`, or `0x12` after the last chunk. |
| `0x19` | Store the system block; then `0x24`. |
| `0x28` | Store a backup chunk. |
| `0x25` | Next download chunk / next slot / `0x04`. |
| `0x40` | Update meters and LEDs. |

Other answers, including `0x11` and `0x12`, are not handled.

## 9. Configuration dump

The software keeps the device state in one byte array that is exactly the dump.
Every setter writes into that array at the offsets below.

- **Reading.** `0x27 chunk` returns 50 bytes per chunk; the last chunk is
  shorter.
- **Writing.** There is no command that writes the live configuration as a
  block.

| | DSP 408 | DSP 206 |
|---|---|---|
| Header | `0x000` | `0x000` |
| Input records (140 bytes each) | `0x010`, 4 × | `0x010`, 2 × |
| Output records | `0x240`, 8 × 104 bytes | `0x128`, 6 × 100 bytes |
| Switches block | `0x580` | `0x380` |
| Total length | `0x5A8` = 1448 bytes = 29 chunks (last 48 bytes) | `0x3A0` = 928 bytes = 19 chunks (last 28 bytes) |

### 9.1 Header

| Offset | Content |
|---|---|
| `0x000` | u16 flags. Byte 0 = `0xFF` marks a valid stored preset. |
| `0x002` | Preset name, 14 bytes ASCII, space-padded. |

### 9.2 Input record (140 bytes)

| Offset | Content |
|---|---|
| +0 | Name, 8 bytes, NUL-padded |
| +8, +10, +12, +14 | Gate attack, release, hold, threshold |
| +16 … +77 | GEQ, 31 × u16 |
| +78 … +125 | PEQ, 8 × 6 bytes: gain (u16), freq (u16), Q (u8), type (u8) |
| +126, +128 | High-pass freq, low-pass freq |
| +130, +131 | High-pass slope (u8), low-pass slope (u8) |
| +132, +134, +136, +138 | Gain, phase, delay, link word |

### 9.3 Output record (104 bytes on the 408, 100 bytes on the 206)

| DSP 408 | DSP 206 | Content |
|---|---|---|
| +0 | +0 | Name, 8 bytes |
| +8 | +8 | Routing mask (u16, bit = input) |
| +10 … +17 | +10 … +13 | Crossing gain per input (u16 each) |
| +18, +20 | +14, +16 | High-pass freq, low-pass freq |
| +22, +23 | +18, +19 | High-pass slope, low-pass slope (u8) |
| +24 … +77 | +20 … +73 | PEQ, 9 × 6 bytes |
| +78 … +86 | +74 … +82 | Compressor ratio, attack, release, knee, threshold |
| +88 … +94 | +84 … +90 | Limiter attack, release, knee, threshold |
| +96 … +102 | +92 … +98 | Gain, phase, delay, link word |

### 9.4 Switches block

| DSP 408 | DSP 206 | Content |
|---|---|---|
| `0x580` | `0x380` | Mute bits of the inputs (u16, bit = input) |
| `0x582` | `0x382` | Mute bits of the outputs (u16, bit = output) |
| `0x584 + 2·c` | `0x384 + 2·c` | PEQ band bypass, one u16 per channel byte *c*, bit = band |
| `0x59C` | `0x394` | Test signal type (as in `0x39`) |
| `0x59E` | `0x396` | Test signal sine index |
| `0x5A0` | `0x398` | Delay display unit (as in `0x15`) |
| `0x5A2` | `0x39A` | EQ-bypass-all bits of the inputs (`0x3C`) |
| `0x5A4` | `0x39C` | EQ-bypass-all bits of the outputs (`0x3C`) |
| `0x5A6` | `0x39E` | GEQ bypass bits of the inputs (`0x49`) |

### 9.5 Factory defaults

These defaults come from the factory table in the software, which is also used
by the PEQ reset.

| Setting | Default |
|---|---|
| Gain, crossing gains | 280 (0 dB) |
| Routing | Out *n* is fed from input ((*n* − 1) mod number of inputs) *(seen for the 408; the 206 is inferred)* |
| High-pass | 0 (19.7 Hz), slope 0 (bypass) |
| Low-pass | 300 (20.16 kHz), slope 0 (bypass) |
| Gate (attack, release, hold, threshold) | 0, 99, 99, 0 |
| Compressor (ratio, attack, release, knee, threshold) | 0, 49, 499, 0, 220 |
| Limiter (attack, release, knee, threshold) | 49, 499, 0, 220 |
| PEQ bands, common | Peak, gain 120 (0 dB), Q index 35 (Q 3.0) |
| PEQ frequency indices, inputs | 41, 71, 101, 140, 170, 200, 240, 270 (≈ 51 Hz, 101 Hz, 203 Hz, 500 Hz, 1 kHz, 2 kHz, 5 kHz, 10 kHz) |
| PEQ frequency indices, outputs | 31, 63, 95, 127, 158, 190, 222, 253, 285 (≈ 40 Hz … 14 kHz) |
| GEQ | 120 (0 dB) |
| Link words | own bit |

## 10. Value encodings

### 10.1 Gain (channel gain, crossing gain)

| Raw | dB | Step |
|---|---|---|
| 0 … 80 | raw / 2 − 60 → −60 … −20 dB | 0.5 dB |
| 80 … 280 | (raw − 280) / 10 → −20 … 0 dB | 0.1 dB |
| 280 … 400 | (raw − 280) / 10 → 0 … +12 dB | 0.1 dB |

The crossing gain uses the same scale, limited to 0 … 280 (−60 … 0 dB).

### 10.2 Frequency (PEQ, high-pass, low-pass)

`f = 1000 Hz · 2^((i − 170) / 30)`, i = 0 … 300

- This gives 1/30-octave steps from 19.7 Hz to 20.16 kHz.
- i = 170 is 1 kHz.

### 10.3 PEQ

**Q**

- Formula: `Q = 2^((raw − 16) / 12)`, raw 0 … 100, i.e. 12 steps per doubling.
  Anchor points: 0 → 0.40, 10 → 0.71, 16 → 1.00, 35 → 3.0, 100 → 128.
- The software shows Q from a fixed display table that is this formula
  rounded.
- Peak and both all-pass types use the full range.
- Shelves and the PEQ low-/high-pass types are limited to Q ≤ 3. Q is reset to
  0.71 when the type changes.

**Gain**

`raw = 10 · dB + 120` (0 … 240, ±12 dB, 0.1 dB steps). The GEQ uses the same
encoding.

**Types**

| Byte | Type |
|---|---|
| 0 | Peak |
| 1 | Low Shelf |
| 2 | High Shelf |
| 3 | LP −6 dB |
| 4 | LP −12 dB |
| 5 | HP −6 dB |
| 6 | HP −12 dB |
| 7 | All-pass 1 |
| 8 | All-pass 2 |

**Crossover slopes (`0x31` / `0x32`)**

- Byte 0 = bypass; otherwise the byte is the list index + 1.
- BW = Butterworth, BL = Bessel, LK = Linkwitz-Riley *(names inferred from the
  abbreviations)*.

| Byte | Slope | Byte | Slope | Byte | Slope | Byte | Slope |
|---|---|---|---|---|---|---|---|
| 1 | BW −6 | 6 | BW −18 | 11 | BW −30 | 16 | BW −42 |
| 2 | BL −6 | 7 | BL −18 | 12 | BL −30 | 17 | BL −42 |
| 3 | BW −12 | 8 | BW −24 | 13 | BW −36 | 18 | BW −48 |
| 4 | BL −12 | 9 | BL −24 | 14 | BL −36 | 19 | BL −48 |
| 5 | LK −12 | 10 | LK −24 | 15 | LK −36 | 20 | LK −48 |

**GEQ bands**

31 ISO 1/3-octave bands: 20, 25, 31.5, 40, 50, 63, 80, 100, 125, 160, 200, 250,
315, 400, 500, 630, 800, 1k, 1.25k, 1.6k, 2k, 2.5k, 3.15k, 4k, 5k, 6.3k, 8k, 10k,
12.5k, 16k, 20k Hz. The test-signal sine uses the same list (index 0 … 30).

### 10.4 Dynamics

- **Threshold:** `raw = 2 · dB + 180` (0.5 dB steps).
  - Gate: 0 … 180 = −90 … 0 dB.
  - Compressor and limiter: 0 … 220 = −90 … +20 dB.
- **Ratio** (raw 0 … 15): 1:1.0, 1.1, 1.3, 1.5, 1.7, 2.0, 2.5, 3.0, 3.5, 4.0,
  5.0, 6.0, 8.0, 10, 20, Limit.
- **Times:**
  - Ranges: attack 0 … 998, release 9 … 2999, gate hold 9 … 998.
  - Display: shown as raw + 1 ms *(inferred from the default values: 49 is
    shown as 50 ms, 99 as 100 ms, 499 as 500 ms)*.
- **Knee:** raw = dB, 0 … 12. This applies to the compressor and the limiter;
  the limiter knee is hidden in the UI.

### 10.5 Delay

- **Encoding:** raw = samples at 96 kHz, i.e. 96 per ms, max. 65280 = 680 ms.
- **Display:** in ms, m or ft with a speed of sound of 344 m/s. The chosen
  unit is sent with `0x15` and stored in the dump.

### 10.6 Routing and link masks

- **Routing (`0x3A`):** one byte, bit *n* = input *n*.
- **Link word (`0x3B`):** one byte, bits relative to the group. Inputs: In A =
  bit 0. Outputs: Out 1 = bit 0.

### 10.7 Names

| Name | Length | Padding |
|---|---|---|
| Channel name | max. 8 characters | NUL |
| Preset name | max. 14 characters | spaces |

Both are ANSI text.

### 10.8 Meters (`0x40` answer)

The answer holds 12 slots of 3 bytes, followed by a u16 (little-endian) status
word.

**Slot value**

- Each slot holds the upper three bytes of an IEEE float32:
  `bits = d1 << 24 | d0 << 16 | d2 << 8`.
- The software shows `20 · log10(value)` and uses 1e-7 for a value of 0.
- 1.0 is presumably 0 dBFS *(inferred)*.

**Slot order**

| Model | Slot use |
|---|---|
| DSP 408 | In A – In D, Out 1 – Out 8 |
| DSP 206 | 0–1 inputs, 2–3 unused, 4–9 outputs |

**LED thresholds**

| Meter | LED segment *k* lights when `dB >` | Thresholds (dBFS) |
|---|---|---|
| Inputs | {−50, −30, −20, −10, −5, 0, 5, 12, 15}[k] − 18 | −68 … −3 |
| Outputs | {−50, −20, −10, −5, 0, 5, 12, 14}[k] − 24 | −74 … −10 |

The labels are probably dBu with 0 dBFS = +18 dBu (inputs) and +24 dBu
(outputs) *(inferred)*.

**Status word**

- Bit *n* = compressor of output *n* active.
- Bit 8 + *n* = limiter of output *n* active.
- The compressor LED lights only if the ratio is not 1:1. The limit LED lights
  only if the threshold is below the maximum (+20 dB).

## 11. EQ curves drawn by the software

The software draws its graphs with bilinear biquads in Zölzer's K form,
`K = tan(π · f / 48000)`, i.e. at 48 kHz. It does not use the RBJ "cookbook"
forms.

- **Peak:**
  - Boost: `(1 + V·K/Q + K²) / (1 + K/Q + K²)` with `V = 10^(|G|/20)`.
  - Cut: the exact inverse.
  - This equals an RBJ peak whose Q is divided by `10^(|G|/40)`, so at the same
    Q both boosts and cuts are wider than with the RBJ formula.
- **Shelves:**
  - Second order; the band's Q replaces 1/√2.
  - Boost: `√V·K/Q` in the numerator over `K/Q` in the denominator.
  - Cut: the inverse of the boost.
  - This equals an RBJ shelf whose corner frequency is shifted by `10^(|G|/80)`
    *(inferred by comparing the transfer functions)*.
- **LP/HP:** first order for LP/HP −6, second order with Q for LP/HP −12.
- **All-pass:** first order for all-pass 1, second order with Q for all-pass 2.
- **Crossover:** the crossover high-/low-pass curve is drawn by a separate
  routine.

## 12. Files

### 12.1 Preset file (`.prs`)

| Part | DSP 408 | DSP 206 |
|---|---|---|
| Header | `***DSP408V010***` (16 bytes) | `***DSP206V010***` |
| Body | dump bytes `0x002` … `0x59B` | dump bytes `0x002` … `0x393` |
| End | CR LF | CR LF |
| Total | `0x5AC` = 1452 bytes | `0x3A4` = 932 bytes |

- **What is stored:** the body holds the preset name, all channel records, the
  mute words and the PEQ-bypass words.
- **What is not stored:** the flags, the test signal, the delay unit and the
  EQ/GEQ bypass-all bits.
- **Saving** works offline.
- **Opening** rejects any file with a different size or header ("Open file
  error"). Loading while connected is described in 8.8.

### 12.2 Whole-device backup (`.unt`)

**Layout:**

1. The same 16-byte header.
2. The 18-byte system block (`0x19`).
3. 20 slots, each followed by CR LF.

| Slot | Content |
|---|---|
| Used | The slot number, then dump bytes `0x000` … `0x5A4` (408) / `0x000` … `0x39C` (206) |
| Unused | Filled with `0x64` |

| | DSP 408 | DSP 206 |
|---|---|---|
| Nominal size | 28 994 bytes | 18 594 bytes |
| Download rejects files shorter than | 28 954 bytes | 18 554 bytes |

A file with a different header is rejected too.

**Quirk:** the last bytes of every stored dump are not saved (on the 408 the
GEQ bypass and the high byte of the outputs' EQ-bypass word). On a download the
device receives the CR LF and the start of the next slot in their place
*(inferred from the slot length)*.

### 12.3 Other data

- **No settings file:** the software keeps no ini file and no registry
  settings.
- **Language:** English and Simplified Chinese string tables are built in.
  English is always used; the language menu is hidden.

## 13. RS232 remote-control protocol (help page)

The help menu documents a separate, simple protocol for external control
systems, identical for both models. It is not used by the software itself.

- **Frame:** 9 bytes, no checksum.
- **Port:** 115200 baud, 8 data bits, no parity, 1 stop bit.
- **Address:** device ID, default 1.
- **Timing:** at least about 20 ms between commands *(read from a help image)*.

```
7B 7D  <addr>  <cmd>  <d1> <d2> <d3>  7D 7B
```

| Cmd | d1 | d2 | d3 | Meaning |
|---|---|---|---|---|
| `0x41` | 0 = input, 1 = output | channel 0 … 7 | 0 = up, 1 = down | Gain step *(step size not documented)* |
| `0x42` | 0 = input, 1 = output | channel | 0 = unmute, 1 = mute | Mute |
| `0x43` | 0 = factory, 1 = user | preset number | 0 | Load preset. The example for U01 uses 0, so the number is probably 0-based *(inferred)*. |
| `0x44` | channel | value high byte | value low byte | Set input gain |
| `0x45` | channel | value high byte | value low byte | Set output gain |
| `0x48` | 0 = input, 1 = output | channel | 0 | Read the current gain |
| `0x49` | 0 = input, 1 = output | channel | 0 | Read mute: 0 = unmuted, 1 = muted |
| `0x4A` | 0 | 0 | 0 | Read the current preset number |

- **Value scale:** the same as in 10.1, but big-endian here.
- **Examples:**
  - `7B 7D 01 44 00 01 18 7D 7B` sets In A to 0.0 dB (0x0118 = 280).
  - `7B 7D 01 45 01 00 FA 7D 7B` sets Out 2 to −3.0 dB (0x00FA = 250).
- **Gaps:** `0x46` and `0x47` are not documented. The help text's description
  of the `0x4A` answer range is inconsistent.

## 14. Differences between the 408 and the 206 original software

The two programs share the same code. They differ only where the channel count
matters.

| | 408 original software | 206 original software |
|---|---|---|
| Channels | 4 in / 8 out | 2 in / 6 out |
| Output channel bytes | `0x04`–`0x0B` | `0x02`–`0x07` |
| Output record | 104 bytes, 4 crossing gains | 100 bytes, 2 crossing gains |
| Dump | `0x5A8` bytes, 29 chunks | `0x3A0` bytes, 19 chunks |
| Switches block | `0x580` | `0x380` |
| Meter slots | 12 used | 0–1 inputs, 4–9 outputs |
| Model check (`0x13`) | `DSP408`, `DSP408 V010` | `DSP206`, `DSP206 V010` |
| File header | `***DSP408V010***` | `***DSP206V010***` |
| `.prs` / `.unt` size | 1452 / 28 994 bytes | 932 / 18 594 bytes |
| Copy | – | Also stops meter polling during the copy |

Everything else is the same in both:

- the interfaces, USB IDs, TCP port and default IP;
- the commands and the 20 user presets;
- the PEQ band counts;
- the passwords;
- the dialogs and the RS232 help.

## 15. Status in this project

Version 0.9.0 of this project follows the original software in everything a
DSP sees on the network (except where 15.2 says otherwise). The table shows
since when each part is used.

**Version column:**

- "upstream" means the original Flutter app (dsp-408-ui) that this project was
  forked from.
- Version 0.3.0 left no build or commit, so the earliest version that can be
  checked is 0.4.0.

**Code locations:** the protocol code lives in
[`crates/dsp-core/src/protocol.rs`](crates/dsp-core/src/protocol.rs),
[`config_dump.rs`](crates/dsp-core/src/config_dump.rs),
[`ops.rs`](crates/dsp-core/src/ops.rs) and
[`crates/dsp-server/src/session.rs`](crates/dsp-server/src/session.rs); in the
old Dart version (`dart/`, behaviour of 0.9.0) in
[`protocol_service.dart`](dart/packages/dsp_core/lib/src/protocol_service.dart),
[`config_dump.dart`](dart/packages/dsp_core/lib/src/config_dump.dart),
[`device_ops.dart`](dart/packages/dsp_core/lib/src/device_ops.dart) and
[`dsp_session.dart`](dart/server/lib/src/dsp_session.dart).

### 15.1 Used the same way

| Topic | Since | Note |
|---|---|---|
| TCP port 9761, frame format, XOR checksum | upstream | The project writes the checksum as "seed 01, XOR from src to the last data byte". That is the same value (section 6). |
| Device ID as the address byte (1 … 250) | 0.9.0 | Set per DSP; before, the address was fixed to 1. |
| Connect order `10 → 13 → 2C → 22 → 14 → 29 × 20 → 27 × n → 12` | ≤ 0.4.0 | 29 chunks since upstream; the DSP 206 with 19 chunks since 0.8.0 (0.4.0 – 0.7.x read 18). |
| Model and version check (`DSP408` / `DSP206`, `V010`); wrong device → `0x11` | 0.9.0 | Before, "408"/"206" anywhere in the answer was accepted. |
| Lock/PTS status `0x2C`, password check `0x2D` | 0.9.0 | The password is stored with the DSP's settings in the panel. Before, the answer was read as a "preset count". |
| Slot table `0x22`, current preset `0x14` | 0.9.0 | Sent since ≤ 0.4.0, answers used since 0.9.0. |
| Timeouts: 4 s and 3 attempts for the handshake, 1 s and one retry afterwards, then "link broken"; store 10 s, recall 50 s | 0.9.0 | Before: 500 ms, 3 attempts for required steps, 5 timeouts in a row. |
| Heartbeat `0x12` when idle; disconnect `0x11` | 0.9.0 | |
| Meters `0x40` every 300 ms | 0.9.0 | Default interval; 40 ms in 0.7.0 – 0.8.x. |
| Gain encoding (two ranges, 0 … 400), one `0x34` per change | upstream / 0.9.0 | The gain was sent twice up to 0.8.x. |
| PEQ/GEQ gain `10·dB + 120`, PEQ type list, GEQ 31 bands | upstream | |
| Mute `0x35`, routing `0x3A` | upstream | |
| Frequency `1000·2^((i−170)/30)`, Q `2^((raw−16)/12)` with the display table | 0.9.0 | Scales 0 … 300 and 0 … 100 since 0.4.0, approximated by `20·1000^(i/300)` and `10^((raw−16)/40)` until 0.8.x; upstream used 0 … 1000 and 0 … 255. |
| Q ≤ 3 for shelves and PEQ LP/HP, Q 0.71 when a band changes to such a type | 0.9.0 | |
| Slope table with 0 = bypass | 0.4.0 | |
| Phase `0x36`, delay `0x38` (96 per ms, max. 680 ms) | ≤ 0.4.0 | |
| Delay unit `0x15` (ms/m/ft, 344 m/s), stored on the DSP | 0.9.0 | Before: a browser setting, ms/m, 343 m/s. |
| Gate `0x3E`, compressor `0x30`, ratio table, thresholds `2·dB + 180`, compressor knee 0 … 12 | ≤ 0.4.0 | |
| Limiter `0x3F` with its knee (0 … 12) | 0.9.0 | Before, the third value was "unknown" and not editable. |
| Time ranges: attack 0 … 998, release 9 … 2999, hold 9 … 998 | 0.9.0 | Before: up to 2998 / 4999; such values are limited when read. |
| Channel names `0x3D` (8 bytes, NUL-padded) | 0.4.0 | The project only allows printable ASCII. |
| Recall `0x20` (1 … 20 = U01 … U20), names `0x29` | ≤ 0.4.0 | |
| Store `0x26 → 0x21 → 0x12` | 0.9.0 | One `0x12`; two up to 0.8.x. |
| Linking: `0x2A` copy to each new slave, one `0x3B` for the master, read everything again | 0.9.0 | Symmetric masks in 0.5.0 – 0.6.x; master/slave since 0.7.0, but up to 0.8.x with `0x3B` for every slave and the changes sent to every member. |
| A change of a linked channel is sent once; the other members follow by the group rule of the original software (8.3) | 0.9.0 | |
| Channel copy `0x2A`, then read everything again | 0.9.0 | Up to 0.8.x by single commands, without mute. |
| EQ bypass `0x3C`, GEQ bypass `0x49` | 0.9.0 | |
| Test signal `0x39` (type 0 = analog input), 31 ISO sine frequencies | 0.7.0 | Read back from the dump since 0.9.0. |
| DSP 206 meter slots (outputs from slot 4) | 0.7.0 | |
| Dump: channel tail, mute words, PEQ-bypass words, totals `0x5A8` / `0x3A0` | 0.8.0 | |
| Dump end: test signal, delay unit, EQ/GEQ bypass bits | 0.9.0 | |
| Matrix crossing gain `0x41` (0 … 280) | 0.8.0 | Shown read-only in 0.6.0 – 0.7.x. |
| Meters: float32 from 3 bytes, status word | 0.8.0 | Upstream – 0.7.x read the slots as float16 (0.5 … 2.0). |
| Compressor LED only for a ratio other than 1:1, limiter LED only below +20 dB | 0.9.0 | |
| EQ curves: Zölzer K form at 48 kHz | 0.9.0 | RBJ cookbook forms up to 0.8.x. |
| Loading a stored preset: name `0x26`, every value as a single command in the order of a preset file, then `0x12` | 0.9.0 | The order is that of section 8.8. |

### 15.2 Remaining differences

| Topic | Original software | This project |
|---|---|---|
| Interfaces | TCP, USB HID, serial, UDP discovery | TCP only |
| Meter polling | Always while connected | Only while a browser shows the DSP; otherwise the heartbeat keeps the link alive. The interval can be set (5 … 300 ms). |
| Meter display | 9 / 8 LED segments with the thresholds of 10.8 | Segmented meter in dBFS, clip at −0.5 dBFS |
| Unanswered `0x2C`, `0x22`, `0x14` or dump chunk | Link broken | Tolerated (a chunk that is not answered ends the dump) |
| `0x12` after the dump | Once | Twice, as seen in a capture of the original software |
| Stored presets of the panel | Preset files (`.prs`) | A preset library on the server. Loading one also sends the EQ/GEQ bypass and the delay unit, which a preset file does not hold. |
| Linking and copying without a DSP | Done in the editor's model | The same, for presets in the panel's offline editor |
| Crossover curves | Drawn by a routine not covered here | Butterworth, Bessel and Linkwitz-Riley responses |

### 15.3 Not used by this project

- **Commands:** `0x04`, `0x19`, `0x1A`, `0x23` (factory reset), `0x24` (sent),
  `0x25`, `0x28` (whole-device backup), `0x2E` (PTS setup), `0x2F` (lock setup).
  Setting up a lock or a PTS limit and the factory reset are left to the
  original software.
- **Interfaces:** USB HID, serial and UDP discovery.
- **Files:** `.prs` and `.unt`.
- **RS232:** the remote-control protocol.

## 16. Open points (hardware needed)

1. **`0x14` numbering.** Is the current preset 0-based or 1-based? The original
   software compares it with the 0-based `0x29` index. A capture shows `0x14`
   answering 20 while U20 was active, which suggests 1-based.
2. **Meaning of `0x04`, `0x19`/`0x1A` and `0x23`.** What do `0x04` and the
   18-byte system block of `0x19`/`0x1A` do? Is `0x23` a full factory reset or
   only a clear of the presets?
3. **Slot numbering of `0x25`.** It is sent 0-based, while `0x24` is 1-based.
4. **USB HID report length.**
5. **PTS counter.** What does it count, and in which units?
6. **Flag value 0 of `0x49`.** Assumed to mean active, as for `0x3C`.
7. **Meter calibration.** Is 1.0 really 0 dBFS?
8. **Linked channels.** Does the DSP apply a change to linked channels itself
   when only one channel is addressed? Since version 0.9.0 this project relies
   on it, like the original software.
9. **Limiter knee.** Does the device use it, given that the UI hides it?
10. **RS232 protocol.** The step size of the gain command `0x41`, and the
    preset numbering of `0x43` / `0x4A`.
