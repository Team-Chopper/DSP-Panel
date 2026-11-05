//! Dialogs: add/edit DSP, settings, and the channel menu with rename, copy
//! and link.

use std::collections::BTreeSet;

use dsp_core::api::DEFAULT_REFRESH_MS;
use dsp_core::protocol::{MAX_DEVICE_ID, MIN_DEVICE_ID};
use dsp_core::{DeviceModel, DspInfo, ops};
use leptos::prelude::*;

use crate::components::common::{Check, Modal};
use crate::fmt::{format_db_value, parse_number};
use crate::icons::{I, Icon};
use crate::pages::dsp::{Dialog, ui};
use crate::rta::RtaSettings;
use crate::store::{alias_in, app};

fn valid_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c))
}

/// Add a DSP ([initial] = None) or edit/delete one.
#[component]
pub fn DspDialog(initial: Option<DspInfo>, on_close: Callback<()>) -> impl IntoView {
    let app = app();
    let editing = initial.clone();
    let name = RwSignal::new(initial.as_ref().map(|i| i.name.clone()).unwrap_or_default());
    let host = RwSignal::new(initial.as_ref().map_or("192.168.3.100".into(), |i| i.host.clone()));
    let port = RwSignal::new(initial.as_ref().map_or(9761, |i| i.port).to_string());
    let device_id = RwSignal::new(initial.as_ref().map_or(1, |i| i.device_id).to_string());
    let password = RwSignal::new(String::new());
    let model = RwSignal::new(initial.as_ref().map_or("auto".to_string(), |i| i.model_setting.clone()));
    let remove_password = RwSignal::new(false);
    let confirm_delete = RwSignal::new(false);
    let submitted = RwSignal::new(false);
    let has_password = initial.as_ref().is_some_and(|i| i.has_password);

    let name_error = move || name.with(|n| n.trim().is_empty()).then_some("Please enter a name");
    let host_error = move || (!valid_host(host.with(|h| h.trim().to_string()).as_str())).then_some("Invalid address");
    let port_value = move || port.with(|p| p.trim().parse::<i64>().ok()).filter(|p| (1..=65535).contains(p));
    let id_value = move || {
        device_id.with(|d| d.trim().parse::<i64>().ok()).filter(|d| (MIN_DEVICE_ID as i64..=MAX_DEVICE_ID as i64).contains(d))
    };
    let password_error = move || password.with(|p| {
        let t = p.trim();
        (!t.is_empty() && t.chars().count() != 4).then_some("4 characters")
    });
    let valid = move || {
        name_error().is_none()
            && host_error().is_none()
            && port_value().is_some()
            && id_value().is_some()
            && password_error().is_none()
    };

    let close = on_close;
    let submit = {
        let editing = editing.clone();
        move || {
            submitted.set(true);
            if !valid() {
                return;
            }
            let typed = password.get_untracked().trim().to_uppercase();
            let (n, h) = (name.get_untracked().trim().to_string(), host.get_untracked().trim().to_string());
            let (p, d, m) = (port_value().unwrap_or(9761), id_value().unwrap_or(1), model.get_untracked());
            match &editing {
                None => app.add_dsp(&n, &h, p, &m, d, &typed),
                Some(info) => {
                    let pw = if remove_password.get_untracked() {
                        Some(String::new())
                    } else if !typed.is_empty() {
                        Some(typed)
                    } else if has_password {
                        None
                    } else {
                        Some(String::new())
                    };
                    app.update_dsp(&info.id, &n, &h, p, &m, d, pw.as_deref());
                }
            }
            close.run(());
        }
    };
    let submit_click = submit.clone();

    let title = if initial.is_some() { "Edit DSP" } else { "Add DSP" };
    let detected = initial.as_ref().map(|i| DeviceModel::by_id(Some(&i.model)).name);
    let delete_id = initial.as_ref().map(|i| (i.id.clone(), i.name.clone()));
    let actions = view! {
        {delete_id.clone().map(|_| view! {
            <button class="btn text danger-text" on:click=move |_| confirm_delete.set(true)>"Delete"</button>
        })}
        <span class="spacer"></span>
        <button class="btn text" on:click=move |_| close.run(())>"Cancel"</button>
        <button class="btn text" on:click=move |_| submit_click()>"Save"</button>
    }
    .into_any();

    view! {
        <Modal title=title on_close=close width=400 actions=actions>
            <form class="col" on:submit=move |ev| {
                ev.prevent_default();
                submit();
            }>
                <div class="field" class:invalid=move || submitted.get() && name_error().is_some()>
                    <label>"Name"</label>
                    <input maxlength="40" placeholder="e.g. FOH, Monitors" autofocus
                        prop:value=move || name.get() on:input=move |ev| name.set(event_target_value(&ev))/>
                    {move || (submitted.get()).then(name_error).flatten().map(|e| view! { <span class="error">{e}</span> })}
                </div>
                <div class="row">
                    <div class="field grow" class:invalid=move || submitted.get() && host_error().is_some()>
                        <label>"IP Address"</label>
                        <input prop:value=move || host.get() on:input=move |ev| host.set(event_target_value(&ev))/>
                    </div>
                    <div class="field" style="width:100px" class:invalid=move || submitted.get() && port_value().is_none()>
                        <label>"Port"</label>
                        <input inputmode="numeric" prop:value=move || port.get() on:input=move |ev| port.set(event_target_value(&ev))/>
                    </div>
                </div>
                <div class="field">
                    <label>"Model"</label>
                    <select on:change=move |ev| model.set(event_target_value(&ev))>
                        <option value="auto" selected=move || model.get() == "auto">
                            {detected.map_or("Auto".to_string(), |d| format!("Auto (detected: {d})"))}
                        </option>
                        {DeviceModel::ALL.iter().map(|m| {
                            let id = m.id;
                            view! { <option value=id selected=move || model.get() == id>{m.name}</option> }
                        }).collect_view()}
                    </select>
                    <span class="help">"Auto detects the model when connecting"</span>
                </div>
                <div class="row" style="align-items:flex-start">
                    <div class="field" style="width:100px" class:invalid=move || submitted.get() && id_value().is_none()>
                        <label>"Device ID"</label>
                        <input inputmode="numeric" prop:value=move || device_id.get()
                            on:input=move |ev| device_id.set(event_target_value(&ev))/>
                        <span class="help">"Default 1"</span>
                    </div>
                    <div class="field grow" class:invalid=move || password_error().is_some()>
                        <label>"Lock password"</label>
                        <input type="password" maxlength="4" autocomplete="off"
                            prop:disabled=move || remove_password.get()
                            prop:value=move || password.get()
                            on:input=move |ev| password.set(event_target_value(&ev).chars().filter(|c| ('!'..='~').contains(c)).collect())/>
                        <span class="help">{if has_password { "Stored – empty keeps it" } else { "Only for a DSP locked with the original software" }}</span>
                    </div>
                </div>
                {has_password.then(|| view! {
                    <Check checked=remove_password on_toggle=Callback::new(move |_| remove_password.update(|r| *r = !*r))
                        label="Remove the stored password"/>
                })}
                <button type="submit" hidden></button>
            </form>
        </Modal>
        <Show when=move || confirm_delete.get()>
            {
                let (id, dsp_name) = delete_id.clone().unwrap_or_default();
                let actions = view! {
                    <button class="btn text" on:click=move |_| confirm_delete.set(false)>"Cancel"</button>
                    <button class="btn text danger-text" on:click={
                        let id = id.clone();
                        move |_| {
                            app.remove_dsp(&id);
                            confirm_delete.set(false);
                            close.run(());
                        }
                    }>"Delete"</button>
                }.into_any();
                view! {
                    <Modal title="Delete DSP" on_close=Callback::new(move |_| confirm_delete.set(false)) actions=actions>
                        <div>{format!("Remove \"{dsp_name}\" from the panel? The settings on the device itself are not changed.")}</div>
                    </Modal>
                }
            }
        </Show>
    }
}

