//! Dialogs: add/edit DSP (with the network search), settings, and the channel
//! menu with rename, copy (also to other DSPs) and the link matrix.

use std::collections::BTreeSet;
use std::time::Duration;

use dsp_core::api::{COPY_PARTS, DEFAULT_REFRESH_MS};
use dsp_core::protocol::{MAX_DEVICE_ID, MIN_DEVICE_ID};
use dsp_core::{DeviceModel, DspInfo, DspStatus, ops};
use leptos::prelude::*;

use crate::components::common::{Check, Modal, Segmented};
use crate::fmt::{format_db_value, parse_number};
use crate::icons::{I, Icon};
use crate::local_settings;
use crate::pages::dsp::{Dialog, ui};
use crate::rta::RtaSettings;
use crate::store::{App, Device, ScanResult, alias_in, app};

/// TCP port the DSPs listen on (and the network search probes).
const DSP_PORT: i64 = 9761;

fn valid_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c))
}

/// A /24 network as the server reads it: "192.168.1", "192.168.1.0" or
/// "192.168.1.0/24".
fn valid_network(text: &str) -> bool {
    let parts: Vec<&str> = text.trim().trim_end_matches("/24").split('.').collect();
    (3..=4).contains(&parts.len()) && parts.iter().all(|p| p.parse::<u8>().is_ok())
}

/// The networks typed for the search (separated by commas or spaces); `None`
/// if one is not a network. Empty: the server picks its own.
fn typed_networks(text: &str) -> Option<Vec<String>> {
    let list: Vec<String> = text
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    list.iter().all(|n| valid_network(n)).then_some(list)
}

