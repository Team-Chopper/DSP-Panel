//! Differences between two states of one model: what a preset load has to
//! send to a DSP that already has most of the values, and what did not arrive
//! after a load.

use crate::ops::{self, Op};
use crate::state::DeviceState;

/// Ops that bring a device from [current] to [target], in the order of
/// [ops::full_state], without the ones that would change nothing. `None` if
/// the models or the link masks differ: forming a group makes the DSP copy
/// settings, so then everything has to be sent in the full order.
pub fn changes(current: &DeviceState, target: &DeviceState) -> Option<Vec<Op>> {
    if current.model != target.model || current.links != target.links {
        return None;
    }
    let mut state = current.clone();
    let mut out = Vec::new();
    for op in ops::full_state(target) {
        let mut next = state.clone();
        ops::apply(&mut next, &op);
        if next != state {
            out.push(op);
            state = next;
        }
    }
    Some(out)
}

/// Readable list of the values in which [actual] differs from [target] (the
/// settings a preset holds; names of presets and slots are left out), e.g.
/// "Out 2 gain", "In A PEQ 3". Empty if they match.
pub fn mismatches(target: &DeviceState, actual: &DeviceState) -> Vec<String> {
    let mut out = Vec::new();
    for op in ops::full_state(target) {
        let mut probe = actual.clone();
        ops::apply(&mut probe, &op);
        if probe != *actual {
            out.push(describe(&op));
        }
    }
    out.dedup();
    out
}

fn describe(op: &Op) -> String {
    let json = op.to_json();
    let what = json["op"].as_str().unwrap_or("value").to_string();
    let ch = json["ch"].as_str().map(|c| format!("{c} ")).unwrap_or_default();
    match json.get("band").and_then(|b| b.as_i64()) {
        Some(band) => format!("{ch}{what} {}", band + 1),
        None => format!("{ch}{what}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceModel;
    use crate::state::PeqBand;

    #[test]
    fn only_what_differs_is_sent() {
        let current = DeviceState::new(DeviceModel::DSP206);
        assert_eq!(changes(&current, &current), Some(Vec::new()));
        let mut target = current.clone();
        ops::apply(&mut target, &Op::gain("Out 2", -6.0));
        ops::apply(&mut target, &Op::peq_band("In A", 2, PeqBand { gain_db: 3.0, ..PeqBand::default() }));
        let list = changes(&current, &target).unwrap();
        assert_eq!(list, [Op::gain("Out 2", -6.0), Op::peq_band("In A", 2, PeqBand { gain_db: 3.0, ..PeqBand::default() })]);
        let mut done = current.clone();
        for op in &list {
            ops::apply(&mut done, op);
        }
        assert_eq!(done, target);
        assert_eq!(mismatches(&target, &done), Vec::<String>::new());
        assert_eq!(mismatches(&target, &current), ["Out 2 gain", "In A peqBand 3"]);
    }

    #[test]
    fn other_links_need_the_full_state() {
        let current = DeviceState::new(DeviceModel::DSP408);
        let mut target = current.clone();
        ops::apply(&mut target, &Op::link("Out 1", 0x03));
        assert_eq!(changes(&current, &target), None);
        assert_eq!(changes(&current, &DeviceState::new(DeviceModel::DSP206)), None);
    }
}