/// Meter refresh interval of the DSP and the microphone for the RTA.
#[component]
pub fn SettingsDialog() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let value = RwSignal::new(device.info.get_untracked().refresh_ms.to_string());
    let close = Callback::new(move |_| ui.dialog.set(None));
    let set = move || {
        if let Some(ms) = value.get_untracked().trim().parse::<i64>().ok().filter(|m| (5..=300).contains(m)) {
            device.set_refresh(ms);
            ui.dialog.set(None);
        }
    };
    let rta = RtaSettings::get();
    rta.enumerate_devices();
    let actions = view! {
        <button class="btn text" on:click=move |_| {
            device.set_refresh(DEFAULT_REFRESH_MS);
            ui.dialog.set(None);
        }>"Reset"</button>
        <span class="spacer"></span>
        <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Cancel"</button>
        <button class="btn text" on:click=move |_| set()>"Set"</button>
    }
    .into_any();
    view! {
        <Modal title="Settings" on_close=close actions=actions>
            <form class="col" on:submit=move |ev| {
                ev.prevent_default();
                set();
            }>
                <div class="field">
                    <label>"Meter refresh interval (5–300 ms)"</label>
                    <input inputmode="numeric" autofocus prop:value=move || value.get()
                        on:input=move |ev| value.set(event_target_value(&ev))/>
                    <span class="help">{format!("Applies to this DSP for all clients (default {DEFAULT_REFRESH_MS} ms)")}</span>
                </div>
            </form>
            <div class="field">
                <label>"Mic input"</label>
                <select on:change=move |ev| rta.select_device(event_target_value(&ev))>
                    {move || {
                        let devices = rta.devices.get();
                        if devices.is_empty() {
                            return view! { <option>"No devices found"</option> }.into_any();
                        }
                        let selected = rta.selected.get();
                        devices.into_iter().map(|(id, label)| {
                            let is_selected = selected.as_deref() == Some(id.as_str());
                            view! { <option value=id selected=is_selected>{label}</option> }
                        }).collect_view().into_any()
                    }}
                </select>
            </div>
        </Modal>
    }
}

