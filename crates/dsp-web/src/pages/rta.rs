//! Real-time analysis (RTA): microphone spectrum in 1/24-octave bands with
//! averaging and peak hold, plus the browser and DSP signal generators.
//!
//! The grid and labels are an SVG layer that changes with the size only; the
//! bars are drawn on a canvas per spectrum (~12 per second). On narrow
//! screens the graph is 3.5 times wider than the view and scrolls sideways,
//! the dB labels stay in place.

use dsp_core::rta::{BANDS_PER_OCTAVE, DB_RANGE, FFT_SIZE, MIN_DB, Averaging};
use leptos::html;
use leptos::prelude::*;

use crate::components::common::Segmented;
use crate::components::size::use_size;
use crate::components::spectrum::{self, BarStyle};
use crate::fmt::fixed;
use crate::icons::{I, Icon};
use crate::pages::dsp::ui;
use crate::pages::generators::{DspGeneratorPanel, ToneGeneratorPanel};
use crate::pages::geq::format_freq;
use crate::rta::Rta;
use crate::store::app;

const MIN_HZ: f64 = 20.0;
const MAX_HZ: f64 = 20000.0;
const PAD_RIGHT: f64 = 8.0;
const DB_STEPS: [f64; 9] = [10.0, 0.0, -20.0, -40.0, -60.0, -80.0, -100.0, -120.0, -140.0];
const NARROW_ZOOM: f64 = 3.5;

