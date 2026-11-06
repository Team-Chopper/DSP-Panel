//! Presets dialog: the 20 presets stored on the DSP (U01–U20) and the preset
//! library on the server. Server presets can be loaded onto a DSP of the same
//! model, saved from the current settings and opened in an offline editor.

use dsp_core::api::msg;
use dsp_core::{DeviceModel, DspStatus, ServerPresetInfo};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsValue;

use crate::components::common::{MenuButton, Modal};
use crate::icons::{I, Icon};
use crate::pages::dsp::ui;
use crate::store::{Device, app};

/// "U01" for index 0.
fn slot(index: i64) -> String {
    format!("U{:02}", index + 1)
}

/// dd.mm.yyyy hh:mm in local time.
fn format_date(ms: i64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    format!(
        "{:02}.{:02}.{} {:02}:{:02}",
        d.get_date(),
        d.get_month() + 1,
        d.get_full_year(),
        d.get_hours(),
        d.get_minutes()
    )
}

/// A follow-up question of the presets dialogs.
#[derive(Clone)]
enum Ask {
    Name { title: String, initial: String, max_length: u32, action: &'static str, hint: Option<String>, done: Callback<String> },
    Apply { preset: ServerPresetInfo, dsp_id: String, dsp_name: String, done: Callback<()> },
    Delete { preset: ServerPresetInfo },
    /// Back to the values before the last server preset load; [label] is the
    /// newest kept state, [steps] how many are kept.
    Undo { dsp_id: String, dsp_name: String, label: String, steps: usize, done: Callback<()> },
}

/// The question dialogs on top of a presets dialog.
#[component]
fn AskDialog(ask: RwSignal<Option<Ask>>) -> impl IntoView {
    let close = Callback::new(move |_| ask.set(None));
    move || {
        ask.get().map(|a| match a {
            Ask::Name { title, initial, max_length, action, hint, done } => {
                let input = NodeRef::<html::Input>::new();
                let submit = move || {
                    let name = input.get_untracked().map(|i| i.value().trim().to_string()).unwrap_or_default();
                    ask.set(None);
                    if !name.is_empty() {
                        done.run(name);
                    }
                };
                Effect::new(move |_| {
                    if let Some(el) = input.get() {
                        let _ = el.focus();
                        el.select();
                    }
                });
                view! {
                    <Modal title=title on_close=close actions=view! {
                        <button class="btn text" on:click=move |_| ask.set(None)>"Cancel"</button>
                        <button class="btn filled" on:click=move |_| submit()>{action}</button>
                    }.into_any()>
                        {hint.map(|h| view! { <div class="note">{h}</div> })}
                        <div class="field">
                            <label>"Name"</label>
                            <input node_ref=input value=initial maxlength=max_length
                                on:keydown=move |ev| if ev.key() == "Enter" { submit() }/>
                        </div>
                    </Modal>
                }
                .into_any()
            }
            Ask::Apply { preset, dsp_id, dsp_name, done } => {
                let id = preset.id.clone();
                // The server keeps the values before only if it could read them.
                let undo = if app().device(&dsp_id).is_none_or(|d| d.info.with_untracked(|i| i.config_readable)) {
                    "The load can be undone with \"Undo last load\" in the presets dialog of the DSP."
                } else {
                    "The configuration of this DSP could not be read, so this load cannot be undone."
                };
                view! {
                    <Modal title="Load server preset" on_close=close actions=view! {
                        <button class="btn text" on:click=move |_| ask.set(None)>"Cancel"</button>
                        <button class="btn filled" on:click=move |_| {
                            ask.set(None);
                            app().apply_server_preset(&id, &dsp_id);
                            done.run(());
                        }>"Load"</button>
                    }.into_any()>
                        <div class="note-text">{format!("Load \"{}\" onto {dsp_name}?", preset.name)}</div>
                        <div class="note-text">
                            "Only the values that differ are sent (all of them if the channel links differ), then the DSP is read back and checked. "
                            {undo}
                        </div>
                        <div class="note">"The presets U01–U20 stored on the DSP are not changed."</div>
                    </Modal>
                }
                .into_any()
            }
            Ask::Undo { dsp_id, dsp_name, label, steps, done } => {
                let more = match steps.saturating_sub(1) {
                    0 => String::new(),
                    1 => " 1 more step can be undone afterwards.".to_string(),
                    n => format!(" {n} more steps can be undone afterwards."),
                };
                view! {
                    <Modal title="Undo last load" on_close=close actions=view! {
                        <button class="btn text" on:click=move |_| ask.set(None)>"Cancel"</button>
                        <button class="btn filled" on:click=move |_| {
                            ask.set(None);
                            app().undo_preset_load(&dsp_id);
                            done.run(());
                        }>"Undo"</button>
                    }.into_any()>
                        <div class="note-text">{format!("Undo the last preset load on {dsp_name}?")}</div>
                        <div class="undo-ask-step"><Icon icon=I::Undo size=18/><span>{label}</span></div>
                        <div class="note">
                            {format!("Only the values that differ are sent, then the DSP is read back and checked. This step cannot be redone.{more}")}
                        </div>
                    </Modal>
                }
                .into_any()
            }
            Ask::Delete { preset } => {
                let id = preset.id.clone();
                view! {
                    <Modal title="Delete preset" on_close=close actions=view! {
                        <button class="btn text" on:click=move |_| ask.set(None)>"Cancel"</button>
                        <button class="btn danger" on:click=move |_| {
                            ask.set(None);
                            app().delete_server_preset(&id);
                        }>"Delete"</button>
                    }.into_any()>
                        <div class="note-text">{format!("Delete \"{}\" from the server? This cannot be undone.", preset.name)}</div>
                    </Modal>
                }
                .into_any()
            }
        })
    }
}

#[component]
fn Empty(icon: I, text: &'static str) -> impl IntoView {
    view! {
        <div class="empty-state">
            <Icon icon=icon size=32/>
            <div>{text}</div>
        </div>
    }
}

#[component]
pub fn PresetsDialog() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let close = Callback::new(move |_| ui.dialog.set(None));
    let tab = RwSignal::new(0usize);
    let ask = RwSignal::new(None::<Ask>);
    let offline = device.offline;
    let tabs = (!offline).then(|| {
        view! {
            <div class="tabs">
                <button class="tab" class:selected=move || tab.get() == 0 on:click=move |_| tab.set(0)>"On the DSP"</button>
                <button class="tab" class:selected=move || tab.get() == 1 on:click=move |_| tab.set(1)>"On the server"</button>
            </div>
        }
    });
    let body = move || {
        if !offline && tab.get() == 0 {
            view! { <DevicePresets device=device ask=ask close=close/> }.into_any()
        } else {
            view! { <ServerPresets device=device ask=ask close=close/> }.into_any()
        }
    };
    view! {
        <div class="modal-backdrop" on:pointerdown=move |ev| {
            if ev.target() == ev.current_target() {
                close.run(());
            }
        }>
            <div class="modal presets-dialog" on:keydown=move |ev| if ev.key() == "Escape" && ask.get_untracked().is_none() { close.run(()) }>
                <div class="presets-head">
                    <Icon icon=I::Library size=20 color="var(--accent)"/>
                    <span class="grow">{move || format!("Presets · {}", device.info.with(|i| i.name.clone()))}</span>
                    <button class="icon-button" title="Close" on:click=move |_| close.run(())><Icon icon=I::Close size=20/></button>
                </div>
                {tabs}
                <div class="presets-body">{body}</div>
                {(!offline).then(|| view! { <UndoLoad device=device ask=ask close=close/> })}
            </div>
        </div>
        <AskDialog ask=ask/>
    }
}

