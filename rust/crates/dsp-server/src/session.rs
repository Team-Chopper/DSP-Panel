//! DSP session.
//!
//! One instance per configured DSP. Owns the TCP connection, the request
//! scheduler and the authoritative device state. The protocol flow follows the
//! original editor (ORIGINAL-SOFTWARE.md) and is a port of the Dart server's
//! `dsp_session.dart`.
//!
//! Scheduling rules:
//! - Exactly one request is in flight at a time; every request has a timeout
//!   and is sent once more if it is not answered. A request that stays
//!   unanswered breaks the link (like the original editor); the session then
//!   reconnects with backoff.
//! - Init, preset load/save, linking and copying run as exclusive sequences.
//! - Value changes are coalesced per key (e.g. "gain:In A"): only the newest
//!   value that has not been sent yet goes to the device.
//! - Meter polling is just another request, sent only when idle, so its
//!   response can never be mistaken for the response to a command. While no
//!   client shows the DSP, the session sends the idle heartbeat (0x12)
//!   instead.
//!
//! Live safety:
//! - Gains are capped by the channel's max level (`DspConfig::gain_caps`):
//!   never above max(cap, current gain), unless a client forces the value.
//! - The DSP test generator (0x39) runs in a safe mode: the output gains are
//!   saved (safety.json) and every output is lowered before the generator
//!   starts; switching it off stops the generator first and then restores the
//!   gains. Meanwhile only mutes and the test controls are accepted. After a
//!   reconnect a DSP that still plays goes back into this mode, one that is
//!   back on the analog input gets its saved gains back.
//!
//! Like the Dart server, everything runs on one thread (a tokio `LocalSet`):
//! the session state lives in an `Rc<RefCell<…>>` and is never borrowed across
//! an `.await`.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use dsp_core::api::{DspInfo, DspStatus, LogEntry, TEST_LEVEL_MAX_DB, TEST_LEVEL_MIN_DB, TEST_START_LEVEL_DB};
use dsp_core::describe::{describe, describe_rx};
use dsp_core::diff;
use dsp_core::ops::{self, Op};
use dsp_core::protocol::{self as p, Frame, Message, Protocol, format_as_ascii, format_as_hex, with_device_id};
use dsp_core::{ConfigDump, DeviceModel, DeviceState, FrameDecoder};
use indexmap::IndexMap;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, spawn_local};

use crate::log;
use crate::registry::{DspConfig, is_valid_gain_cap};
use crate::safety::SafetyStore;

pub use dsp_core::api::TEST_RUNNING;

/// Something about the session changed.
#[derive(Clone, Debug)]
pub enum SessionEvent {
    /// Connection status, progress or settings changed.
    InfoChanged,
    /// The whole device state was replaced (after init or preset load).
    StateReplaced,
    /// A single op was applied; [origin] identifies the client that sent it.
    OpApplied { op: Op, origin: Option<u64> },
    MetersUpdated,
    LogAppended { entries: Vec<LogEntry>, reset: bool },
    /// Persistent settings changed (refresh interval, address, …).
    ConfigChanged,
    /// Result of a job for the clients (e.g. values a preset load missed).
    Notice { text: String, error: bool },
}

/// States kept for undoing preset loads.
pub const MAX_UNDO: usize = 5;

/// Timeouts and delays (the defaults are the original editor's).
#[derive(Clone, Debug)]
pub struct Timing {
    /// Answer timeout of a request (the original editor: 1 s on TCP); an
    /// unanswered request is sent once more.
    pub request: Duration,
    /// Answer timeout of the handshake and device info (4 s, 3 attempts).
    pub handshake: Duration,
    /// Answer timeouts of a preset store and recall.
    pub store: Duration,
    pub recall: Duration,
    /// Idle time after which the heartbeat (0x12) is sent.
    pub heartbeat_idle: Duration,
    pub sequence_gap: Duration,
    pub reconnect_delays: Vec<Duration>,
    pub meter_interval: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            request: Duration::from_secs(1),
            handshake: Duration::from_secs(4),
            store: Duration::from_secs(10),
            recall: Duration::from_secs(50),
            heartbeat_idle: Duration::from_secs(2),
            sequence_gap: Duration::from_millis(50),
            reconnect_delays: [1, 2, 5, 10, 30].map(Duration::from_secs).to_vec(),
            meter_interval: Duration::from_millis(25),
        }
    }
}

/// Why connecting or a sequence failed.
#[derive(Debug)]
pub enum SessionError {
    Connect(std::io::Error),
    Timeout,
    Failure(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Connect(e) => {
                // "Connection refused (os error 111)" → "Connection refused"
                let text = e.to_string();
                let text = text.split(" (os error").next().unwrap_or(&text).to_string();
                write!(f, "Connection failed: {text}")
            }
            SessionError::Timeout => f.write_str("Connection timed out"),
            SessionError::Failure(m) => f.write_str(m),
        }
    }
}

type SResult<T> = Result<T, SessionError>;

fn failure<T>(message: impl Into<String>) -> SResult<T> {
    Err(SessionError::Failure(message.into()))
}

/// Settings changed by `updateDsp`.
#[derive(Clone, Debug, Default)]
pub struct ConfigUpdate {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub device_id: Option<i64>,
    pub password: Option<String>,
}

#[derive(Clone, Copy)]
struct RequestOptions {
    keepalive: bool,
    timeout: Option<Duration>,
    retries: u32,
    /// Unanswered request: the link is broken (else the caller gets `None`).
    fatal: bool,
}

impl Default for RequestOptions {
    fn default() -> Self {
        RequestOptions { keepalive: false, timeout: None, retries: 1, fatal: true }
    }
}

struct InFlight {
    id: u64,
    attempt: u32,
    frame: Frame,
    keepalive: bool,
    timeout: Duration,
    retries_left: u32,
    fatal: bool,
    conn: u64,
    done: Option<oneshot::Sender<Option<Frame>>>,
}

enum WriterMessage {
    Frame(Frame),
    Flush(oneshot::Sender<()>),
}

struct Conn {
    id: u64,
    writer: mpsc::UnboundedSender<WriterMessage>,
    tasks: Vec<AbortHandle>,
}

/// Progress of a sequence of known length.
struct StepCounter {
    total: usize,
    done: usize,
    status: DspStatus,
}

pub const MAX_LOG_ENTRIES: usize = 500;

struct Inner {
    weak: Weak<RefCell<Inner>>,
    config: DspConfig,
    timing: Timing,
    listeners: Vec<mpsc::UnboundedSender<SessionEvent>>,

    state: DeviceState,
    status: DspStatus,
    progress: f64,
    message: Option<String>,
    /// Linear meter levels (1.0 = full scale) in channel order.
    meters: Vec<f64>,
    /// Compressor/limiter status of the outputs from the last meter frame.
    dynamics: u16,
    packets_sent: u64,
    packets_received: u64,
    log_rx: bool,
    log_tx: bool,
    /// False if the last config dump could not be decoded plausibly.
    config_readable: bool,

    running: bool,
    disposed: bool,
    generation: u64,
    conn: Option<Conn>,
    next_conn: u64,
    closed: Option<oneshot::Sender<()>>,
    wake_up: Option<oneshot::Sender<()>>,
    decoder: FrameDecoder,
    started: Option<Instant>,

    in_flight: Option<InFlight>,
    next_request: u64,
    exclusive: bool,
    idle_waiters: Vec<oneshot::Sender<()>>,
    /// Coalesced value changes waiting to be sent, in first-queued order.
    pending_ops: IndexMap<String, Vec<Frame>>,
    /// Frames of the op currently being sent.
    batch: VecDeque<Frame>,
    /// Config chunks collected while a dump is being read.
    chunks: Option<BTreeMap<u8, Vec<u8>>>,
    keepalive: Option<AbortHandle>,
    meter_timer_active: bool,
    log_flush_scheduled: bool,
    log: VecDeque<LogEntry>,
    log_pending: Vec<LogEntry>,
    /// Poll the meters (a client shows this DSP); otherwise only the
    /// heartbeat keeps the link alive.
    meters_wanted: bool,
    /// Last time a request was sent or answered.
    last_activity: Instant,
    /// Saved output gains of the test generator mode (and mute all, used by
    /// the hub).
    safety: SafetyStore,
    /// Level of all outputs while the DSP test generator runs; `None` while
    /// it is off.
    test_level: Option<f64>,
    /// Device states before preset loads (label, state), newest last.
    undo: Vec<(String, DeviceState)>,
}

/// Handle to a DSP session (cheap to clone; single-threaded).
#[derive(Clone)]
pub struct DspSession(Rc<RefCell<Inner>>);

fn configured_model(config: &DspConfig) -> Option<DeviceModel> {
    (config.model != "auto").then(|| DeviceModel::by_id(Some(&config.model)))
}

fn host_for_connect(host: &str) -> &str {
    host.trim_start_matches('[').trim_end_matches(']')
}

