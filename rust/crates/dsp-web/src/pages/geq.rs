//! Graphic EQ (GEQ, inputs): curve on top, below it one fader per band, each
//! exactly under its point of the curve (like the original editor). The RTA
//! can run behind the curve; it is drawn on its own canvas layer, so the
//! faders and the curve are not touched by the ~12 spectra per second.

use dsp_core::eq_math::GEQ_CENTER_FREQUENCIES;
use leptos::html;
use leptos::prelude::*;
use web_sys::{PointerEvent, WheelEvent};

use crate::components::common::ChannelSelector;
use crate::components::fader::Fader;
use crate::components::size::use_size;
use crate::components::spectrum::{self, BarStyle};
use crate::components::value_field::ValueField;
use crate::fmt::{fixed, parse_number};
use crate::pages::auto_eq::{AutoEqPanel, AutoEqTarget, PROPOSAL_COLOR};
use crate::pages::dsp::ui;
use crate::rta::Rta;
use crate::store::app;
use crate::taper::Taper;

const BANDS: usize = GEQ_CENTER_FREQUENCIES.len();
const GEQ_MARKS: [f64; 9] = [12.0, 9.0, 6.0, 3.0, 0.0, -3.0, -6.0, -9.0, -12.0];
const GEQ_TAPER: Taper = Taper::Linear(-12.0, 12.0);
const GEQ_CAP: f64 = 22.0;
const CURVE_COLOR: &str = "#38BDF8";

// Graph geometry (log axis 10 Hz – 30 kHz).
const LOG_MIN_HZ: f64 = 10.0;
const LOG_MAX_HZ: f64 = 30000.0;
const PAD_LEFT: f64 = 36.0;
const PAD_RIGHT: f64 = 8.0;
const PAD_TOP: f64 = 10.0;
const PAD_BOTTOM: f64 = 28.0;

/// Position of [hz] on the log axis, 0..1.
fn norm(hz: f64) -> f64 {
    (hz / LOG_MIN_HZ).ln() / (LOG_MAX_HZ / LOG_MIN_HZ).ln()
}

/// "20", "31.5", "1K", "1.25K"
pub fn format_freq(hz: f64) -> String {
    if hz >= 1000.0 {
        let k = hz / 1000.0;
        if k.fract() == 0.0 { format!("{}K", k as i64) } else { format!("{k}K") }
    } else if hz.fract() == 0.0 {
        format!("{}", hz as i64)
    } else {
        format!("{hz}")
    }
}

fn signed(db: f64) -> String {
    format!("{}{}", if db >= 0.0 { "+" } else { "" }, fixed(db, 1))
}

#[derive(Clone, Copy, PartialEq)]
struct Geo {
    width: f64,
    height: f64,
}

impl Geo {
    fn gw(&self) -> f64 {
        (self.width - PAD_LEFT - PAD_RIGHT).max(1.0)
    }
    fn gh(&self) -> f64 {
        (self.height - PAD_TOP - PAD_BOTTOM).max(1.0)
    }
    fn x(&self, hz: f64) -> f64 {
        PAD_LEFT + norm(hz) * self.gw()
    }
    fn y(&self, db: f64) -> f64 {
        PAD_TOP + self.gh() * (1.0 - (db + 12.0) / 24.0)
    }
    fn usable(&self) -> bool {
        self.width > 60.0 && self.height > 60.0
    }
    /// Nearest band to pixel x (log distance).
    fn band_at(&self, x: f64) -> usize {
        let n = ((x - PAD_LEFT) / self.gw()).clamp(0.0, 1.0);
        let hz = LOG_MIN_HZ * (LOG_MAX_HZ / LOG_MIN_HZ).powf(n);
        (0..BANDS)
            .min_by(|a, b| {
                let da = (GEQ_CENTER_FREQUENCIES[*a].ln() - hz.ln()).abs();
                let db = (GEQ_CENTER_FREQUENCIES[*b].ln() - hz.ln()).abs();
                da.total_cmp(&db)
            })
            .unwrap_or(0)
    }
    fn db_at(&self, y: f64) -> f64 {
        ((1.0 - (y - PAD_TOP) / self.gh()) * 24.0 - 12.0).clamp(-12.0, 12.0)
    }
}