/// One line of a preset list.
#[component]
fn PresetRow(
    #[prop(default = None)] leading: Option<String>,
    title: String,
    #[prop(default = None)] subtitle: Option<String>,
    current: bool,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="preset-row" class:current=current>
            {leading.map(|l| view! { <span class="preset-slot">{l}</span> })}
            <div class="preset-text">
                <div class="preset-title">{if title.is_empty() { "—".to_string() } else { title }}</div>
                {subtitle.map(|s| view! { <div class="preset-sub">{s}</div> })}
            </div>
            {current.then(|| view! { <span class="preset-current">"current"</span> })}
            {children()}
        </div>
    }
}

// ─── Undo the last server preset load ───

/// The server keeps the DSP's values before each server preset load (newest
/// last); this goes back one step.
#[component]
fn UndoLoad(device: Device, ask: RwSignal<Option<Ask>>, close: Callback<()>) -> impl IntoView {
    let undo = Memo::new(move |_| device.info.with(|i| i.undo.clone()));
    let ready = Memo::new(move |_| device.info.with(|i| i.status == DspStatus::Ready));
    let steps = move || undo.with(Vec::len);
    let label = move || undo.with(|u| u.last().cloned()).unwrap_or_else(|| "No preset load to undo".into());
    let sub = move || match steps() {
        0 => "Loading a server preset keeps the values from before.".to_string(),
        1 => "1 step kept".to_string(),
        n => format!("{n} steps kept"),
    };
    let hint = move || {
        if !ready.get() {
            "The DSP is not ready"
        } else {
            "Back to the values before the last server preset load"
        }
    };
    let start = move |_| {
        let Some(label) = undo.with_untracked(|u| u.last().cloned()) else { return };
        ask.set(Some(Ask::Undo {
            dsp_id: device.id(),
            dsp_name: device.info.with_untracked(|i| i.name.clone()),
            label,
            steps: undo.with_untracked(Vec::len),
            done: close,
        }));
    };
    view! {
        <div class="preset-undo">
            <div class="preset-text">
                <div class="preset-title" class:muted=move || steps() == 0>{label}</div>
                <div class="preset-sub">{sub}</div>
            </div>
            <button class="btn outlined compact" title=hint prop:disabled=move || !ready.get() || steps() == 0 on:click=start>
                <Icon icon=I::Undo size=16/>"Undo last load"
            </button>
        </div>
    }
}

