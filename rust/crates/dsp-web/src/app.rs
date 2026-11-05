//! Root of the web client: login page or the panel with one tab per DSP and
//! one per server preset open in the offline editor.

use dsp_core::DeviceModel;
use dsp_core::api::{DspStatus, msg};
use leptos::prelude::*;

use crate::components::common::Modal;
use crate::dialogs::DspDialog;
use crate::icons::{I, Icon};
use crate::net;
use crate::pages::dsp::DspPage;
use crate::store::{App, ServerState, app};

#[component]
pub fn Root() -> impl IntoView {
    let app = App::init();
    // Panel-wide audio state, owned by the root so it lives as long as the panel.
    crate::rta::RtaSettings::get();
    crate::tone::ToneGenerator::get();
    crate::read_only::install();
    net::start();
    view! {
        {move || match app.server.get() {
            ServerState::Checking => view! { <div class="login"><div class="spinner big"></div></div> }.into_any(),
            ServerState::LoginRequired => view! { <LoginPage/> }.into_any(),
            _ => view! { <PanelPage/> }.into_any(),
        }}
        <Toasts/>
    }
}

#[component]
fn LoginPage() -> impl IntoView {
    let app = app();
    let password = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let submit = move || {
        let pw = password.get_untracked();
        if pw.is_empty() || busy.get_untracked() {
            return;
        }
        busy.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            let result = net::login(pw).await;
            busy.set(false);
            if result.is_some() {
                password.set(String::new());
            }
            error.set(result);
        });
    };
    view! {
        <div class="login">
            <form class="login-card" on:submit=move |ev| {
                ev.prevent_default();
                submit();
            }>
                <h1>"DSP Panel"</h1>
                <Show when=move || !app.password_set.get()>
                    <div class="note">
                        "No password has been set on the server yet."<br/>
                        "Run \"dsp-server passwd\" on the server first."
                    </div>
                </Show>
                <div class="field" class:invalid=move || error.get().is_some()>
                    <label for="password">"Password"</label>
                    <input id="password" type="password" autocomplete="current-password" autofocus
                        prop:value=move || password.get()
                        prop:disabled=move || busy.get()
                        on:input=move |ev| password.set(event_target_value(&ev))/>
                    {move || error.get().map(|e| view! { <span class="error">{e}</span> })}
                </div>
                <button class="btn" type="submit" prop:disabled=move || busy.get()>
                    {move || if busy.get() { view! { <span class="spinner"></span> }.into_any() } else { "Log in".into_any() }}
                </button>
            </form>
        </div>
    }
}

pub fn status_color(status: DspStatus) -> &'static str {
    match status {
        DspStatus::Ready => "var(--accent)",
        DspStatus::Connecting | DspStatus::Initializing | DspStatus::Busy => "var(--warning)",
        DspStatus::Error | DspStatus::Disconnected => "var(--danger)",
    }
}

fn model_short(model: &str) -> String {
    DeviceModel::by_id(Some(model)).name.replace("DSP ", "")
}

