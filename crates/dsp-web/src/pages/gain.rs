//! Gain page: EQ overview on top, the faders below. The input and output EQ
//! blocks have exactly the width of the fader blocks below them; both scroll
//! together if the window is too narrow. Strips of one link group get a
//! bracket with the group's name above them.

use std::collections::{BTreeMap, BTreeSet};

use dsp_core::eq_math::ChannelEq;
use dsp_core::ops;
use dsp_core::panel::{LINK_NAME_MAX, group_key};
use dsp_core::protocol as p;
use leptos::html;
use leptos::prelude::*;

use crate::components::common::{Check, PanelCard, ToggleButton, link_groups_memo, link_names_memo};
use crate::components::fader::{Fader, MeterSpec};
use crate::components::value_field::ValueField;
use crate::fmt::{db_mark_label, format_db_value, parse_number};
use crate::icons::{I, Icon};
use crate::local_settings;
use crate::pages::dsp::{Dialog, ui};
use crate::store::{alias_in, app};
use crate::taper::{GAIN_MARKS, GAIN_TAPER};

/// Distinct colours for the curves of the channels of one group.
pub const CURVE_COLORS: [&str; 8] =
    ["#38BDF8", "#4ADE80", "#FBBF24", "#F472B6", "#A78BFA", "#F87171", "#2DD4BF", "#FB923C"];

pub fn curve_color(index: usize) -> &'static str {
    CURVE_COLORS[index % CURVE_COLORS.len()]
}

/// Channels of a group; tracks only the model, not the whole state.
pub fn group_channels(inputs: bool) -> Vec<&'static str> {
    let model = ui().model.get();
    if inputs { model.inputs.to_vec() } else { model.outputs.to_vec() }
}

#[component]
pub fn GainPage() -> impl IntoView {
    let device = ui().device;
    let key = format!("hiddenEqCurves.{}", device.id());
    let hidden = RwSignal::new(local_settings::get_string_set(&key));
    let toggle = Callback::new(move |ch: String| {
        hidden.update(|h| {
            if !h.remove(&ch) {
                h.insert(ch);
            }
        });
        local_settings::set_string_set(&key, &hidden.get_untracked());
    });
    let model = ui().model;
    let counts = move || {
        let m = model.get();
        format!("--n-in:{};--n-out:{}", m.inputs.len(), m.outputs.len())
    };
    let links = LinkInfo { groups: link_groups_memo(device), names: link_names_memo(device) };
    // Both fader blocks reserve the bracket row as soon as the DSP has a link
    // group on either side, so inputs and outputs stay aligned.
    let groups = links.groups;
    let show_links = Memo::new(move |_| groups.with(|g| !g.is_empty()));
    view! {
        <div class="gain-page">
            <div class="gain-grid" style=counts>
                <EqOverview inputs=true hidden=hidden on_toggle=toggle/>
                <EqOverview inputs=false hidden=hidden on_toggle=toggle/>
                <FaderGroup inputs=true links=links show_links=show_links/>
                <FaderGroup inputs=false links=links show_links=show_links/>
            </div>
        </div>
    }
}

/// Link groups of the DSP and their names (from the panel data).
#[derive(Clone, Copy)]
struct LinkInfo {
    groups: Memo<Vec<Vec<String>>>,
    names: Memo<BTreeMap<String, String>>,
}

#[component]
fn FaderGroup(inputs: bool, links: LinkInfo, show_links: Memo<bool>) -> impl IntoView {
    let color = if inputs { "var(--input)" } else { "var(--output)" };
    let strips = move || {
        group_channels(inputs)
            .into_iter()
            .enumerate()
            .map(|(i, ch)| {
                view! {
                    {(i > 0).then(|| view! { <div class="sep"></div> })}
                    <ChannelStrip channel=ch.to_string() color=color/>
                }
            })
            .collect_view()
    };
    view! {
        <div class="fader-group" style=format!("--gc:{color}")>
            <div class="group-title">{if inputs { "Inputs" } else { "Outputs" }}</div>
            {move || show_links.get().then(|| view! { <LinkRow inputs=inputs links=links/> })}
            <div class="fader-row">{strips}</div>
        </div>
    }
}

