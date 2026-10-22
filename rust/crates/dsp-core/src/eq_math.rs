//! EQ response math for the EQ curves and the Auto-EQ.
//!
//! PEQ bands are drawn like the original editor draws them: bilinear biquads
//! in Zölzer's K form at 48 kHz (K = tan(π·f/48000)). A peak boost is
//! (s² + V·s/Q + 1) / (s² + s/Q + 1) with V = 10^(|G|/20), a cut its exact
//! inverse; shelves are 2nd order with the band's Q in place of 1/√2, a cut the
//! inverse of the boost; LP/HP -6 are 1st order, LP/HP -12 2nd order with Q.
//! The digital response is the analog prototype at the pre-warped frequency
//! tan(π·f/fs) / tan(π·fc/fs). The HPF/LPF crossover slopes are drawn as
//! Butterworth (−3 dB at the cut-off), Linkwitz-Riley (−6 dB) and Bessel
//! (−3 dB) filters of the slope's order.

use std::f64::consts::{LN_10, PI};
use std::sync::OnceLock;

use crate::model::GEQ_BAND_COUNT;
use crate::protocol as p;
use crate::state::{DeviceState, PeqBand};

/// ISO 1/3-octave centre frequencies of the 31 GEQ bands.
pub const GEQ_CENTER_FREQUENCIES: [f64; GEQ_BAND_COUNT] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0, 630.0, 800.0,
    1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10000.0, 12500.0, 16000.0,
    20000.0,
];

/// Sample rate the original editor draws its EQ curves with.
pub const EQ_DRAW_SAMPLE_RATE: f64 = 48000.0;

// ─── Polynomials at s = j·w ───

/// |p(jw)|² of a polynomial with real coefficients (ascending powers):
/// j^k cycles 1, j, −1, −j.
fn abs2(c: &[f64], w: f64) -> f64 {
    let (mut re, mut im, mut power) = (0.0, 0.0, 1.0);
    for (k, coefficient) in c.iter().enumerate() {
        let term = coefficient * power;
        match k & 3 {
            0 => re += term,
            1 => im += term,
            2 => re -= term,
            _ => im -= term,
        }
        power *= w;
    }
    re * re + im * im
}

/// |num(jw) / den(jw)| in dB.
fn ratio_db(num: &[f64], den: &[f64], w: f64) -> f64 {
    let (n, d) = (abs2(num, w), abs2(den, w));
    if d <= 0.0 || n <= 0.0 {
        return if n <= 0.0 { -120.0 } else { 0.0 };
    }
    10.0 * (n / d).ln() / LN_10
}

/// |(a0 + a1 s + a2 s²) / (b0 + b1 s + b2 s²)| in dB at s = j·w.
fn biquad_db(a0: f64, a1: f64, a2: f64, b0: f64, b1: f64, b2: f64, w: f64) -> f64 {
    let w2 = w * w;
    let (nr, ni) = (a0 - a2 * w2, a1 * w);
    let (dr, di) = (b0 - b2 * w2, b1 * w);
    let (n, d) = (nr * nr + ni * ni, dr * dr + di * di);
    if d <= 0.0 || n <= 0.0 {
        return if n <= 0.0 { -120.0 } else { 0.0 };
    }
    10.0 * (n / d).ln() / LN_10
}

// ─── Crossover (HPF/LPF) ───

/// Filter family ("BW", "BL", "LK") and order of a crossover slope index.
pub fn crossover_shape(slope: i64) -> (&'static str, i64) {
    let name = p::CROSSOVER_SLOPE_NAMES[slope.clamp(0, p::CROSSOVER_SLOPE_NAMES.len() as i64 - 1) as usize];
    (name.split(' ').next().unwrap_or(name), p::slope_db_per_octave(slope) / 6)
}

/// Reverse Bessel polynomial coefficients of [order], frequency-scaled so that
/// the low-pass is −3 dB at w = 1.
fn bessel(order: usize) -> &'static [f64] {
    static CACHE: OnceLock<Vec<Vec<f64>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| (0..=8).map(compute_bessel).collect());
    &cache[order.min(8)]
}