/// Smooth curve through [points] (ascending x) that does not overshoot:
/// monotone cubic interpolation (Fritsch–Carlson), so a flat band next to a
/// boosted one stays flat.
fn monotone_path(points: &[(f64, f64)]) -> String {
    let n = points.len();
    let Some(first) = points.first() else { return String::new() };
    let mut out = String::with_capacity(n * 48);
    out.push_str(&format!("M{:.1},{:.1}", first.0, first.1));
    if n < 2 {
        return out;
    }
    let slopes: Vec<f64> =
        (0..n - 1).map(|k| (points[k + 1].1 - points[k].1) / (points[k + 1].0 - points[k].0)).collect();
    let mut t = vec![0.0; n];
    t[0] = slopes[0];
    t[n - 1] = slopes[n - 2];
    for k in 1..n - 1 {
        t[k] = if slopes[k - 1] * slopes[k] <= 0.0 { 0.0 } else { (slopes[k - 1] + slopes[k]) / 2.0 };
    }
    for k in 0..n - 1 {
        if slopes[k] == 0.0 {
            t[k] = 0.0;
            t[k + 1] = 0.0;
            continue;
        }
        let (a, b) = (t[k] / slopes[k], t[k + 1] / slopes[k]);
        let s = a * a + b * b;
        if s > 9.0 {
            let f = 3.0 / s.sqrt();
            t[k] = f * a * slopes[k];
            t[k + 1] = f * b * slopes[k];
        }
    }
    for k in 0..n - 1 {
        let (p1, p2) = (points[k], points[k + 1]);
        let h = (p2.0 - p1.0) / 3.0;
        out.push_str(&format!(
            "C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
            p1.0 + h,
            p1.1 + t[k] * h,
            p2.0 - h,
            p2.1 - t[k + 1] * h,
            p2.0,
            p2.1
        ));
    }
    out
}

fn round_band(db: f64) -> f64 {
    ((db * 10.0).round() / 10.0).clamp(-12.0, 12.0)
}