impl DspSession {
    /// A session whose saved test gains are not written to disk.
    pub fn new(config: DspConfig, timing: Timing) -> Self {
        Self::with_safety(config, timing, SafetyStore::memory())
    }

    pub fn with_safety(config: DspConfig, timing: Timing, safety: SafetyStore) -> Self {
        let model = configured_model(&config).unwrap_or(DeviceModel::DSP408);
        DspSession(Rc::new_cyclic(|weak| {
            RefCell::new(Inner {
                weak: weak.clone(),
                config,
                timing,
                listeners: Vec::new(),
                state: DeviceState::new(model),
                status: DspStatus::Disconnected,
                progress: 0.0,
                message: None,
                meters: vec![0.0; model.meter_count()],
                dynamics: 0,
                packets_sent: 0,
                packets_received: 0,
                log_rx: false,
                log_tx: false,
                config_readable: true,
                running: false,
                disposed: false,
                generation: 0,
                conn: None,
                next_conn: 0,
                closed: None,
                wake_up: None,
                decoder: FrameDecoder::new(),
                started: None,
                in_flight: None,
                next_request: 0,
                exclusive: false,
                idle_waiters: Vec::new(),
                pending_ops: IndexMap::new(),
                batch: VecDeque::new(),
                chunks: None,
                keepalive: None,
                meter_timer_active: false,
                log_flush_scheduled: false,
                log: VecDeque::new(),
                log_pending: Vec::new(),
                meters_wanted: true,
                last_activity: Instant::now(),
                safety,
                test_level: None,
                undo: Vec::new(),
            })
        }))
    }

    // ─── Public state ───