fn compute_bessel(order: usize) -> Vec<f64> {
    fn fact(n: usize) -> f64 {
        (1..=n).fold(1.0, |acc, k| acc * k as f64)
    }
    let base: Vec<f64> = (0..=order)
        .map(|k| fact(2 * order - k) / (2f64.powi((order - k) as i32) * fact(k) * fact(order - k)))
        .collect();
    let scaled = |a: f64| -> Vec<f64> { (0..=order).map(|k| base[k] * a.powi(k as i32)).collect() };
    let gain_at_1 = |a: f64| ratio_db(&[base[0]], &scaled(a), 1.0);
    // Bisection on the scale factor for −3.01 dB at w = 1.
    let (mut lo, mut hi) = (0.01f64, 100.0f64);
    for _ in 0..80 {
        let mid = (lo * hi).sqrt();
        if gain_at_1(mid) > -3.0103 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    scaled((lo * hi).sqrt())
}

/// HPF/LPF response in dB at [freq] for a crossover slope index (0 = bypass).
pub fn filter_response_db(freq: f64, cutoff_hz: f64, slope: i64, is_high_pass: bool) -> f64 {
    if cutoff_hz <= 0.0 || slope <= 0 {
        return 0.0;
    }
    let (family, order) = crossover_shape(slope);
    if order <= 0 {
        return 0.0;
    }
    // A high-pass is the low-pass mirrored on a log-frequency axis.
    let w = if is_high_pass { cutoff_hz / freq } else { freq / cutoff_hz };
    match family {
        // Linkwitz-Riley: two cascaded Butterworth filters of half the order.
        "LK" => -20.0 * (1.0 + w.powi(order as i32)).ln() / LN_10,
        "BL" => {
            let den = bessel(order as usize);
            ratio_db(&[den[0]], den, w)
        }
        // Butterworth
        _ => -10.0 * (1.0 + w.powi(2 * order as i32)).ln() / LN_10,
    }
}

// ─── PEQ band ───

/// Frequency of the analog prototype for [freq] when the digital filter is
/// tuned to [center] (bilinear transform with pre-warping).
fn warped(freq: f64, center: f64) -> f64 {
    const LIMIT: f64 = EQ_DRAW_SAMPLE_RATE / 2.0 * 0.999;
    let k = |f: f64| (PI * f.min(LIMIT) / EQ_DRAW_SAMPLE_RATE).tan();
    k(freq) / k(center)
}

/// Response of one PEQ band in dB at [freq].
pub fn peq_response_db(freq: f64, center: f64, q: f64, gain_db: f64, peq_type: i64) -> f64 {
    if center <= 0.0 {
        return 0.0;
    }
    let q = q.clamp(0.05, 200.0);
    let w = warped(freq, center);
    let v = 10f64.powf(gain_db.abs() / 20.0);
    // Cuts are the exact inverse of the boost with the same |gain|.
    let boost_or_cut = |boost: f64| if gain_db >= 0.0 { boost } else { -boost };
    match peq_type {
        // Peak: (s² + V·s/Q + 1) / (s² + s/Q + 1)
        0 => {
            if gain_db.abs() < 0.01 {
                return 0.0;
            }
            boost_or_cut(biquad_db(1.0, v / q, 1.0, 1.0, 1.0 / q, 1.0, w))
        }
        // Low shelf: (V + √V·s/Q + s²) / (1 + s/Q + s²)
        1 => {
            if gain_db.abs() < 0.01 {
                return 0.0;
            }
            boost_or_cut(biquad_db(v, v.sqrt() / q, 1.0, 1.0, 1.0 / q, 1.0, w))
        }
        // High shelf: (1 + √V·s/Q + V·s²) / (1 + s/Q + s²)
        2 => {
            if gain_db.abs() < 0.01 {
                return 0.0;
            }
            boost_or_cut(biquad_db(1.0, v.sqrt() / q, v, 1.0, 1.0 / q, 1.0, w))
        }
        // LP -6 dB: 1 / (1 + s)
        3 => biquad_db(1.0, 0.0, 0.0, 1.0, 1.0, 0.0, w),
        // LP -12 dB: 1 / (s² + s/Q + 1)
        4 => biquad_db(1.0, 0.0, 0.0, 1.0, 1.0 / q, 1.0, w),
        // HP -6 dB: s / (1 + s)
        5 => biquad_db(0.0, 1.0, 0.0, 1.0, 1.0, 0.0, w),
        // HP -12 dB: s² / (s² + s/Q + 1)
        6 => biquad_db(0.0, 0.0, 1.0, 1.0, 1.0 / q, 1.0, w),
        // All pass: no magnitude change
        _ => 0.0,
    }
}

/// Whether a band changes the magnitude at all (used to skip flat curves).
pub fn peq_band_audible(b: &PeqBand) -> bool {
    !b.bypass && ((3..=6).contains(&b.peq_type) || b.gain_db.abs() >= 0.05 && b.peq_type <= 2)
}

/// GEQ response in dB at [freq]: bands interpolated on a log-frequency axis.
pub fn geq_response_db(freq: f64, bands: &[f64]) -> f64 {
    let (Some(first), Some(last)) = (bands.first(), bands.last()) else {
        return 0.0;
    };
    let centers = &GEQ_CENTER_FREQUENCIES;
    if freq <= centers[0] {
        return *first;
    }
    if freq >= centers[centers.len() - 1] {
        return *last;
    }
    for i in 0..centers.len() - 1 {
        let (f0, f1) = (centers[i], centers[i + 1]);
        if freq <= f1 {
            let t = (freq / f0).ln() / (f1 / f0).ln();
            let (b0, b1) = (bands.get(i).copied().unwrap_or(0.0), bands.get(i + 1).copied().unwrap_or(0.0));
            return b0 + (b1 - b0) * t;
        }
    }
    0.0
}

/// Free PEQ bands for an Auto-EQ: peak bands at 0 dB (bypassed or not). Bands
/// in use (shelves, pass filters, peaks with gain) are not touched.
pub fn free_peq_bands(bands: &[PeqBand]) -> Vec<usize> {
    bands.iter().enumerate().filter(|(_, b)| b.peq_type == 0 && b.gain_db.abs() < 0.05).map(|(i, _)| i).collect()
}

/// Limits of [fit_peq_correction].
#[derive(Clone, Copy, Debug)]
pub struct FitLimits {
    pub max_boost: f64,
    pub max_cut: f64,
    pub min_db: f64,
}

impl Default for FitLimits {
    fn default() -> Self {
        FitLimits { max_boost: 6.0, max_cut: 12.0, min_db: 1.0 }
    }
}

/// Auto-EQ for a channel without GEQ: approximates [target] (the wanted change
/// in dB at the 1/3-octave [centers]) with peak filters in the free bands of
/// [bands]. Greedy: the largest remaining deviation gets the next free band,
/// its width sets Q; stops when the rest is below `min_db`. Returns the new
/// band list (unchanged bands are copies).
pub fn fit_peq_correction(target: &[f64], centers: &[f64], bands: &[PeqBand], limits: FitLimits) -> Vec<PeqBand> {
    let FitLimits { max_boost, max_cut, min_db } = limits;
    let mut result = bands.to_vec();
    let mut residual = target.to_vec();
    if residual.is_empty() {
        return result;
    }
    for index in free_peq_bands(bands) {
        // Largest deviation that can be corrected (boosts may be limited to 0).
        let usable = |r: &[f64], i: usize| r[i].clamp(-max_cut, max_boost).abs();
        let mut k = 0;
        for i in 1..residual.len() {
            if usable(&residual, i) > usable(&residual, k) {
                k = i;
            }
        }
        let peak = residual[k];
        if usable(&residual, k) < min_db || peak.abs() < min_db {
            break;
        }
        // Width: neighbours with the same sign and at least half the deviation.
        let similar = |r: &[f64], i: usize| r[i].signum() == peak.signum() && r[i].abs() >= peak.abs() / 2.0;
        let (mut lo, mut hi) = (k, k);
        while lo > 0 && similar(&residual, lo - 1) {
            lo -= 1;
        }
        while hi < residual.len() - 1 && similar(&residual, hi + 1) {
            hi += 1;
        }
        let octaves = (hi - lo + 1) as f64 / 3.0;
        let n = 2f64.powf(octaves);
        let q = (n.sqrt() / (n - 1.0)).clamp(0.5, 8.0);
        let gain = (peak.clamp(-max_cut, max_boost) * 10.0).round() / 10.0;
        let band = PeqBand {
            freq_raw: p::peq_hz_to_freq(centers[k]),
            q_raw: p::peq_q_to_raw(q),
            gain_db: gain,
            ..PeqBand::default()
        };
        let (fc, bq) = (p::peq_freq_to_hz(band.freq_raw), p::peq_raw_to_q(band.q_raw));
        result[index] = band;
        for (i, r) in residual.iter_mut().enumerate() {
            *r -= peq_response_db(centers[i], fc, bq, gain, 0);
        }
    }
    result
}

/// The EQ settings of one channel, read once, for evaluating its total
/// response at many frequencies (a curve has ~160–240 points).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelEq {
    /// Audible PEQ bands: (Hz, Q, gain dB, type).
    bands: Vec<(f64, f64, f64, i64)>,
    hp: Option<(f64, i64)>,
    lp: Option<(f64, i64)>,
    geq: Option<Vec<f64>>,
}