// ─── Presets stored on the DSP ───

#[component]
fn DevicePresets(device: Device, ask: RwSignal<Option<Ask>>, close: Callback<()>) -> impl IntoView {
    move || {
        if !device.is_connected() {
            return view! { <Empty icon=I::LinkOff text="Connect the DSP to load or store its presets."/> }.into_any();
        }
        let (entries, slots, number, current) = device.state.with(|s| {
            (
                s.presets.iter().map(|(k, v)| (*k, v.clone())).collect::<Vec<_>>(),
                s.preset_slots.clone(),
                s.current_preset_number,
                s.current_preset.clone(),
            )
        });
        if entries.is_empty() {
            return view! { <Empty icon=I::Timer text="No presets read from the DSP."/> }.into_any();
        }
        let rows = entries
            .into_iter()
            .map(|(index, name)| {
                // The slot table (0x22) tells which slots hold a stored preset.
                let subtitle = (!slots.is_empty() && !slots.contains(&index)).then(|| "empty".to_string());
                // The DSP reports the current preset number (0x14); older
                // states only have the name from the dump.
                let is_current = if number > 0 { number == index + 1 } else { name == current };
                let initial = name.clone();
                let store = move |_| {
                    let initial = initial.clone();
                    ask.set(Some(Ask::Name {
                        title: format!("Store in {}", slot(index)),
                        initial,
                        max_length: 14,
                        action: "Save",
                        hint: Some(format!("The current settings replace preset {} on the DSP.", slot(index))),
                        done: Callback::new(move |name: String| device.save_preset(index as usize, &name)),
                    }));
                };
                view! {
                    <PresetRow leading=Some(slot(index)) title=name subtitle=subtitle current=is_current>
                        <button class="btn text compact" on:click=move |_| {
                            device.load_preset(index as usize);
                            close.run(());
                        }><Icon icon=I::Download size=16/>"Load"</button>
                        <button class="icon-button" title=format!("Store current settings in {}", slot(index)) on:click=store>
                            <Icon icon=I::Save size=18/>
                        </button>
                    </PresetRow>
                }
            })
            .collect_view();
        view! { <div class="preset-list">{rows}</div> }.into_any()
    }
}