    /// Receive the session's events (several listeners are possible).
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<SessionEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.0.borrow_mut().listeners.push(tx);
        rx
    }

    pub fn id(&self) -> String {
        self.0.borrow().config.id.clone()
    }

    pub fn config(&self) -> DspConfig {
        self.0.borrow().config.clone()
    }

    pub fn status(&self) -> DspStatus {
        self.0.borrow().status
    }

    pub fn message(&self) -> Option<String> {
        self.0.borrow().message.clone()
    }

    pub fn model(&self) -> DeviceModel {
        self.0.borrow().state.model
    }

    pub fn config_readable(&self) -> bool {
        self.0.borrow().config_readable
    }

    /// A copy of the device state.
    pub fn state(&self) -> DeviceState {
        self.0.borrow().state.clone()
    }

    pub fn state_json(&self) -> Value {
        self.0.borrow().state.to_json()
    }

    pub fn meters(&self) -> Vec<f64> {
        self.0.borrow().meters.clone()
    }

    pub fn dynamics(&self) -> u16 {
        self.0.borrow().dynamics
    }

    pub fn packets(&self) -> (u64, u64) {
        let i = self.0.borrow();
        (i.packets_sent, i.packets_received)
    }

    pub fn log_entries(&self) -> Vec<LogEntry> {
        self.0.borrow().log.iter().cloned().collect()
    }

    pub fn info(&self) -> DspInfo {
        let i = self.0.borrow();
        DspInfo {
            id: i.config.id.clone(),
            name: i.config.name.clone(),
            host: i.config.host.clone(),
            port: i.config.port,
            enabled: i.config.enabled,
            status: i.status,
            progress: i.progress,
            message: i.message.clone(),
            refresh_ms: i.config.refresh_ms,
            log_rx: i.log_rx,
            log_tx: i.log_tx,
            model: i.state.model.id.into(),
            model_setting: i.config.model.clone(),
            config_readable: i.config_readable,
            device_id: i.config.device_id,
            has_password: !i.config.password.is_empty(),
            gain_caps: i.config.gain_caps.clone(),
            test_level: i.test_level,
            undo: i.undo.iter().map(|(label, _)| label.clone()).collect(),
        }
    }

    /// Level of all outputs while the DSP test generator runs.
    pub fn test_level(&self) -> Option<f64> {
        self.0.borrow().test_level
    }

    // ─── Lifecycle ───

    /// Start connecting (if enabled). Reconnects automatically until [stop].
    pub fn start(&self) {
        let generation = {
            let mut i = self.0.borrow_mut();
            if i.running || i.disposed || !i.config.enabled {
                return;
            }
            i.running = true;
            i.generation += 1;
            i.generation
        };
        let session = self.clone();
        spawn_local(async move { session.connect_loop(generation).await });
    }

    /// Disconnect (with 0x11, like the original editor) and stop reconnecting.
    pub async fn stop(&self) {
        let was_running = {
            let mut i = self.0.borrow_mut();
            let was = i.running;
            i.running = false;
            i.generation += 1;
            if let Some(w) = i.wake_up.take() {
                let _ = w.send(());
            }
            was
        };
        self.close_socket(was_running.then_some("Disconnected"), true).await;
        self.0.borrow_mut().set_status(DspStatus::Disconnected, Some(0.0), Some(None));
    }

    /// Apply new settings; reconnects if the address, model, device ID,
    /// password or enabled flag changed.
    pub async fn update_config(&self, update: ConfigUpdate) {
        let reconnect = {
            let mut i = self.0.borrow_mut();
            let c = &i.config;
            let password = update.password.as_ref().map(|p| p.to_uppercase());
            let reconnect = update.host.as_ref().is_some_and(|h| *h != c.host)
                || update.port.is_some_and(|p| p != c.port)
                || update.enabled.is_some_and(|e| e != c.enabled)
                || update.model.as_ref().is_some_and(|m| *m != c.model)
                || update.device_id.is_some_and(|d| d != c.device_id)
                || password.as_ref().is_some_and(|p| *p != c.password);
            let c = &mut i.config;
            if let Some(v) = update.name {
                c.name = v;
            }
            if let Some(v) = update.host {
                c.host = v;
            }
            if let Some(v) = update.port {
                c.port = v;
            }
            if let Some(v) = update.enabled {
                c.enabled = v;
            }
            if let Some(v) = update.model {
                c.model = v;
            }
            if let Some(v) = update.device_id {
                c.device_id = v;
            }
            if let Some(v) = password {
                c.password = v;
            }
            i.emit(SessionEvent::ConfigChanged);
            reconnect
        };
        if reconnect {
            self.stop().await;
            self.start();
        }
        self.0.borrow().emit(SessionEvent::InfoChanged);
    }

    pub async fn dispose(&self) {
        self.stop().await;
        let mut i = self.0.borrow_mut();
        i.disposed = true;
        i.listeners.clear();
    }

    fn active(&self, generation: u64) -> bool {
        let i = self.0.borrow();
        i.running && i.generation == generation
    }

    async fn connect_loop(&self, generation: u64) {
        let mut attempt = 0usize;
        while self.active(generation) {
            let result = match self.open(generation).await {
                Err(e) => Err(e),
                Ok(closed) => match self.run_init().await {
                    Err(e) => Err(e),
                    Ok(()) => {
                        attempt = 0;
                        {
                            let mut i = self.0.borrow_mut();
                            i.set_status(DspStatus::Ready, Some(1.0), Some(None));
                            i.start_keepalive();
                            i.pump();
                        }
                        self.resume_test_mode();
                        let _ = closed.await;
                        if self.active(generation) {
                            let mut i = self.0.borrow_mut();
                            let text = format!("Connection lost: {}", i.message.clone().unwrap_or_default());
                            i.add_status_log(text);
                        }
                        Ok(())
                    }
                },
            };
            if let Err(e) = result
                && self.active(generation) {
                    let mut i = self.0.borrow_mut();
                    let message = e.to_string();
                    i.message = Some(message.clone());
                    i.add_status_log(format!("Error: {message}"));
                    log::warn(format!("[{}] {message}", i.config.name));
                }
            if !self.active(generation) {
                break;
            }
            self.close_socket(None, false).await;

            let (delay, wake_rx) = {
                let mut i = self.0.borrow_mut();
                let delays = &i.timing.reconnect_delays;
                let delay = delays[attempt.min(delays.len() - 1)];
                let text = format!(
                    "{} – retrying in {} s",
                    i.message.clone().unwrap_or_else(|| "Disconnected".into()),
                    delay.as_secs()
                );
                i.set_status(DspStatus::Error, None, Some(Some(text)));
                let (tx, rx) = oneshot::channel();
                i.wake_up = Some(tx);
                (delay, rx)
            };
            attempt += 1;
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = wake_rx => {}
            }
        }
    }

    async fn open(&self, generation: u64) -> SResult<oneshot::Receiver<()>> {
        let (host, port) = {
            let mut i = self.0.borrow_mut();
            i.set_status(DspStatus::Connecting, Some(0.0), Some(None));
            i.started = Some(Instant::now());
            let text = format!("Connecting to {}:{} …", i.config.host, i.config.port);
            i.add_status_log(text);
            i.packets_sent = 0;
            i.packets_received = 0;
            i.decoder.reset();
            (i.config.host.clone(), i.config.port)
        };
        let port = u16::try_from(port).map_err(|_| SessionError::Failure("Invalid port".into()))?;
        let connect = TcpStream::connect((host_for_connect(&host), port));
        let stream = tokio::time::timeout(Duration::from_secs(5), connect)
            .await
            .map_err(|_| SessionError::Timeout)?
            .map_err(SessionError::Connect)?;
        if !self.active(generation) {
            return failure("Cancelled");
        }
        let _ = stream.set_nodelay(true);
        let (reader, writer) = stream.into_split();

        let mut i = self.0.borrow_mut();
        i.next_conn += 1;
        let id = i.next_conn;
        let (tx, rx) = mpsc::unbounded_channel();
        let writer_task = spawn_local(writer_loop(writer, rx, i.weak.clone(), id));
        let reader_task = spawn_local(reader_loop(reader, i.weak.clone(), id));
        i.conn = Some(Conn { id, writer: tx, tasks: vec![writer_task.abort_handle(), reader_task.abort_handle()] });
        let (closed_tx, closed_rx) = oneshot::channel();
        i.closed = Some(closed_tx);
        i.add_status_log("Connected – initializing");
        Ok(closed_rx)
    }

    /// Close the connection. [say_goodbye]: send the disconnect command (0x11)
    /// first, like the original editor when it ends a session.
    async fn close_socket(&self, reason: Option<&str>, say_goodbye: bool) {
        let conn = {
            let mut i = self.0.borrow_mut();
            i.stop_keepalive();
            i.conn.take()
        };
        if let Some(conn) = conn {
            if say_goodbye {
                self.send_disconnect(&conn).await;
            }
            for task in conn.tasks {
                task.abort();
            }
        }
        let mut i = self.0.borrow_mut();
        // Fail whatever is in flight and drop queued work.
        if let Some(mut request) = i.in_flight.take()
            && let Some(done) = request.done.take() {
                let _ = done.send(None);
            }
        i.pending_ops.clear();
        i.batch.clear();
        i.chunks = None;
        i.notify_idle();
        if let Some(closed) = i.closed.take() {
            let _ = closed.send(());
        }
        if let Some(reason) = reason {
            i.add_status_log(reason);
        }
    }

    /// Send 0x11 (no answer expected) and wait up to 500 ms for it to leave.
    async fn send_disconnect(&self, conn: &Conn) {
        let (flushed_tx, flushed_rx) = oneshot::channel();
        {
            let mut i = self.0.borrow_mut();
            let frame = with_device_id(&p::build_disconnect_command(), i.config.device_id as u8);
            let _ = conn.writer.send(WriterMessage::Frame(frame));
            let _ = conn.writer.send(WriterMessage::Flush(flushed_tx));
            i.packets_sent += 1;
        }
        let _ = tokio::time::timeout(Duration::from_millis(500), flushed_rx).await;
    }

    // ─── Init ───

    fn conn_id(&self) -> Option<u64> {
        self.0.borrow().conn.as_ref().map(|c| c.id)
    }

    fn still_connected(&self, conn: Option<u64>) -> bool {
        conn.is_some() && self.conn_id() == conn
    }

    fn lost_message(&self, fallback: &str) -> String {
        self.0.borrow().message.clone().unwrap_or_else(|| fallback.into())
    }

    fn step(&self, steps: &mut StepCounter) {
        steps.done += 1;
        let progress = (steps.done as f64 / steps.total.max(1) as f64).clamp(0.0, 1.0);
        self.0.borrow_mut().set_status(steps.status, Some(progress), None);
    }

    async fn gap(&self) {
        let gap = self.0.borrow().timing.sequence_gap;
        if !gap.is_zero() {
            tokio::time::sleep(gap).await;
        }
    }

    /// Like the original editor: handshake, device info (model and version
    /// check), lock/PTS status (with the stored password if the DSP is
    /// locked), then everything [read_all] reads. The state is only published
    /// once everything has been read.
    async fn run_init(&self) -> SResult<()> {
        self.0.borrow_mut().set_status(DspStatus::Initializing, Some(0.0), None);
        let mut steps = StepCounter { total: 3 + self.read_all_steps(), done: 0, status: DspStatus::Initializing };
        let conn = self.conn_id();
        self.0.borrow_mut().exclusive = true;
        let result = async {
            self.handshake(conn, p::handshake_command()).await?;
            self.step(&mut steps);

            let info = self.handshake(conn, p::request_device_info()).await?;
            let device_id = self.0.borrow().config.device_id as u8;
            let device_name = match Protocol::with_id(DeviceModel::DSP408, device_id).decode(&info) {
                Some(Message::DeviceInfo { name }) => name,
                _ => String::new(),
            };
            self.step(&mut steps);
            let model = self.check_device(conn, &device_name).await?;
            {
                let mut i = self.0.borrow_mut();
                i.add_status_log(format!("Device: {device_name} ({})", model.name));
                // Collect slots and preset names into a fresh state of the
                // right model; it is published after the config dump.
                i.state = DeviceState::new(model);
                i.state.device_name = device_name;
                i.meters = vec![0.0; model.meter_count()];
                i.dynamics = 0;
            }
            self.check_lock(conn).await?;
            self.step(&mut steps);
            self.read_all(conn, &mut steps).await
        }
        .await;
        {
            let mut i = self.0.borrow_mut();
            i.exclusive = false;
            i.chunks = None;
        }
        result?;
        self.0.borrow().emit(SessionEvent::StateReplaced);
        Ok(())
    }

    /// The model of the device; like the original editor, a device that does
    /// not report itself as a DSP 408/206 with protocol version V010 (or not as
    /// the configured model) is refused.
    async fn check_device(&self, conn: Option<u64>, device_name: &str) -> SResult<DeviceModel> {
        let detected = DeviceModel::detect(device_name);
        let configured = configured_model(&self.0.borrow().config);
        let wrong = match detected {
            None => Some(format!("Wrong device: the DSP reports \"{device_name}\"")),
            Some(d) if configured.is_some_and(|c| c != d) => Some(format!(
                "Wrong device: the DSP reports \"{device_name}\", configured as {}",
                configured.map_or("", |c| c.name)
            )),
            Some(d) if !d.supports_version(device_name) => Some(format!("Wrong device version: \"{device_name}\"")),
            _ => None,
        };
        if let Some(wrong) = wrong {
            if self.still_connected(conn) {
                self.disconnect_now().await;
            }
            return failure(wrong);
        }
        Ok(detected.unwrap_or_default())
    }

    /// Send 0x11 on the current connection.
    async fn disconnect_now(&self) {
        let writer = self.0.borrow().conn.as_ref().map(|c| c.writer.clone());
        if let Some(writer) = writer {
            let conn = Conn { id: 0, writer, tasks: Vec::new() };
            self.send_disconnect(&conn).await;
        }
    }

    /// Ask for the lock/PTS status (0x2C). A locked DSP, or one whose PTS
    /// limit is reached, needs its password (0x2D); without a stored password,
    /// or with a wrong one, the session ends like the original editor's Cancel
    /// (0x11).
    async fn check_lock(&self, conn: Option<u64>) -> SResult<()> {
        let answer = self.optional(conn, p::request_lock_status()).await?;
        let proto = self.0.borrow().proto();
        let lock = answer.and_then(|a| proto.decode(&a));
        let Some(lock @ Message::LockStatus { locked, .. }) = lock else { return Ok(()) };
        let expired = lock.pts_expired();
        if !expired && !locked {
            return Ok(());
        }
        let mode = if expired { 1 } else { 0 };
        let what = if mode == 1 { "The PTS limit of the DSP is reached" } else { "The DSP is locked" };
        self.0.borrow_mut().add_status_log(format!("{what} – checking the password"));

        let password = self.0.borrow().config.password.clone();
        if password.is_empty() {
            return self.refuse(conn, what, "enter its password in the DSP settings").await;
        }
        let options = RequestOptions { timeout: Some(Duration::from_millis(3100)), retries: 0, fatal: false, ..Default::default() };
        let check = self.request(p::build_password_command(mode, &password), options).await;
        match check.and_then(|c| proto.decode(&c)) {
            Some(Message::PasswordResult { ok: true, .. }) => {}
            Some(Message::PasswordResult { ok: false, .. }) => return self.refuse(conn, what, "wrong password").await,
            _ => return self.refuse(conn, what, "the password check was not answered").await,
        }
        self.0.borrow_mut().add_status_log("Password accepted");
        self.gap().await;
        Ok(())
    }

    async fn refuse(&self, conn: Option<u64>, what: &str, reason: &str) -> SResult<()> {
        if self.still_connected(conn) {
            self.disconnect_now().await;
        }
        failure(format!("{what} – {reason}"))
    }

    /// Requests of [read_all].
    fn read_all_steps(&self) -> usize {
        2 + p::PRESET_COUNT + ConfigDump::new(self.model()).chunk_count() + 2
    }

    /// Preset slot table (0x22), current preset (0x14), preset names (0x29),
    /// the config dump (0x27) and the heartbeats (0x12) – what the original
    /// editor reads when it connects and after linking or copying.
    async fn read_all(&self, conn: Option<u64>, steps: &mut StepCounter) -> SResult<()> {
        self.optional(conn, p::request_preset_slots()).await?;
        self.step(steps);
        self.optional(conn, p::request_current_preset()).await?;
        self.step(steps);
        for index in 0..p::PRESET_COUNT as u8 {
            self.require(conn, p::request_preset_name(index), None).await?;
            self.step(steps);
        }
        let stream = self.read_config(conn, steps).await?;
        self.apply_dump(&stream);
        for _ in 0..2 {
            self.optional(conn, p::build_heartbeat_command()).await?;
            self.step(steps);
        }
        Ok(())
    }

    /// Read the config dump: as many chunks as the model's layout needs. A
    /// chunk the device does not answer ends the dump.
    async fn read_config(&self, conn: Option<u64>, steps: &mut StepCounter) -> SResult<Vec<u8>> {
        let count = ConfigDump::new(self.model()).chunk_count();
        self.0.borrow_mut().chunks = Some(BTreeMap::new());
        for index in 0..count {
            let options = RequestOptions { fatal: false, ..Default::default() };
            let response = self.request(p::config_dump_command(index as u8), options).await;
            self.step(steps);
            if response.is_none() {
                if !self.still_connected(conn) {
                    return failure(self.lost_message("Connection lost"));
                }
                self.0.borrow_mut().add_status_log(format!("Config chunk {index} not answered – dump ends here"));
                break;
            }
            self.gap().await;
        }
        let chunks = self.0.borrow_mut().chunks.take().unwrap_or_default();
        let mut stream = Vec::new();
        for i in 0..=u8::MAX {
            match chunks.get(&i) {
                Some(chunk) => stream.extend_from_slice(chunk),
                None => break,
            }
        }
        Ok(stream)
    }

    /// Decode [stream] into the state. Keeps the device name and what was read
    /// before the dump (preset names, slots, current preset).
    fn apply_dump(&self, stream: &[u8]) {
        let mut i = self.0.borrow_mut();
        let model = i.state.model;
        let result = ConfigDump::new(model).decode(stream);
        i.config_readable = result.is_plausible();
        let mut next = if i.config_readable { result.state.clone() } else { DeviceState::new(model) };
        next.presets.extend(i.state.presets.clone());
        next.preset_slots.extend(i.state.preset_slots.iter().copied());
        next.current_preset_number = i.state.current_preset_number;
        next.device_name = i.state.device_name.clone();
        i.state = next;
        if i.config_readable {
            let text = format!("Configuration read – active preset: {}", i.state.current_preset);
            i.add_status_log(text);
            if !result.switches_read {
                i.add_status_log("The dump ends before the mute and PEQ bypass bits – shown as off");
            }
        } else {
            let first3: Vec<&str> = result.problems.iter().take(3).map(String::as_str).collect();
            i.add_status_log(format!(
                "Configuration could not be decoded ({} implausible values, e.g. {}) – showing defaults",
                result.problems.len(),
                first3.join("; ")
            ));
            let first5: Vec<&str> = result.problems.iter().take(5).map(String::as_str).collect();
            log::warn(format!("[{}] config dump not recognised: {}", i.config.name, first5.join("; ")));
        }
        i.emit(SessionEvent::InfoChanged);
    }

    /// Handshake request: 4 s timeout, 3 attempts, else the init fails.
    async fn handshake(&self, conn: Option<u64>, frame: Frame) -> SResult<Frame> {
        if !self.still_connected(conn) {
            return failure(self.lost_message("Connection lost"));
        }
        let timeout = self.0.borrow().timing.handshake;
        let options = RequestOptions { timeout: Some(timeout), retries: 2, fatal: false, ..Default::default() };
        if let Some(response) = self.request(frame, options).await {
            self.gap().await;
            return Ok(response);
        }
        if !self.still_connected(conn) {
            return failure(self.lost_message("Connection lost during initialization"));
        }
        failure("Initialization failed: DSP is not responding")
    }

    /// Request that must be answered, else the link is broken.
    async fn require(&self, conn: Option<u64>, frame: Frame, timeout: Option<Duration>) -> SResult<Frame> {
        if !self.still_connected(conn) {
            return failure(self.lost_message("Connection lost"));
        }
        if let Some(response) = self.request(frame, RequestOptions { timeout, ..Default::default() }).await {
            self.gap().await;
            return Ok(response);
        }
        failure(self.lost_message("DSP is not responding"))
    }

    /// Request whose answer is optional (a device that does not answer it is
    /// tolerated). Returns the answer or `None`.
    async fn optional(&self, conn: Option<u64>, frame: Frame) -> SResult<Option<Frame>> {
        if !self.still_connected(conn) {
            return failure(self.lost_message("Connection lost"));
        }
        let response = self.request(frame.clone(), RequestOptions { fatal: false, ..Default::default() }).await;
        if response.is_none() {
            if !self.still_connected(conn) {
                return failure(self.lost_message("Connection lost"));
            }
            let (name, model) = {
                let i = self.0.borrow();
                (i.config.name.clone(), i.state.model)
            };
            log::debug(format!("[{name}] no answer to optional {}", describe(&frame, &model)));
        }
        self.gap().await;
        Ok(response)
    }

    /// Send [frames] one by one, each after the previous answer. Returns false
    /// if the device stopped answering.
    async fn run_sequence(&self, frames: Vec<Frame>, timeout_for: impl Fn(&Frame) -> Option<Duration>) -> bool {
        let conn = self.conn_id();
        let total = frames.len();
        for (index, frame) in frames.into_iter().enumerate() {
            if !self.still_connected(conn) {
                return false;
            }
            let timeout = timeout_for(&frame);
            if self.request(frame, RequestOptions { timeout, ..Default::default() }).await.is_none() {
                return false;
            }
            let progress = (index + 1) as f64 / total as f64;
            self.0.borrow_mut().set_status(DspStatus::Busy, Some(progress), None);
            self.gap().await;
        }
        self.still_connected(conn)
    }

    /// Send [frame] and wait for the answer (sent right away, even if the
    /// returned future is not awaited).
    fn request(&self, frame: Frame, options: RequestOptions) -> impl Future<Output = Option<Frame>> + use<> {
        let rx = self.0.borrow_mut().request(frame, options);
        async move { rx.await.ok().flatten() }
    }

    // ─── Meters & heartbeat ───

    /// Poll the meters only while a client shows this DSP; otherwise the idle
    /// heartbeat (0x12) keeps the link alive, like the original editor.
    pub fn set_meters_wanted(&self, wanted: bool) {
        self.0.borrow_mut().meters_wanted = wanted;
    }

    /// Change the meter polling interval (5–300 ms).
    pub fn set_refresh_interval(&self, ms: i64) {
        let mut i = self.0.borrow_mut();
        i.config.refresh_ms = ms.clamp(5, 300);
        if i.keepalive.is_some() {
            i.start_keepalive();
        }
        i.emit(SessionEvent::ConfigChanged);
        i.emit(SessionEvent::InfoChanged);
    }

    // ─── Operations ───

    /// Validate and apply an op from a client. Returns an error message.
    pub fn apply_op(&self, op: &Value, origin: Option<u64>) -> Option<String> {
        self.apply_op_with(op, origin, false)
    }

    /// Like [apply_op]; [force] sets a gain above the channel's max level
    /// (the client asked "Set anyway").
    pub fn apply_op_with(&self, op: &Value, origin: Option<u64>, force: bool) -> Option<String> {
        let op = {
            let i = self.0.borrow();
            if let Some(error) = ops::validate(op, &i.state.model) {
                return Some(error);
            }
            if let Some(error) = i.not_ready() {
                return Some(error);
            }
            match Op::parse(op) {
                Ok(op) => op,
                Err(e) => return Some(e),
            }
        };
        let running = self.0.borrow().test_level.is_some();
        match op {
            Op::TestTone { tone_type, param } if tone_type != 0 && !running => return self.start_test(tone_type, param),
            Op::TestTone { tone_type: 0, param } if running => return self.stop_test(param),
            _ => {}
        }
        let mut i = self.0.borrow_mut();
        if running && !ops::allowed_during_test(&op) {
            return Some(TEST_RUNNING.into());
        }
        let (op, origin) = match op {
            Op::Gain { ch, db } if !force => {
                let capped = i.capped_gain(&ch, db);
                // A capped value goes back to the sender as well.
                let origin = if capped < db { None } else { origin };
                (Op::Gain { ch, db: capped }, origin)
            }
            other => (other, origin),
        };
        i.queue_op(&op, origin, None)
    }

    // ─── Live safety ───

    /// Change the max level of [ch] and of the channels linked with it
    /// (`None` removes it). The current gains are not touched.
    pub fn set_gain_cap(&self, ch: &str, db: Option<f64>) -> Option<String> {
        let mut i = self.0.borrow_mut();
        if !i.state.model.has_channel(ch) {
            return Some("Unknown channel".into());
        }
        if db.is_some_and(|d| !is_valid_gain_cap(d)) {
            return Some("The max level must be between -60 and +12 dB".into());
        }
        for c in ops::link_group_of(&i.state, ch) {
            match db {
                Some(db) => i.config.gain_caps.insert(c, p::quantize_gain(db)),
                None => i.config.gain_caps.remove(&c),
            };
        }
        let text = match db {
            Some(db) => format!("Max level of {ch}: {} dB", p::quantize_gain(db)),
            None => format!("Max level of {ch} removed"),
        };
        i.add_status_log(text);
        i.emit(SessionEvent::ConfigChanged);
        i.emit(SessionEvent::InfoChanged);
        None
    }

    /// Mute every open channel (mute all), ahead of other queued changes.
    /// Returns the channels that were open, or `None` if the DSP is not ready.
    pub fn mute_open_channels(&self) -> Option<Vec<String>> {
        let mut i = self.0.borrow_mut();
        if i.not_ready().is_some() {
            return None;
        }
        let model = i.state.model;
        let is_open = |s: &DeviceState, c: &str| !s.mutes.get(c).copied().unwrap_or(false);
        let open: Vec<String> = model.channels().filter(|c| is_open(&i.state, c)).map(String::from).collect();
        let mut position = 0;
        for ch in &open {
            // Linked channels may have been muted with their group already.
            if is_open(&i.state, ch) {
                i.queue_op(&Op::mute(ch, true), None, Some(position));
                position += 1;
            }
        }
        if !open.is_empty() {
            i.add_status_log(format!("Mute all: {} channel(s) muted", open.len()));
        }
        Some(open)
    }

    /// Unmute [channels] (mute all released). False if the DSP is not ready.
    pub fn unmute_channels(&self, channels: &[String]) -> bool {
        let mut i = self.0.borrow_mut();
        if i.not_ready().is_some() {
            return false;
        }
        let model = i.state.model;
        for ch in channels.iter().filter(|c| model.has_channel(c)) {
            if i.state.mutes.get(ch).copied().unwrap_or(false) {
                i.queue_op(&Op::mute(ch, false), None, None);
            }
        }
        i.add_status_log(format!("Mute all released: {} channel(s) unmuted", channels.len()));
        true
    }

    /// Set every output to [db] (within the max levels) while the DSP test
    /// generator runs.
    pub fn set_test_level(&self, db: f64) -> Option<String> {
        let mut i = self.0.borrow_mut();
        if i.test_level.is_none() {
            return Some("The DSP test generator is not running".into());
        }
        if let Some(error) = i.not_ready() {
            return Some(error);
        }
        if !db.is_finite() {
            return Some("db must be a number".into());
        }
        let level = p::quantize_gain(db.clamp(TEST_LEVEL_MIN_DB, TEST_LEVEL_MAX_DB));
        i.test_level = Some(level);
        for out in i.state.model.outputs {
            let op = Op::gain(out, i.capped_gain(out, level));
            i.queue_op(&op, None, None);
        }
        i.emit(SessionEvent::InfoChanged);
        None
    }

    /// Switch the generator on: save the output gains (to disk first), lower
    /// every output, then start it.
    fn start_test(&self, tone_type: i64, param: i64) -> Option<String> {
        let mut i = self.0.borrow_mut();
        let id = i.config.id.clone();
        // Gains saved by a run that did not end cleanly are the real ones.
        if i.safety.test_gains(&id).is_none() {
            let gains = i.output_gains();
            if let Err(e) = i.safety.set_test_gains(&id, Some(gains)) {
                return Some(format!("The output gains could not be saved – generator not started ({e})"));
            }
        }
        i.test_level = Some(TEST_START_LEVEL_DB);
        i.emit(SessionEvent::InfoChanged);
        drop(i);
        self.test_job("Starting the test generator …".into(), Some((tone_type, param)));
        None
    }

    /// Lower every output to the test level (on a reconnect: the generator is
    /// still running), then start the generator if [start] is given.
    fn test_job(&self, label: String, start: Option<(i64, i64)>) {
        let s = self.clone();
        let body = async move {
            let conn = s.conn_id();
            let outputs = s.model().outputs;
            let mut steps = StepCounter { total: outputs.len() + 1, done: 0, status: DspStatus::Busy };
            for out in outputs {
                let frames = {
                    let mut i = s.0.borrow_mut();
                    let level = i.test_level.unwrap_or(TEST_START_LEVEL_DB);
                    let op = Op::gain(out, i.capped_gain(out, level));
                    i.apply_now(&op)?
                };
                for frame in frames {
                    s.require(conn, frame, None).await?;
                }
                s.step(&mut steps);
            }
            if let Some((tone_type, param)) = start {
                let frames = s.0.borrow_mut().apply_now(&Op::test_tone(tone_type, param))?;
                for frame in frames {
                    s.require(conn, frame, None).await?;
                }
            }
            s.step(&mut steps);
            let mut i = s.0.borrow_mut();
            let text = format!("Test generator running – outputs at {} dB", i.test_level.unwrap_or(TEST_START_LEVEL_DB));
            i.add_status_log(text);
            Ok(())
        };
        self.spawn_exclusive_job(label, true, body);
    }

    /// Switch the generator off first, then restore the saved output gains.
    fn stop_test(&self, param: i64) -> Option<String> {
        self.restore_job("Stopping the test generator …".into(), Some(param));
        None
    }

    /// Restore the saved output gains (after sending "analog input" if
    /// [stop_param] is given) and leave the test mode.
    fn restore_job(&self, label: String, stop_param: Option<i64>) {
        let s = self.clone();
        let body = async move {
            let conn = s.conn_id();
            let (id, saved) = {
                let i = s.0.borrow();
                (i.config.id.clone(), i.safety.test_gains(&i.config.id).unwrap_or_default())
            };
            let mut steps = StepCounter { total: saved.len() + 1, done: 0, status: DspStatus::Busy };
            if let Some(param) = stop_param {
                let frames = s.0.borrow_mut().apply_now(&Op::test_tone(0, param))?;
                for frame in frames {
                    s.require(conn, frame, None).await?;
                }
            }
            s.step(&mut steps);
            let model = s.model();
            for (ch, db) in saved.iter().filter(|(ch, _)| model.is_output(ch)) {
                let frames = s.0.borrow_mut().apply_now(&Op::gain(ch, *db))?;
                for frame in frames {
                    s.require(conn, frame, None).await?;
                }
                s.step(&mut steps);
            }
            let mut i = s.0.borrow_mut();
            if let Err(e) = i.safety.set_test_gains(&id, None) {
                i.add_status_log(format!("Error: {e}"));
            }
            i.test_level = None;
            i.add_status_log("Test generator off – output gains restored");
            i.emit(SessionEvent::InfoChanged);
            Ok(())
        };
        self.spawn_exclusive_job(label, true, body);
    }

    /// After (re)connecting: a DSP whose generator still runs goes back into
    /// the test mode; one that is back on the analog input gets its saved
    /// output gains back.
    fn resume_test_mode(&self) {
        let mut i = self.0.borrow_mut();
        let id = i.config.id.clone();
        let saved = i.safety.test_gains(&id).is_some();
        if i.state.test_tone_type != 0 {
            if !saved {
                // Started by other software: save what is there now.
                let gains = i.output_gains();
                if let Err(e) = i.safety.set_test_gains(&id, Some(gains)) {
                    i.add_status_log(format!("Error: {e}"));
                }
            }
            if i.test_level.is_none() {
                i.test_level = Some(TEST_START_LEVEL_DB);
            }
            i.emit(SessionEvent::InfoChanged);
            drop(i);
            self.test_job("Test generator running – lowering the outputs …".into(), None);
        } else if saved {
            drop(i);
            self.restore_job("Restoring the output gains …".into(), None);
        } else if i.test_level.take().is_some() {
            i.emit(SessionEvent::InfoChanged);
        }
    }

    /// Link [slaves] to [master] like the original editor's link dialog: every
    /// newly linked slave gets the master's settings on the device (0x2A), the
    /// master gets its new mask (0x3B), then everything is read again. An empty
    /// list unlinks [master].
    pub fn link_channels(&self, master: &str, slaves: &[String]) -> Option<String> {
        let (frames, label, done) = {
            let i = self.0.borrow();
            if i.status != DspStatus::Ready {
                return Some("DSP is not ready".into());
            }
            if i.test_level.is_some() {
                return Some(TEST_RUNNING.into());
            }
            let change = match ops::link_change(&i.state, master, slaves) {
                Ok(change) => change,
                Err(e) => return Some(e),
            };
            let proto = i.proto();
            let mut frames = Vec::new();
            for s in &change.added {
                match proto.build_copy_channel_command(master, s) {
                    Ok(f) => frames.push(f),
                    Err(e) => return Some(e.to_string()),
                }
            }
            match proto.build_link_command(master, change.mask) {
                Ok(f) => frames.push(f),
                Err(e) => return Some(e.to_string()),
            }
            // Masks of older panel versions that still list other channels.
            for s in &change.reset {
                if let Ok(f) = proto.build_link_command(s, i.state.model.link_bit(s) as i64) {
                    frames.push(f);
                }
            }
            let members: Vec<&str> = std::iter::once(master).chain(slaves.iter().map(String::as_str)).collect();
            let label = if slaves.is_empty() {
                format!("Unlinking {master} …")
            } else {
                format!("Linking {} …", members.join(" + "))
            };
            let done = format!("Link {}: {}", if slaves.is_empty() { "released" } else { "set" }, members.join(" + "));
            (frames, label, done)
        };
        self.device_job(label, frames, done);
        None
    }

    /// Copy [from] to every channel of [targets] on the device (0x2A), then
    /// read everything again, like the original editor.
    pub fn copy_channels(&self, from: &str, targets: &[String]) -> Option<String> {
        let frames = {
            let i = self.0.borrow();
            if i.status != DspStatus::Ready {
                return Some("DSP is not ready".into());
            }
            if i.test_level.is_some() {
                return Some(TEST_RUNNING.into());
            }
            if targets.is_empty() {
                return Some("No channel to copy to".into());
            }
            if targets.iter().any(|t| t == from) {
                return Some(format!("{from} cannot be copied to itself"));
            }
            let proto = i.proto();
            let mut frames = Vec::new();
            for to in targets {
                match proto.build_copy_channel_command(from, to) {
                    Ok(f) => frames.push(f),
                    Err(e) => return Some(e.to_string()),
                }
            }
            frames
        };
        self.device_job(format!("Copying {from} …"), frames, format!("Copied {from} to {}", targets.join(", ")));
        None
    }

    /// Send [frames] exclusively (after the queued value changes), then read
    /// everything again.
    fn device_job(&self, label: String, frames: Vec<Frame>, done_log: String) {
        let body_session = self.clone();
        let body = async move {
            let s = body_session;
            let conn = s.conn_id();
            let mut steps = StepCounter { total: frames.len() + s.read_all_steps(), done: 0, status: DspStatus::Busy };
            for frame in frames {
                s.require(conn, frame, None).await?;
                s.step(&mut steps);
            }
            s.read_all(conn, &mut steps).await?;
            let mut i = s.0.borrow_mut();
            i.emit(SessionEvent::StateReplaced);
            i.add_status_log(done_log);
            Ok(())
        };
        self.spawn_exclusive_job(label, true, body);
    }

    /// Bring the device to [target] (loading a preset stored on the server)
    /// like the original editor loads a preset file: the name (0x26), the
    /// values as single commands, the heartbeat. Only the values that differ
    /// are sent, unless the link masks differ (then all of them, in the
    /// editor's order). The values before are kept for [undo_load]; the
    /// configuration is read back afterwards and compared with [target].
    pub fn apply_full_state(&self, target: &DeviceState, label: &str) -> Option<String> {
        self.load_state(target, label, true)
    }

    /// Back to the values before the last preset load.
    pub fn undo_load(&self) -> Option<String> {
        let (label, state) = {
            let mut i = self.0.borrow_mut();
            if i.status != DspStatus::Ready {
                return Some("DSP is not ready".into());
            }
            match i.undo.pop() {
                Some(entry) => entry,
                None => return Some("Nothing to undo".into()),
            }
        };
        let error = self.load_state(&state, &label, false);
        if error.is_some() {
            self.0.borrow_mut().undo.push((label, state));
        } else {
            self.0.borrow_mut().emit(SessionEvent::InfoChanged);
        }
        error
    }

    fn load_state(&self, target: &DeviceState, label: &str, keep_undo: bool) -> Option<String> {
        let (list, all) = {
            let mut i = self.0.borrow_mut();
            if i.status != DspStatus::Ready {
                return Some("DSP is not ready".into());
            }
            if i.test_level.is_some() {
                return Some(TEST_RUNNING.into());
            }
            if target.model != i.state.model {
                return Some(format!(
                    "This preset is for a {}, the DSP is a {}",
                    target.model.name, i.state.model.name
                ));
            }
            // The preset's gains stay within the max levels too.
            let mut target = target.clone();
            for (ch, db) in target.gains.clone() {
                target.gains.insert(ch.clone(), i.capped_gain(&ch, db));
            }
            if keep_undo && i.config_readable {
                let before = format!("Before \"{label}\"");
                let state = i.state.clone();
                i.undo.push((before, state));
                if i.undo.len() > MAX_UNDO {
                    i.undo.remove(0);
                }
            }
            i.pending_ops.clear();
            i.batch.clear();
            match diff::changes(&i.state, &target) {
                Some(list) => (list, target),
                None => (ops::full_state(&target), target),
            }
        };
        let label = label.to_string();
        let s = self.clone();
        let job_label = format!("Loading \"{label}\" …");
        let body = async move {
            let conn = s.conn_id();
            let chunks = ConfigDump::new(s.model()).chunk_count();
            let mut steps = StepCounter { total: list.len() + 2 + chunks, done: 0, status: DspStatus::Busy };
            s.require(conn, p::build_store_preset_name_command(&label), None).await?;
            s.step(&mut steps);
            for op in &list {
                let frames = {
                    let mut i = s.0.borrow_mut();
                    ops::apply(&mut i.state, op);
                    ops::commands(&i.state, op).map_err(|e| SessionError::Failure(e.to_string()))?
                };
                for (_, frames) in frames {
                    for frame in frames {
                        s.require(conn, frame, None).await?;
                    }
                }
                s.step(&mut steps);
            }
            s.optional(conn, p::build_heartbeat_command()).await?;
            s.step(&mut steps);
            // Read back what the device took.
            let stream = s.read_config(conn, &mut steps).await;
            let mut i = s.0.borrow_mut();
            i.chunks = None;
            let read_back = match stream {
                Ok(stream) => {
                    drop(i);
                    s.apply_dump(&stream);
                    i = s.0.borrow_mut();
                    i.config_readable
                }
                Err(_) => false,
            };
            i.state.current_preset = label.clone();
            i.emit(SessionEvent::StateReplaced);
            i.add_status_log(format!("\"{label}\" loaded ({} changed settings)", list.len()));
            if read_back {
                let missing = diff::mismatches(&all, &i.state);
                if missing.is_empty() {
                    i.emit(SessionEvent::Notice { text: format!("\"{label}\" loaded and checked"), error: false });
                } else {
                    let shown: Vec<&str> = missing.iter().take(8).map(String::as_str).collect();
                    let text = format!(
                        "\"{label}\": {} value{} did not take: {}{}",
                        missing.len(),
                        if missing.len() == 1 { "" } else { "s" },
                        shown.join(", "),
                        if missing.len() > shown.len() { ", …" } else { "" }
                    );
                    i.add_status_log(text.clone());
                    i.emit(SessionEvent::Notice { text, error: true });
                }
            }
            Ok(())
        };
        self.spawn_exclusive_job(job_label, false, body);
        self.0.borrow_mut().emit(SessionEvent::InfoChanged);
        None
    }

    /// Load a preset (0-based index).
    pub fn load_preset(&self, index: i64) -> Option<String> {
        {
            let mut i = self.0.borrow_mut();
            if i.status != DspStatus::Ready {
                return Some("DSP is not ready".into());
            }
            if i.test_level.is_some() {
                return Some(TEST_RUNNING.into());
            }
            if !(0..p::PRESET_COUNT as i64).contains(&index) {
                return Some("Invalid preset".into());
            }
            // Values queued before the load would overwrite the loaded preset.
            i.pending_ops.clear();
            i.batch.clear();
        }
        let label = format!("U{:02}", index + 1);
        let s = self.clone();
        let job_label = format!("Loading preset {label} …");
        let body = async move {
            let conn = s.conn_id();
            let total = 1 + ConfigDump::new(s.model()).chunk_count() + 2;
            let mut steps = StepCounter { total, done: 0, status: DspStatus::Busy };
            let result: SResult<()> = async {
                let recall = s.0.borrow().timing.recall;
                s.require(conn, p::build_load_preset_command(index as u8), Some(recall)).await?;
                s.step(&mut steps);
                s.0.borrow_mut().state.current_preset_number = index + 1;
                let stream = s.read_config(conn, &mut steps).await?;
                s.apply_dump(&stream);
                for _ in 0..2 {
                    s.optional(conn, p::build_heartbeat_command()).await?;
                    s.step(&mut steps);
                }
                let mut i = s.0.borrow_mut();
                i.emit(SessionEvent::StateReplaced);
                let text = format!("Preset {label} loaded: {}", i.state.current_preset);
                i.add_status_log(text);
                Ok(())
            }
            .await;
            let mut i = s.0.borrow_mut();
            i.chunks = None;
            match result {
                Err(SessionError::Failure(_)) => {
                    i.add_status_log(format!("Preset {label} could not be loaded completely"));
                    Ok(())
                }
                other => other,
            }
        };
        self.spawn_exclusive_job(job_label, false, body);
        None
    }

    /// Store the current settings in a preset slot.
    pub fn save_preset(&self, index: i64, name: &str) -> Option<String> {
        if self.status() != DspStatus::Ready {
            return Some("DSP is not ready".into());
        }
        if self.test_level().is_some() {
            return Some(TEST_RUNNING.into());
        }
        if !(0..p::PRESET_COUNT as i64).contains(&index) {
            return Some("Invalid preset".into());
        }
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() || trimmed.encode_utf16().count() > 14 {
            return Some("Name must be 1–14 characters".into());
        }
        let label = format!("U{:02}", index + 1);
        let s = self.clone();
        let job_label = format!("Saving preset {label} …");
        let body = async move {
            let store_timeout = s.0.borrow().timing.store;
            let frames = p::store_preset_sequence(index as u8, &trimmed);
            let ok = s.run_sequence(frames, |f| (f[5] == 0x21).then_some(store_timeout)).await;
            let mut i = s.0.borrow_mut();
            if ok {
                // Same sanitising as the name command
                let frame = p::build_store_preset_name_command(&trimmed);
                let stored = String::from_utf8_lossy(&frame[6..20]).trim().to_string();
                i.state.presets.insert(index, stored.clone());
                i.state.preset_slots.insert(index);
                i.state.current_preset_number = index + 1;
                i.state.current_preset = stored.clone();
                i.emit(SessionEvent::StateReplaced);
                i.add_status_log(format!("Preset {label} saved: \"{stored}\""));
            } else {
                i.add_status_log(format!("Preset {label} could not be saved"));
            }
            Ok(())
        };
        self.spawn_exclusive_job(job_label, true, body);
        None
    }

    /// Start an exclusive job: the session is busy right away (like the Dart
    /// server, where the job runs synchronously up to its first await), the
    /// body runs as a task.
    fn spawn_exclusive_job(&self, label: String, drain_first: bool, body: impl Future<Output = SResult<()>> + 'static) {
        {
            let mut i = self.0.borrow_mut();
            i.set_status(DspStatus::Busy, Some(0.0), Some(Some(label.clone())));
            i.add_status_log(label);
        }
        let session = self.clone();
        spawn_local(async move { session.exclusive_job(drain_first, body).await });
    }

    async fn exclusive_job(&self, drain_first: bool, body: impl Future<Output = SResult<()>>) {
        if drain_first {
            // Let queued value changes reach the device first.
            loop {
                let busy = {
                    let mut i = self.0.borrow_mut();
                    i.pump();
                    i.conn.is_some() && !i.is_idle()
                };
                if !busy {
                    break;
                }
                self.wait_for_idle().await;
                tokio::task::yield_now().await;
            }
        }
        self.0.borrow_mut().exclusive = true;
        self.wait_for_idle().await;
        if let Err(e) = body.await {
            self.0.borrow_mut().add_status_log(format!("Error: {e}"));
        }
        let mut i = self.0.borrow_mut();
        i.exclusive = false;
        if i.conn.is_some() && i.status == DspStatus::Busy {
            i.set_status(DspStatus::Ready, Some(1.0), Some(None));
        }
        i.pump();
    }

    fn wait_for_idle(&self) -> impl Future<Output = ()> + use<> {
        let rx = {
            let mut i = self.0.borrow_mut();
            if i.in_flight.is_none() {
                None
            } else {
                let (tx, rx) = oneshot::channel();
                i.idle_waiters.push(tx);
                Some(rx)
            }
        };
        async move {
            if let Some(rx) = rx {
                let _ = rx.await;
            }
        }
    }

    // ─── Debug log ───

    pub fn set_debug(&self, rx: Option<bool>, tx: Option<bool>) {
        let mut i = self.0.borrow_mut();
        i.log_rx = rx.unwrap_or(i.log_rx);
        i.log_tx = tx.unwrap_or(i.log_tx);
        i.emit(SessionEvent::InfoChanged);
    }

    pub fn clear_log(&self) {
        let mut i = self.0.borrow_mut();
        i.log.clear();
        i.log_pending.clear();
        i.emit(SessionEvent::LogAppended { entries: Vec::new(), reset: true });
    }
}