/// Channel menu and the rename, copy and link dialogs.
#[component]
pub fn ChannelDialogs(dialog: Dialog) -> impl IntoView {
    match dialog {
        Dialog::ChannelMenu(ch) => view! { <ChannelMenu channel=ch/> }.into_any(),
        Dialog::Rename(ch) => view! { <RenameDialog channel=ch/> }.into_any(),
        Dialog::Copy(ch) => view! { <CopyDialog channel=ch/> }.into_any(),
        Dialog::Link(ch) => view! { <LinkDialog channel=ch/> }.into_any(),
        Dialog::MaxLevel(ch) => view! { <MaxLevelDialog channel=ch/> }.into_any(),
        Dialog::ForceGain(ch, db) => view! { <ForceGainDialog channel=ch db=db/> }.into_any(),
        _ => ().into_any(),
    }
}

/// Max level of a channel: kept by the server (the DSP does not know it), it
/// never sets the gain higher; a channel already above can only go down.
#[component]
fn MaxLevelDialog(channel: String) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let (name, linked) = device.state.with_untracked(|s| {
        let others: Vec<String> = ops::linked_with(s, &channel).iter().map(|c| alias_in(s, c)).collect();
        (alias_in(s, &channel), others)
    });
    let current = device.gain_cap(&channel);
    let value = RwSignal::new(current.map(format_db_value).unwrap_or_default());
    let error = move || value.with(|v| parse_number(v).filter(|db| (-60.0..=12.0).contains(db)).is_none());
    // An empty field is not wrong yet, it just cannot be set.
    let invalid = move || value.with(|v| !v.trim().is_empty()) && error();
    let ch = channel.clone();
    let set = move || {
        if let Some(db) = parse_number(&value.get_untracked()).filter(|db| (-60.0..=12.0).contains(db)) {
            device.set_gain_cap(&ch, Some(db));
            ui.dialog.set(None);
        }
    };
    let set_click = set.clone();
    let remove_ch = channel.clone();
    let actions = view! {
        {current.is_some().then(|| view! {
            <button class="btn text danger-text" on:click=move |_| {
                device.set_gain_cap(&remove_ch, None);
                ui.dialog.set(None);
            }>"Remove"</button>
        })}
        <span class="spacer"></span>
        <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Cancel"</button>
        <button class="btn filled" prop:disabled=error on:click=move |_| set_click()>"Set"</button>
    }
    .into_any();
    let note = if linked.is_empty() {
        String::new()
    } else {
        format!(" Linked channels share it: {}.", linked.join(", "))
    };
    view! {
        <Modal title=format!("Max level of {name}") on_close=Callback::new(move |_| ui.dialog.set(None)) actions=actions>
            <form on:submit=move |ev| {
                ev.prevent_default();
                set();
            }>
                <div class="field" class:invalid=invalid>
                    <label>"Max level (dB)"</label>
                    <input inputmode="decimal" autofocus placeholder="e.g. -6" prop:value=move || value.get()
                        on:input=move |ev| value.set(event_target_value(&ev))/>
                    <span class="help">{format!(
                        "The panel never sets the gain higher (-60 … +12 dB); typing a higher value asks first. \
                         Kept by the panel server, not on the DSP. A channel that is already above it can only be \
                         turned down.{note}"
                    )}</span>
                </div>
            </form>
        </Modal>
    }
}