/// Add a DSP ([initial] = None) or edit/delete one.
#[component]
pub fn DspDialog(initial: Option<DspInfo>, on_close: Callback<()>) -> impl IntoView {
    let app = app();
    let adding = initial.is_none();
    let editing = initial.clone();
    let name = RwSignal::new(initial.as_ref().map(|i| i.name.clone()).unwrap_or_default());
    let host = RwSignal::new(initial.as_ref().map_or("192.168.3.100".into(), |i| i.host.clone()));
    let port = RwSignal::new(initial.as_ref().map_or(DSP_PORT, |i| i.port).to_string());
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
            let (p, d, m) = (port_value().unwrap_or(DSP_PORT), id_value().unwrap_or(1), model.get_untracked());
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

    // ─── Network search (only when adding) ───
    let scan_text = RwSignal::new(String::new());
    let scan_invalid = RwSignal::new(false);
    // A search was started from this dialog; its result arrives in app.scan.
    let scan_requested = RwSignal::new(false);
    // No answer at all (the server's own errors come with the result).
    let scan_error = RwSignal::new(None::<String>);
    let scan_seq = RwSignal::new(0u32);
    let searching = move || scan_requested.get() && app.scan.with(Option::is_none) && scan_error.with(Option::is_none);
    let search = move || {
        let Some(networks) = typed_networks(&scan_text.get_untracked()) else {
            scan_invalid.set(true);
            return;
        };
        scan_invalid.set(false);
        if !app.scan_network(&networks) {
            return;
        }
        scan_error.set(None);
        let seq = scan_seq.get_untracked() + 1;
        scan_seq.set(seq);
        scan_requested.set(true);
        // A search of four networks takes about 30 s at most.
        set_timeout(
            move || {
                if scan_seq.try_get_untracked() == Some(seq) && app.scan.with_untracked(Option::is_none) {
                    let _ = scan_error.try_update(|e| {
                        if e.is_none() {
                            *e = Some("No answer from the panel server – please try again".into());
                        }
                    });
                }
            },
            Duration::from_secs(60),
        );
    };
    let scan_results = move || {
        if !scan_requested.get() {
            return ().into_any();
        }
        if let Some(result) = app.scan.get() {
            if let Some(error) = result.error {
                return view! { <div class="scan-status error">{error}</div> }.into_any();
            }
            return scan_list(result, name, host, port, model).into_any();
        }
        match scan_error.get() {
            Some(error) => view! { <div class="scan-status error">{error}</div> }.into_any(),
            None => view! {
                <div class="scan-status"><span class="spinner"></span>"Searching … (this takes a few seconds)"</div>
            }
            .into_any(),
        }
    };
    let scan_box = adding.then(|| {
        view! {
            <div class="scan-box">
                <div class="scan-row">
                    <div class="field grow" class:invalid=move || scan_invalid.get()>
                        <input placeholder="Network (optional), e.g. 192.168.1.0/24"
                            title="Empty: the network of the panel server and those of the added DSPs"
                            prop:value=move || scan_text.get()
                            on:input=move |ev| {
                                scan_text.set(event_target_value(&ev));
                                scan_invalid.set(false);
                            }
                            on:keydown=move |ev| if ev.key() == "Enter" {
                                ev.prevent_default();
                                if !searching() {
                                    search();
                                }
                            }/>
                    </div>
                    <button type="button" class="btn outlined" prop:disabled=searching on:click=move |_| search()
                        title="Connects briefly to every address on TCP port 9761. A DSP that other control software \
                               is connected to may drop that connection – search before the show, not during it.">
                        <Icon icon=I::Search size=18/>"Search network"
                    </button>
                </div>
                {move || scan_invalid.get().then(|| view! {
                    <div class="scan-status error">"Not a network – e.g. 192.168.1.0/24 (up to 4, separated by commas)"</div>
                })}
                {scan_results}
            </div>
        }
    });

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
        <Modal title=title on_close=close width={if adding { 440 } else { 400 }} actions=actions>
            {scan_box}
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

/// The devices a network search found. Clicking one that is free fills the
/// address (and the model, and the name if it is still empty).
fn scan_list(
    result: ScanResult,
    name: RwSignal<String>,
    host: RwSignal<String>,
    port: RwSignal<String>,
    model: RwSignal<String>,
) -> AnyView {
    let networks = result.networks.join(", ");
    if result.found.is_empty() {
        return view! {
            <div class="scan-status">{format!("No DSP answered on port {DSP_PORT} in {networks}")}</div>
        }
        .into_any();
    }
    let head = match result.found.len() {
        1 => format!("1 device in {networks}"),
        n => format!("{n} devices in {networks}"),
    };
    let items = result
        .found
        .into_iter()
        .map(|f| {
            let model_name = f.model.as_deref().map(|m| DeviceModel::by_id(Some(m)).name);
            let mut detail = Vec::new();
            match (model_name, f.info.trim()) {
                (Some(m), "") => detail.push(m.to_string()),
                (Some(m), info) => detail.push(format!("{m} ({info})")),
                (None, "") => {}
                (None, info) => detail.push(format!("{info} (model not supported)")),
            }
            if let Some(known) = &f.known {
                detail.push(format!("already added as \"{known}\""));
            }
            if detail.is_empty() {
                detail.push("No device info".into());
            }
            let address = if f.port == DSP_PORT { f.host.clone() } else { format!("{}:{}", f.host, f.port) };
            let connected = f.connected;
            let title = match (&f.known, connected) {
                (_, true) => "The panel is connected to this DSP".to_string(),
                (Some(known), false) => format!("Already added as \"{known}\" – use this address anyway"),
                (None, false) => "Use this address".to_string(),
            };
            let this_host = f.host.clone();
            view! {
                <button type="button" class="list-item scan-item" title=title prop:disabled=connected
                    class:selected=move || host.with(|h| h.trim() == this_host)
                    on:click=move |_| {
                        host.set(f.host.clone());
                        port.set(f.port.to_string());
                        if let Some(m) = &f.model {
                            model.set(m.clone());
                        }
                        if name.with_untracked(|n| n.trim().is_empty()) {
                            name.set(format!("{} · {}", model_name.unwrap_or("DSP"), f.host));
                        }
                    }>
                    <Icon icon=I::Ethernet size=20/>
                    <div class="grow">
                        <div class="scan-host">{address}</div>
                        <div class="sub">{detail.join(" · ")}</div>
                    </div>
                    {connected.then(|| view! { <span class="scan-badge">"connected"</span> })}
                </button>
            }
        })
        .collect_view();
    view! {
        <div class="scan-head">{head}</div>
        <div class="scan-list">{items}</div>
    }
    .into_any()
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
        Dialog::Link => view! { <LinkDialog/> }.into_any(),
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
    let copy_sub = if device.offline {
        "Gain, mute, phase, delay, EQ, filters, dynamics"
    } else {
        "The whole channel, or chosen parts – also to other DSPs"
    };
    view! {
        <Modal title=title on_close=Callback::new(move |_| ui.dialog.set(None)) width=420>
            <div class="row muted" style="font-size:12px"><Icon icon=I::GraphicEq size=16 color=color/>{subtitle}</div>
            <div class="col" style="gap:2px">
                {item(I::Edit, "Rename", "Stored on the DSP, max. 8 characters", Dialog::Rename(channel.clone()))}
                {item(I::Copy, "Copy settings to …", copy_sub, Dialog::Copy(channel.clone()))}
                {item(I::Link, "Link channels …", "Linked channels change together", Dialog::Link)}
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

// ─── Copy ───

/// The channels of one DSP offered as copy targets: (channel, shown name).
#[derive(Clone, Debug, PartialEq)]
struct CopyTargets {
    dsp: String,
    name: String,
    channels: Vec<(String, String)>,
}

/// Label of a part of [COPY_PARTS] in the copy dialog, and its short name.
fn part_label(part: &str) -> (&'static str, &'static str) {
    match part {
        "level" => ("Level (gain, phase, delay)", "level"),
        "filters" => ("Filters (HPF/LPF)", "filters"),
        "eq" => ("EQ (PEQ, for inputs also GEQ)", "EQ"),
        "dynamics" => ("Dynamics (gate / compressor + limiter)", "dynamics"),
        _ => ("Routing (outputs only: matrix and crossing gains)", "routing"),
    }
}

fn part_allowed(part: &str, input: bool) -> bool {
    COPY_PARTS.contains(&part) && !(input && part == "routing")
}

fn parts_setting(input: bool) -> &'static str {
    if input { "copyParts.in" } else { "copyParts.out" }
}

/// The parts chosen last time in this browser (at first all but routing).
fn load_copy_parts(input: bool) -> BTreeSet<String> {
    match local_settings::get_string(parts_setting(input)) {
        Some(raw) => raw.split(',').map(str::trim).filter(|p| part_allowed(p, input)).map(String::from).collect(),
        None => ["level", "filters", "eq", "dynamics"].into_iter().map(String::from).collect(),
    }
}

fn store_copy_parts(input: bool, parts: &BTreeSet<String>) {
    let list: Vec<&str> = COPY_PARTS.into_iter().filter(|p| parts.contains(*p)).collect();
    local_settings::set_string(parts_setting(input), &list.join(","));
}

/// The channels of the kind of [from] (inputs or outputs) on every DSP that is
/// ready, the DSP [source] first; [from] itself is left out. The channels come
/// from the model the server reports (the client only holds the state of the
/// DSPs it showed); the names from that state where it matches the model.
fn copy_targets(app: App, source: &str, from: &str, input: bool) -> Vec<CopyTargets> {
    let mut ids = app.dsps.get();
    ids.sort_by_key(|id| id.as_str() != source);
    ids.iter()
        .filter_map(|id| {
            let device = app.device(id)?;
            let (ready, name, model) =
                device.info.with(|i| (i.status == DspStatus::Ready, i.name.clone(), DeviceModel::by_id(Some(&i.model))));
            if !ready {
                return None;
            }
            let channels: Vec<(String, String)> = device.state.with(|s| {
                model
                    .channels()
                    .filter(|c| model.is_input(c) == input && !(id.as_str() == source && *c == from))
                    .map(|c| (c.to_string(), if s.model == model { alias_in(s, c) } else { c.to_string() }))
                    .collect()
            });
            (!channels.is_empty()).then(|| CopyTargets { dsp: id.clone(), name, channels })
        })
        .collect()
}

/// The chosen (DSP, channel) pairs that are still offered, in display order.
fn chosen_targets(groups: &[CopyTargets], chosen: &BTreeSet<(String, String)>) -> Vec<(String, String)> {
    groups
        .iter()
        .flat_map(|g| g.channels.iter().map(move |(c, _)| (g.dsp.clone(), c.clone())))
        .filter(|k| chosen.contains(k))
        .collect()
}

/// Copy a channel: by default the device copy (0x2A, everything) to channels
/// of the same DSP; on a DSP also chosen parts to channels of every ready DSP
/// (the server sends the values). The offline editor has the first only.
#[component]
fn CopyDialog(channel: String) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let app = app();
    let targets = RwSignal::new(BTreeSet::<String>::new());
    let (candidates, from_name, input) = device.state.with_untracked(|s| {
        let group: Vec<String> = s.model.group_of(&channel).iter().filter(|c| **c != channel).map(|c| c.to_string()).collect();
        (
            group.into_iter().map(|c| (c.clone(), alias_in(s, &c))).collect::<Vec<_>>(),
            alias_in(s, &channel),
            s.model.is_input(&channel),
        )
    });
    let order: Vec<String> = candidates.iter().map(|(c, _)| c.clone()).collect();
    // 0: device copy on this DSP; 1: chosen parts, also to other DSPs.
    let mode = RwSignal::new(0usize);
    let parts = RwSignal::new(load_copy_parts(input));
    let chosen = RwSignal::new(BTreeSet::<(String, String)>::new());
    let source = device.id();
    let offered = {
        let (source, from) = (source.clone(), channel.clone());
        Memo::new(move |_| if device.offline { Vec::new() } else { copy_targets(app, &source, &from, input) })
    };
    let groups_source = source.clone();

    let from = channel.clone();
    let name = from_name.clone();
    let run = move |_| {
        if mode.get_untracked() == 0 {
            let selected = targets.get_untracked();
            let list: Vec<String> = order.iter().filter(|c| selected.contains(*c)).cloned().collect();
            let count = list.len();
            device.copy_channel(&from, list);
            ui.dialog.set(None);
            app.toast(format!("Copying {name} to {count} channel(s)"), false);
            return;
        }
        let list = offered.with_untracked(|g| chosen.with_untracked(|c| chosen_targets(g, c)));
        let what: Vec<String> = parts.with_untracked(|p| {
            COPY_PARTS.into_iter().filter(|x| part_allowed(x, input) && p.contains(*x)).map(String::from).collect()
        });
        if list.is_empty() || what.is_empty() {
            return;
        }
        let dsps = list.iter().map(|(d, _)| d.as_str()).collect::<BTreeSet<_>>().len();
        app.copy_to(&source, &from, &what, &list);
        ui.dialog.set(None);
        let shown = what.iter().map(|p| part_label(p).1).collect::<Vec<_>>().join(", ");
        let on = if dsps > 1 { format!(" on {dsps} DSPs") } else { String::new() };
        app.toast(format!("Copying {shown} of {name} to {} channel(s){on}", list.len()), false);
    };
    let disabled = move || {
        if mode.get() == 0 {
            targets.with(BTreeSet::is_empty)
        } else {
            parts.with(BTreeSet::is_empty) || offered.with(|g| chosen.with(|c| chosen_targets(g, c).is_empty()))
        }
    };
    let actions = view! {
        <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Cancel"</button>
        <button class="btn filled" prop:disabled=disabled on:click=run>
            {move || if mode.get() == 0 { "Copy" } else { "Apply" }}
        </button>
    }
    .into_any();

    let note = format!(
        "Overwrites gain, mute, phase, delay, EQ, filters and dynamics of the selected channels. Names, links and \
         matrix routing stay.{}",
        if device.offline { "" } else { " The DSP copies the settings itself; the panel reads them back." }
    );
    let device_copy = view! {
        <div class="copy-pane" class:off=move || mode.get() != 0>
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
        </div>
    };

    let parts_copy = (!device.offline).then(|| {
        let checks = COPY_PARTS
            .into_iter()
            .map(|p| {
                let allowed = part_allowed(p, input);
                view! {
                    <Check
                        checked=Signal::derive(move || allowed && parts.with(|s| s.contains(p)))
                        disabled=Signal::derive(move || !allowed)
                        on_toggle=Callback::new(move |_| if allowed {
                            parts.update(|s| if !s.remove(p) { s.insert(p.to_string()); });
                            parts.with_untracked(|s| store_copy_parts(input, s));
                        })
                        label=part_label(p).0/>
                }
            })
            .collect_view();
        let color = if input { "var(--input)" } else { "var(--output)" };
        let source = groups_source;
        let groups = move || {
            let groups = offered.get();
            if groups.is_empty() {
                return view! { <div class="note">"No DSP is ready."</div> }.into_any();
            }
            groups
                .into_iter()
                .map(|g| {
                    let this = g.dsp == source;
                    let all: Vec<(String, String)> = g.channels.iter().map(|(c, _)| (g.dsp.clone(), c.clone())).collect();
                    let all_chosen = {
                        let all = all.clone();
                        move || chosen.with(|s| all.iter().all(|k| s.contains(k)))
                    };
                    let toggle_all = move |_| {
                        chosen.update(|s| {
                            if all.iter().all(|k| s.contains(k)) {
                                for k in &all {
                                    s.remove(k);
                                }
                            } else {
                                s.extend(all.iter().cloned());
                            }
                        })
                    };
                    let chips = g
                        .channels
                        .into_iter()
                        .map(|(ch, label)| {
                            let key = (g.dsp.clone(), ch.clone());
                            let toggle = key.clone();
                            view! {
                                <button type="button" class="chip" style=format!("--chip:{color}") title=ch
                                    class:active=move || chosen.with(|s| s.contains(&key))
                                    on:click=move |_| chosen.update(|s| if !s.remove(&toggle) { s.insert(toggle.clone()); })>
                                    {label}
                                </button>
                            }
                        })
                        .collect_view();
                    view! {
                        <div class="copy-group">
                            <div class="copy-group-head">
                                <span class="copy-group-name">{g.name}</span>
                                {this.then(|| view! { <span class="sub">"this DSP"</span> })}
                                <span class="spacer"></span>
                                <button type="button" class="btn text compact" on:click=toggle_all>
                                    {move || if all_chosen() { "None" } else { "All" }}
                                </button>
                            </div>
                            <div class="copy-chips">{chips}</div>
                        </div>
                    }
                })
                .collect_view()
                .into_any()
        };
        let kind = if input { "inputs" } else { "outputs" };
        view! {
            <div class="copy-pane" class:off=move || mode.get() != 1>
                <div class="note">{format!(
                    "Copies only the chosen parts to the selected {kind}, also on other DSPs (other models get the \
                     bands and inputs both have). Mute, names and links stay. The panel server sends the values."
                )}</div>
                <div class="copy-label">"Parts"</div>
                <div class="copy-parts">{checks}</div>
                <div class="copy-label">"Copy to"</div>
                <div class="copy-targets">{groups}</div>
            </div>
        }
    });
    let mode_switch = (!device.offline).then(|| {
        view! {
            <div class="copy-mode">
                <Segmented labels=vec!["Whole channel, this DSP", "Selected parts, also to other DSPs"]
                    selected=mode on_select=Callback::new(move |i| mode.set(i))/>
            </div>
        }
    });
    let width = if device.offline { 340 } else { 460 };

    view! {
        <Modal title=format!("Copy {from_name} to …") on_close=Callback::new(move |_| ui.dialog.set(None)) width=width actions=actions>
            {mode_switch}
            {device_copy}
            {parts_copy}
        </Modal>
    }
}

// ─── Link ───

/// Pending links of one group (inputs or outputs): per channel (group order)
/// the index of the channel it follows, `None` for a channel of its own or a
/// master.
type Follows = Vec<Option<usize>>;

/// Whether the matrix offers [row] following [col]: like the original editor
/// only a lower channel can be the master (Out1 column: Out2 … Out8), and a
/// channel that follows another cannot be a master itself.
fn can_follow(f: &Follows, row: usize, col: usize) -> bool {
    col < row && f[col].is_none()
}

/// A click on the matrix button of [row] (the channel) in the column of
/// [col] (its master). The diagonal or a set button frees the row; otherwise
/// the row follows [col] and takes its own followers along.
fn follow_click(f: &mut Follows, row: usize, col: usize) {
    if row == col || f[row] == Some(col) {
        f[row] = None;
        return;
    }
    if !can_follow(f, row, col) {
        return;
    }
    for k in f.iter_mut() {
        if *k == Some(row) {
            *k = Some(col);
        }
    }
    f[row] = Some(col);
}

fn followers(f: &Follows, master: usize) -> Vec<usize> {
    (0..f.len()).filter(|k| f[*k] == Some(master)).collect()
}

/// The link jobs (master, slaves) that turn [old] into [new]: first every
/// master releases the slaves it loses, then the masters get their new slaves
/// (a channel is free before it is linked elsewhere).
fn link_steps(group: &[String], old: &Follows, new: &Follows) -> Vec<(String, Vec<String>)> {
    let names = |list: &[usize]| list.iter().map(|k| group[*k].clone()).collect::<Vec<_>>();
    let kept = |m: usize| followers(old, m).into_iter().filter(|k| new[*k] == Some(m)).collect::<Vec<_>>();
    let mut steps = Vec::new();
    for (m, master) in group.iter().enumerate() {
        if kept(m) != followers(old, m) {
            steps.push((master.clone(), names(&kept(m))));
        }
    }
    for (m, master) in group.iter().enumerate() {
        let after = followers(new, m);
        if !after.is_empty() && after != kept(m) {
            steps.push((master.clone(), names(&after)));
        }
    }
    steps
}

/// Members (channel order) of the group of [master] in [f], if it has slaves.
fn group_members(group: &[String], f: &Follows, master: usize) -> Option<Vec<String>> {
    let list = followers(f, master);
    (!list.is_empty()).then(|| {
        group.iter().enumerate().filter(|(k, _)| *k == master || list.contains(k)).map(|(_, c)| c.clone()).collect()
    })
}

/// Link matrix like the original editor ("Link Selection"): for the inputs and
/// for the outputs a grid with a row per channel and a column per channel it
/// can follow (master/slave). Apply runs the needed link jobs one after the
/// other; a changed group keeps the name of its master's group.
#[component]
fn LinkDialog() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let app = app();
    let model = device.model_untracked();
    let groups: [Vec<String>; 2] =
        [model.inputs.iter().map(|c| c.to_string()).collect(), model.outputs.iter().map(|c| c.to_string()).collect()];
    let current: [Follows; 2] = device.state.with_untracked(|s| {
        groups.clone().map(|g| g.iter().map(|c| ops::master_of(s, c).and_then(|m| g.iter().position(|x| *x == m))).collect())
    });
    let pending = [RwSignal::new(current[0].clone()), RwSignal::new(current[1].clone())];
    let changed = {
        let current = current.clone();
        Memo::new(move |_| pending[0].with(|p| *p != current[0]) || pending[1].with(|p| *p != current[1]))
    };

    // The remaining jobs while applying, and (started, total).
    let steps = StoredValue::new(Vec::<(String, Vec<String>)>::new());
    let progress = RwSignal::new(None::<(usize, usize)>);
    let ready = Memo::new(move |_| device.info.with(|i| i.status == DspStatus::Ready));
    let next_step = move || {
        let next = steps.try_update_value(|s| (!s.is_empty()).then(|| s.remove(0))).flatten();
        match next {
            Some((master, slaves)) => {
                progress.update(|p| {
                    if let Some((started, _)) = p {
                        *started += 1;
                    }
                });
                device.set_link_slaves(&master, slaves);
            }
            None => ui.dialog.set(None),
        }
    };
    // On a DSP every link is a job of its own: the next one starts when the
    // DSP is ready again after the previous one.
    Effect::new(move |_| {
        if ready.get() && progress.get_untracked().is_some() {
            next_step();
        }
    });

    let dsp_id = device.id();
    let apply = {
        let (groups, current) = (groups.clone(), current.clone());
        move || {
            let mut all = Vec::new();
            for g in 0..2 {
                let new = pending[g].get_untracked();
                all.extend(link_steps(&groups[g], &current[g], &new));
                if device.offline {
                    continue;
                }
                for m in 0..groups[g].len() {
                    let before = group_members(&groups[g], &current[g], m);
                    let after = group_members(&groups[g], &new, m);
                    let (Some(before), Some(after)) = (before, after) else { continue };
                    let name = app.panel.with_untracked(|p| {
                        p.link_name(&dsp_id, &after).is_none().then(|| p.link_name(&dsp_id, &before).map(String::from)).flatten()
                    });
                    if let Some(name) = name.filter(|_| before != after) {
                        app.set_link_name(&dsp_id, &after, &name);
                    }
                }
            }
            if all.is_empty() {
                ui.dialog.set(None);
            } else if device.offline {
                for (master, slaves) in all {
                    device.set_link_slaves(&master, slaves);
                }
                ui.dialog.set(None);
            } else {
                progress.set(Some((0, all.len())));
                steps.set_value(all);
                next_step();
            }
        }
    };

    let close = Callback::new(move |_| ui.dialog.set(None));
    let busy = move || progress.with(Option::is_some);
    let actions = view! {
        <button class="btn text" prop:disabled=busy on:click=move |_| {
            for p in pending {
                p.update(|f| f.iter_mut().for_each(|x| *x = None));
            }
        }>"Unlink all"</button>
        <span class="spacer"></span>
        {move || progress.get().map(|(started, total)| view! {
            <span class="small muted">{format!("Linking {started}/{total} …")}</span>
        })}
        <button class="btn text" on:click=move |_| ui.dialog.set(None)>"Close"</button>
        <button class="btn filled" prop:disabled=move || !changed.get() || busy() on:click=move |_| apply()>"Apply"</button>
    }
    .into_any();
    let blocks = [("Inputs", "var(--input)"), ("Outputs", "var(--output)")]
        .into_iter()
        .enumerate()
        .map(|(g, (title, color))| {
            view! { <LinkMatrix device=device group=groups[g].clone() follows=pending[g] color=color title=title/> }
        })
        .collect_view();
    view! {
        <Modal title="Link channels" on_close=close width=1000 actions=actions>
            <div class="note">
                "Like the original editor: each column is a master, the lit button of a channel shows the column it \
                 belongs to. A channel can only follow a lower one, and a linked channel cannot be a master. A click on \
                 a channel in its own column or on a lit button releases it. Newly linked channels first \
                 get the settings of their master (gain, mute, phase, delay, EQ, filters, dynamics – not names or \
                 routing); afterwards a change of the master applies to the whole group. The links are stored on the DSP."
            </div>
            <div class="link-matrices">{blocks}</div>
        </Modal>
    }
}