#[component]
fn PanelPage() -> impl IntoView {
    let app = app();
    let adding = RwSignal::new(false);

    // Keep the selection valid when DSPs are added or removed.
    Effect::new(move |_| {
        let dsps = app.dsps.get();
        let editors = app.editors.get();
        let selected = app.selected.get();
        let valid = selected.as_ref().is_some_and(|s| dsps.contains(s) || editors.contains(s));
        if !valid {
            let first = dsps.first().or(editors.first()).cloned();
            if first != selected {
                app.select(first);
            }
        }
    });

    let menu_items = Signal::derive(move || {
        let mut items = vec![(I::Add, "Add DSP".to_string(), Callback::new(move |_| adding.set(true)))];
        if app.auth_enabled.get() {
            items.push((
                I::Logout,
                "Log out of the panel".to_string(),
                Callback::new(|_| leptos::task::spawn_local(net::logout())),
            ));
        }
        items
    });

    let dsp_tabs = move || {
        app.dsps
            .get()
            .into_iter()
            .filter_map(|id| app.device(&id))
            .map(|device| {
                let id = device.id();
                let selected = {
                    let id = id.clone();
                    move || app.selected.get().as_deref() == Some(id.as_str())
                };
                view! {
                    <button class="dsp-tab" class:selected=selected on:click=move |_| app.select(Some(id.clone()))>
                        <span class="status-dot" style=move || format!("background:{}", status_color(device.info.with(|i| i.status)))></span>
                        {move || device.info.with(|i| format!("{} · {}", i.name, model_short(&i.model)))}
                    </button>
                }
            })
            .collect_view()
    };
    let editor_tabs = move || {
        app.editors
            .get()
            .into_iter()
            .filter_map(|key| app.device(&key))
            .map(|device| {
                let key = device.id();
                let preset_id = key[msg::PRESET_PREFIX.len()..].to_string();
                let selected = {
                    let key = key.clone();
                    move || app.selected.get().as_deref() == Some(key.as_str())
                };
                view! {
                    <button class="dsp-tab closable" class:selected=selected on:click=move |_| app.select(Some(key.clone()))>
                        <Icon icon=I::Cloud size=14 color="var(--info)"/>
                        {move || device.info.with(|i| format!("{} · {}", i.name, model_short(&i.model)))}
                        <span class="sub">"offline"</span>
                        <span class="icon-button small" title="Close editor" on:click=move |ev| {
                            ev.stop_propagation();
                            app.close_editor(&preset_id);
                        }>
                            <Icon icon=I::Close size=14/>
                        </span>
                    </button>
                }
            })
            .collect_view()
    };
    let has_tabs = move || !app.dsps.with(Vec::is_empty) || !app.editors.with(Vec::is_empty);

    view! {
        <div class="app">
            <header class="appbar">
                <div class="appbar-top">
                    <div class="logo"><Icon icon=I::GraphicEq size=18/></div>
                    <span class="app-title">"DSP Panel"</span>
                    <span class="spacer"></span>
                    <ReadOnlyToggle/>
                    <Show when=move || !app.dsps.with(Vec::is_empty)>
                        <MuteAllButton/>
                    </Show>
                    <crate::components::common::MenuButton icon=I::Menu title="Menu" items=menu_items/>
                </div>
                <Show when=has_tabs>
                    <nav class="dsp-tabs">
                        {dsp_tabs}
                        <button class="icon-button add-tab" title="Add DSP" on:click=move |_| adding.set(true)>
                            <Icon icon=I::Add size=18/>
                        </button>
                        <Show when=move || !app.editors.with(Vec::is_empty)>
                            <span class="tabs-sep"></span>
                        </Show>
                        {editor_tabs}
                    </nav>
                </Show>
            </header>
            <Show when=move || app.server.get() != ServerState::Connected>
                <div class="server-banner">
                    <span class="spinner"></span>
                    {move || app.server_error.get().unwrap_or_else(|| "Connecting to server …".into())}
                </div>
            </Show>
            {move || match app.selected.get().and_then(|id| app.device(&id)) {
                Some(device) => view! { <DspPage device=device/> }.into_any(),
                None => view! { <EmptyState on_add=Callback::new(move |_| adding.set(true))/> }.into_any(),
            }}
            <Show when=move || adding.get()>
                <DspDialog initial=None on_close=Callback::new(move |_| adding.set(false))/>
            </Show>
        </div>
    }
}

/// View-only switch of this browser tab (see read_only.rs).
#[component]
fn ReadOnlyToggle() -> impl IntoView {
    let app = app();
    let on = app.read_only;
    view! {
        <button class="btn compact read-only-toggle" class:on=move || on.get()
            title=move || if on.get() {
                "View only: nothing on the page can be changed in this browser tab. Click to allow changes."
            } else {
                "Changes allowed. Click to switch this browser tab to view only."
            }
            on:click=move |_| on.update(|v| *v = !*v)>
            {move || view! { <Icon icon=if on.get() { I::Lock } else { I::Edit } size=18/> }}
            <span class="wide-only">{move || if on.get() { "View only" } else { "Edit" }}</span>
        </button>
    }
}

/// Mutes every channel of every DSP at once; releasing (after a confirmation)
/// opens only the channels that were open before. Any client can do both.
#[component]
fn MuteAllButton() -> impl IntoView {
    let app = app();
    let on = app.mute_all;
    let confirm = RwSignal::new(false);
    view! {
        <button class="btn compact mute-all" class:on=move || on.get()
            title=move || if on.get() {
                "All DSPs are muted. Click to release: only the channels that were open before are unmuted."
            } else {
                "Mute every channel of every DSP"
            }
            on:click=move |_| if on.get_untracked() { confirm.set(true) } else { app.set_mute_all(true) }>
            <Icon icon=I::VolumeOff size=18/>
            <span>{move || if on.get() { "All muted" } else { "Mute all" }}</span>
        </button>
        <Show when=move || confirm.get()>
            <Modal title="Release mute all?" on_close=Callback::new(move |_| confirm.set(false)) actions=view! {
                <button class="btn text" on:click=move |_| confirm.set(false)>"Cancel"</button>
                <button class="btn filled" on:click=move |_| {
                    confirm.set(false);
                    app.set_mute_all(false);
                }>"Release"</button>
            }.into_any()>
                <div class="note-text">
                    "The channels that were open before mute all are unmuted again; channels that were muted stay muted. \
                     A DSP that is offline now is unmuted when it connects again."
                </div>
            </Modal>
        </Show>
    }
}

#[component]
fn EmptyState(on_add: Callback<()>) -> impl IntoView {
    let app = app();
    view! {
        <div class="empty">
            {move || if !app.loaded.get() {
                view! { <div class="spinner big"></div> }.into_any()
            } else {
                view! {
                    <Icon icon=I::Speakers size=48 color="var(--muted)"/>
                    <div>"No DSPs configured yet."</div>
                    <button class="btn filled" on:click=move |_| on_add.run(())>
                        <Icon icon=I::Add size=18/>"Add DSP"
                    </button>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn Toasts() -> impl IntoView {
    let app = app();
    view! {
        <div class="toasts">
            <For each=move || app.toasts.get() key=|t| t.id let:toast>
                <div class="toast" class:error=toast.error>{toast.text}</div>
            </For>
        </div>
    }
}