/// Confirmation for a typed gain above the max level.
#[component]
fn ForceGainDialog(channel: String, db: f64) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let name = device.alias(&channel);
    let cap = device.gain_cap(&channel).map(format_db_value).unwrap_or_default();
    let close = Callback::new(move |_| ui.dialog.set(None));
    let actions = view! {
        <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Cancel"</button>
        <button class="btn danger" on:click=move |_| {
            device.set_gain_forced(&channel, db);
            ui.dialog.set(None);
        }>"Set anyway"</button>
    }
    .into_any();
    view! {
        <Modal title="Above the max level" on_close=close actions=actions>
            <div class="note-text">{format!(
                "The max level of {name} is {cap} dB. Set it to {} dB anyway?",
                format_db_value(db)
            )}</div>
        </Modal>
    }
}

#[component]
fn ChannelMenu(channel: String) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let (title, subtitle, color) = device.state.with_untracked(|s| {
        let name = alias_in(s, &channel);
        let link = match ops::master_of(s, &channel) {
            Some(m) => format!("linked to {}", alias_in(s, &m)),
            None => {
                let slaves = ops::slaves_of(s, &channel);
                if slaves.is_empty() {
                    "not linked".to_string()
                } else {
                    format!("master of {}", slaves.iter().map(|c| alias_in(s, c)).collect::<Vec<_>>().join(", "))
                }
            }
        };
        let mut sub = Vec::new();
        if name != channel {
            sub.push(channel.clone());
        }
        sub.push(link);
        (name, sub.join(" · "), if s.model.is_input(&channel) { "var(--input)" } else { "var(--output)" })
    });
    let item = move |icon: I, label: &'static str, sub: &'static str, next: Dialog| {
        view! {
            <button class="list-item" on:click=move |_| ui.dialog.set(Some(next.clone()))>
                <Icon icon=icon size=22/>
                <div><div>{label}</div><div class="sub">{sub}</div></div>
            </button>
        }
    };
    view! {
        <Modal title=title on_close=Callback::new(move |_| ui.dialog.set(None)) width=420>
            <div class="row muted" style="font-size:12px"><Icon icon=I::GraphicEq size=16 color=color/>{subtitle}</div>
            <div class="col" style="gap:2px">
                {item(I::Edit, "Rename", "Stored on the DSP, max. 8 characters", Dialog::Rename(channel.clone()))}
                {item(I::Copy, "Copy settings to …", "Gain, mute, phase, delay, EQ, filters, dynamics", Dialog::Copy(channel.clone()))}
                {item(I::Link, "Link with …", "Linked channels change together", Dialog::Link(channel.clone()))}
                {(!device.offline).then(|| item(
                    I::VerticalTop,
                    "Max level …",
                    "Panel-side gain limit, not stored on the DSP",
                    Dialog::MaxLevel(channel.clone()),
                ))}
            </div>
        </Modal>
    }
}

