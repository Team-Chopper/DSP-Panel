//! Everything for one DSP (or a server preset in the offline editor): section
//! bar (Gain · EQ · GEQ · Gate · Comp · Limit · Matrix · Delay · RTA) with
//! presets, connection status and connect/disconnect on the right, the section
//! content and the optional debug log. While the DSP is not connected the
//! content is blurred and a connect button is shown on top.

use dsp_core::DeviceModel;
use dsp_core::api::DspStatus;
use dsp_core::format::to_fixed;
use leptos::html;
use leptos::prelude::*;

use crate::app::status_color;
use crate::components::common::{Check, MenuButton, ProgressRing};
use crate::dialogs::{ChannelDialogs, DspDialog, SettingsDialog};
use crate::fmt::format_db_value;
use crate::icons::{I, Icon};
use crate::pages;
use crate::store::{Device, app};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Gain,
    Eq,
    Geq,
    Gate,
    Comp,
    Limit,
    Matrix,
    Delay,
    Rta,
}

impl Section {
    pub const ALL: [Section; 9] = [
        Section::Gain,
        Section::Eq,
        Section::Geq,
        Section::Gate,
        Section::Comp,
        Section::Limit,
        Section::Matrix,
        Section::Delay,
        Section::Rta,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Gain => "Gain",
            Section::Eq => "EQ",
            Section::Geq => "GEQ",
            Section::Gate => "Gate",
            Section::Comp => "Comp",
            Section::Limit => "Limit",
            Section::Matrix => "Matrix",
            Section::Delay => "Delay",
            Section::Rta => "RTA",
        }
    }

    fn icon(self) -> I {
        match self {
            Section::Gain => I::Tune,
            Section::Eq => I::ShowChart,
            Section::Geq => I::Equalizer,
            Section::Gate => I::Door,
            Section::Comp => I::Compress,
            Section::Limit => I::VerticalTop,
            Section::Matrix => I::Grid,
            Section::Delay => I::Timer,
            Section::Rta => I::GraphicEq,
        }
    }
}

/// Dialogs of a DSP page (one at a time).
#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    ChannelMenu(String),
    Rename(String),
    Copy(String),
    Link(String),
    /// Max level of a channel.
    MaxLevel(String),
    /// Confirm a typed gain above the channel's max level.
    ForceGain(String, f64),
    EditDsp,
    Settings,
    Presets,
    ApplyPreset(String),
}

/// Shared state of a DSP page, provided as context to its sections.
#[derive(Clone, Copy)]
pub struct DspUi {
    pub device: Device,
    /// The model (changes only when the device turns out to be another one):
    /// views that depend on the channel list read this, not the state.
    pub model: Memo<DeviceModel>,
    pub section: RwSignal<Section>,
    /// Channel to show when the EQ/GEQ page is opened from the overview.
    pub eq_channel: RwSignal<Option<String>>,
    pub dialog: RwSignal<Option<Dialog>>,
}

impl DspUi {
    pub fn open_eq(&self, channel: &str) {
        self.eq_channel.set(Some(channel.to_string()));
        self.section.set(Section::Eq);
    }
}

pub fn ui() -> DspUi {
    expect_context::<DspUi>()
}

