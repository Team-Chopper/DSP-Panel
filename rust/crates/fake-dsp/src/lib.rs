//! Fake DSP 204 / 206 / 408.
//!
//! A TCP server that answers the DSP protocol well enough to develop and test
//! the panel without hardware. It keeps a device state, applies every set
//! command to it, stores and recalls presets, copies channels, and serves the
//! config dump from that state (for the DSP 408 the initial state is the dump
//! captured from a real device). Startup answers mirror the captured DSP 408
//! session. Like the original editor assumes, a change of a linked channel is
//! applied to its link group by the device itself.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dsp_core::describe::describe;
use dsp_core::ops;
use dsp_core::protocol::{self as p, Frame, calculate_checksum};
use dsp_core::testing::captured_config_chunks;
use dsp_core::{
    CompressorState, ConfigDump, DeviceModel, DeviceState, FilterState, FrameDecoder, GateState, LimiterState, PeqBand,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

/// Settings of a simulated device.
#[derive(Clone, Debug)]
pub struct FakeOptions {
    pub model: DeviceModel,
    /// Artificial response delay.
    pub latency: Duration,
    /// Print received set commands.
    pub verbose: bool,
    /// Device ID: the fake answers only frames addressed to it.
    pub device_id: u8,
    /// Label used in log output.
    pub label: Option<String>,
}

impl Default for FakeOptions {
    fn default() -> Self {
        FakeOptions { model: DeviceModel::DSP408, latency: Duration::ZERO, verbose: false, device_id: 1, label: None }
    }
}

/// The simulated device, shared between the connections and the tests.
pub struct FakeDevice {
    pub model: DeviceModel,
    pub label: String,
    pub device_id: u8,
    pub latency: Duration,
    pub verbose: bool,
    /// Device info answered to 0x13.
    pub device_info: String,
    /// Lock password (4 characters); the device asks for it while [locked].
    pub password: String,
    pub locked: bool,
    /// PTS limit: switched on, limit and current count.
    pub pts_on: bool,
    pub pts_limit: u16,
    pub pts_count: u16,
    /// Do not answer these command bytes (to test timeouts).
    pub ignore_commands: HashSet<u8>,
    /// Every frame received from the host, in order.
    pub received: Vec<Frame>,
    /// Current settings of the simulated device.
    pub state: DeviceState,
    /// Stored presets by 0-based slot.
    presets: HashMap<u8, DeviceState>,
    /// Current preset number (0 = factory, n = Un).
    pub current_preset: u8,
    pending_preset_name: String,
    random: u64,
    connections: HashMap<u64, Vec<AbortHandle>>,
    next_connection: u64,
}

impl FakeDevice {
    fn new(options: &FakeOptions) -> Self {
        let model = options.model;
        let state = initial_state(model);
        let mut presets = HashMap::new();
        for i in 0..p::PRESET_COUNT as u8 {
            let mut preset = initial_state(model);
            preset.current_preset = if i == 0 { state.current_preset.clone() } else { format!("Preset {}", i + 1) };
            presets.insert(i, preset);
        }
        FakeDevice {
            model,
            label: options.label.clone().unwrap_or_else(|| format!("Fake {}", model.name)),
            device_id: options.device_id,
            latency: options.latency,
            verbose: options.verbose,
            device_info: format!("{} V0104P", model.info_prefix()),
            password: "1234".into(),
            locked: false,
            pts_on: false,
            pts_limit: 9999,
            pts_count: 0,
            ignore_commands: HashSet::new(),
            received: Vec::new(),
            state,
            presets,
            current_preset: 0,
            pending_preset_name: String::new(),
            random: 42,
            connections: HashMap::new(),
            next_connection: 0,
        }
    }

    /// Mark a user preset slot (0-based) as empty.
    pub fn clear_preset(&mut self, slot: u8) {
        self.presets.remove(&slot);
    }

    /// Set commands (gain, mute, …) received so far, decoded for display.
    pub fn received_set_commands(&self) -> Vec<String> {
        self.received.iter().filter(|f| is_set_command(f[5])).map(|f| describe(f, &self.model)).collect()
    }

    fn next_random(&mut self) -> f64 {
        // xorshift64*: deterministic noise for the meters
        let mut x = self.random;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.random = x;
        (x.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Device → host frame.
    fn response(&self, cmd: u8, payload: &[u8]) -> Frame {
        let mut data = vec![self.device_id, 0x00, payload.len() as u8 + 1, cmd];
        data.extend_from_slice(payload);
        let checksum = calculate_checksum(&data);
        let mut frame = vec![0x10, 0x02];
        frame.extend_from_slice(&data);
        frame.extend_from_slice(&[0x10, 0x03, checksum]);
        frame
    }

    fn ack(&self) -> Frame {
        self.response(0x01, &[])
    }

    /// Handle one frame from the host; returns the answer (if any).
    fn handle(&mut self, frame: &[u8]) -> Option<Frame> {
        if frame.len() < 8 || frame[3] != self.device_id {
            return None; // addressed to another device
        }
        self.received.push(frame.to_vec());
        let cmd = frame[5];
        let end = (5 + frame[4] as usize).min(frame.len());
        let args: Vec<u8> = frame.get(6..end).unwrap_or(&[]).to_vec();
        if self.verbose && is_set_command(cmd) {
            println!("[{}] {}", self.label, describe(frame, &self.model));
        }
        if self.ignore_commands.contains(&cmd) || cmd == 0x11 {
            return None; // 0x11 is not answered
        }
        let arg = args.first().copied().unwrap_or(0);
        Some(match cmd {
            0x10 => self.response(0x10, &[0x13]),
            0x13 => self.response(0x13, self.device_info.as_bytes()),
            0x2C => {
                let payload = [
                    self.pts_on as u8,
                    (self.pts_limit >> 8) as u8,
                    (self.pts_limit & 0xFF) as u8,
                    (self.pts_count >> 8) as u8,
                    (self.pts_count & 0xFF) as u8,
                    self.locked as u8,
                ];
                self.response(0x2C, &payload)
            }
            0x2D => self.check_password(&args),
            0x22 => {
                let slots: Vec<u8> =
                    (0..p::PRESET_COUNT as u8).map(|i| if self.presets.contains_key(&i) { 0xFF } else { 0 }).collect();
                self.response(0x22, &slots)
            }
            0x14 => self.response(0x14, &[self.current_preset]),
            0x29 => {
                let name = self.presets.get(&arg).map(|s| s.current_preset.clone()).unwrap_or_default();
                let mut payload = vec![arg];
                payload.extend(padded(&name, 14));
                self.response(0x29, &payload)
            }
            0x27 => self.config_chunk(arg),
            0x40 => {
                let payload = self.meter_payload();
                self.response(0x40, &payload)
            }
            _ => self.apply(cmd, &args),
        })
    }

    fn check_password(&mut self, a: &[u8]) -> Frame {
        let mode = a.first().copied().unwrap_or(0);
        let ok = a.len() >= 5 && a[1..5] == *self.password.to_uppercase().as_bytes();
        if ok && mode == 0 {
            self.locked = false;
        }
        if ok && mode == 1 {
            self.pts_count = 0;
        }
        self.response(0x2D, &[mode, ok as u8])
    }

    fn config_chunk(&self, index: u8) -> Frame {
        let chunks = ConfigDump::split(&ConfigDump::new(self.model).encode(&self.state));
        match chunks.get(index as usize) {
            None => self.ack(),
            Some(chunk) => {
                let mut payload = vec![index];
                payload.extend_from_slice(chunk);
                self.response(0x24, &payload)
            }
        }
    }

    /// Apply a set command to the simulated state; answers with an ACK.
    fn apply(&mut self, cmd: u8, a: &[u8]) -> Frame {
        let u16_at = |at: usize| a.get(at).copied().unwrap_or(0) as i64 | ((a.get(at + 1).copied().unwrap_or(0) as i64) << 8);
        let model = self.model;
        let c = a.first().and_then(|b| model.channel_for_byte(*b));
        // The link group the change applies to (the device mirrors linked
        // channels; matrix and names are per channel).
        let group = |state: &DeviceState| c.map(|c| ops::edit_group_of(state, c)).unwrap_or_default();
        let s = &mut self.state;
        match (cmd, c) {
            (0x34, Some(_)) => {
                for g in group(s) {
                    s.gains.insert(g, p::gain_value_to_db(u16_at(1)));
                }
            }
            (0x35, Some(_)) if a.len() >= 2 => {
                for g in group(s) {
                    s.mutes.insert(g, a[1] == 1);
                }
            }
            (0x36, Some(_)) if a.len() >= 2 => {
                for g in group(s) {
                    s.phase.insert(g, a[1] == 1);
                }
            }
            (0x38, Some(_)) => {
                for g in group(s) {
                    s.delays.insert(g, u16_at(1));
                }
            }
            (0x3A, Some(c)) if a.len() >= 2 => {
                s.matrix_routing.insert(c.into(), a[1] as i64);
            }
            (0x41, Some(c)) if a.len() >= 4 && (a[1] as usize) < model.inputs.len() => {
                s.matrix_gains.insert(format!("{c}:{}", model.inputs[a[1] as usize]), p::gain_value_to_db(u16_at(2)));
            }
            (0x48, Some(c)) if s.geq_bands.contains_key(c) && a.len() >= 4 => {
                for g in group(s) {
                    if let Some(band) = s.geq_bands.get_mut(&g).and_then(|b| b.get_mut(a[1] as usize)) {
                        *band = p::geq_value_to_db(u16_at(2));
                    }
                }
            }
            (0x33, Some(_)) if a.len() >= 9 => {
                for g in group(s) {
                    if let Some(band) = s.peq_bands.get_mut(&g).and_then(|b| b.get_mut(a[1] as usize)) {
                        *band = PeqBand {
                            gain_db: p::peq_gain_to_db(u16_at(2)),
                            freq_raw: u16_at(4),
                            q_raw: a[6] as i64,
                            peq_type: a[7] as i64,
                            bypass: a[8] == 1,
                        };
                    }
                }
            }
            (0x32, Some(_)) if a.len() >= 4 => {
                for g in group(s) {
                    s.hi_pass.insert(g, FilterState::new(u16_at(1), a[3] as i64));
                }
            }
            (0x31, Some(_)) if a.len() >= 4 => {
                for g in group(s) {
                    s.lo_pass.insert(g, FilterState::new(u16_at(1), a[3] as i64));
                }
            }
            (0x30, Some(_)) => {
                for g in group(s) {
                    let value = CompressorState {
                        ratio: u16_at(1),
                        attack: u16_at(3),
                        release: u16_at(5),
                        knee: u16_at(7),
                        threshold: u16_at(9),
                    };
                    s.compressors.insert(g, value);
                }
            }
            (0x3E, Some(_)) => {
                for g in group(s) {
                    let value =
                        GateState { attack: u16_at(1), release: u16_at(3), hold: u16_at(5), threshold: u16_at(7) };
                    s.gates.insert(g, value);
                }
            }
            (0x3F, Some(_)) => {
                for g in group(s) {
                    let value =
                        LimiterState { attack: u16_at(1), release: u16_at(3), knee: u16_at(5), threshold: u16_at(7) };
                    s.limiters.insert(g, value);
                }
            }
            (0x3C, Some(_)) if a.len() >= 2 => {
                for g in group(s) {
                    s.eq_bypass.insert(g, a[1] == 1);
                }
            }
            (0x49, Some(c)) if s.geq_bypass.contains_key(c) && a.len() >= 2 => {
                for g in group(s) {
                    s.geq_bypass.insert(g, a[1] == 1);
                }
            }
            (0x3B, Some(c)) if a.len() >= 2 => {
                s.links.insert(c.into(), a[1] as i64);
            }
            (0x3D, Some(c)) if a.len() >= 9 => {
                let name: String = a[1..9].iter().filter(|b| **b != 0).map(|b| *b as char).collect();
                s.channel_names.insert(c.into(), name);
            }
            (0x2A, Some(c)) if a.len() >= 2 => {
                if let Some(to) = model.channel_for_byte(a[1])
                    && to != c && model.is_input(to) == model.is_input(c)
                        && let Ok(list) = ops::copy_channel(s, c, to) {
                            for op in list {
                                ops::apply(s, &op);
                            }
                        }
            }
            (0x39, _) if a.len() >= 2 => {
                s.test_tone_type = a[0] as i64;
                s.test_tone_param = a[1] as i64;
            }
            (0x15, _) if !a.is_empty() => s.delay_unit = a[0] as i64,
            (0x26, _) => {
                self.pending_preset_name = String::from_utf8_lossy(a).trim().to_string();
            }
            (0x21, _) if !a.is_empty() => {
                let slot = a[0].wrapping_sub(1);
                s.current_preset = self.pending_preset_name.clone();
                self.presets.insert(slot, copy_state(s));
                self.current_preset = a[0];
            }
            (0x20, _) if !a.is_empty() => {
                let stored =
                    if a[0] == 0 { Some(initial_state(model)) } else { self.presets.get(&(a[0] - 1)).map(copy_state) };
                if let Some(stored) = stored {
                    self.state = stored;
                    self.current_preset = a[0];
                }
            }
            _ => {}
        }
        self.ack()
    }

    fn meter_payload(&mut self) -> Vec<u8> {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        let model = self.model;
        let mut payload = Vec::with_capacity(38);
        let mut dynamics = 0u16;
        // Like the hardware, both models answer with 12 slots: four input
        // slots (unused ones are zero on the DSP 206), then the outputs.
        let channels: Vec<Option<&str>> = (0..4)
            .map(|i| model.inputs.get(i).copied())
            .chain((0..8).map(|o| model.outputs.get(o).copied()))
            .collect();
        for (i, ch) in channels.iter().enumerate() {
            let Some(ch) = ch else {
                payload.extend_from_slice(&[0, 0, 0]);
                continue;
            };
            // Slowly pulsing levels (about -40 … -6 dBFS) with a little noise,
            // different per channel, shifted by the channel gain; muted
            // channels stay quiet.
            let noise = self.next_random() * 3.0;
            let i = i as f64;
            let db = -23.0 + 15.0 * (t * (0.7 + i * 0.13) + i).sin() + noise + self.state.gains.get(*ch).copied().unwrap_or(0.0);
            let muted = self.state.mutes.get(*ch).copied().unwrap_or(false);
            let level = if muted { 0.0 } else { 10f64.powf(db / 20.0) };
            payload.extend_from_slice(&meter_bytes(level));
            if let Some(o) = model.output_index(ch)
                && level > 0.0 {
                    let comp = &self.state.compressors[*ch];
                    if comp.ratio != 0 && db > p::threshold_raw_to_db(comp.threshold) {
                        dynamics |= 1 << o;
                    }
                    if db > p::threshold_raw_to_db(self.state.limiters[*ch].threshold) {
                        dynamics |= 0x100 << o;
                    }
                }
        }
        payload.extend_from_slice(&[(dynamics & 0xFF) as u8, (dynamics >> 8) as u8]);
        payload
    }
}

fn initial_state(model: DeviceModel) -> DeviceState {
    if model == DeviceModel::DSP408 {
        return ConfigDump::new(model).decode(&captured_config_chunks().concat()).state;
    }
    let mut state = DeviceState::new(model);
    state.current_preset = "Default Preset".into();
    state
}

fn copy_state(s: &DeviceState) -> DeviceState {
    DeviceState::from_json(&s.to_json()).unwrap_or_else(|_| s.clone())
}

fn padded(text: &str, length: usize) -> Vec<u8> {
    let mut units: Vec<u16> = text.encode_utf16().take(length).collect();
    units.resize(length, b' ' as u16);
    units.into_iter().map(|u| u as u8).collect()
}

fn is_set_command(cmd: u8) -> bool {
    matches!(
        cmd,
        0x15 | 0x20
            | 0x21
            | 0x26
            | 0x2A
            | 0x30
            | 0x31
            | 0x32
            | 0x33
            | 0x34
            | 0x35
            | 0x36
            | 0x38
            | 0x39
            | 0x3A
            | 0x3B
            | 0x3C
            | 0x3D
            | 0x3E
            | 0x3F
            | 0x41
            | 0x48
            | 0x49
    )
}

/// The upper three bytes of [level] as float32, in the order the DSP sends
/// them (see `protocol::decode_meter_value`).
fn meter_bytes(level: f64) -> [u8; 3] {
    let bits = (level as f32).to_bits();
    [((bits >> 16) & 0xFF) as u8, (bits >> 24) as u8, ((bits >> 8) & 0xFF) as u8]
}

/// A running simulated device.
pub struct FakeDsp {
    device: Arc<Mutex<FakeDevice>>,
    accept: Option<AbortHandle>,
    port: u16,
}

impl FakeDsp {
    pub fn new(options: FakeOptions) -> Self {
        FakeDsp { device: Arc::new(Mutex::new(FakeDevice::new(&options))), accept: None, port: 0 }
    }

    /// Access the simulated device (state, received frames, switches).
    pub fn device(&self) -> MutexGuard<'_, FakeDevice> {
        self.device.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Listen on 127.0.0.1 ([port] 0 = any free port).
    pub async fn start(&mut self, port: u16) -> std::io::Result<()> {
        self.start_on(IpAddr::V4(Ipv4Addr::LOCALHOST), port).await
    }

    pub async fn start_on(&mut self, address: IpAddr, port: u16) -> std::io::Result<()> {
        let listener = TcpListener::bind(SocketAddr::new(address, port)).await?;
        self.port = listener.local_addr()?.port();
        let device = self.device.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accept(device.clone(), stream);
            }
        });
        self.accept = Some(task.abort_handle());
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(task) = self.accept.take() {
            task.abort();
        }
        self.drop_connections();
        // Let the aborted tasks close their sockets.
        tokio::task::yield_now().await;
    }

    /// Close all host connections (simulates a cable pull / device reboot).
    pub fn drop_connections(&self) {
        let mut device = self.device();
        for (_, handles) in device.connections.drain() {
            for h in handles {
                h.abort();
            }
        }
    }

    pub fn connection_count(&self) -> usize {
        self.device().connections.len()
    }
}