impl Inner {
    fn proto(&self) -> Protocol {
        Protocol::with_id(self.state.model, self.config.device_id as u8)
    }

    /// Why the DSP cannot take a change now, if it cannot.
    fn not_ready(&self) -> Option<String> {
        match self.status {
            DspStatus::Ready => None,
            DspStatus::Busy => Some("DSP is busy".into()),
            _ => Some("DSP is not connected".into()),
        }
    }

    /// [db] for [ch] within the channel's max level.
    fn capped_gain(&self, ch: &str, db: f64) -> f64 {
        let current = self.state.gains.get(ch).copied().unwrap_or(0.0);
        ops::capped_gain(db, current, ops::group_cap(&self.state, &self.config.gain_caps, ch))
    }

    fn output_gains(&self) -> BTreeMap<String, f64> {
        self.state.model.outputs.iter().map(|o| (o.to_string(), self.state.gains.get(*o).copied().unwrap_or(0.0))).collect()
    }

    /// Apply [op] (and its linked copies) to the state, tell the clients and
    /// queue its commands; [at] puts them at that place of the queue instead
    /// of the end (mute all goes first).
    fn queue_op(&mut self, op: &Op, origin: Option<u64>, at: Option<usize>) -> Option<String> {
        // Like the original editor: the change goes to the device once, for
        // the edited channel; linked channels follow in the state (the DSP is
        // expected to apply the change to them itself).
        for expanded in ops::expand_linked(&self.state, op) {
            ops::apply(&mut self.state, &expanded);
            self.emit(SessionEvent::OpApplied { op: expanded, origin });
        }
        // A newer value for a key replaces the queued one but keeps its place.
        match ops::commands(&self.state, op) {
            Ok(commands) => {
                for (key, frames) in commands {
                    match at {
                        Some(at) => {
                            // An existing key is moved, a new one inserted.
                            let len = self.pending_ops.len();
                            let max = if self.pending_ops.contains_key(&key) { len - 1 } else { len };
                            self.pending_ops.shift_insert(at.min(max), key, frames);
                        }
                        None => {
                            self.pending_ops.insert(key, frames);
                        }
                    }
                }
            }
            Err(e) => return Some(e.to_string()),
        }
        self.pump();
        None
    }