/// The matrix of one group, laid out like the original editor: a column per
/// master with a button for itself and every higher channel; the lit button
/// of a channel shows the column it belongs to (its own column: not linked,
/// or master).
#[component]
fn LinkMatrix(
    device: Device,
    group: Vec<String>,
    follows: RwSignal<Follows>,
    color: &'static str,
    title: &'static str,
) -> impl IntoView {
    let n = group.len();
    let names: Vec<String> = group.iter().map(|c| device.alias(c)).collect();
    let columns = (0..n)
        .map(|col| {
            let cells = (0..n)
                .map(|row| {
                    // A link to a higher master (older panel versions) is shown so it can be released.
                    let shown = move || row >= col || follows.with(|f| f[row] == Some(col));
                    let on = move || follows.with(|f| if row == col { f[row].is_none() } else { f[row] == Some(col) });
                    let offered = move || row == col || follows.with(|f| f[row] == Some(col) || can_follow(f, row, col));
                    let tip = if row == col {
                        format!("{} not linked (or master of its group)", names[row])
                    } else {
                        format!("{} follows {}", names[row], names[col])
                    };
                    let name = names[row].clone();
                    view! {
                        <div class="lm-slot">
                            <Show when=shown>
                                <button class="lm-cell" class:on=on prop:disabled=move || !offered() title=tip.clone()
                                    on:click=move |_| follows.update(|f| follow_click(f, row, col))>
                                    {name.clone()}
                                </button>
                            </Show>
                        </div>
                    }
                })
                .collect_view();
            view! {
                <div class="lm-column">
                    <div class="lm-head" title=names[col].clone()>{names[col].clone()}</div>
                    {cells}
                </div>
            }
        })
        .collect_view();
    view! {
        <div class="link-matrix" style=format!("--gc:{color}")>
            <div class="lm-title">{title}</div>
            <div class="lm-columns">{columns}</div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> Vec<String> {
        ["Out1", "Out2", "Out3", "Out4"].iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn clicks_keep_master_slave_rules() {
        let mut f: Follows = vec![None; 4];
        follow_click(&mut f, 2, 1); // Out3 follows Out2
        follow_click(&mut f, 3, 2); // Out4 → Out3, a slave: refused
        follow_click(&mut f, 0, 1); // Out1 → Out2, a higher channel: refused
        assert_eq!(f, vec![None, None, Some(1), None]);
        follow_click(&mut f, 1, 0); // the master Out2 follows Out1: its slave comes along
        assert_eq!(f, vec![None, Some(0), Some(0), None]);
        follow_click(&mut f, 3, 0);
        follow_click(&mut f, 2, 2); // the diagonal releases
        follow_click(&mut f, 3, 0); // so does a set button
        assert_eq!(f, vec![None, Some(0), None, None]);
    }

    #[test]
    fn steps_release_before_linking() {
        let old = vec![None, Some(0), None, None]; // Out1 + Out2
        let new = vec![None, None, Some(1), None]; // Out2 + Out3
        let steps = link_steps(&group(), &old, &new);
        assert_eq!(steps, vec![("Out1".into(), vec![]), ("Out2".into(), vec!["Out3".to_string()])]);
        // Adding a slave: one job with the whole group, nothing to release.
        let steps = link_steps(&group(), &old, &vec![None, Some(0), Some(0), None]);
        assert_eq!(steps, vec![("Out1".into(), vec!["Out2".to_string(), "Out3".to_string()])]);
        assert!(link_steps(&group(), &old, &old).is_empty());
        assert_eq!(group_members(&group(), &new, 1), Some(vec!["Out2".to_string(), "Out3".to_string()]));
    }
}
