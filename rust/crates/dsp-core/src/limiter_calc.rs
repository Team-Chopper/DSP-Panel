//! Limiter calculator: from the data sheet of the amplifier behind an output
//! (rated power at 8/4/2 Ω, input sensitivity or voltage gain) and the load,
//! the power at which the output limiter holds the level, the headroom to the
//! amplifier's clipping, and the threshold for a wanted power.
//!
//! The DSP does not publish what output level its limiter threshold means.
//! The calculator assumes threshold 0 dB = [LimiterSetup::reference_dbu]
//! (default 0 dBu) at the analog output; measurements on a DSP 204 put the
//! threshold scale about 26 dB above the output meter in dBFS, and the
//! original editor's output LEDs sit 24 dB lower, both near 0 dBu. A planning
//! aid: verify with a measurement.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::protocol as p;

/// 0 dBu in volts (RMS).
pub const DBU_VOLTS: f64 = 0.774_596_7;
/// Limits of the data the server accepts.
pub const MAX_AMPS: usize = 64;
pub const MAX_WATTS: f64 = 100_000.0;

/// One amplifier model (data sheet values).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Amplifier {
    pub id: String,
    pub name: String,
    /// Rated power per channel in W at 8, 4 and 2 Ω (0 = not rated).
    pub watts8: f64,
    pub watts4: f64,
    pub watts2: f64,
    /// Input sensitivity in V (RMS) for the rated power, if given …
    #[serde(default)]
    pub sensitivity_v: Option<f64>,
    /// … otherwise the voltage gain in dB.
    #[serde(default)]
    pub gain_db: Option<f64>,
}

/// The amplifier channel and loudspeaker behind one DSP output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputAmp {
    pub amp: String,
    /// Load in Ω (nominal impedance of the speakers on this amp channel).
    pub ohms: f64,
    /// Rated (RMS / AES) power of the loudspeaker in W, 0 = unknown.
    #[serde(default)]
    pub speaker_w: f64,
}

/// Everything the calculator keeps (on the server, for every client).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimiterSetup {
    pub amps: Vec<Amplifier>,
    /// Per output, key "<dsp id>/<channel>".
    pub outputs: BTreeMap<String, OutputAmp>,
    /// Per DSP id: output level in dBu at limiter threshold 0 dB (default 0).
    #[serde(default)]
    pub reference_dbu: BTreeMap<String, f64>,
}

impl LimiterSetup {
    pub fn output_key(dsp: &str, channel: &str) -> String {
        format!("{dsp}/{channel}")
    }

    pub fn amp(&self, id: &str) -> Option<&Amplifier> {
        self.amps.iter().find(|a| a.id == id)
    }

    pub fn reference(&self, dsp: &str) -> f64 {
        self.reference_dbu.get(dsp).copied().unwrap_or(0.0)
    }

    /// The setup limited to sane values; outputs of unknown amplifiers are
    /// dropped. `Err` if it cannot be used at all.
    pub fn validated(mut self) -> Result<LimiterSetup, String> {
        if self.amps.len() > MAX_AMPS {
            return Err(format!("At most {MAX_AMPS} amplifiers"));
        }
        let finite = |v: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { lo };
        for a in &mut self.amps {
            a.id = a.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(24).collect();
            a.name = a.name.chars().filter(|c| !c.is_control()).take(40).collect();
            a.watts8 = finite(a.watts8, 0.0, MAX_WATTS);
            a.watts4 = finite(a.watts4, 0.0, MAX_WATTS);
            a.watts2 = finite(a.watts2, 0.0, MAX_WATTS);
            a.sensitivity_v = a.sensitivity_v.filter(|v| v.is_finite() && *v > 0.0).map(|v| v.clamp(0.05, 20.0));
            a.gain_db = a.gain_db.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 60.0));
            if a.id.is_empty() {
                return Err("Amplifier without id".into());
            }
        }
        let ids: Vec<String> = self.amps.iter().map(|a| a.id.clone()).collect();
        self.outputs.retain(|k, o| k.len() <= 80 && ids.contains(&o.amp));
        for o in self.outputs.values_mut() {
            o.ohms = finite(o.ohms, 1.0, 64.0);
            o.speaker_w = finite(o.speaker_w, 0.0, MAX_WATTS);
        }
        self.reference_dbu.retain(|k, v| k.len() <= 40 && v.is_finite());
        for v in self.reference_dbu.values_mut() {
            *v = v.clamp(-20.0, 30.0);
        }
        Ok(self)
    }
}

impl Amplifier {
    /// Rated power at [ohms]: interpolated between the rated loads; above the
    /// highest rated load the clip voltage stays the same; below the lowest
    /// one the amplifier is not rated (0).
    pub fn rated_watts(&self, ohms: f64) -> f64 {
        let points: Vec<(f64, f64)> =
            [(2.0, self.watts2), (4.0, self.watts4), (8.0, self.watts8)].into_iter().filter(|(_, w)| *w > 0.0).collect();
        let (Some(low), Some(high)) = (points.first(), points.last()) else { return 0.0 };
        if ohms < low.0 - 1e-9 {
            return 0.0;
        }
        if ohms >= high.0 {
            return high.1 * high.0 / ohms;
        }
        for pair in points.windows(2) {
            let ((r0, w0), (r1, w1)) = (pair[0], pair[1]);
            if ohms <= r1 {
                return w0 + (w1 - w0) * (ohms - r0) / (r1 - r0);
            }
        }
        high.1
    }