    /// Apply [op] to the state of this channel only, tell the clients and
    /// return its frames (sent by an exclusive job).
    fn apply_now(&mut self, op: &Op) -> SResult<Vec<Frame>> {
        ops::apply(&mut self.state, op);
        self.emit(SessionEvent::OpApplied { op: op.clone(), origin: None });
        let commands = ops::commands(&self.state, op).map_err(|e| SessionError::Failure(e.to_string()))?;
        Ok(commands.into_iter().flat_map(|(_, frames)| frames).collect())
    }

    fn emit(&self, event: SessionEvent) {
        for listener in &self.listeners {
            let _ = listener.send(event.clone());
        }
    }

    fn set_status(&mut self, status: DspStatus, progress: Option<f64>, message: Option<Option<String>>) {
        let progress = progress.unwrap_or(self.progress);
        let message = message.unwrap_or_else(|| self.message.clone());
        if status == self.status && progress == self.progress && message == self.message {
            return;
        }
        self.status = status;
        self.progress = progress;
        self.message = message;
        self.emit(SessionEvent::InfoChanged);
    }

    fn elapsed(&self) -> f64 {
        self.started.map_or(0.0, |s| s.elapsed().as_secs_f64())
    }

    fn add_status_log(&mut self, text: impl Into<String>) {
        let entry = LogEntry::status(text, self.elapsed());
        self.add_log(entry);
    }