#[component]
pub fn GeqPage() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let model = ui.model;
    let first_input = move || model.get_untracked().inputs[0].to_string();
    let channel = RwSignal::new(
        ui.eq_channel.get_untracked().filter(|c| model.get_untracked().is_input(c)).unwrap_or_else(first_input),
    );
    // The DSP 206 has fewer inputs.
    Effect::new(move |_| {
        if !model.get().is_input(&channel.get_untracked()) {
            channel.set(first_input());
        }
    });
    let active = RwSignal::new(None::<usize>);
    let rta = Rta::new();
    let auto_eq = RwSignal::new(false);
    let proposal = RwSignal::new(None::<Vec<f64>>);

    let bands = Memo::new(move |_| {
        let ch = channel.get();
        device.state.with(|s| s.geq_bands.get(&ch).cloned().unwrap_or_else(|| vec![0.0; BANDS]))
    });
    let bypass = Memo::new(move |_| {
        let ch = channel.get();
        device.state.with(|s| s.geq_bypass.get(&ch).copied().unwrap_or(false))
    });
    let set_band = Callback::new(move |(i, db): (usize, f64)| {
        active.set(Some(i));
        device.set_geq_band(&channel.get_untracked(), i, round_band(db));
    });

    let rta_on = move || rta.running.get() || rta.starting.get();
    let close_auto_eq = move || {
        auto_eq.set(false);
        proposal.set(None);
    };
    let toggle_rta = move |_| {
        if rta_on() {
            close_auto_eq();
            rta.stop();
        } else {
            rta.start();
        }
    };
    let toggle_auto_eq = move |_| {
        if auto_eq.get_untracked() {
            close_auto_eq();
            return;
        }
        if !rta.running.get_untracked() {
            rta.start();
        }
        auto_eq.set(true);
    };
    Effect::new(move |_| {
        if let Some(e) = rta.error.get()
            && !auto_eq.get_untracked() {
                app().toast(format!("Failed to start RTA: {e}"), false);
            }
    });

    let auto_eq_panel = move || {
        auto_eq.get().then(|| {
            // A new panel per input (restores the mutes of the previous one).
            let ch = channel.get();
            view! {
                <div class="auto-eq-wrap">
                    <AutoEqPanel channel=ch target=AutoEqTarget::Geq rta=rta
                        on_geq_proposal=Callback::new(move |p: Option<Vec<f64>>| proposal.set(p))
                        on_close=Callback::new(move |_| close_auto_eq())/>
                </div>
            }
        })
    };

    view! {
        <div class="geq-page">
            <div class="geq-top">
                <div class="grow">
                    <ChannelSelector device=device selected=channel outputs=false on_select=Callback::new(move |c: String| {
                        // A proposal belongs to one input.
                        if c != channel.get_untracked() {
                            proposal.set(None);
                        }
                        channel.set(c);
                    })/>
                </div>
                <button class="action-button" on:click=move |_| device.reset_geq(&channel.get_untracked())>"Flat"</button>
                <button class="action-button" class:on=move || bypass.get()
                    on:click=move |_| device.set_geq_bypass(&channel.get_untracked(), !bypass.get_untracked())>"Bypass"</button>
                <button class="action-button" class:on=rta_on on:click=toggle_rta>"RTA"</button>
                <button class="action-button" class:on=move || auto_eq.get() on:click=toggle_auto_eq>"Auto-EQ"</button>
            </div>
            {auto_eq_panel}
            <div class="geq-scroll">
                <div class="geq-content">
                    <GeqGraph bands=bands active=active rta=rta
                        proposal=Signal::derive(move || if auto_eq.get() { proposal.get() } else { None })
                        on_band=set_band/>
                    <FaderBank bands=bands active=active on_band=set_band/>
                </div>
            </div>
        </div>
    }
}

// ─── Graph ───