#[component]
pub fn DspPage(device: Device) -> impl IntoView {
    let ui = DspUi {
        device,
        model: Memo::new(move |_| device.state.with(|s| s.model)),
        section: RwSignal::new(Section::Gain),
        eq_channel: RwSignal::new(None),
        dialog: RwSignal::new(None),
    };
    provide_context(ui);
    let show_debug = RwSignal::new(false);
    let locked = move || !device.offline && !device.is_connected();

    // The section content only depends on the section (and the channel to
    // open), never on the connection status: status updates do not re-render
    // the page.
    let content = move || {
        let section = ui.section.get();
        match section {
            Section::Gain => view! { <pages::gain::GainPage/> }.into_any(),
            Section::Eq => view! { <pages::eq::EqPage/> }.into_any(),
            Section::Geq => view! { <pages::geq::GeqPage/> }.into_any(),
            Section::Gate => view! { <pages::dynamics::GatePage/> }.into_any(),
            Section::Comp => view! { <pages::dynamics::CompressorPage/> }.into_any(),
            Section::Limit => view! { <pages::dynamics::LimiterPage/> }.into_any(),
            Section::Matrix => view! { <pages::matrix::MatrixPage/> }.into_any(),
            Section::Delay => view! { <pages::delay::DelayPage/> }.into_any(),
            Section::Rta => view! { <pages::rta::RtaPage/> }.into_any(),
        }
    };

    let config_warning = move || {
        let (ready, readable, model) =
            device.info.with(|i| (i.status == DspStatus::Ready, i.config_readable, DeviceModel::by_id(Some(&i.model))));
        (!device.offline && ready && !readable).then(|| {
            view! {
                <div class="banner">
                    <Icon icon=I::Warning size=18/>
                    {format!(
                        "The configuration of this DSP could not be read (unexpected layout for {}). The panel shows \
                         default values – controls still work. Check the model in \"Edit DSP\" or see TODO.md.",
                        model.name
                    )}
                </div>
            }
        })
    };

    let test_level = Memo::new(move |_| if device.offline { None } else { device.info.with(|i| i.test_level) });
    let test_running = Memo::new(move |_| test_level.with(Option::is_some));
    let test_banner = move || {
        test_running.get().then(|| {
            view! {
                <div class="banner test-banner">
                    <Icon icon=I::Warning size=18/>
                    <span class="grow">{move || format!(
                        "DSP test generator running – all outputs at {} dB. Only mutes and the test controls are \
                         accepted; the saved output gains come back when it is switched off.",
                        format_db_value(test_level.get().unwrap_or_default())
                    )}</span>
                    <button class="btn danger compact" on:click=move |_| device.set_test_tone(0, None)>
                        <Icon icon=I::Stop size=18/>"Stop generator"
                    </button>
                </div>
            }
        })
    };

    view! {
        <div class="dsp-page">
            <SectionBar show_debug=show_debug/>
            {config_warning}
            {test_banner}
            <div class="dsp-content" class:locked=locked>
                <div class="page">{content}</div>
                <Show when=locked>
                    <ConnectOverlay device=device/>
                </Show>
            </div>
            <Show when=move || show_debug.get() && !device.offline>
                <DebugPanel device=device/>
            </Show>
            {move || ui.dialog.get().map(|dialog| match dialog {
                Dialog::EditDsp => view! {
                    <DspDialog initial=Some(device.info.get_untracked()) on_close=Callback::new(move |_| ui.dialog.set(None))/>
                }.into_any(),
                Dialog::Settings => view! { <SettingsDialog/> }.into_any(),
                Dialog::Presets => view! { <pages::presets::PresetsDialog/> }.into_any(),
                Dialog::ApplyPreset(id) => view! { <pages::presets::ApplyPresetDialog preset_id=id/> }.into_any(),
                other => view! { <ChannelDialogs dialog=other/> }.into_any(),
            })}
        </div>
    }
}

#[component]
fn SectionBar(show_debug: RwSignal<bool>) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let sections = Section::ALL
        .into_iter()
        .map(|s| {
            view! {
                <button
                    class="section-button"
                    class:selected=move || ui.section.get() == s
                    title=s.label()
                    on:click=move |_| {
                        if s != Section::Eq && s != Section::Geq {
                            ui.eq_channel.set(None);
                        }
                        ui.section.set(s);
                    }
                >
                    <Icon icon=s.icon() size=18/>
                    <span class="label">{s.label()}</span>
                </button>
            }
        })
        .collect_view();

    let actions = if device.offline {
        view! { <EditorActions/> }.into_any()
    } else {
        let items = Signal::derive(move || {
            vec![
                (I::Edit, "Edit DSP".to_string(), Callback::new(move |_| ui.dialog.set(Some(Dialog::EditDsp)))),
                (I::Settings, "Settings".to_string(), Callback::new(move |_| ui.dialog.set(Some(Dialog::Settings)))),
                (
                    I::Bug,
                    if show_debug.get() { "Hide debug log" } else { "Show debug log" }.to_string(),
                    Callback::new(move |_| show_debug.update(|d| *d = !*d)),
                ),
            ]
        });
        view! {
            <StatusPill device=device/>
            {move || if device.info.with(|i| i.enabled) {
                view! {
                    <button class="btn danger compact" on:click=move |_| device.disconnect()>
                        <Icon icon=I::Power size=18/><span class="wide-only">"Disconnect"</span>
                    </button>
                }.into_any()
            } else {
                view! {
                    <button class="btn filled compact" on:click=move |_| device.connect()>
                        <Icon icon=I::Power size=18/>"Connect"
                    </button>
                }.into_any()
            }}
            <MenuButton icon=I::More title="More" items=items/>
        }
        .into_any()
    };

    view! {
        <div class="section-bar">
            <div class="sections">{sections}</div>
            <PresetsButton/>
            {actions}
        </div>
    }
}

