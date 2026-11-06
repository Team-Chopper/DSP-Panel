//! Small building blocks shared by the pages.

use std::collections::BTreeMap;

use dsp_core::panel::{group_key, link_groups};
use dsp_core::{DeviceModel, DeviceState, ops};
use leptos::prelude::*;

use crate::icons::{I, Icon};
use crate::store::{Device, alias_in, app};

// ─── Link groups ───

/// Model and link masks (channel order) of [device]: changes only when a link
/// changes, not on every fader move.
pub fn link_masks(device: Device) -> Memo<(DeviceModel, Vec<i64>)> {
    Memo::new(move |_| device.state.with(|s| (s.model, s.model.channels().map(|c| s.link_mask(c)).collect())))
}

/// A state that holds only [model] and the link [masks] (channel order), for
/// the link functions of `ops` and `panel`.
pub fn links_only(model: DeviceModel, masks: &[i64]) -> DeviceState {
    let mut state = DeviceState::new(model);
    state.links = model.channels().zip(masks).map(|(c, m)| (c.to_string(), *m)).collect();
    state
}

/// Link groups of [device] (two or more channels, members in channel order);
/// reads only the model and the link masks.
pub fn link_groups_memo(device: Device) -> Memo<Vec<Vec<String>>> {
    let masks = link_masks(device);
    Memo::new(move |_| masks.with(|(model, masks)| link_groups(&links_only(*model, masks))))
}

/// Names of the link groups of [device] by group key (panel data of the
/// server).
pub fn link_names_memo(device: Device) -> Memo<BTreeMap<String, String>> {
    let id = device.id();
    Memo::new(move |_| app().panel.with(|p| p.link_names.get(&id).cloned().unwrap_or_default()))
}

/// Card with a small title row.
#[component]
pub fn PanelCard(
    #[prop(optional, into)] title: Option<Signal<String>>,
    #[prop(optional, into)] accent: Option<String>,
    #[prop(optional)] trailing: Option<AnyView>,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let head = title.map(|title| {
        view! {
            <div class="card-head">
                {accent.map(|a| view! { <span class="dot" style=format!("background:{a}")></span> })}
                <span class="card-title">{move || title.get()}</span>
                {trailing}
            </div>
        }
    });
    view! {
        <div class=format!("card {class}")>
            {head}
            <div class="card-body">{children()}</div>
        </div>
    }
}

/// Small square toggle (mute "M", phase "Ø").
#[component]
pub fn ToggleButton(
    label: &'static str,
    #[prop(into)] active: Signal<bool>,
    active_color: &'static str,
    #[prop(into)] title: Signal<String>,
    on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            class="toggle"
            class:on=move || active.get()
            style=format!("--tc:{active_color}")
            title=move || title.get()
            on:click=move |_| on_click.run(())
        >
            {label}
        </button>
    }
}

/// Checkbox with a label.
#[component]
pub fn Check(
    #[prop(into)] checked: Signal<bool>,
    on_toggle: Callback<()>,
    #[prop(into)] label: Signal<String>,
    #[prop(into, default = "var(--accent)".to_string())] color: String,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] title: Option<String>,
) -> impl IntoView {
    view! {
        <label class="check" class:disabled=move || disabled.get() style=format!("--cc:{color}") title=title>
            <input
                type="checkbox"
                prop:checked=move || checked.get()
                prop:disabled=move || disabled.get()
                on:change=move |_| on_toggle.run(())
            />
            <span class="box"><Icon icon=I::Check size=12/></span>
            <span class="check-label">{move || label.get()}</span>
        </label>
    }
}

/// Modal dialog: a dimmed backdrop and a card. Closing is up to the caller
/// (click on the backdrop calls [on_close]).
#[component]
pub fn Modal(
    #[prop(into)] title: Signal<String>,
    on_close: Callback<()>,
    #[prop(default = 380)] width: u32,
    /// Buttons at the bottom.
    #[prop(optional)]
    actions: Option<AnyView>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="modal-backdrop" on:pointerdown=move |ev| {
            if ev.target() == ev.current_target() {
                on_close.run(());
            }
        }>
            <div class="modal" style=format!("width:min({width}px, calc(100vw - 32px))")
                on:keydown=move |ev| if ev.key() == "Escape" { on_close.run(()) }>
                <div class="modal-title">{move || title.get()}</div>
                <div class="modal-body">{children()}</div>
                {actions.map(|a| view! { <div class="modal-actions">{a}</div> })}
            </div>
        </div>
    }
}

/// A button that opens a small menu below it; the menu closes on a click
/// anywhere.
#[component]
pub fn MenuButton(
    icon: I,
    #[prop(into)] title: String,
    /// Menu items: (icon, label, action).
    items: Signal<Vec<(I, String, Callback<()>)>>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <div class="menu-anchor">
            <button class="icon-button" title=title on:click=move |_| open.update(|o| *o = !*o)>
                <Icon icon=icon size=20/>
            </button>
            <Show when=move || open.get()>
                <div class="menu-catcher" on:pointerdown=move |_| open.set(false)></div>
                <div class="menu">
                    {move || {
                        items
                            .get()
                            .into_iter()
                            .map(|(icon, label, action)| {
                                view! {
                                    <button class="menu-item" on:click=move |_| {
                                        open.set(false);
                                        action.run(());
                                    }>
                                        <Icon icon=icon size=18/>
                                        <span>{label}</span>
                                    </button>
                                }
                            })
                            .collect_view()
                    }}
                </div>
            </Show>
        </div>
    }
}

