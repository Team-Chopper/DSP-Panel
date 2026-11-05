//! Panel data the server keeps for every client (the DSPs do not know it):
//! the limiter calculator setup, names of link groups and the settings of the
//! limit light page. Sent as `{t: panel, data}` (api.rs).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::limiter_calc::LimiterSetup;
use crate::ops;
use crate::state::DeviceState;

/// Longest name of a link group.
pub const LINK_NAME_MAX: usize = 16;

/// Settings of the limit light page (/limit, without login).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightSettings {
    pub enabled: bool,
    /// Outputs not watched, per DSP id.
    #[serde(default)]
    pub ignore: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelData {
    #[serde(default)]
    pub limiter: LimiterSetup,
    /// Per DSP id: names of link groups by [group_key].
    #[serde(default)]
    pub link_names: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub light: LightSettings,
}

impl PanelData {
    /// Name of the link group of [dsp] with [members] (channel order), if any.
    pub fn link_name(&self, dsp: &str, members: &[String]) -> Option<&str> {
        self.link_names.get(dsp)?.get(&group_key(members)).map(String::as_str)
    }
}

/// Key of a link group: its members in channel order, joined by "+".
pub fn group_key(members: &[String]) -> String {
    members.join("+")
}

/// The link groups of [state] (two or more channels), members in channel
/// order. A name stays with its members: a group that changes shows no name,
/// the same group formed again gets its name back.
pub fn link_groups(state: &DeviceState) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    for ch in state.model.channels() {
        let group = ops::link_group_of(state, ch);
        if group.len() < 2 {
            continue;
        }
        let sorted: Vec<String> = state.model.channels().filter(|c| group.iter().any(|g| g == c)).map(String::from).collect();
        if !groups.contains(&sorted) {
            groups.push(sorted);
        }
    }
    groups
}