impl ChannelEq {
    pub fn of(state: &DeviceState, channel: &str) -> ChannelEq {
        let bypassed = state.eq_bypass.get(channel).copied().unwrap_or(false);
        let bands = if bypassed {
            Vec::new()
        } else {
            state
                .peq_bands
                .get(channel)
                .into_iter()
                .flatten()
                .filter(|b| peq_band_audible(b))
                .map(|b| (p::peq_freq_to_hz(b.freq_raw), p::peq_raw_to_q(b.q_raw), b.gain_db, b.peq_type))
                .collect()
        };
        let filter = |f: Option<&crate::state::FilterState>| {
            f.filter(|f| f.enabled()).map(|f| (p::peq_freq_to_hz(f.freq_raw), f.slope))
        };
        let geq_on = state.model.is_input(channel) && !state.geq_bypass.get(channel).copied().unwrap_or(false);
        ChannelEq {
            bands,
            hp: filter(state.hi_pass.get(channel)),
            lp: filter(state.lo_pass.get(channel)),
            geq: if geq_on { state.geq_bands.get(channel).cloned() } else { None },
        }
    }

    /// Total response in dB at [freq]: PEQ bands (not bypassed), HPF/LPF and,
    /// for inputs, the GEQ (unless the PEQ or GEQ of the channel is bypassed).
    pub fn db(&self, freq: f64) -> f64 {
        let mut db = 0.0;
        for (hz, q, gain, t) in &self.bands {
            db += peq_response_db(freq, *hz, *q, *gain, *t);
        }
        if let Some((hz, slope)) = self.hp {
            db += filter_response_db(freq, hz, slope, true);
        }
        if let Some((hz, slope)) = self.lp {
            db += filter_response_db(freq, hz, slope, false);
        }
        if let Some(geq) = &self.geq {
            db += geq_response_db(freq, geq);
        }
        db
    }
}

/// Total response of a channel in dB at [freq] (see [ChannelEq] for many
/// frequencies).
pub fn channel_response_db(state: &DeviceState, channel: &str, freq: f64) -> f64 {
    ChannelEq::of(state, channel).db(freq)
}
