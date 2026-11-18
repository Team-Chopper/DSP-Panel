# DSP Panel

A browser panel for the **DSP 408, DSP 206 and DSP 204** loudspeaker processors, built for live sound on a
venue network. One small server holds the connections to any number of DSPs; every phone, tablet or laptop on
the network opens the panel in the browser and controls all DSPs at once. Every change is shown live on all
devices.

- **Works offline.** Fonts and the WebAssembly client come from the panel server – no internet, no CDN.
- **Server-authoritative.** Only the server talks to the DSPs (TCP 9761); browsers talk to the server over
  one WebSocket.
- **Fast.** Rust server, Rust/WebAssembly client with fine-grained updates: faders and meters stay smooth on
  phones and in Firefox.
- **No login.** Made for a closed venue network: everyone who can reach the server controls the DSPs. HTTPS
  with a self-signed certificate is built in, because browsers only allow the microphone there.

```
Phone / tablet / laptop ──HTTPS + WebSocket──▶ dsp-server ──TCP 9761──▶ DSP 1
                                                           ──TCP 9761──▶ DSP 2 …
```

## Features

**Every page for every DSP** – one tab per DSP, sections Gain · EQ · GEQ · Gate · Comp · Limit · Matrix ·
Delay · RTA. Console-style faders (drag, wheel, arrow keys, −/+ steppers with auto-repeat, double-click to
reset, Shift for fine moves, haptic tick at 0 dB on phones); every value can also be typed.

- **Gain** – faders with dBFS meters, clip and limiter LEDs, mute and polarity; an EQ overview of all channels
  above the faders; named link groups shown as brackets.
- **EQ** – 8 bands per input, 9 per output, HPF/LPF (Butterworth, Bessel, Linkwitz-Riley up to 48 dB/oct).
  Curves drawn with the same formulas as the original editor; magnitude or **phase** view, another channel can
  be drawn for comparison. Filter settings files of measurement programs (REW / Equalizer APO text format) are
  taken over as a proposal.
- **GEQ** – 31 bands on the inputs with an RTA overlay.
- **Auto-EQ** – measure pink noise with a microphone, review the proposed GEQ or PEQ correction, apply it.
- **Gate, compressor, limiter** – transfer curve, live "reducing gain" LEDs from the DSP. A **limiter
  calculator** turns amplifier data (rated power, sensitivity or gain, load) into the threshold for a wanted
  power.
- **Matrix** – routing and crossing gains.
- **Delay** – draggable speakers, ms / m / ft.
- **RTA** – 1/24-octave analyser (FFT 16384) with averaging and peak hold, browser and DSP test generator.
- **Presets** – the DSP's slots U01–U20, and a preset library on the server that can be edited offline.
  Loading a library preset sends only what differs, reads the DSP back and reports values that did not take;
  the last five loads can be undone.
- **Linking and copying** like the original editor; settings can also be copied in parts (level, filters, EQ,
  dynamics, routing) to channels of other DSPs.

**Live safety**

- **Mute all** – one button mutes every channel of every DSP; releasing it opens only what was open before
  (also after a crash, a restart or a preset load).
- **Max level per channel** – the server never sets a gain above it, also not when a preset is loaded.
- **Safe test generator** – all outputs are lowered before the DSP generator starts and restored after it
  stops.
- **View-only mode** – phones and tablets start in it, so nothing moves by accident; one tap allows changes.
- **Limit light** – an optional page `/limit` (e.g. for the DJ booth): one big lamp that lights
  while a limiter acts.

**More:** network search for DSPs, device ID and lock password per DSP, debug log of every frame, automatic
reconnect.

## Supported devices

| Device | Inputs / outputs | Status |
|---|---|---|
| DSP 408 | 4 / 8 | supported |
| DSP 206 | 2 / 6 | supported |
| DSP 204 | 2 / 4 | new, not yet tested on hardware |

The model is detected automatically. A DSP accepts only one connection: disconnect other control software
first. Open questions about the protocol are collected in [TODO.md](TODO.md).

## Install

The release package contains a static Linux binary (x64), the web client and an install script for systemd.

```bash
tar xzf dsp-panel-rs-linux-x64.tar.gz
cd dsp-panel-rs-<version>-linux-x64
sudo bash install.sh
```

Then open `https://<server>:8444`. The script installs to `/opt/dsp-panel-rs` (program),
`/etc/dsp-panel-rs` (configuration, see [deploy/config.example.yaml](deploy/config.example.yaml)) and
`/var/lib/dsp-panel-rs` (data, certificate).
Running it again updates the panel and keeps configuration and data; `sudo bash uninstall.sh` removes it
(`--purge` also deletes configuration and data).

