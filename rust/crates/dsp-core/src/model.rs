//! Device models.
//!
//! The DSP 204, DSP 206 and DSP 408 speak the same protocol; they differ in
//! the number of channels and in the channel bytes of the outputs:
//!
//! ```text
//! DSP 408: In A–D = 0x00–0x03, Out 1–8 = 0x04–0x0B
//! DSP 206: In A–B = 0x00–0x01, Out 1–6 = 0x02–0x07
//! DSP 204: In A–B = 0x00–0x01, Out 1–4 = 0x02–0x05
//! ```
//!
//! The DSP 204 was not analysed in an original editor: its layout follows the
//! same rules (config dump 724 bytes, 15 chunks; meters in slots 0–1 and 4–7),
//! as measured on a device by another project (TODO.md).
//!
//! Channel labels ("In A" … "Out 8") are the keys of the device state, the
//! web-panel API and the UI.

use std::fmt;

#[derive(Clone, Copy, Debug)]
pub struct DeviceModel {
    /// Stable identifier used in configuration and API ("dsp408", "dsp206",
    /// "dsp204").
    pub id: &'static str,
    /// Display name ("DSP 408").
    pub name: &'static str,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    /// Protocol channel byte of the first output.
    first_output_byte: u8,
}

impl PartialEq for DeviceModel {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for DeviceModel {}

impl fmt::Display for DeviceModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

impl Default for DeviceModel {
    fn default() -> Self {
        DeviceModel::DSP408
    }
}

/// Number of GEQ bands per input channel (31-band ISO 1/3 octave).
pub const GEQ_BAND_COUNT: usize = 31;

impl DeviceModel {
    pub const DSP408: DeviceModel = DeviceModel {
        id: "dsp408",
        name: "DSP 408",
        inputs: &["In A", "In B", "In C", "In D"],
        outputs: &["Out 1", "Out 2", "Out 3", "Out 4", "Out 5", "Out 6", "Out 7", "Out 8"],
        first_output_byte: 0x04,
    };

    pub const DSP206: DeviceModel = DeviceModel {
        id: "dsp206",
        name: "DSP 206",
        inputs: &["In A", "In B"],
        outputs: &["Out 1", "Out 2", "Out 3", "Out 4", "Out 5", "Out 6"],
        first_output_byte: 0x02,
    };

    pub const DSP204: DeviceModel = DeviceModel {
        id: "dsp204",
        name: "DSP 204",
        inputs: &["In A", "In B"],
        outputs: &["Out 1", "Out 2", "Out 3", "Out 4"],
        first_output_byte: 0x02,
    };

    pub const ALL: [DeviceModel; 3] = [DeviceModel::DSP408, DeviceModel::DSP206, DeviceModel::DSP204];

    /// Protocol version the original editor requires after the prefix.
    pub const INFO_VERSION: &'static str = "V010";

    /// Model by id; unknown ids fall back to the DSP 408.
    pub fn by_id(id: Option<&str>) -> DeviceModel {
        Self::ALL.into_iter().find(|m| Some(m.id) == id).unwrap_or(Self::DSP408)
    }

    pub fn is_known_id(id: &str) -> bool {
        Self::ALL.iter().any(|m| m.id == id)
    }

    /// Start of the device info answer (0x13) of this model, "DSP408".
    pub fn info_prefix(&self) -> String {
        self.name.replace(' ', "")
    }

    /// Recognise the model from the device info string (e.g. "DSP408 V0104P").
    /// Like the original editor, the answer must start with "DSP408" or
    /// "DSP206" (or "DSP204").
    pub fn detect(device_info: &str) -> Option<DeviceModel> {
        let info = device_info.trim().to_uppercase();
        Self::ALL.into_iter().find(|m| info.starts_with(&m.info_prefix()))
    }

    /// The device info reports the protocol version the original editor
    /// accepts ("DSP408 V010…").
    pub fn supports_version(&self, device_info: &str) -> bool {
        device_info
            .trim()
            .to_uppercase()
            .starts_with(&format!("{} {}", self.info_prefix(), Self::INFO_VERSION))
    }

    /// All channels in protocol and meter order: inputs, then outputs.
    pub fn channels(&self) -> impl Iterator<Item = &'static str> + Clone + use<> {
        self.inputs.iter().chain(self.outputs.iter()).copied()
    }

    pub fn channel_count(&self) -> usize {
        self.inputs.len() + self.outputs.len()
    }

    /// Number of meter levels in a 0x40 response.
    pub fn meter_count(&self) -> usize {
        self.channel_count()
    }

    pub fn is_input(&self, channel: &str) -> bool {
        self.inputs.contains(&channel)
    }

    pub fn is_output(&self, channel: &str) -> bool {
        self.outputs.contains(&channel)
    }

    pub fn has_channel(&self, channel: &str) -> bool {
        self.is_input(channel) || self.is_output(channel)
    }

    /// The model's own label for [channel] (a `'static` str), if it has it.
    pub fn label(&self, channel: &str) -> Option<&'static str> {
        self.channels().find(|c| *c == channel)
    }

    pub fn input_index(&self, channel: &str) -> Option<usize> {
        self.inputs.iter().position(|c| *c == channel)
    }

    pub fn output_index(&self, channel: &str) -> Option<usize> {
        self.outputs.iter().position(|c| *c == channel)
    }

    /// Protocol channel byte, or `None` if the model has no such channel.
    pub fn channel_byte(&self, channel: &str) -> Option<u8> {
        if let Some(i) = self.input_index(channel) {
            return Some(i as u8);
        }
        self.output_index(channel).map(|o| self.first_output_byte + o as u8)
    }

    /// Channel label for a protocol channel byte, or `None`.
    pub fn channel_for_byte(&self, byte: u8) -> Option<&'static str> {
        let byte = byte as usize;
        if byte < self.inputs.len() {
            return Some(self.inputs[byte]);
        }
        let first = self.first_output_byte as usize;
        if byte >= first {
            return self.outputs.get(byte - first).copied();
        }
        None
    }

    /// Matrix routing bit of an input (In A = 0x01, In B = 0x02, …).
    pub fn matrix_bit(&self, input: &str) -> i32 {
        self.input_index(input).map_or(0, |i| 1 << i)
    }

    /// Bitmask with one bit per input.
    pub fn matrix_mask(&self) -> i32 {
        (1 << self.inputs.len()) - 1
    }

    /// Channel link bit: position of the channel within its group (inputs or
    /// outputs), as stored in the config dump (In A = 0x01, Out 1 = 0x01).
    pub fn link_bit(&self, channel: &str) -> i32 {
        match self.input_index(channel).or_else(|| self.output_index(channel)) {
            Some(i) => 1 << i,
            None => 0,
        }
    }

    /// The channels of the same group (inputs or outputs) as [channel].
    pub fn group_of(&self, channel: &str) -> &'static [&'static str] {
        if self.is_input(channel) { self.inputs } else { self.outputs }
    }

    /// Bitmask with one bit per channel of [channel]'s group.
    pub fn group_mask(&self, channel: &str) -> i32 {
        (1 << self.group_of(channel).len()) - 1
    }

    /// Name the device uses for a channel by default ("InA", "Out1").
    pub fn default_device_name(&self, channel: &str) -> String {
        channel.replace(' ', "")
    }

    /// PEQ bands of a channel: inputs have 8, outputs 9 (all models).
    pub fn peq_band_count(&self, channel: &str) -> usize {
        if self.is_input(channel) { 8 } else { 9 }
    }
}