    fn add_log(&mut self, entry: LogEntry) {
        self.log.push_back(entry.clone());
        while self.log.len() > MAX_LOG_ENTRIES {
            self.log.pop_front();
        }
        self.log_pending.push(entry);
        if !self.log_flush_scheduled {
            self.log_flush_scheduled = true;
            let weak = self.weak.clone();
            spawn_local(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if let Some(inner) = weak.upgrade() {
                    let mut i = inner.borrow_mut();
                    i.log_flush_scheduled = false;
                    if !i.log_pending.is_empty() {
                        let entries = std::mem::take(&mut i.log_pending);
                        i.emit(SessionEvent::LogAppended { entries, reset: false });
                    }
                }
            });
        }
    }

    fn connection_lost(&mut self, conn: u64, reason: String) {
        if self.conn.as_ref().map(|c| c.id) != Some(conn) {
            return; // stale callback of an old connection
        }
        self.message = Some(reason);
        if let Some(closed) = self.closed.take() {
            let _ = closed.send(());
        }
    }

    // ─── Requests ───

    fn request(&mut self, frame: Frame, options: RequestOptions) -> oneshot::Receiver<Option<Frame>> {
        let (tx, rx) = oneshot::channel();
        let Some(conn) = self.conn.as_ref().map(|c| c.id) else {
            let _ = tx.send(None);
            return rx;
        };
        if self.in_flight.is_some() {
            let _ = tx.send(None);
            return rx;
        }
        self.next_request += 1;
        self.in_flight = Some(InFlight {
            id: self.next_request,
            attempt: 0,
            frame: with_device_id(&frame, self.config.device_id as u8),
            keepalive: options.keepalive,
            timeout: options.timeout.unwrap_or(self.timing.request),
            retries_left: options.retries,
            fatal: options.fatal,
            conn,
            done: Some(tx),
        });
        self.send_in_flight();
        rx
    }

    fn send_in_flight(&mut self) {
        let Some(request) = &self.in_flight else { return };
        let (frame, id, attempt, timeout) = (request.frame.clone(), request.id, request.attempt, request.timeout);
        if let Some(conn) = &self.conn {
            let _ = conn.writer.send(WriterMessage::Frame(frame.clone()));
        }
        self.packets_sent += 1;
        self.last_activity = Instant::now();
        if self.log_tx {
            let entry = LogEntry::packet(
                format!("Tx: {} → {}", format_as_hex(&frame), format_as_ascii(&frame)),
                format!("Tx: {}", describe(&frame, &self.state.model)),
                self.elapsed(),
            );
            self.add_log(entry);
        }
        let weak = self.weak.clone();
        spawn_local(async move {
            tokio::time::sleep(timeout).await;
            if let Some(inner) = weak.upgrade() {
                inner.borrow_mut().on_timeout(id, attempt);
            }
        });
    }

    fn on_timeout(&mut self, id: u64, attempt: u32) {
        let Some(request) = self.in_flight.as_mut() else { return };
        if request.id != id || request.attempt != attempt {
            return;
        }
        log::debug(format!("[{}] timeout for {}", self.config.name, describe(&request.frame, &self.state.model)));
        let same_conn = self.conn.as_ref().map(|c| c.id) == Some(request.conn);
        if request.retries_left > 0 && same_conn {
            request.retries_left -= 1;
            request.attempt += 1;
            self.send_in_flight();
            return;
        }
        let Some(mut request) = self.in_flight.take() else { return };
        if let Some(done) = request.done.take() {
            let _ = done.send(None);
        }
        if request.fatal {
            self.connection_lost(request.conn, "DSP is not responding (link broken)".into());
        }
        self.after_request();
    }

    fn after_request(&mut self) {
        self.notify_idle();
        let weak = self.weak.clone();
        spawn_local(async move {
            if let Some(inner) = weak.upgrade() {
                inner.borrow_mut().pump();
            }
        });
    }

    fn notify_idle(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        for waiter in self.idle_waiters.drain(..) {
            let _ = waiter.send(());
        }
    }

    fn on_data(&mut self, conn: u64, data: &[u8]) {
        if self.conn.as_ref().map(|c| c.id) != Some(conn) {
            return;
        }
        for frame in self.decoder.add(data) {
            self.on_frame(frame);
        }
    }

    fn on_frame(&mut self, frame: Frame) {
        self.packets_received += 1;
        let Some(message) = self.proto().decode(&frame) else {
            return; // not an answer of our device
        };
        if self.log_rx {
            let entry = LogEntry::packet(
                format!("Rx: {} → {}", format_as_hex(&frame), format_as_ascii(&frame)),
                format!("Rx: {}", describe_rx(&frame, &self.state.model, self.config.device_id as u8)),
                self.elapsed(),
            );
            self.add_log(entry);
        }

        match message {
            Message::Keepalive { levels, dynamics } => {
                self.meters = levels;
                self.dynamics = dynamics;
                self.schedule_meters();
            }
            Message::Preset { index, name } => {
                self.state.presets.insert(index as i64, name);
                if self.status == DspStatus::Ready {
                    self.emit(SessionEvent::StateReplaced);
                }
            }
            Message::PresetSlots { used } => {
                self.state.preset_slots =
                    used.iter().enumerate().filter(|(_, u)| **u).map(|(i, _)| i as i64).collect();
            }
            Message::CurrentPreset { number } => self.state.current_preset_number = number as i64,
            Message::ChannelConfig { sub_index, chunk } => {
                if let Some(chunks) = self.chunks.as_mut() {
                    chunks.insert(sub_index, chunk);
                }
            }
            _ => {}
        }

        // Meter frames only answer meter requests; any other response answers
        // the command in flight.
        let is_meter = frame[5] == 0x40;
        if self.in_flight.as_ref().is_some_and(|r| r.keepalive == is_meter)
            && let Some(mut request) = self.in_flight.take() {
                self.last_activity = Instant::now();
                if let Some(done) = request.done.take() {
                    let _ = done.send(Some(frame));
                }
                self.after_request();
            }
    }

    /// Send the next queued value change, if the line is free.
    fn pump(&mut self) {
        if self.conn.is_none() || self.in_flight.is_some() || self.exclusive {
            return;
        }
        if self.status != DspStatus::Ready && self.status != DspStatus::Busy {
            return;
        }
        if self.batch.is_empty()
            && let Some((_, frames)) = self.pending_ops.shift_remove_index(0) {
                self.batch.extend(frames);
            }
        if let Some(frame) = self.batch.pop_front() {
            drop(self.request(frame, RequestOptions::default()));
        }
    }

    fn is_idle(&self) -> bool {
        self.in_flight.is_none() && self.batch.is_empty() && self.pending_ops.is_empty() && !self.exclusive
    }

    fn start_keepalive(&mut self) {
        self.stop_keepalive();
        let period = Duration::from_millis(self.config.refresh_ms.max(1) as u64);
        let weak = self.weak.clone();
        let task = spawn_local(async move {
            let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                inner.borrow_mut().keepalive_tick();
            }
        });
        self.keepalive = Some(task.abort_handle());
    }

    fn keepalive_tick(&mut self) {
        if self.status != DspStatus::Ready || !self.is_idle() {
            return;
        }
        if self.meters_wanted {
            let options = RequestOptions { keepalive: true, ..Default::default() };
            drop(self.request(p::build_keepalive_command(), options));
        } else if self.last_activity.elapsed() >= self.timing.heartbeat_idle {
            drop(self.request(p::build_heartbeat_command(), RequestOptions::default()));
        }
    }

    fn stop_keepalive(&mut self) {
        if let Some(task) = self.keepalive.take() {
            task.abort();
        }
    }

    fn schedule_meters(&mut self) {
        if self.meter_timer_active {
            return;
        }
        self.meter_timer_active = true;
        let weak = self.weak.clone();
        let interval = self.timing.meter_interval;
        spawn_local(async move {
            tokio::time::sleep(interval).await;
            if let Some(inner) = weak.upgrade() {
                let mut i = inner.borrow_mut();
                i.meter_timer_active = false;
                i.emit(SessionEvent::MetersUpdated);
            }
        });
    }
}