fn norm(hz: f64) -> f64 {
    (hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()
}

#[derive(Clone, Copy, PartialEq)]
struct Geo {
    /// Width of the drawn content (wider than the view when narrow).
    width: f64,
    height: f64,
    narrow: bool,
}

impl Geo {
    fn pad_left(&self) -> f64 {
        if self.narrow { 40.0 } else { 36.0 }
    }
    fn pad_top(&self) -> f64 {
        if self.narrow { 14.0 } else { 10.0 }
    }
    fn pad_bottom(&self) -> f64 {
        if self.narrow { 36.0 } else { 28.0 }
    }
    fn gw(&self) -> f64 {
        (self.width - self.pad_left() - PAD_RIGHT).max(1.0)
    }
    fn gh(&self) -> f64 {
        (self.height - self.pad_top() - self.pad_bottom()).max(1.0)
    }
    fn x(&self, hz: f64) -> f64 {
        self.pad_left() + norm(hz) * self.gw()
    }
    fn y(&self, db: f64) -> f64 {
        self.pad_top() + self.gh() * (1.0 - (db - MIN_DB) / DB_RANGE)
    }
    fn usable(&self) -> bool {
        self.width > 80.0 && self.height > 60.0
    }
    fn label_size(&self) -> f64 {
        if self.narrow { 12.0 } else { 9.0 }
    }
}

#[component]
pub fn RtaPage() -> impl IntoView {
    let device = ui().device;
    let rta = Rta::new();
    Effect::new(move |_| {
        if let Some(e) = rta.error.get() {
            app().toast(format!("Failed to start: {e}"), false);
        }
    });
    let on = move || rta.running.get() || rta.starting.get();
    let averaging_index = Signal::derive(move || Averaging::ALL.iter().position(|a| *a == rta.averaging.get()).unwrap_or(0));
    let info = move || {
        let rate = rta.sample_rate.get();
        format!(
            "1/{BANDS_PER_OCTAVE} octave · FFT {FFT_SIZE} · {} kHz · {} Hz/bin",
            fixed(rate / 1000.0, 1),
            fixed(rate / FFT_SIZE as f64, 1)
        )
    };
    view! {
        <div class="rta-page">
            <RtaGraph rta=rta/>
            // The RTA and the browser generator only look and listen: allowed in view-only mode.
            <div class="rta-controls wrap-row" data-view-ok>
                <button class="btn compact" style=move || format!("background:{};color:var(--bg-deep)",
                        if on() { "var(--danger)" } else { "var(--accent)" })
                    on:click=move |_| if on() { rta.stop() } else { rta.start() }>
                    {move || if on() { view! { <Icon icon=I::Stop size=18/> } } else { view! { <Icon icon=I::Mic size=18/> } }}
                    {move || if on() { "Stop" } else { "Start" }}
                </button>
                <Segmented labels=Averaging::ALL.iter().map(|a| a.label()).collect() selected=averaging_index
                    on_select=Callback::new(move |i: usize| rta.set_averaging(Averaging::ALL[i]))/>
                <button class="btn outlined compact" on:click=move |_| rta.reset_peaks()>"Reset peaks"</button>
                <span class="small muted">{info}</span>
                <span class="small muted">"Auto-EQ: GEQ page (inputs) or EQ page (any channel)"</span>
            </div>
            <div class="gen-box">
                <div class="gen-line" data-view-ok>
                    <span class="gen-title" title="Plays through the audio output of this computer/phone">"Browser generator"</span>
                    <ToneGeneratorPanel/>
                </div>
                {(!device.offline).then(|| view! {
                    <div class="gen-divider"></div>
                    <div class="gen-line">
                        <span class="gen-title" title="Generator built into the DSP (cmd 0x39)">"DSP generator"</span>
                        <DspGeneratorPanel device=device/>
                    </div>
                })}
            </div>
        </div>
    }
}

#[component]
fn RtaGraph(rta: Rta) -> impl IntoView {
    let container = NodeRef::<html::Div>::new();
    let canvas = NodeRef::<html::Canvas>::new();
    let size = use_size(container);
    let geo = Memo::new(move |_| {
        let (w, h) = size.get();
        let narrow = w > 0.0 && w < 500.0;
        Geo { width: if narrow { w * NARROW_ZOOM } else { w }, height: h, narrow }
    });

    // Bars: redrawn per spectrum.
    Effect::new(move |_| {
        rta.frame.track();
        let g = geo.get();
        let Some(el) = canvas.get() else { return };
        let Some(ctx) = spectrum::prepare(&el, g.width, g.height) else { return };
        if !g.usable() {
            return;
        }
        let style = BarStyle {
            bar: "rgba(74,222,128,0.71)",
            peak: "#E6E8EB",
            peak_width: 1.5,
            gaps: true,
            hide_silent_peaks: true,
        };
        rta.with_spectrum(|levels, peaks| {
            spectrum::draw_bars(
                &ctx,
                levels,
                peaks,
                |hz| g.x(hz),
                g.pad_left(),
                g.pad_left() + g.gw(),
                g.pad_top(),
                g.gh(),
                &style,
            );
        });
    });

    let db_labels = move |g: Geo| {
        DB_STEPS
            .map(|db| {
                view! {
                    <text class="rta-label" x=g.pad_left() - 4.0 y=g.y(db) + g.label_size() / 3.0 text-anchor="end"
                        font-size=g.label_size()>{format!("{}", db as i64)}</text>
                }
            })
            .collect_view()
    };
    let grid = move || {
        let g = geo.get();
        if !g.usable() {
            return ().into_any();
        }
        let (left, right, top, bottom) = (g.pad_left(), g.pad_left() + g.gw(), g.pad_top(), g.pad_top() + g.gh());
        let db_lines = DB_STEPS
            .map(|db| view! { <line x1=left y1=g.y(db) x2=right y2=g.y(db) stroke="var(--muted)" stroke-width="0.5"/> })
            .collect_view();
        let mut freqs: Vec<f64> = (2..10).map(|m| m as f64 * 10.0).collect();
        freqs.extend((1..10).map(|m| m as f64 * 100.0));
        freqs.extend((1..10).map(|m| m as f64 * 1000.0));
        freqs.extend([10000.0, 20000.0]);
        let hz_lines = freqs
            .into_iter()
            .map(|hz| {
                let thick = hz == 100.0 || hz == 1000.0 || hz == 10000.0;
                view! { <line x1=g.x(hz) y1=top x2=g.x(hz) y2=bottom stroke="var(--muted)" stroke-width=if thick { 1.5 } else { 0.5 }/> }
            })
            .collect_view();
        let label_hz: &[f64] = if g.narrow {
            &[20.0, 31.5, 50.0, 80.0, 100.0, 160.0, 250.0, 500.0, 800.0, 1000.0, 1600.0, 2500.0, 5000.0, 8000.0, 10000.0, 16000.0, 20000.0]
        } else {
            &[20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0]
        };
        let labels = label_hz
            .iter()
            .map(|hz| {
                view! {
                    <text class="rta-label" x=g.x(*hz) y=bottom + 4.0 + g.label_size() text-anchor="middle"
                        font-size=g.label_size()>{format_freq(*hz)}</text>
                }
            })
            .collect_view();
        view! {
            <rect x=left y=top width=g.gw() height=g.gh() fill="var(--bg)" stroke="var(--muted)" stroke-width="1"/>
            {db_lines}
            {hz_lines}
            {labels}
            {(!g.narrow).then(|| db_labels(g))}
        }
        .into_any()
    };
    // Narrow: the dB labels stay in place while the graph scrolls.
    let fixed_axis = move || {
        let g = geo.get();
        (g.narrow && g.usable()).then(|| {
            view! {
                <svg class="rta-fixed-axis" width=g.pad_left() height=g.height>
                    <rect x="0" y="0" width=g.pad_left() height=g.height fill="var(--bg)"/>
                    {db_labels(g)}
                </svg>
            }
        })
    };
    view! {
        <div class="rta-graph" node_ref=container>
            <div class="rta-scroll">
                <div class="rta-content" style=move || format!("width:{}px", geo.get().width)>
                    <svg class="layer">{grid}</svg>
                    <canvas class="layer" node_ref=canvas></canvas>
                </div>
            </div>
            {fixed_axis}
        </div>
    }
}
