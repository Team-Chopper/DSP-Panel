//! Protocol, device model and web-panel API for the DSP 206 and DSP 408,
//! shared by the server and the web client.
//!
//! This is the Rust port of `packages/dsp_core` (Dart). Byte sequences, value
//! encodings, JSON formats and op semantics are identical, so the Rust and
//! Dart implementations can talk to each other and read each other's files.

pub mod api;
pub mod config_dump;
pub mod describe;
pub mod eq_math;
pub mod format;
pub mod frame_decoder;
pub mod model;
pub mod ops;
pub mod protocol;
pub mod state;
pub mod testing;

pub use api::{DEFAULT_REFRESH_MS, DspInfo, DspStatus, LogEntry, ServerPresetInfo};
pub use config_dump::{ConfigDump, ConfigDumpResult};
pub use frame_decoder::FrameDecoder;
pub use model::{DeviceModel, GEQ_BAND_COUNT};
pub use ops::Op;
pub use protocol::{Frame, Message, Protocol, ProtocolError};
pub use state::{CompressorState, DeviceState, FilterState, GateState, LimiterState, PeqBand};