async fn reader_loop(mut reader: OwnedReadHalf, session: Weak<RefCell<Inner>>, conn: u64) {
    let mut buf = vec![0u8; 4096];
    let reason = loop {
        match reader.read(&mut buf).await {
            Ok(0) => break "Connection closed by the DSP".to_string(),
            Ok(n) => match session.upgrade() {
                Some(inner) => inner.borrow_mut().on_data(conn, &buf[..n]),
                None => return,
            },
            Err(e) => break format!("Socket error: {e}"),
        }
    };
    if let Some(inner) = session.upgrade() {
        inner.borrow_mut().connection_lost(conn, reason);
    }
}

async fn writer_loop(
    mut writer: OwnedWriteHalf,
    mut rx: mpsc::UnboundedReceiver<WriterMessage>,
    session: Weak<RefCell<Inner>>,
    conn: u64,
) {
    while let Some(message) = rx.recv().await {
        let result = match message {
            WriterMessage::Frame(frame) => writer.write_all(&frame).await,
            WriterMessage::Flush(done) => {
                let result = writer.flush().await;
                let _ = done.send(());
                result
            }
        };
        if let Err(e) = result {
            if let Some(inner) = session.upgrade() {
                inner.borrow_mut().connection_lost(conn, format!("Socket error: {e}"));
            }
            return;
        }
    }
}