/// Label of the presets button: the current preset with its slot.
#[component]
fn PresetsButton() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let label = move || {
        if device.offline {
            return device.info.with(|i| i.name.clone());
        }
        if !device.is_connected() {
            return "Presets".to_string();
        }
        device.state.with(|s| {
            let name = &s.current_preset;
            match s.presets.iter().find(|(_, v)| *v == name) {
                Some((index, _)) => format!("{} · {name}", preset_slot(*index + 1)),
                None => name.clone(),
            }
        })
    };
    view! {
        <button class="btn outlined compact presets-button" title="Presets on the DSP and on the server"
            on:click=move |_| ui.dialog.set(Some(Dialog::Presets))>
            <Icon icon=I::Library size=18 color="var(--accent)"/>
            <span class="label">{label}</span>
        </button>
    }
}

/// Preset number as the original editor shows it: 0 = F00, n = Un.
pub fn preset_slot(n: i64) -> String {
    if n == 0 { "F00".into() } else { format!("U{n:02}") }
}

#[component]
fn StatusPill(device: Device) -> impl IntoView {
    let text = move || {
        device.info.with(|i| {
            let pct = (i.progress * 100.0).round() as i64;
            match i.status {
                DspStatus::Ready => "Connected".to_string(),
                DspStatus::Connecting => "Connecting".to_string(),
                DspStatus::Initializing => format!("Loading {pct} %"),
                DspStatus::Busy => format!("{} {pct} %", i.message.clone().unwrap_or_else(|| "Busy".into())),
                DspStatus::Error | DspStatus::Disconnected => "Disconnected".to_string(),
            }
        })
    };
    let title = move || {
        let (tx, rx) = device.counters.get();
        device.info.with(|i| {
            let model = DeviceModel::by_id(Some(&i.model));
            let mut lines = vec![connection_status(i), format!("{} · {}:{}", model.name, i.host, i.port)];
            if i.status == DspStatus::Ready {
                lines.push(format!("TX {tx} · RX {rx}"));
            }
            lines.join("\n")
        })
    };
    let color = move || status_color(device.info.with(|i| i.status));
    view! {
        <div class="status-pill" title=title style=move || {
            let c = color();
            format!("color:{c};border-color:color-mix(in srgb, {c} 55%, transparent);background:color-mix(in srgb, {c} 12%, transparent)")
        }>
            <span class="dot"></span>
            <span class="label">{text}</span>
        </div>
    }
}

fn connection_status(i: &dsp_core::DspInfo) -> String {
    let address = format!("{}:{}", i.host, i.port);
    match i.status {
        DspStatus::Disconnected => if i.enabled { "Disconnected" } else { "Disabled" }.into(),
        DspStatus::Connecting => format!("Connecting to {address} …"),
        DspStatus::Initializing => format!("Initializing {address} …"),
        DspStatus::Ready => format!("Connected to {address}"),
        DspStatus::Busy => i.message.clone().unwrap_or_else(|| "Busy …".into()),
        DspStatus::Error => i.message.clone().unwrap_or_else(|| "Error".into()),
    }
}

#[component]
fn EditorActions() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let app = app();
    let preset_id = device.id()[dsp_core::api::msg::PRESET_PREFIX.len()..].to_string();
    let model = move || DeviceModel::by_id(Some(&device.info.with(|i| i.model.clone()))).name;
    let close_id = preset_id.clone();
    view! {
        <div class="offline-pill" title=move || format!(
            "Server preset for the {}, edited without a DSP.\nEvery change is saved on the server.", model())>
            <Icon icon=I::Cloud size=14/>
            <span class="wide-only">{move || format!("Offline · {}", model())}</span>
        </div>
        <button class="btn filled compact" on:click=move |_| ui.dialog.set(Some(Dialog::ApplyPreset(preset_id.clone())))>
            <Icon icon=I::Send size=16/><span class="wide-only">"Load onto DSP …"</span>
        </button>
        <button class="icon-button" title="Close editor" on:click=move |_| app.close_editor(&close_id)>
            <Icon icon=I::Close size=20/>
        </button>
    }
}