// ─── Link group brackets ───

/// One bracket: the strips [start]..=[end] (indexes into the channels of the
/// fader block) of the link group [members].
#[derive(Clone, PartialEq)]
struct LinkRun {
    members: Vec<String>,
    start: usize,
    end: usize,
}

/// Brackets for the link groups among [channels]: one per run of adjacent
/// strips of a group.
fn link_runs(channels: &[&str], groups: &[Vec<String>]) -> Vec<LinkRun> {
    let mut runs = Vec::new();
    for members in groups {
        let mut indexes: Vec<usize> =
            members.iter().filter_map(|m| channels.iter().position(|c| *c == m.as_str())).collect();
        indexes.sort_unstable();
        let mut indexes = indexes.into_iter();
        let Some(first) = indexes.next() else { continue };
        let (mut start, mut end) = (first, first);
        for i in indexes {
            if i != end + 1 {
                runs.push(LinkRun { members: members.clone(), start, end });
                start = i;
            }
            end = i;
        }
        runs.push(LinkRun { members: members.clone(), start, end });
    }
    runs
}

/// The row of link brackets above the strips: a grid with one column per
/// strip (the strips are equal flex items with 1 px separators).
#[component]
fn LinkRow(inputs: bool, links: LinkInfo) -> impl IntoView {
    let model = ui().model;
    let groups = links.groups;
    let runs = Memo::new(move |_| {
        let channels = group_channels(inputs);
        groups.with(|g| link_runs(&channels, g))
    });
    // The bracket whose name is being edited: (first strip, group key). A
    // change of the links ends the edit (the brackets are built anew).
    let editing = RwSignal::new(None::<(usize, String)>);
    Effect::new(move |prev: Option<()>| {
        runs.track();
        if prev.is_some() {
            editing.set(None);
        }
    });
    let columns = move || {
        let m = model.get();
        format!("--cols:{}", if inputs { m.inputs.len() } else { m.outputs.len() })
    };
    view! {
        <div class="link-row" style=columns>
            {move || {
                runs.get()
                    .into_iter()
                    .map(|run| view! { <LinkBracket run=run names=links.names editing=editing/> })
                    .collect_view()
            }}
        </div>
    }
}