#[component]
fn RenameDialog(channel: String) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let current = device.alias(&channel);
    let value = RwSignal::new(if current == channel { String::new() } else { current });
    let ch = channel.clone();
    let save = move || {
        device.set_alias(&ch, &value.get_untracked());
        ui.dialog.set(None);
    };
    let save_click = save.clone();
    let actions = view! {
        <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Cancel"</button>
        <button class="btn filled" on:click=move |_| save_click()>"Save"</button>
    }
    .into_any();
    view! {
        <Modal title=format!("Rename {channel}") on_close=Callback::new(move |_| ui.dialog.set(None)) actions=actions>
            <form on:submit=move |ev| {
                ev.prevent_default();
                save();
            }>
                <div class="field">
                    <label>"Name"</label>
                    <input maxlength="8" autofocus prop:value=move || value.get()
                        on:input=move |ev| value.set(event_target_value(&ev).chars().filter(|c| (' '..='~').contains(c)).take(8).collect())/>
                    <span class="help">"Stored on the DSP (max. 8 characters, no umlauts). Empty = default name."</span>
                </div>
            </form>
        </Modal>
    }
}

#[component]
fn CopyDialog(channel: String) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let app = app();
    let targets = RwSignal::new(BTreeSet::<String>::new());
    let (candidates, from_name) = device.state.with_untracked(|s| {
        let group: Vec<String> = s.model.group_of(&channel).iter().filter(|c| **c != channel).map(|c| c.to_string()).collect();
        (group.into_iter().map(|c| (c.clone(), alias_in(s, &c))).collect::<Vec<_>>(), alias_in(s, &channel))
    });
    let order: Vec<String> = candidates.iter().map(|(c, _)| c.clone()).collect();
    let from = channel.clone();
    let name = from_name.clone();
    let copy = move |_| {
        let selected = targets.get_untracked();
        let list: Vec<String> = order.iter().filter(|c| selected.contains(*c)).cloned().collect();
        let count = list.len();
        device.copy_channel(&from, list);
        ui.dialog.set(None);
        app.toast(format!("Copying {name} to {count} channel(s)"), false);
    };
    let actions = view! {
        <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Cancel"</button>
        <button class="btn filled" prop:disabled=move || targets.with(BTreeSet::is_empty) on:click=copy>"Copy"</button>
    }
    .into_any();
    let note = format!(
        "Overwrites gain, mute, phase, delay, EQ, filters and dynamics of the selected channels. Names, links and \
         matrix routing stay.{}",
        if device.offline { "" } else { " The DSP copies the settings itself; the panel reads them back." }
    );
    view! {
        <Modal title=format!("Copy {from_name} to …") on_close=Callback::new(move |_| ui.dialog.set(None)) width=340 actions=actions>
            <div class="note">{note}</div>
            <div class="col" style="gap:2px">
                {candidates.into_iter().map(|(ch, label)| {
                    let key = ch.clone();
                    let shown = if label == ch { label } else { format!("{label} ({ch})") };
                    view! {
                        <Check
                            checked=Signal::derive(move || targets.with(|t| t.contains(&key)))
                            on_toggle=Callback::new(move |_| targets.update(|t| if !t.remove(&ch) { t.insert(ch.clone()); }))
                            label=shown/>
                    }
                }).collect_view()}
            </div>
        </Modal>
    }
}