/// Shown on top of the blurred content while the DSP is not ready. No endless
/// animation: it would re-render the blurred page behind it on every frame.
#[component]
fn ConnectOverlay(device: Device) -> impl IntoView {
    let body = move || {
        device.info.with(|info| {
            let pct = (info.progress * 100.0).round() as i64;
            let (title, detail) = match info.status {
                DspStatus::Connecting => ("Connecting …".to_string(), Some(format!("{}:{}", info.host, info.port))),
                DspStatus::Initializing => (format!("Reading the configuration … {pct} %"), None),
                DspStatus::Busy => (format!("{} {pct} %", info.message.clone().unwrap_or_else(|| "Working …".into())), None),
                DspStatus::Error => (
                    "Connection failed".to_string(),
                    Some(format!("{}\nRetrying automatically.", info.message.clone().unwrap_or_else(|| "Error".into()))),
                ),
                DspStatus::Disconnected | DspStatus::Ready => {
                    ("Disconnected".to_string(), Some(format!("{} · {}:{}", info.name, info.host, info.port)))
                }
            };
            let symbol = match info.status {
                DspStatus::Connecting => view! { <Icon icon=I::Ethernet size=40 color="var(--warning)"/> }.into_any(),
                DspStatus::Initializing | DspStatus::Busy => {
                    let progress = info.progress;
                    view! { <ProgressRing value=Signal::derive(move || progress)/> }.into_any()
                }
                DspStatus::Error => view! { <Icon icon=I::Error size=40 color="var(--danger)"/> }.into_any(),
                _ => view! { <Icon icon=I::LinkOff size=40 color="var(--danger)"/> }.into_any(),
            };
            let enabled = info.enabled;
            view! {
                {symbol}
                <div class="title">{title}</div>
                {detail.map(|d| view! { <div class="detail">{d}</div> })}
                {(!enabled).then(|| view! {
                    <button class="btn filled" on:click=move |_| device.connect()>
                        <Icon icon=I::Power size=20/>"Connect"
                    </button>
                })}
            }
        })
    };
    view! {
        <div class="connect-overlay">
            <div class="connect-box">{body}</div>
        </div>
    }
}

#[component]
fn DebugPanel(device: Device) -> impl IntoView {
    let timestamps = RwSignal::new(false);
    let list = NodeRef::<html::Div>::new();
    let at_newest = StoredValue::new(true);
    // Keep the newest line in view unless the user scrolled up.
    Effect::new(move |_| {
        device.log.track();
        if let Some(el) = list.get()
            && at_newest.get_value() {
                el.set_scroll_top(el.scroll_height());
            }
    });
    let lines = move || {
        let decoded = device.show_decoded.get();
        let ts = timestamps.get();
        device.log.with(|log| {
            log.iter()
                .map(|e| {
                    let text = e.display(decoded).to_string();
                    let line = if ts { format!("[{}] {text}", to_fixed(e.timestamp, 4)) } else { text };
                    view! { <div>{line}</div> }
                })
                .collect_view()
        })
    };
    view! {
        <div class="debug-panel">
            <div class="debug-head">
                <span class="title">"Debug log"</span>
                <Check checked=Signal::derive(move || device.info.with(|i| i.log_rx)) on_toggle=Callback::new(move |_| device.toggle_rx()) label="Rx"/>
                <Check checked=Signal::derive(move || device.info.with(|i| i.log_tx)) on_toggle=Callback::new(move |_| device.toggle_tx()) label="Tx"/>
                <Check checked=device.show_decoded on_toggle=Callback::new(move |_| device.show_decoded.update(|d| *d = !*d)) label="Decode"/>
                <Check checked=timestamps on_toggle=Callback::new(move |_| timestamps.update(|t| *t = !*t)) label="Time"/>
                <button class="icon-button small" title="Clear" on:click=move |_| device.clear_log()>
                    <Icon icon=I::DeleteSweep size=18/>
                </button>
            </div>
            <div class="debug-lines" node_ref=list on:scroll=move |_| {
                if let Some(el) = list.get_untracked() {
                    at_newest.set_value(el.scroll_top() + el.client_height() >= el.scroll_height() - 10);
                }
            }>{lines}</div>
        </div>
    }
}
