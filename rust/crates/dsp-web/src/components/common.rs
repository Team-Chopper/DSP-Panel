//! Small building blocks shared by the pages.

use dsp_core::ops;
use leptos::prelude::*;

use crate::icons::{I, Icon};
use crate::store::{Device, alias_in};

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

/// Single-select channel chips; inputs and outputs are visually separated.
/// Channels linked with the selected one are highlighted too: a change on the
/// page applies to all of them.
#[component]
pub fn ChannelSelector(
    device: Device,
    #[prop(into)] selected: Signal<String>,
    on_select: Callback<String>,
    #[prop(default = true)] inputs: bool,
    #[prop(default = true)] outputs: bool,
) -> impl IntoView {
    let chips = move || {
        let sel = selected.get();
        device.state.with(|s| {
            let partners = ops::linked_with(s, &sel);
            let chip = |ch: &str| {
                let is_input = s.model.is_input(ch);
                let color = if is_input { "var(--input)" } else { "var(--output)" };
                let active = ch == sel;
                let partner = partners.iter().any(|p| p == ch);
                let linked = !ops::linked_with(s, ch).is_empty();
                let title = if linked {
                    match ops::master_of(s, ch) {
                        Some(m) => format!("Linked to {}", alias_in(s, &m)),
                        None => format!(
                            "Master of {}",
                            ops::linked_with(s, ch).iter().map(|c| alias_in(s, c)).collect::<Vec<_>>().join(", ")
                        ),
                    }
                } else {
                    String::new()
                };
                let name = alias_in(s, ch);
                let ch = ch.to_string();
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
            let ins = if inputs { s.model.inputs.iter().map(|c| chip(c)).collect_view().into_any() } else { ().into_any() };
            let outs = if outputs { s.model.outputs.iter().map(|c| chip(c)).collect_view().into_any() } else { ().into_any() };
            view! {
                {ins}
                {(inputs && outputs).then(|| view! { <span class="chip-sep"></span> })}
                {outs}
            }
        })
    };
    view! { <div class="channel-selector">{chips}</div> }
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
