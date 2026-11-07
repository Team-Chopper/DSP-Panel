//! Meters without reactivity: meter messages (every 300 ms by default, up to
//! 40/s) write straight into the registered DOM elements as CSS transforms,
//! so a meter update never re-renders anything and the browser only
//! composites the moved layers.

use dsp_core::protocol::{SILENCE_DB, THRESHOLD_RAW_MAX, compressor_active, limiter_active};
use dsp_core::{DeviceModel, DeviceState};
use web_sys::{Element, HtmlElement};

use crate::taper::Taper;

const PEAK_HOLD_MS: f64 = 900.0;
const PEAK_FALL_DB_PER_SECOND: f64 = 20.0;
const INDICATOR_HOLD_MS: f64 = 1500.0;

/// Level in dBFS at which the clip indicator lights up.
pub const CLIP_DB: f64 = -0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Led {
    Clip,
    Comp,
    Limit,
}

enum View {
    /// Vertical bar: the lit part is scaled, the peak line moved.
    Bar { lit: HtmlElement, peak: HtmlElement, taper: Taper },
    /// An indicator that gets the class "on".
    Led { el: Element, led: Led },
}

struct Registration {
    id: u64,
    channel: String,
    view: View,
}

#[derive(Default)]
pub struct MeterBank {
    channels: Vec<String>,
    db: Vec<f64>,
    peak: Vec<f64>,
    peak_until: Vec<f64>,
    clip_until: Vec<f64>,
    comp_until: Vec<f64>,
    limit_until: Vec<f64>,
    last_time: Option<f64>,
    views: Vec<Registration>,
    next_id: u64,
}

fn now() -> f64 {
    js_sys::Date::now()
}

impl MeterBank {
    pub fn new(model: &DeviceModel) -> Self {
        let mut bank = MeterBank::default();
        bank.reset(model);
        bank
    }

    pub fn reset(&mut self, model: &DeviceModel) {
        let n = model.channel_count();
        self.channels = model.channels().map(String::from).collect();
        self.db = vec![SILENCE_DB; n];
        self.peak = vec![SILENCE_DB; n];
        self.peak_until = vec![0.0; n];
        self.clip_until = vec![0.0; n];
        self.comp_until = vec![0.0; n];
        self.limit_until = vec![0.0; n];
        self.last_time = None;
        self.paint_all();
    }

    fn index(&self, channel: &str) -> Option<usize> {
        self.channels.iter().position(|c| c == channel)
    }

    pub fn register_bar(&mut self, channel: &str, lit: HtmlElement, peak: HtmlElement, taper: Taper) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.views.push(Registration { id, channel: channel.into(), view: View::Bar { lit, peak, taper } });
        self.paint(self.views.len() - 1);
        id
    }

    pub fn register_led(&mut self, channel: &str, el: Element, led: Led) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.views.push(Registration { id, channel: channel.into(), view: View::Led { el, led } });
        self.paint(self.views.len() - 1);
        id
    }

    pub fn unregister(&mut self, id: u64) {
        self.views.retain(|v| v.id != id);
    }

    /// New meter levels in dBFS (channel order) and the compressor/limiter
    /// status word of the outputs. Like the original editor, the compressor
    /// LED only lights for a ratio other than 1:1, the limiter LED only for a
    /// threshold below +20 dB.
    pub fn apply(&mut self, levels: &[f64], dynamics: u16, state: &DeviceState) {
        let t = now();
        let dt = self.last_time.map_or(0.0, |last| (t - last) / 1000.0);
        self.last_time = Some(t);
        if levels.len() != self.db.len() {
            self.reset(&state.model);
            if levels.len() != self.db.len() {
                return;
            }
        }
        let model = state.model;
        for (i, db) in levels.iter().copied().enumerate() {
            // Peak hold: jumps up, falls slowly after a short hold.
            if db >= self.peak[i] {
                self.peak[i] = db;
                self.peak_until[i] = t + PEAK_HOLD_MS;
            } else if t > self.peak_until[i] {
                self.peak[i] = db.max(self.peak[i] - PEAK_FALL_DB_PER_SECOND * dt);
            }
            if db >= CLIP_DB {
                self.clip_until[i] = t + INDICATOR_HOLD_MS;
            }
            let ch = &self.channels[i];
            if let Some(output) = model.output_index(ch) {
                let comp_on = state.compressors.get(ch).is_some_and(|c| c.ratio != 0);
                if compressor_active(dynamics, output) && comp_on {
                    self.comp_until[i] = t + INDICATOR_HOLD_MS;
                }
                let limit_on = state.limiters.get(ch).is_some_and(|l| l.threshold < THRESHOLD_RAW_MAX);
                if limiter_active(dynamics, output) && limit_on {
                    self.limit_until[i] = t + INDICATOR_HOLD_MS;
                }
            }
            self.db[i] = db;
        }
        self.paint_all();
    }

    fn paint_all(&self) {
        for i in 0..self.views.len() {
            self.paint(i);
        }
    }

    fn paint(&self, index: usize) {
        let reg = &self.views[index];
        let Some(i) = self.index(&reg.channel) else { return };
        match &reg.view {
            View::Bar { lit, peak, taper } => {
                // The lit layer keeps its gradient fixed to the meter and is
                // clipped (no layout, a small repaint).
                let level = taper.to_position(self.db[i]);
                let _ = lit.style().set_property("clip-path", &format!("inset({:.3}% 0 0 0)", (1.0 - level) * 100.0));
                let p = taper.to_position(self.peak[i]);
                let style = peak.style();
                let _ = style.set_property("bottom", &format!("{:.3}%", p * 100.0));
                let _ = style.set_property("opacity", if p > 0.01 { "1" } else { "0" });
                // Colour of the meter gradient at the peak position.
                let color = if p >= 0.78 {
                    "var(--danger)"
                } else if p >= 0.66 {
                    "#FDE68A"
                } else {
                    "var(--accent)"
                };
                let _ = style.set_property("background", color);
            }
            View::Led { el, led } => {
                let until = match led {
                    Led::Clip => self.clip_until[i],
                    Led::Comp => self.comp_until[i],
                    Led::Limit => self.limit_until[i],
                };
                let _ = el.class_list().toggle_with_force("on", now() < until);
            }
        }
    }
}