/// Link dialog, master/slave like the original editor: [channel] becomes the
/// master of the selected channels. A slave cannot be linked further; its
/// dialog only offers to leave its master.
#[component]
fn LinkDialog(channel: String) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let close = Callback::new(move |_| ui.dialog.set(None));
    let (master, name) = device.state.with_untracked(|s| (ops::master_of(s, &channel), alias_in(s, &channel)));
    if let Some(master) = master {
        let master_name = device.alias(&master);
        let ch = channel.clone();
        let edit_master = master.clone();
        let actions = view! {
            <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Close"</button>
            <button class="btn text" on:click=move |_| ui.dialog.set(Some(Dialog::Link(edit_master.clone())))>
                {format!("Edit {master_name} …")}
            </button>
            <button class="btn filled" on:click=move |_| {
                device.unlink(&ch);
                ui.dialog.set(None);
            }>"Unlink"</button>
        }
        .into_any();
        let master_name = device.alias(&master);
        return view! {
            <Modal title=format!("Link {name}") on_close=close width=380 actions=actions>
                <div style="font-size:13px;line-height:1.5">{format!(
                    "{name} is linked to {master_name} (master): changes of the master apply to it. A linked channel \
                     cannot be linked to other channels – to add more channels, link them to {master_name}."
                )}</div>
            </Modal>
        }
        .into_any();
    }

    let (group, current, candidates, reasons) = device.state.with_untracked(|s| {
        let group: Vec<String> = s.model.group_of(&channel).iter().map(|c| c.to_string()).collect();
        let current = ops::slaves_of(s, &channel);
        let candidates = ops::link_candidates(s, &channel);
        let reasons: Vec<(String, String, Option<String>)> = group
            .iter()
            .filter(|c| **c != channel)
            .map(|c| {
                let reason = (!candidates.contains(c)).then(|| match ops::master_of(s, c) {
                    Some(m) => format!("linked to {}", alias_in(s, &m)),
                    None => format!(
                        "master of {}",
                        ops::slaves_of(s, c).iter().map(|x| alias_in(s, x)).collect::<Vec<_>>().join(", ")
                    ),
                });
                (c.clone(), alias_in(s, c), reason)
            })
            .collect();
        (group, current, candidates, reasons)
    });
    let selection = RwSignal::new(current.iter().cloned().collect::<BTreeSet<String>>());
    let ch = channel.clone();
    let unlink_ch = channel.clone();
    let has_current = !current.is_empty();
    let actions = view! {
        {has_current.then(|| view! {
            <button class="btn text" on:click=move |_| {
                device.unlink(&unlink_ch);
                ui.dialog.set(None);
            }>"Unlink all"</button>
        })}
        <span class="spacer"></span>
        <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Cancel"</button>
        <button class="btn filled" on:click=move |_| {
            let selected = selection.get_untracked();
            device.set_link_slaves(&ch, group.iter().filter(|c| selected.contains(*c)).cloned().collect());
            ui.dialog.set(None);
        }>"Apply"</button>
    }
    .into_any();
    view! {
        <Modal title=format!("Link {name} with …") on_close=close width=380 actions=actions>
            <div class="note">{format!(
                "{name} becomes the master, the selected channels follow it. Like the original editor, newly linked \
                 channels first get the settings of {name} (gain, mute, phase, delay, EQ, filters, dynamics – not \
                 names or routing). The link is stored on the DSP; a change is sent once and the DSP applies it to \
                 the linked channels."
            )}</div>
            <div class="col" style="gap:2px">
                {reasons.into_iter().map(|(c, label, reason)| {
                    let key = c.clone();
                    let enabled = candidates.contains(&c);
                    let shown = match reason {
                        Some(r) => format!("{label} – {r}"),
                        None => label,
                    };
                    view! {
                        <Check
                            checked=Signal::derive(move || selection.with(|s| s.contains(&key)))
                            disabled=Signal::derive(move || !enabled)
                            on_toggle=Callback::new(move |_| if enabled {
                                selection.update(|s| if !s.remove(&c) { s.insert(c.clone()); })
                            })
                            label=shown/>
                    }
                }).collect_view()}
            </div>
        </Modal>
    }
    .into_any()
}