// ─── Presets stored on the server ───

#[component]
fn ServerPresets(device: Device, ask: RwSignal<Option<Ask>>, close: Callback<()>) -> impl IntoView {
    let app = app();
    let ui = ui();
    let offline = device.offline;
    let model = Memo::new(move |_| DeviceModel::by_id(Some(&device.info.with(|i| i.model.clone()))));
    let connected = move || device.is_connected();
    let can_save = move || !offline && connected() && device.info.with(|i| i.config_readable);

    let save_current = move |_| {
        let name = device.info.with_untracked(|i| i.name.clone());
        ask.set(Some(Ask::Name {
            title: "Save to server".into(),
            initial: device.state.with_untracked(|s| s.current_preset.clone()),
            max_length: 40,
            action: "Save",
            hint: Some(format!("Saves all settings of {name} as a new server preset.")),
            done: Callback::new(move |preset: String| app.save_preset_from_dsp(&device.id(), &preset)),
        }));
    };
    let new_preset = move |_| {
        let m = model.get_untracked();
        ask.set(Some(Ask::Name {
            title: format!("New {} preset", m.name),
            initial: String::new(),
            max_length: 40,
            action: "Create",
            hint: Some("Starts with the default settings.".into()),
            done: Callback::new(move |name: String| app.create_server_preset(&name, &m)),
        }));
    };
    let note = move || {
        let name = model.get().name;
        let unreadable = if !offline && connected() && !can_save() {
            "The configuration of this DSP could not be read, so it cannot be saved. "
        } else {
            ""
        };
        format!("Only presets for the {name} are listed. {unreadable}Open a preset to edit it without a DSP.")
    };

    let list = move || {
        let mut presets = app.presets_for(&model.get());
        presets.sort_by_key(|p| p.name.to_lowercase());
        if presets.is_empty() {
            return view! { <Empty icon=I::Cloud text="No server presets for this model yet."/> }.into_any();
        }
        let rows = presets
            .into_iter()
            .map(|p| {
                let current = offline && device.id() == format!("{}{}", msg::PRESET_PREFIX, p.id);
                let load = if offline {
                    let id = p.id.clone();
                    view! {
                        <button class="btn text compact" on:click=move |_| {
                            ui.dialog.set(Some(crate::pages::dsp::Dialog::ApplyPreset(id.clone())));
                        }><Icon icon=I::Send size=16/>"To DSP …"</button>
                    }
                    .into_any()
                } else {
                    let preset = p.clone();
                    view! {
                        <button class="btn text compact" prop:disabled=move || !connected() on:click=move |_| {
                            ask.set(Some(Ask::Apply {
                                preset: preset.clone(),
                                dsp_id: device.id(),
                                dsp_name: device.info.with_untracked(|i| i.name.clone()),
                                done: close,
                            }));
                        }><Icon icon=I::Download size=16/>"Load"</button>
                    }
                    .into_any()
                };
                let edit_id = p.id.clone();
                let (rename, delete) = (p.clone(), p.clone());
                let menu = Signal::derive(move || {
                    let (rename, delete) = (rename.clone(), delete.clone());
                    vec![
                        (
                            I::Edit,
                            "Rename".to_string(),
                            Callback::new(move |_| {
                                let id = rename.id.clone();
                                ask.set(Some(Ask::Name {
                                    title: "Rename preset".into(),
                                    initial: rename.name.clone(),
                                    max_length: 40,
                                    action: "Rename",
                                    hint: None,
                                    done: Callback::new(move |name: String| app.rename_server_preset(&id, &name)),
                                }));
                            }),
                        ),
                        (I::Delete, "Delete".to_string(), Callback::new(move |_| ask.set(Some(Ask::Delete { preset: delete.clone() })))),
                    ]
                });
                view! {
                    <PresetRow title=p.name.clone() subtitle=Some(format!("changed {}", format_date(p.updated))) current=current>
                        {load}
                        <button class="icon-button" title="Edit offline" on:click=move |_| {
                            close.run(());
                            app.open_editor(&edit_id);
                        }><Icon icon=I::Edit size=18/></button>
                        <MenuButton icon=I::More title="More" items=menu/>
                    </PresetRow>
                }
            })
            .collect_view();
        view! { <div class="preset-list">{rows}</div> }.into_any()
    };

    view! {
        <div class="preset-tools">
            {(!offline).then(|| view! {
                <button class="btn filled compact" prop:disabled=move || !can_save() on:click=save_current>
                    <Icon icon=I::Cloud size=18/>"Save current settings"
                </button>
            })}
            <button class="btn outlined compact" on:click=new_preset>
                <Icon icon=I::Add size=18/>"New empty preset"
            </button>
        </div>
        <div class="preset-note">{note}</div>
        {list}
    }
}