/// Bracket over the strips of one run with the group's name (or a link icon
/// while it has none). On a DSP a click on the name edits it inline: Enter
/// saves on the server, Escape or leaving the field cancels.
#[component]
fn LinkBracket(
    run: LinkRun,
    names: Memo<BTreeMap<String, String>>,
    editing: RwSignal<Option<(usize, String)>>,
) -> impl IntoView {
    let device = ui().device;
    let editable = !device.offline;
    let LinkRun { members, start, end } = run;
    let key = group_key(&members);
    let k = key.clone();
    let name = Memo::new(move |_| names.with(|n| n.get(&k).cloned()));
    let k = key.clone();
    let is_editing = Memo::new(move |_| editing.with(|e| e.as_ref().is_some_and(|(s, g)| *s == start && *g == k)));
    let members = StoredValue::new(members);
    let dsp = device.id();

    let content = move || match name.get() {
        Some(n) => view! { <span class="link-name">{n}</span> }.into_any(),
        None => view! { <Icon icon=I::Link size=12/> }.into_any(),
    };
    let label = move || {
        if editable {
            let key = key.clone();
            let title = move || match name.get() {
                Some(n) => format!("Link group \"{n}\" – click to rename"),
                None => "Linked channels – click to name the group".to_string(),
            };
            view! {
                <button class="link-label" title=title on:click=move |_| editing.set(Some((start, key.clone())))>
                    {content}
                </button>
            }
            .into_any()
        } else {
            let title = move || name.get().map_or("Linked channels".to_string(), |n| format!("Link group \"{n}\""));
            view! { <span class="link-label" title=title>{content}</span> }.into_any()
        }
    };
    let editor = move || {
        let input = NodeRef::<html::Input>::new();
        Effect::new(move |_| {
            if let Some(el) = input.get() {
                let _ = el.focus();
                el.select();
            }
        });
        let current = name.get_untracked().unwrap_or_default();
        let initial = current.clone();
        let dsp = dsp.clone();
        let save = move || {
            let typed = input.get_untracked().map(|el| el.value()).unwrap_or_default();
            let typed: String = typed.chars().filter(|c| !c.is_control()).collect();
            let typed: String = typed.trim().chars().take(LINK_NAME_MAX).collect();
            let typed = typed.trim_end().to_string();
            editing.set(None);
            if typed != current {
                members.with_value(|m| app().set_link_name(&dsp, m, &typed));
            }
        };
        view! {
            <input
                class="link-input"
                node_ref=input
                type="text"
                maxlength=LINK_NAME_MAX
                spellcheck="false"
                autocomplete="off"
                placeholder="Group name"
                title="Enter saves, Escape cancels; empty removes the name"
                prop:value=initial
                on:keydown=move |ev| match ev.key().as_str() {
                    "Enter" => save(),
                    "Escape" => editing.set(None),
                    _ => {}
                }
                on:blur=move |_| {
                    let _ = editing.try_set(None);
                }
            />
        }
    };

    view! {
        <div class="link-bracket" style=format!("grid-column:{} / {}", start + 1, end + 2)>
            {move || if is_editing.get() { editor().into_any() } else { label().into_any() }}
        </div>
    }
}

/// One channel of the gain page: name (opens the channel menu), fader with
/// meter, typed value, mute and phase.
#[component]
pub fn ChannelStrip(channel: String, color: &'static str, #[prop(default = true)] show_name: bool) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let ch = channel.clone();
    let gain = Memo::new(move |_| device.state.with(|s| s.gains.get(&ch).copied().unwrap_or(0.0)));
    let ch = channel.clone();
    let muted = Memo::new(move |_| device.state.with(|s| s.mutes.get(&ch).copied().unwrap_or(false)));
    let ch = channel.clone();
    let inverted = Memo::new(move |_| device.state.with(|s| s.phase.get(&ch).copied().unwrap_or(false)));
    let ch = channel.clone();
    let name = Memo::new(move |_| device.state.with(|s| alias_in(s, &ch)));
    let ch = channel.clone();
    let link_title = Memo::new(move |_| {
        device.state.with(|s| {
            if ops::linked_with(s, &ch).is_empty() {
                return None;
            }
            Some(match ops::master_of(s, &ch) {
                Some(m) => format!("Linked to {}", alias_in(s, &m)),
                None => format!(
                    "Master of {}",
                    ops::linked_with(s, &ch).iter().map(|c| alias_in(s, c)).collect::<Vec<_>>().join(", ")
                ),
            })
        })
    });
    let ch = channel.clone();
    let cap = Memo::new(move |_| device.gain_cap(&ch));
    let is_output = device.model_untracked().is_output(&channel);

    let ch = channel.clone();
    let set_gain = Callback::new(move |v: f64| device.set_gain(&ch, v));
    // A typed value above the max level needs a second confirmation.
    let ch = channel.clone();
    let submit = Callback::new(move |input: String| match parse_number(&input) {
        Some(v) => {
            let v = p::quantize_gain(v.clamp(-60.0, 12.0));
            match device.gain_limit(&ch) {
                Some(limit) if v > limit && !device.test_running() => {
                    ui.dialog.set(Some(Dialog::ForceGain(ch.clone(), v)))
                }
                _ => device.set_gain(&ch, v),
            }
            true
        }
        None => false,
    });
    let gain_title = move || match cap.get() {
        Some(cap) => format!("Gain in dB (-60 … +12), max level {} dB", format_db_value(cap)),
        None => "Gain in dB (-60 … +12)".to_string(),
    };
    let ch = channel.clone();
    let toggle_mute = Callback::new(move |_| device.toggle_mute(&ch));
    let ch = channel.clone();
    let toggle_phase = Callback::new(move |_| device.toggle_phase(&ch));
    let menu_ch = channel.clone();

    view! {
        <div class="strip" style=format!("--gc:{color}")>
            {show_name.then(|| view! {
                <button class="strip-name" on:click=move |_| ui.dialog.set(Some(Dialog::ChannelMenu(menu_ch.clone())))>
                    <span class="name">{move || name.get()}</span>
                    {move || link_title.get().map(|t| view! { <span class="link" title=t><Icon icon=I::Link size=13/></span> })}
                    <Icon icon=I::ExpandMore size=14/>
                </button>
            })}
            <div class="fader-box">
                <Fader
                    value=gain
                    taper=GAIN_TAPER
                    on_change=set_gain
                    reset=0.0
                    marks=GAIN_MARKS.to_vec()
                    major=vec![0.0]
                    label={db_mark_label as fn(f64) -> String}
                    color=color
                    meter={MeterSpec { device, channel: channel.clone(), clip: true, limit: is_output }}
                    step=Signal::derive(move || if gain.get() < -20.0 { 0.5 } else { 0.1 })
                    limit=cap
                />
            </div>
            <div class="gain-value" title=gain_title>
                <ValueField text=Signal::derive(move || format_db_value(gain.get())) on_submit=submit/>
            </div>
            <div class="toggles">
                <ToggleButton label="M" active=muted active_color="var(--danger)"
                    title=Signal::derive(move || if muted.get() { "Unmute" } else { "Mute" }.to_string())
                    on_click=toggle_mute/>
                <ToggleButton label="Ø" active=inverted active_color="var(--warning)"
                    title=Signal::derive(move || if inverted.get() { "Phase inverted" } else { "Invert phase" }.to_string())
                    on_click=toggle_phase/>
            </div>
        </div>
    }
}

