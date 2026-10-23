# DSP Panel – Rust

Rewrite of the DSP panel in Rust, built next to the Dart/Flutter version
(which stays unchanged until the Rust version has been tested on hardware).

| Crate | What it is |
|---|---|
| `crates/dsp-core` | Protocol, frame decoder, config dump, device state, ops/linking, packet descriptions, web API types, EQ math. Port of `packages/dsp_core` (byte-for-byte identical, see the parity test). |
| `crates/dsp-server` | The panel server: TCP to the DSPs, HTTP(S), WebSocket, login, server presets. Port of `server/` with the same API and data files. |
| `crates/fake-dsp` | Simulated DSP 408 / 206 for development and tests. |
| `crates/dsp-web` | The web client in Leptos (WebAssembly, built with trunk). Same pages as the Flutter client; the RTA runs in an AudioWorklet + WebAssembly. |

The WebSocket API, `config.yaml`, `devices.json`, `presets.json`, `auth.json`
and `secret.key` have the formats of the Dart server: the Rust server works
with the Flutter client, and data can be copied between both installations.

## Develop

```powershell
cd rust
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\cargo-target\dsp-panel"   # build outside the network share
cargo test --workspace
cargo run -p fake-dsp -- --port 19761 --models 408,206
cd crates\dsp-web; trunk build --release; cd ..\..                  # web client -> crates/dsp-web/dist
cargo run -p dsp-server -- serve --no-tls --no-auth --port 8090 --data-dir <dir> --web-root crates\dsp-web\dist
```

Then open http://localhost:8090 and add the fake DSPs (127.0.0.1:19761 / 19762).
`--web-root ..\build\web` serves the Flutter client instead. The web client
uses the cargo profile `web-release` (size-optimised, `dsp-core` at full
speed); trunk needs the `wasm32-unknown-unknown` target (`rust-toolchain.toml`).
The microphone (RTA, Auto-EQ) needs HTTPS or `localhost`.

`crates/dsp-core/tests/parity.rs` replays reference data of the Dart
implementation (`tests/fixtures/parity.json`). Regenerate it after changing
the Dart core:

```powershell
cd rust\tool\parity
dart pub get --offline
dart run bin/generate.dart ..\..\crates\dsp-core\tests\fixtures\parity.json
```

## Release package

Needs zig and cargo-zigbuild (`winget install zig.zig`, `cargo install cargo-zigbuild`):

```powershell
.\rust\tool\build_dist.ps1 -Release
```

builds a static Linux binary (musl) and writes

- `rust/release/dsp-panel-rs-linux-x64.tar.gz` – server, web client, install scripts
- `rust/release/latest-x64.json` – `version`, `sha256`, `size`, `folder` of that package

## Install on the VM (next to the Dart panel)

The Rust panel installs as its own service `dsp-panel-rs` (port **8444**,
`/opt/dsp-panel-rs`, `/etc/dsp-panel-rs`, `/var/lib/dsp-panel-rs`) and does
not touch the Dart installation (`dsp-panel`, port 8443).

```bash
tar xzf dsp-panel-rs-linux-x64.tar.gz
cd dsp-panel-rs-<version>-linux-x64
sudo bash install.sh --import-from /var/lib/dsp-panel   # or without import for fresh data
```

`--import-from` copies the DSP list, server presets, password and TLS
certificate of the Dart panel. Imported DSPs start **disconnected**: a DSP
probably accepts only one connection, so disconnect it in the old panel
before connecting it in the new one.

Automatic installation (e.g. by a VM management tool): fetch
`rust/release/latest-x64.json` from the repository, compare `version` with
the installed one, download `rust/release/dsp-panel-rs-linux-x64.tar.gz`,
check `sha256`, unpack and run `sudo bash <folder>/install.sh`. Without a
terminal the script does not ask for a password; set it with
`sudo dsp-panel-rs-passwd` (or import it). Re-running `install.sh` updates
program and web client and keeps configuration and data.

Remove with `sudo bash uninstall.sh` (`--purge` also deletes configuration and data).