// ─── Load a server preset from the offline editor ───

/// Asks for a connected DSP of the preset's model and loads the preset onto it.
#[component]
pub fn ApplyPresetDialog(preset_id: String) -> impl IntoView {
    let ui = ui();
    let app = app();
    let close = Callback::new(move |_| ui.dialog.set(None));
    let ask = RwSignal::new(None::<Ask>);
    let preset = app.presets.with_untracked(|l| l.iter().find(|p| p.id == preset_id).cloned());
    let Some(preset) = preset else {
        return view! {
            <Modal title="Load onto DSP" on_close=close>
                <div class="note-text">"The preset no longer exists."</div>
            </Modal>
        }
        .into_any();
    };
    let model = DeviceModel::by_id(Some(&preset.model));
    let title = format!("Load \"{}\" onto …", preset.name);
    let targets = move || {
        let preset = preset.clone();
        let devices: Vec<Device> = app
            .dsps
            .get()
            .iter()
            .filter_map(|id| app.device(id))
            .filter(|d| d.info.with(|i| i.model == preset.model))
            .collect();
        if devices.is_empty() {
            return view! { <div class="note-text">{format!("No {} is configured.", model.name)}</div> }.into_any();
        }
        devices
            .into_iter()
            .map(|d| {
                let preset = preset.clone();
                let (name, host, port, status) = d.info.with(|i| (i.name.clone(), i.host.clone(), i.port, i.status));
                let connected = status == DspStatus::Ready;
                view! {
                    <button class="list-item" prop:disabled=!connected on:click=move |_| {
                        ask.set(Some(Ask::Apply {
                            preset: preset.clone(),
                            dsp_id: d.id(),
                            dsp_name: name.clone(),
                            done: close,
                        }));
                    }>
                        <span class="status-dot" style=format!("background:{}", if connected { "var(--accent)" } else { "var(--danger)" })></span>
                        <div>
                            <div>{d.info.with(|i| i.name.clone())}</div>
                            <div class="sub">{if connected { format!("{host}:{port}") } else { "not connected".to_string() }}</div>
                        </div>
                    </button>
                }
            })
            .collect_view()
            .into_any()
    };
    view! {
        <Modal title=title on_close=close actions=view! {
            <button class="btn text" on:click=move |_| close.run(())>"Cancel"</button>
        }.into_any()>
            <div class="col">{targets}</div>
            <div class="note">
                "Only the values that differ are sent, then the DSP is read back and checked. "
                "The load can be undone in the presets dialog of the DSP."
            </div>
        </Modal>
        <AskDialog ask=ask/>
    }
    .into_any()
}
