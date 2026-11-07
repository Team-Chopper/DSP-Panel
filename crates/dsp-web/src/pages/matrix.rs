//! Matrix: which inputs feed which output.
//!
//! Left: routing diagram (inputs on the left, outputs on the right, one curve
//! per active crossing, coloured by input). Right: the crossing grid, inputs
//! are the columns, outputs the rows; click a crossing to toggle it and type
//! its gain (−60 … 0 dB, cmd 0x41) into the field of the cell. On narrow
//! windows the diagram sits above the grid.
//!
//! Linked channels do not share their routing or crossing gains (left/right
//! pairs are fed from different inputs).

use dsp_core::ops;
use leptos::html;
use leptos::prelude::*;

use crate::components::common::PanelCard;
use crate::components::size::use_size;
use crate::components::value_field::ValueField;
use crate::fmt::{fixed, format_db_value, parse_number};
use crate::icons::{I, Icon};
use crate::pages::dsp::ui;
use crate::pages::gain::curve_color;
use crate::store::{Device, alias_in};

const NODE_WIDTH: f64 = 92.0;
const NODE_HEIGHT: f64 = 30.0;

fn routed(device: Device, output: &str, input: &str) -> bool {
    device.state.with(|s| s.matrix_routing.get(output).copied().unwrap_or(0) & s.model.matrix_bit(input) as i64 != 0)
}

fn crossing_gain(device: Device, output: &str, input: &str) -> f64 {
    device.state.with(|s| s.matrix_gains.get(&format!("{output}:{input}")).copied().unwrap_or(0.0))
}

#[component]
pub fn MatrixPage() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let model = ui.model;
    let counts = move || format!("--n-in:{};--n-out:{}", model.get().inputs.len(), model.get().outputs.len());
    let trailing = view! { <span class="muted" style="font-size:11px">"click to toggle · type the gain in dB"</span> }.into_any();
    view! {
        <div class="matrix-page" style=counts>
            <div class="matrix-layout">
                <PanelCard title=Signal::derive(|| "Routing".to_string()) class="routing-card">
                    <RoutingDiagram device=device/>
                </PanelCard>
                <PanelCard title=Signal::derive(|| "Matrix".to_string()) trailing=trailing class="matrix-card">
                    <MatrixGrid device=device/>
                </PanelCard>
            </div>
        </div>
    }
}

// ─── Routing diagram ───

#[derive(Clone, PartialEq)]
struct Diagram {
    /// (name, connected) per input and per output.
    inputs: Vec<(String, bool)>,
    outputs: Vec<(String, bool)>,
    /// (input index, output index, gain dB)
    routes: Vec<(usize, usize, f64)>,
}

#[component]
fn RoutingDiagram(device: Device) -> impl IntoView {
    let container = NodeRef::<html::Div>::new();
    let size = use_size(container);
    let diagram = Memo::new(move |_| {
        device.state.with(|s| {
            let m = s.model;
            let on = |o: &str, i: &str| s.matrix_routing.get(o).copied().unwrap_or(0) & m.matrix_bit(i) as i64 != 0;
            let mut routes = Vec::new();
            for (oi, o) in m.outputs.iter().enumerate() {
                for (ii, i) in m.inputs.iter().enumerate() {
                    if on(o, i) {
                        routes.push((ii, oi, s.matrix_gains.get(&format!("{o}:{i}")).copied().unwrap_or(0.0)));
                    }
                }
            }
            Diagram {
                inputs: m.inputs.iter().map(|i| (alias_in(s, i), m.outputs.iter().any(|o| on(o, i)))).collect(),
                outputs: m.outputs.iter().map(|o| (alias_in(s, o), m.inputs.iter().any(|i| on(o, i)))).collect(),
                routes,
            }
        })
    });

    let curves = move || {
        let (w, h) = size.get();
        if w < 2.0 * NODE_WIDTH || h < 10.0 {
            return ().into_any();
        }
        diagram.with(|d| {
            let y = |i: usize, n: usize| h * (i as f64 + 0.5) / n as f64;
            d.routes
                .iter()
                .map(|(i, o, gain)| {
                    let (fx, fy) = (NODE_WIDTH, y(*i, d.inputs.len()));
                    let (tx, ty) = (w - NODE_WIDTH, y(*o, d.outputs.len()));
                    let dx = (tx - fx) * 0.45;
                    let color = curve_color(*i);
                    let path = format!("M{fx:.1},{fy:.1}C{:.1},{fy:.1} {:.1},{ty:.1} {tx:.1},{ty:.1}", fx + dx, tx - dx);
                    let label = (*gain != 0.0).then(|| {
                        let text = format!("{}{}", if *gain > 0.0 { "+" } else { "" }, fixed(*gain, 1));
                        view! { <text class="route-gain" x=tx - 34.0 y=ty - 4.0>{text}</text> }
                    });
                    view! {
                        <path d=path fill="none" stroke=color stroke-opacity="0.78" stroke-width="2"/>
                        <circle cx=tx cy=ty r="3.5" fill=color/>
                        <circle cx=fx cy=fy r="3.5" fill=color/>
                        {label}
                    }
                })
                .collect_view()
                .into_any()
        })
    };
    let node = |name: String, connected: bool, index: usize, count: usize, input: bool| {
        let color = if input { "var(--input)" } else { "var(--output)" };
        let side = if input { "left:0" } else { "right:0" };
        let top = format!("top:calc({:.4}% - {}px)", (index as f64 + 0.5) / count as f64 * 100.0, NODE_HEIGHT / 2.0);
        let title = name.clone();
        view! {
            <div class="route-node" class:on=connected style=format!("{side};{top};--nc:{color}") title=title>{name}</div>
        }
    };
    let nodes = move || {
        diagram.with(|d| {
            let ins = d.inputs.iter().enumerate().map(|(i, (n, c))| node(n.clone(), *c, i, d.inputs.len(), true));
            let outs = d.outputs.iter().enumerate().map(|(i, (n, c))| node(n.clone(), *c, i, d.outputs.len(), false));
            ins.chain(outs).collect_view()
        })
    };
    view! {
        <div class="routing" node_ref=container>
            <svg class="layer">{curves}</svg>
            {nodes}
        </div>
    }
}