    /// Voltage gain (factor): from the gain in dB, or rated output voltage at
    /// the highest rated load divided by the sensitivity.
    pub fn voltage_gain(&self) -> Option<f64> {
        if let Some(s) = self.sensitivity_v {
            let (r, w) = [(8.0, self.watts8), (4.0, self.watts4), (2.0, self.watts2)].into_iter().find(|(_, w)| *w > 0.0)?;
            return Some((w * r).sqrt() / s);
        }
        self.gain_db.map(|g| 10f64.powf(g / 20.0))
    }
}

/// What a threshold means for one output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimitResult {
    /// Power into the load while the limiter holds the level, W.
    pub watts: f64,
    /// Rated power of the amplifier at this load, W (0 = not rated).
    pub rated_watts: f64,
    /// How far the limited level drives the amplifier past clipping, dB
    /// (> 0: the amplifier clips before the limiter acts).
    pub clip_db: f64,
}

/// Power at a limiter threshold of [threshold_db] (DSP scale).
pub fn limited_power(amp: &Amplifier, ohms: f64, threshold_db: f64, reference_dbu: f64) -> Option<LimitResult> {
    let gain = amp.voltage_gain()?;
    let rated = amp.rated_watts(ohms);
    let v_out = DBU_VOLTS * 10f64.powf((threshold_db + reference_dbu) / 20.0) * gain;
    let v_clip = (rated * ohms).sqrt();
    let v = if rated > 0.0 { v_out.min(v_clip) } else { v_out };
    let clip_db = if rated > 0.0 { 20.0 * (v_out / v_clip).log10() } else { 0.0 };
    Some(LimitResult { watts: v * v / ohms, rated_watts: rated, clip_db })
}

/// Raw limiter threshold (0.5 dB steps) that keeps the power at or below
/// [watts]: rounded down, never above the target.
pub fn threshold_for_watts(amp: &Amplifier, ohms: f64, watts: f64, reference_dbu: f64) -> Option<i64> {
    let gain = amp.voltage_gain()?;
    if watts <= 0.0 {
        return None;
    }
    let v_in = (watts * ohms).sqrt() / gain;
    let db = 20.0 * (v_in / DBU_VOLTS).log10() - reference_dbu;
    Some(((db * 2.0).floor() as i64 + 180).clamp(0, p::THRESHOLD_RAW_MAX))
}

/// Attack and release (ms) for a band whose lowest frequency is [low_hz]
/// (the HPF of the output): attack about one period, release 16 × attack.
pub fn timing_for_band(low_hz: f64) -> (f64, f64) {
    let attack = (1000.0 / low_hz.max(20.0)).clamp(1.0, 50.0).round();
    (attack, (attack * 16.0).clamp(10.0, 3000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amp() -> Amplifier {
        Amplifier {
            id: "a".into(),
            name: "Test".into(),
            watts8: 500.0,
            watts4: 800.0,
            watts2: 0.0,
            sensitivity_v: Some(1.4),
            gain_db: None,
        }
    }

    #[test]
    fn rated_power_over_the_load() {
        let a = amp();
        assert_eq!(a.rated_watts(8.0), 500.0);
        assert_eq!(a.rated_watts(4.0), 800.0);
        assert_eq!(a.rated_watts(6.0), 650.0);
        assert_eq!(a.rated_watts(16.0), 250.0, "same clip voltage above 8 Ω");
        assert_eq!(a.rated_watts(2.0), 0.0, "not rated for 2 Ω");
        let g = a.voltage_gain().unwrap();
        assert!((g - (4000f64).sqrt() / 1.4).abs() < 1e-9);
        let by_gain = Amplifier { sensitivity_v: None, gain_db: Some(32.0), ..amp() };
        assert!((20.0 * by_gain.voltage_gain().unwrap().log10() - 32.0).abs() < 1e-9);
    }

    #[test]
    fn threshold_and_power_both_ways() {
        let a = amp();
        // At the sensitivity the amp delivers its rated power.
        let sens_dbu = 20.0 * (1.4 / DBU_VOLTS).log10();
        let r = limited_power(&a, 8.0, sens_dbu, 0.0).unwrap();
        assert!((r.watts - 500.0).abs() < 0.01 && r.clip_db.abs() < 1e-9);
        let over = limited_power(&a, 8.0, sens_dbu + 3.0, 0.0).unwrap();
        assert!((over.watts - 500.0).abs() < 0.01 && (over.clip_db - 3.0).abs() < 1e-9, "clipped");
        // 250 W into 8 Ω is 3 dB below the rated power.
        let raw = threshold_for_watts(&a, 8.0, 250.0, 0.0).unwrap();
        let db = p::threshold_raw_to_db(raw);
        assert!(db <= sens_dbu - 3.0 + 1e-9 && db > sens_dbu - 3.5, "{db}");
        assert!(limited_power(&a, 8.0, db, 0.0).unwrap().watts <= 250.0);
        // A higher reference level means a lower threshold for the same power.
        assert_eq!(threshold_for_watts(&a, 8.0, 250.0, 6.0).unwrap(), raw - 12);
    }

    #[test]
    fn validation_drops_unknown_amps() {
        let mut setup = LimiterSetup { amps: vec![amp()], ..LimiterSetup::default() };
        setup.outputs.insert("d/Out 1".into(), OutputAmp { amp: "a".into(), ohms: 0.0, speaker_w: -5.0 });
        setup.outputs.insert("d/Out 2".into(), OutputAmp { amp: "x".into(), ohms: 8.0, speaker_w: 0.0 });
        let v = setup.validated().unwrap();
        assert_eq!(v.outputs.len(), 1);
        assert_eq!(v.outputs["d/Out 1"], OutputAmp { amp: "a".into(), ohms: 1.0, speaker_w: 0.0 });
    }

    #[test]
    fn band_timing() {
        assert_eq!(timing_for_band(40.0), (25.0, 400.0));
        assert_eq!(timing_for_band(1000.0), (1.0, 16.0));
    }
}