impl Drop for FakeDsp {
    fn drop(&mut self) {
        if let Some(task) = self.accept.take() {
            task.abort();
        }
        self.drop_connections();
    }
}

fn accept(device: Arc<Mutex<FakeDevice>>, stream: tokio::net::TcpStream) {
    let _ = stream.set_nodelay(true);
    let (mut reader, mut writer) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Frame>();
    let id = {
        let mut d = device.lock().unwrap_or_else(|e| e.into_inner());
        d.next_connection += 1;
        d.next_connection
    };
    let write_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if writer.write_all(&frame).await.is_err() {
                break;
            }
        }
    });
    let read_device = device.clone();
    let read_task = tokio::spawn(async move {
        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for frame in decoder.add(&buf[..n]) {
                let (answer, latency) = {
                    let mut d = read_device.lock().unwrap_or_else(|e| e.into_inner());
                    (d.handle(&frame), d.latency)
                };
                let Some(answer) = answer else { continue };
                if latency.is_zero() {
                    let _ = tx.send(answer);
                } else {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(latency).await;
                        let _ = tx.send(answer);
                    });
                }
            }
        }
        read_device.lock().unwrap_or_else(|e| e.into_inner()).connections.remove(&id);
    });
    let mut d = device.lock().unwrap_or_else(|e| e.into_inner());
    d.connections.insert(id, vec![read_task.abort_handle(), write_task.abort_handle()]);
}