// ─── EQ overview ───

const MIN_HZ: f64 = 20.0;
const MAX_HZ: f64 = 20000.0;
const RANGE_DB: f64 = 15.0;
/// SVG coordinate system of the graphs (stretched to the element).
const W: f64 = 1000.0;
const H: f64 = 300.0;

fn x_to_hz(x: f64) -> f64 {
    MIN_HZ * (MAX_HZ / MIN_HZ).powf(x.clamp(0.0, 1.0))
}

fn hz_to_x(hz: f64) -> f64 {
    (hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()
}

fn db_to_y(db: f64) -> f64 {
    0.5 - db.clamp(-RANGE_DB, RANGE_DB) / RANGE_DB * 0.5
}

/// SVG path of a response curve with [steps] points over 20 Hz – 20 kHz.
pub fn curve_path(eq: &ChannelEq, steps: usize) -> String {
    let mut path = String::with_capacity(steps * 14);
    for s in 0..=steps {
        let x = s as f64 / steps as f64;
        let y = db_to_y(eq.db(x_to_hz(x)));
        path.push_str(&format!("{}{:.1},{:.1}", if s == 0 { "M" } else { "L" }, x * W, y * H));
    }
    path
}

/// The total response of every channel of one group (PEQ, filters and, for
/// inputs, the GEQ). The legend has a checkbox per channel to hide its curve;
/// clicking a curve opens the EQ page for it.
#[component]
fn EqOverview(inputs: bool, hidden: RwSignal<BTreeSet<String>>, on_toggle: Callback<String>) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let svg = NodeRef::<html::Div>::new();

    // One memo per channel: a curve is recomputed only when its own EQ changes.
    let curves = Memo::new(move |_| {
        group_channels(inputs)
            .into_iter()
            .enumerate()
            .map(|(i, ch)| {
                let eq = Memo::new(move |_| device.state.with(|s| ChannelEq::of(s, ch)));
                let path = Memo::new(move |_| eq.with(|e| curve_path(e, 160)));
                (ch, i, eq, path)
            })
            .collect::<Vec<_>>()
    });

    let on_click = move |ev: web_sys::MouseEvent| {
        let Some(el) = svg.get_untracked() else { return };
        let rect = el.get_bounding_client_rect();
        let x = (ev.client_x() as f64 - rect.left()) / rect.width().max(1.0);
        let y = (ev.client_y() as f64 - rect.top()) / rect.height().max(1.0);
        let hz = x_to_hz(x);
        let hidden = hidden.get_untracked();
        let best = curves.with_untracked(|list| {
            list.iter()
                .filter(|(ch, ..)| !hidden.contains(*ch))
                .map(|(ch, _, eq, _)| (*ch, (eq.with_untracked(|e| db_to_y(e.db(hz))) - y).abs()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(ch, _)| ch)
        });
        if let Some(ch) = best {
            ui.open_eq(ch);
        }
    };

    let grid = [100.0, 1000.0, 10000.0]
        .map(|hz| {
            let x = hz_to_x(hz) * W;
            view! { <line class="graph-grid" x1=x y1="0" x2=x y2=H/> }
        })
        .collect_view();
    let db_lines = [-12.0, -6.0, 6.0, 12.0]
        .map(|db| {
            let y = db_to_y(db) * H;
            view! { <line class="graph-grid" x1="0" y1=y x2=W y2=y opacity="0.47"/> }
        })
        .collect_view();
    let labels = [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")]
        .map(|(hz, text)| {
            view! { <span class="graph-label" style=format!("left:calc({:.3}% + 3px)", hz_to_x(hz) * 100.0)>{text}</span> }
        })
        .collect_view();
    let paths = move || {
        let hidden = hidden.get();
        curves
            .get()
            .into_iter()
            .filter(|(ch, ..)| !hidden.contains(*ch))
            .map(|(_, i, _, path)| view! { <path class="graph-curve" stroke=curve_color(i) d=move || path.get()/> })
            .collect_view()
    };
    let legend = move || {
        curves
            .get()
            .into_iter()
            .map(|(ch, i, ..)| {
                let channel = ch.to_string();
                view! {
                    <Check
                        checked=Signal::derive(move || !hidden.with(|h| h.contains(ch)))
                        on_toggle=Callback::new(move |_| on_toggle.run(channel.clone()))
                        label=Signal::derive(move || device.alias(ch))
                        color=curve_color(i)
                        title="Show or hide the curve"/>
                }
            })
            .collect_view()
    };

    let title = if inputs { "Inputs · EQ" } else { "Outputs · EQ" };
    let accent = if inputs { "var(--input)" } else { "var(--output)" };
    view! {
        <PanelCard title=Signal::derive(move || title.to_string()) accent=accent>
            // Opening the EQ page and hiding curves only change the view.
            <div class="eq-overview" data-view-ok node_ref=svg on:click=on_click>
                <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none">
                    {grid}
                    {db_lines}
                    <line class="graph-zero" x1="0" y1=H / 2.0 x2=W y2=H / 2.0/>
                    {paths}
                </svg>
                {labels}
            </div>
            <div class="legend" data-view-ok>{legend}</div>
        </PanelCard>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(members: &[&str]) -> Vec<String> {
        members.iter().map(|m| m.to_string()).collect()
    }

    #[test]
    fn runs_of_adjacent_strips() {
        let channels = ["Out1", "Out2", "Out3", "Out4", "Out5", "Out6"];
        let groups = vec![group(&["Out1", "Out2", "Out4"]), group(&["Out5", "Out6"]), group(&["InA", "InB"])];
        let runs = link_runs(&channels, &groups);
        let spans: Vec<(usize, usize)> = runs.iter().map(|r| (r.start, r.end)).collect();
        assert_eq!(spans, vec![(0, 1), (3, 3), (4, 5)]);
        assert_eq!(runs[1].members, groups[0]);
    }
}