// ─── Crossing grid ───

const LABEL_WIDTH: f64 = 96.0;
const HEADER_HEIGHT: f64 = 32.0;
const GAP: f64 = 6.0;

#[component]
fn MatrixGrid(device: Device) -> impl IntoView {
    let model = ui().model;
    let container = NodeRef::<html::Div>::new();
    let size = use_size(container);
    // Cell size (fills the card, but keeps the cells usable on small windows).
    let cell = Memo::new(move |_| {
        let (w, h) = size.get();
        let m = model.get();
        let (n_in, n_out) = (m.inputs.len() as f64, m.outputs.len() as f64);
        let width = w.max(LABEL_WIDTH + n_in * 64.0);
        let height = h.max(HEADER_HEIGHT + n_out * 36.0);
        ((width - LABEL_WIDTH) / n_in - GAP, (height - HEADER_HEIGHT - GAP) / n_out - GAP)
    });
    // Room for the gain field.
    let editable = Memo::new(move |_| {
        let (w, h) = cell.get();
        h >= 46.0 && w >= 58.0
    });
    let icon_size = Memo::new(move |_| {
        let (_, h) = cell.get();
        if editable.get() { (h - 28.0) * 0.8 } else { h * 0.5 }.clamp(10.0, if editable.get() { 24.0 } else { 26.0 }).round()
    });

    let grid = move || {
        let m = model.get();
        let head = m
            .inputs
            .iter()
            .map(|i| view! { <div class="mx-head">{move || device.alias(i)}</div> })
            .collect_view();
        let rows = m
            .outputs
            .iter()
            .map(|o| {
                let linked = Memo::new(move |_| device.state.with(|s| !ops::linked_with(s, o).is_empty()));
                let cells = m
                    .inputs
                    .iter()
                    .enumerate()
                    .map(|(n, i)| {
                        view! { <Cell device=device input=i output=o color=curve_color(n) editable=editable icon_size=icon_size/> }
                    })
                    .collect_view();
                view! {
                    <div class="mx-out">
                        <span class="name">{move || device.alias(o)}</span>
                        {move || linked.get().then(|| view! { <Icon icon=I::Link size=13/> })}
                    </div>
                    {cells}
                }
            })
            .collect_view();
        view! {
            <div class="mx-corner">"OUT \\ IN"</div>
            {head}
            {rows}
        }
    };
    view! {
        <div class="matrix-scroll" node_ref=container>
            <div class="matrix-grid">{grid}</div>
        </div>
    }
}

#[component]
fn Cell(
    device: Device,
    input: &'static str,
    output: &'static str,
    color: &'static str,
    editable: Memo<bool>,
    icon_size: Memo<f64>,
) -> impl IntoView {
    let enabled = Memo::new(move |_| routed(device, output, input));
    let gain = Memo::new(move |_| crossing_gain(device, output, input));
    let title = move || {
        format!(
            "{} → {}\nCrossing gain {} dB",
            device.alias(input),
            device.alias(output),
            format_db_value(gain.get())
        )
    };
    let icon = move || {
        let size = icon_size.get() as u32;
        if enabled.get() {
            view! { <Icon icon=I::CheckCircle size=size color=color/> }
        } else {
            view! { <Icon icon=I::CircleOutline size=size color="var(--border)"/> }
        }
    };
    view! {
        <div class="mx-cell" class:on=move || enabled.get() style=format!("--cc:{color}") title=title
            on:click=move |_| device.toggle_matrix_input(output, input)>
            <div class="mx-icon">{icon}</div>
            <Show when=move || editable.get()>
                <div class="mx-field" style=move || format!("color:{}", if gain.get() == 0.0 { "var(--muted)" } else { "var(--warning)" })
                    on:click=|ev| ev.stop_propagation()>
                    <ValueField height=22 font=10.0 color="inherit" title="Crossing gain in dB (-60 … 0)"
                        text=Signal::derive(move || format_db_value(gain.get()))
                        on_submit=Callback::new(move |text: String| match parse_number(&text) {
                            Some(v) => {
                                device.set_matrix_gain(output, input, v.clamp(-60.0, 0.0));
                                true
                            }
                            None => false,
                        })/>
                </div>
            </Show>
        </div>
    }
}
