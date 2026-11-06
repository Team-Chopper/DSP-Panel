//! Dialogs: add/edit DSP (with the network search), settings, and the channel
//! menu with rename, copy (also to other DSPs) and link (with group names).

use std::collections::BTreeSet;
use std::time::Duration;

use dsp_core::api::{COPY_PARTS, DEFAULT_REFRESH_MS};
use dsp_core::panel::LINK_NAME_MAX;
use dsp_core::protocol::{MAX_DEVICE_ID, MIN_DEVICE_ID};
use dsp_core::{DeviceModel, DspInfo, DspStatus, ops};
use leptos::prelude::*;

use crate::components::common::{Check, Modal, Segmented};
use crate::fmt::{format_db_value, parse_number};
use crate::icons::{I, Icon};
use crate::local_settings;
use crate::pages::dsp::{Dialog, ui};
use crate::rta::RtaSettings;
use crate::store::{App, ScanResult, alias_in, app};

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
    let scan_error = RwSignal::new(None::<String>);
    let scan_seq = RwSignal::new(0u32);
    // Newest toast before the search and the error of the previous search: an
    // error the server answers with arrives as a toast (a repeated text is not
    // shown twice, so a visible one with the same text counts too).
    let scan_mark = StoredValue::new((0u64, None::<String>));
    let searching = move || scan_requested.get() && app.scan.with(Option::is_none) && scan_error.with(Option::is_none);
    let search = move || {
        let Some(networks) = typed_networks(&scan_text.get_untracked()) else {
            scan_invalid.set(true);
            return;
        };
        scan_invalid.set(false);
        let newest = app.toasts.with_untracked(|t| t.iter().map(|x| x.id).max().unwrap_or(0));
        scan_mark.set_value((newest, scan_error.get_untracked()));
        scan_error.set(None);
        let seq = scan_seq.get_untracked() + 1;
        scan_seq.set(seq);
        scan_requested.set(true);
        app.scan_network(&networks);
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
    if adding {
        Effect::new(move |_| {
            if !scan_requested.get() || app.scan.with(Option::is_some) || scan_error.with(Option::is_some) {
                return;
            }
            let (newest, previous) = scan_mark.get_value();
            let error = app.toasts.with(|t| {
                t.iter()
                    .find(|x| x.error && (x.id > newest || previous.as_ref() == Some(&x.text)))
                    .map(|x| x.text.clone())
            });
            if error.is_some() {
                scan_error.set(error);
            }
        });
    }
    let scan_results = move || {
        if !scan_requested.get() {
            return ().into_any();
        }
        if let Some(result) = app.scan.get() {
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
                    <button type="button" class="btn outlined" prop:disabled=searching on:click=move |_| search()>
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
/// ready, the DSP [source] first; [from] itself is left out.
fn copy_targets(app: App, source: &str, from: &str, input: bool) -> Vec<CopyTargets> {
    let mut ids = app.dsps.get();
    ids.sort_by_key(|id| id.as_str() != source);
    ids.iter()
        .filter_map(|id| {
            let device = app.device(id)?;
            let (ready, name) = device.info.with(|i| (i.status == DspStatus::Ready, i.name.clone()));
            if !ready {
                return None;
            }
            let channels: Vec<(String, String)> = device.state.with(|s| {
                s.model
                    .channels()
                    .filter(|c| s.model.is_input(c) == input && !(id.as_str() == source && *c == from))
                    .map(|c| (c.to_string(), alias_in(s, c)))
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

/// Members of the link group [master] + [slaves], in channel order ([group]
/// is the channel's group of the model).
fn link_members(group: &[String], master: &str, slaves: &BTreeSet<String>) -> Vec<String> {
    group.iter().filter(|c| c.as_str() == master || slaves.contains(*c)).cloned().collect()
}

/// Link dialog, master/slave like the original editor: [channel] becomes the
/// master of the selected channels. A slave cannot be linked further; its
/// dialog only offers to leave its master. On a DSP the group can be named
/// (kept by the panel server for exactly these members).
#[component]
fn LinkDialog(channel: String) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let app = app();
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
    let current_set: BTreeSet<String> = current.iter().cloned().collect();
    let selection = RwSignal::new(current_set.clone());

    // Group name: the stored name of master + chosen slaves (or, while these
    // have none, the name of the group as it is now, so it carries over when
    // channels are added or removed) until the user types one.
    let dsp_id = device.id();
    let current_members = link_members(&group, &channel, &current_set);
    let stored_name = {
        let (group, master, dsp) = (group.clone(), channel.clone(), dsp_id.clone());
        Memo::new(move |_| {
            let members = selection.with(|s| link_members(&group, &master, s));
            if members.len() < 2 {
                return String::new();
            }
            app.panel.with(|p| {
                p.link_name(&dsp, &members)
                    .or_else(|| p.link_name(&dsp, &current_members))
                    .unwrap_or_default()
                    .to_string()
            })
        })
    };
    let typed_name = RwSignal::new(None::<String>);
    let group_name = move || typed_name.get().unwrap_or_else(|| stored_name.get());

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
            // An unchanged group is not linked again (a rename needs no link job).
            if selected != current_set {
                device.set_link_slaves(&ch, group.iter().filter(|c| selected.contains(*c)).cloned().collect());
            }
            if !device.offline {
                let members = link_members(&group, &ch, &selected);
                let shown = typed_name.get_untracked().unwrap_or_else(|| stored_name.get_untracked());
                let shown: String = shown.trim().chars().take(LINK_NAME_MAX).collect();
                // Compared with the name stored for exactly these members: a
                // carried-over name is stored for the new group too.
                let stored = app.panel.with_untracked(|p| p.link_name(&dsp_id, &members).unwrap_or_default().to_string());
                if members.len() >= 2 && shown != stored {
                    app.set_link_name(&dsp_id, &members, &shown);
                }
            }
            ui.dialog.set(None);
        }>"Apply"</button>
    }
    .into_any();
    let name_field = (!device.offline).then(|| {
        view! {
            <div class="field link-name-field">
                <label>"Group name"</label>
                <input maxlength=LINK_NAME_MAX.to_string() placeholder="e.g. Mains, Subs, Front fill"
                    prop:disabled=move || selection.with(BTreeSet::is_empty)
                    prop:value=group_name
                    on:input=move |ev| typed_name.set(Some(
                        event_target_value(&ev).chars().filter(|c| !c.is_control()).take(LINK_NAME_MAX).collect(),
                    ))/>
                <span class="help">{format!(
                    "Optional, max. {LINK_NAME_MAX} characters. Kept by the panel server for exactly these channels; \
                     empty removes it."
                )}</span>
            </div>
        }
    });
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
            {name_field}
        </Modal>
    }
    .into_any()
}