- `--import-from /var/lib/dsp-panel` takes over DSP list, presets and certificate of the old Dart
  panel. Imported DSPs start disconnected – disconnect them in the old panel before connecting them here.
- **Automatic updates** (e.g. from a VM management tool): fetch
  `https://github.com/Team-Chopper/DSP-Panel/releases/latest/download/latest-x64.json`, compare `version` with the
  installed one, download `…/releases/latest/download/dsp-panel-rs-linux-x64.tar.gz`, check `sha256`, unpack and
  run `sudo bash <folder>/install.sh`. The script asks no questions.
- `GET /api/status` (no login) lists the DSPs for a monitoring hub: `{version, devices: [{name, kind, state,
  detail}]}` with state `ok`, `warn` or `offline` and the active preset as detail.
- The microphone (RTA, Auto-EQ) only works on HTTPS or `localhost` – keep TLS on.

## Data

All data lives in the data directory as JSON/YAML files:

| File | Content |
|---|---|
| `devices.json` | the DSPs (address, model, device ID, lock password, max levels) |
| `presets.json` | the preset library |
| `safety.json` | what mute all has to unmute, output gains saved for the test generator |
| `panel.json` | limiter calculator, link group names, limit light settings |

`devices.json` and `presets.json` have the format of the old Dart server, so data can be copied between both.
`auth.json` and `secret.key` of an earlier version are no longer used and can be deleted.

## Development

Needs Rust (stable, see [rust-toolchain.toml](rust-toolchain.toml), with the `wasm32-unknown-unknown` target)
and [trunk](https://trunkrs.dev) for the web client.

```powershell
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\cargo-target\dsp-panel"   # keep build output off network shares
cargo test --workspace
cargo run -p fake-dsp -- --port 19761 --models 408,206,204          # simulated DSPs on 19761–19763
cd crates\dsp-web; trunk build --release; cd ..\..                  # web client -> crates/dsp-web/dist
cargo run -p dsp-server -- serve --no-tls --port 8090 --data-dir <dir> --web-root crates\dsp-web\dist
```

Open http://localhost:8090 and add the simulated DSPs (127.0.0.1, ports 19761–19763).

`crates/dsp-core/tests/parity.rs` checks the Rust core byte for byte against reference data of the old Dart
core. Regenerate it after changing `dart/packages/dsp_core`:

```powershell
cd tool\parity
dart pub get --offline
dart run bin/generate.dart ..\..\crates\dsp-core\tests\fixtures\parity.json
```

### Release

GitHub builds the releases ([.github/workflows/release.yml](.github/workflows/release.yml)): bump `version` in
[Cargo.toml](Cargo.toml) and push to `main`. If there is no release `v<version>` yet, the workflow runs the tests,
builds the package on Linux and publishes the release with `dsp-panel-rs-linux-x64.tar.gz` and `latest-x64.json`.
Pushes without a new version build nothing.

A package for testing can still be built locally with `.\tool\build_dist.ps1` (needs zig and cargo-zigbuild:
`winget install zig.zig`, `cargo install cargo-zigbuild`); it goes to `dist/`, `-Arch arm64` builds for ARM.

## Repository

| Path | Content |
|---|---|
| [crates/dsp-core](crates/dsp-core) | protocol, config dump, device state, ops and linking, EQ and RTA math, web API types |
| [crates/dsp-server](crates/dsp-server) | the server: DSP sessions, HTTP(S), WebSocket, presets, live safety |
| [crates/dsp-web](crates/dsp-web) | the web client (Leptos, WebAssembly) |
| [crates/fake-dsp](crates/fake-dsp) | simulated DSPs for development and tests |
| [deploy](deploy) | install scripts, systemd unit, example configuration |
| [tool](tool) | local package script, parity reference generator |
| [release](release) | old update location, frozen at 1.0.0-alpha.5 – new releases are GitHub releases |
| [dart](dart) | the old Dart/Flutter version (v0.9.0), frozen, kept as reference |
| [PROTOCOL-RE.md](PROTOCOL-RE.md), [ORIGINAL-SOFTWARE.md](ORIGINAL-SOFTWARE.md) | the protocol and how the original editor uses it |
| [TODO.md](TODO.md) | open checks on real hardware and how to capture them |

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). This project started as a fork of dsp-408-ui. It is not affiliated
with the maker of the devices; the protocol was documented for interoperability only.