/// What one channel chip shows.
#[derive(Clone, PartialEq)]
struct Chip {
    ch: String,
    name: String,
    input: bool,
    active: bool,
    partner: bool,
    linked: bool,
    title: Option<String>,
}

/// Single-select channel chips; inputs and outputs are visually separated.
/// Channels linked with the selected one are highlighted too: a change on the
/// page applies to all of them. The tooltip of a linked chip names its master
/// or slaves and the name of the link group.
#[component]
pub fn ChannelSelector(
    device: Device,
    #[prop(into)] selected: Signal<String>,
    on_select: Callback<String>,
    #[prop(default = true)] inputs: bool,
    #[prop(default = true)] outputs: bool,
) -> impl IntoView {
    // Only what the chips show (links, channel names, group names): a value
    // dragged elsewhere on the page does not re-render them.
    let masks = link_masks(device);
    let aliases =
        Memo::new(move |_| device.state.with(|s| s.model.channels().map(|c| alias_in(s, c)).collect::<Vec<_>>()));
    let names = link_names_memo(device);
    let chips = Memo::new(move |_| {
        let sel = selected.get();
        masks.with(|(model, masks)| {
            let s = links_only(*model, masks);
            let groups = link_groups(&s);
            let partners = ops::linked_with(&s, &sel);
            aliases.with(|aliases| {
                names.with(|names| {
                    let alias = |ch: &str| {
                        model.channels().position(|c| c == ch).and_then(|i| aliases.get(i)).cloned().unwrap_or_else(|| ch.to_string())
                    };
                    model
                        .channels()
                        .filter(|ch| if model.is_input(ch) { inputs } else { outputs })
                        .map(|ch| {
                            let linked_with = ops::linked_with(&s, ch);
                            let linked = !linked_with.is_empty();
                            let title = linked.then(|| {
                                let base = match ops::master_of(&s, ch) {
                                    Some(m) => format!("Linked to {}", alias(&m)),
                                    None => format!(
                                        "Master of {}",
                                        linked_with.iter().map(|c| alias(c)).collect::<Vec<_>>().join(", ")
                                    ),
                                };
                                let group_name = groups
                                    .iter()
                                    .find(|g| g.iter().any(|m| m == ch))
                                    .and_then(|g| names.get(&group_key(g)));
                                match group_name {
                                    Some(name) => format!("{base} · \"{name}\""),
                                    None => base,
                                }
                            });
                            Chip {
                                ch: ch.to_string(),
                                name: alias(ch),
                                input: model.is_input(ch),
                                active: ch == sel,
                                partner: partners.iter().any(|p| p == ch),
                                linked,
                                title,
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
        })
    });

    let chip = move |c: Chip| {
        let color = if c.input { "var(--input)" } else { "var(--output)" };
        let Chip { ch, name, active, partner, linked, title, .. } = c;
        view! {
            <button
                class="chip"
                class:active=active
                class:partner=partner
                style=format!("--chip:{color}")
                title=title
                on:click=move |_| on_select.run(ch.clone())
            >
                {name}
                {linked.then(|| view! { <Icon icon=I::Link size=12/> })}
            </button>
        }
    };
    let content = move || {
        chips.with(|list| {
            let ins = list.iter().filter(|c| c.input).cloned().map(chip).collect_view();
            let outs = list.iter().filter(|c| !c.input).cloned().map(chip).collect_view();
            view! {
                {ins}
                {(inputs && outputs).then(|| view! { <span class="chip-sep"></span> })}
                {outs}
            }
        })
    };
    view! { <div class="channel-selector">{content}</div> }
}

/// Segmented button (one of several).
#[component]
pub fn Segmented(
    labels: Vec<&'static str>,
    #[prop(into)] selected: Signal<usize>,
    on_select: Callback<usize>,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView {
    let buttons = labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            view! {
                <button class:selected=move || selected.get() == i prop:disabled=move || disabled.get()
                    on:click=move |_| on_select.run(i)>
                    {label}
                </button>
            }
        })
        .collect_view();
    view! { <div class="segmented">{buttons}</div> }
}

/// Horizontal slider (native range input).
#[component]
pub fn Slider(
    #[prop(into)] value: Signal<f64>,
    min: f64,
    max: f64,
    #[prop(default = 0.0)] step: f64,
    on_input: Callback<f64>,
    #[prop(default = 160)] width: u32,
) -> impl IntoView {
    let step = if step > 0.0 { step.to_string() } else { "any".to_string() };
    view! {
        <input type="range" class="slider" style=format!("width:{width}px")
            min=min max=max step=step
            prop:value=move || value.get().to_string()
            on:input=move |ev| {
                if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                    on_input.run(v);
                }
            }/>
    }
}

/// Determinate ring (0..1).
#[component]
pub fn ProgressRing(#[prop(into)] value: Signal<f64>, #[prop(default = 44)] size: u32) -> impl IntoView {
    let r = 18.0;
    let circumference = 2.0 * std::f64::consts::PI * r;
    view! {
        <svg width=size height=size viewBox="0 0 44 44" class="progress-ring">
            <circle cx="22" cy="22" r=r fill="none" stroke="var(--border)" stroke-width="4"/>
            <circle cx="22" cy="22" r=r fill="none" stroke="var(--accent)" stroke-width="4"
                stroke-linecap="round" transform="rotate(-90 22 22)"
                stroke-dasharray=circumference
                stroke-dashoffset=move || circumference * (1.0 - value.get().clamp(0.0, 1.0))/>
        </svg>
    }
}