#[component]
fn GeqGraph(
    bands: Memo<Vec<f64>>,
    active: RwSignal<Option<usize>>,
    rta: Rta,
    #[prop(into)] proposal: Signal<Option<Vec<f64>>>,
    on_band: Callback<(usize, f64)>,
) -> impl IntoView {
    let container = NodeRef::<html::Div>::new();
    let canvas = NodeRef::<html::Canvas>::new();
    let size = use_size(container);
    let geo = Memo::new(move |_| {
        let (width, height) = size.get();
        Geo { width, height }
    });

    // RTA layer: redrawn per spectrum.
    Effect::new(move |_| {
        rta.frame.track();
        let running = rta.running.get();
        let g = geo.get();
        let Some(el) = canvas.get() else { return };
        let Some(ctx) = spectrum::prepare(&el, g.width, g.height) else { return };
        if !running || !g.usable() {
            return;
        }
        let style = BarStyle {
            bar: "rgba(56,189,248,0.31)",
            peak: "rgba(255,255,255,0.47)",
            peak_width: 1.0,
            gaps: false,
            hide_silent_peaks: false,
        };
        rta.with_spectrum(|levels, peaks| {
            spectrum::draw_bars(
                &ctx,
                levels,
                peaks,
                |hz| g.x(hz.clamp(LOG_MIN_HZ, LOG_MAX_HZ)),
                PAD_LEFT,
                PAD_LEFT + g.gw(),
                PAD_TOP,
                g.gh(),
                &style,
            );
        });
    });

    // ─── Interaction: vertical drag sets the band under the pointer ───
    let drag = StoredValue::new(None::<(f64, f64, bool)>);
    let local = move |x: i32, y: i32| -> Option<(f64, f64)> {
        let rect = container.get_untracked()?.get_bounding_client_rect();
        Some((x as f64 - rect.left(), y as f64 - rect.top()))
    };
    let apply = move |x: f64, y: f64| {
        let g = geo.get_untracked();
        on_band.run((g.band_at(x), g.db_at(y)));
    };
    let on_down = move |ev: PointerEvent| {
        if ev.button() != 0 {
            return;
        }
        let Some((x, y)) = local(ev.client_x(), ev.client_y()) else { return };
        drag.set_value(Some((x, y, false)));
        if let Some(el) = container.get_untracked() {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
    };
    let on_move = move |ev: PointerEvent| {
        let Some((sx, sy, started)) = drag.get_value() else { return };
        let Some((x, y)) = local(ev.client_x(), ev.client_y()) else { return };
        if !started {
            if (y - sy).abs() < 3.0 {
                return;
            }
            drag.set_value(Some((sx, sy, true)));
            apply(sx, sy);
        }
        apply(x, y);
    };
    let on_up = move |_: PointerEvent| {
        if drag.get_value().is_some_and(|d| d.2) {
            active.set(None);
        }
        drag.set_value(None);
    };
    let on_dblclick = move |ev: web_sys::MouseEvent| {
        let Some((x, _)) = local(ev.client_x(), ev.client_y()) else { return };
        on_band.run((geo.get_untracked().band_at(x), 0.0));
    };
    let on_wheel = move |ev: WheelEvent| {
        let Some((x, _)) = local(ev.client_x(), ev.client_y()) else { return };
        ev.prevent_default();
        let px = ev.delta_y()
            * match ev.delta_mode() {
                1 => 40.0,
                2 => 800.0,
                _ => 1.0,
            };
        let i = geo.get_untracked().band_at(x);
        let current = bands.with_untracked(|b| b.get(i).copied().unwrap_or(0.0));
        on_band.run((i, (current - px / 50.0).clamp(-12.0, 12.0)));
    };

    // ─── Static layer: background, grid, labels ───
    let grid = move || {
        let g = geo.get();
        if !g.usable() {
            return ().into_any();
        }
        let (left, right, top, bottom) = (PAD_LEFT, PAD_LEFT + g.gw(), PAD_TOP, PAD_TOP + g.gh());
        let db_lines = [-12.0, -6.0, 0.0, 6.0, 12.0]
            .map(|db| {
                let y = g.y(db);
                let (stroke, width) = if db == 0.0 { ("var(--danger)", 1.0) } else { ("var(--muted)", 0.5) };
                let text = if db > 0.0 { format!("+{}", db as i64) } else { format!("{}", db as i64) };
                view! {
                    <line x1=left y1=y x2=right y2=y stroke=stroke stroke-width=width/>
                    <text class="rta-axis" x=left - 4.0 y=y + 3.0 text-anchor="end">{text}</text>
                }
            })
            .collect_view();
        let mut freqs: Vec<f64> = Vec::new();
        freqs.extend((2..10).map(|m| m as f64 * 10.0));
        freqs.extend((1..10).map(|m| m as f64 * 100.0));
        freqs.extend((1..10).map(|m| m as f64 * 1000.0));
        freqs.extend([10000.0, 20000.0]);
        let hz_lines = freqs
            .into_iter()
            .map(|hz| {
                let x = g.x(hz);
                let thick = hz == 100.0 || hz == 1000.0 || hz == 10000.0;
                view! { <line x1=x y1=top x2=x y2=bottom stroke="var(--muted)" stroke-width=if thick { 1.5 } else { 0.5 }/> }
            })
            .collect_view();
        let labels = [20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0]
            .map(|hz| {
                view! { <text class="rta-axis" x=g.x(hz) y=bottom + 13.0 text-anchor="middle">{format_freq(hz)}</text> }
            })
            .collect_view();
        view! {
            <rect x=left y=top width=g.gw() height=g.gh() fill="var(--bg)" stroke="var(--muted)" stroke-width="1"/>
            {hz_lines}
            {db_lines}
            {labels}
        }
        .into_any()
    };

    // ─── Curve layer ───
    let points = Memo::new(move |_| {
        let g = geo.get();
        bands.with(|b| (0..BANDS).map(|i| (g.x(GEQ_CENTER_FREQUENCIES[i]), g.y(b.get(i).copied().unwrap_or(0.0)))).collect::<Vec<_>>())
    });
    let curve = move || {
        let g = geo.get();
        if !g.usable() {
            return ().into_any();
        }
        let pts = points.get();
        let line = monotone_path(&pts);
        let zero = g.y(0.0);
        let fill = format!(
            "{line}L{:.1},{zero:.1}L{:.1},{zero:.1}Z",
            pts.last().map_or(0.0, |p| p.0),
            pts.first().map_or(0.0, |p| p.0)
        );
        view! {
            <path d=fill fill=CURVE_COLOR fill-opacity="0.18" stroke="none"/>
            <path d=line fill="none" stroke=CURVE_COLOR stroke-width="2.5" stroke-linejoin="round"/>
        }
        .into_any()
    };
    let proposal_view = move || {
        let g = geo.get();
        proposal.get().filter(|p| p.len() == BANDS && g.usable()).map(|p| {
            let pts: Vec<(f64, f64)> = (0..BANDS).map(|i| (g.x(GEQ_CENTER_FREQUENCIES[i]), g.y(p[i]))).collect();
            let dots = pts.iter().map(|(x, y)| view! { <circle cx=*x cy=*y r="2.5" fill=PROPOSAL_COLOR/> }).collect_view();
            view! {
                <path d=monotone_path(&pts) fill="none" stroke=PROPOSAL_COLOR stroke-width="2" stroke-linejoin="round"/>
                {dots}
            }
        })
    };
    let handles = (0..BANDS)
        .map(|i| {
            let db = Memo::new(move |_| bands.with(|b| b.get(i).copied().unwrap_or(0.0)));
            view! { <Handle index=i db=db geo=geo active=active/> }
        })
        .collect_view();
    let bubble = move || {
        let g = geo.get();
        let i = active.get().filter(|i| *i < BANDS && g.usable())?;
        let db = bands.with(|b| b.get(i).copied().unwrap_or(0.0));
        let (cx, y) = (g.x(GEQ_CENTER_FREQUENCIES[i]), g.y(db));
        let text = format!("{} dB", signed(db));
        let (w, h) = (text.chars().count() as f64 * 6.4 + 16.0, 21.0);
        let above = y - h - 8.0;
        let by = if above >= PAD_TOP { above } else { y + 12.0 };
        let bx = (cx - w / 2.0).clamp(PAD_LEFT, (PAD_LEFT + g.gw() - w).max(PAD_LEFT));
        Some(view! {
            <rect x=bx y=by width=w height=h rx="4" fill="rgba(34,38,44,0.87)"/>
            <text class="geq-bubble" x=bx + w / 2.0 y=by + 15.0 text-anchor="middle">{text}</text>
        })
    };

    view! {
        <div class="geq-graph" node_ref=container
            on:pointerdown=on_down on:pointermove=on_move on:pointerup=on_up on:pointercancel=on_up
            on:dblclick=on_dblclick on:wheel=on_wheel>
            <svg class="layer">{grid}</svg>
            <canvas class="layer" node_ref=canvas></canvas>
            <svg class="layer">
                {curve}
                {proposal_view}
                {handles}
                {bubble}
            </svg>
        </div>
    }
}

/// Point of one band on the curve with its dB value.
#[component]
fn Handle(index: usize, db: Memo<f64>, geo: Memo<Geo>, active: RwSignal<Option<usize>>) -> impl IntoView {
    move || {
        let g = geo.get();
        if !g.usable() {
            return ().into_any();
        }
        let v = db.get();
        let is_active = active.get() == Some(index);
        let (cx, cy) = (g.x(GEQ_CENTER_FREQUENCIES[index]), g.y(v));
        let r = 4.5 * if is_active { 1.6 } else { 1.0 };
        let stroke = if v == 0.0 { "var(--muted)" } else { CURVE_COLOR };
        let label = (v != 0.0).then(|| {
            let text = signed(v);
            let tw = text.chars().count() as f64 * 4.0;
            let y = if v >= 0.0 { cy - r - 2.0 } else { cy + r + 7.0 };
            let x = cx.clamp(PAD_LEFT + tw / 2.0, PAD_LEFT + g.gw() - tw / 2.0);
            (y >= PAD_TOP + 5.0 && y <= PAD_TOP + g.gh() + 2.0)
                .then(|| view! { <text class="geq-value" x=x y=y text-anchor="middle">{text}</text> })
        });
        view! {
            <circle cx=cx cy=cy r=r fill="var(--bg)" stroke=stroke stroke-width="2"/>
            {label}
        }
        .into_any()
    }
}

// ─── Fader bank ───

/// One fader per band, centred under the band's point of the curve: the same
/// x positions as the graph (both span the full content width).
#[component]
fn FaderBank(bands: Memo<Vec<f64>>, active: RwSignal<Option<usize>>, on_band: Callback<(usize, f64)>) -> impl IntoView {
    let positions: Vec<f64> = GEQ_CENTER_FREQUENCIES.iter().map(|f| norm(*f)).collect();
    let spacing = positions.windows(2).map(|w| w[1] - w[0]).fold(f64::INFINITY, f64::min);
    let width = format!("calc({spacing:.6} * (100% - {}px) - 2px)", PAD_LEFT + PAD_RIGHT);
    let faders = positions
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let db = Memo::new(move |_| bands.with(|b| b.get(i).copied().unwrap_or(0.0)));
            let style = format!(
                "left:calc({PAD_LEFT}px + {n:.6} * (100% - {}px) - ({width}) / 2);width:{width}",
                PAD_LEFT + PAD_RIGHT
            );
            let hz = GEQ_CENTER_FREQUENCIES[i];
            // The fader colour follows --band-c (accent while the band is active).
            let style = move || {
                let color = if active.get() == Some(i) { "var(--accent)" } else { "var(--input)" };
                format!("{style};--band-c:{color}")
            };
            view! {
                <div class="geq-band" style=style>
                    <div class="fader-box">
                        <Fader value=db taper=GEQ_TAPER marks=GEQ_MARKS.to_vec() major=vec![0.0] cap=GEQ_CAP
                            color="var(--band-c)" step=Signal::derive(|| 0.5) reset=0.0
                            on_change=Callback::new(move |v: f64| on_band.run((i, v)))/>
                    </div>
                    <ValueField height=20 font=9.0 title=format!("{} Hz", format_freq(hz))
                        text=Signal::derive(move || fixed(db.get(), 1))
                        on_submit=Callback::new(move |input: String| match parse_number(&input) {
                            Some(v) => {
                                on_band.run((i, v));
                                true
                            }
                            None => false,
                        })/>
                    <div class="geq-freq">{format_freq(hz)}</div>
                </div>
            }
        })
        .collect_view();
    let half = GEQ_CAP / 2.0 + 2.0;
    let scale = GEQ_MARKS
        .iter()
        .filter(|db| *db % 6.0 == 0.0)
        .map(|db| {
            let p = GEQ_TAPER.to_position(*db);
            let text = if *db > 0.0 { format!("+{}", *db as i64) } else { format!("{}", *db as i64) };
            view! { <span class="fader-label" class:major=*db == 0.0 style=format!("top:{:.3}%", (1.0 - p) * 100.0)>{text}</span> }
        })
        .collect_view();
    view! {
        <div class="geq-bank">
            <div class="geq-scale" style=format!("width:{}px", PAD_LEFT - 2.0)>
                <div class="geq-scale-travel" style=format!("top:{half}px;bottom:{half}px")>{scale}</div>
            </div>
            {faders}
        </div>
    }
}
