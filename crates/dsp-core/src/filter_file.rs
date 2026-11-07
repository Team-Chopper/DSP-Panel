//! Filter settings text files as exported by room measurement software
//! (one line per filter, e.g. `Filter  1: ON  PK  Fc 63.0 Hz  Gain -5.0 dB
//! Q 4.00`, or `Filter: ON LS Fc 100 Hz Gain 3 dB` without a number), mapped
//! onto the PEQ bands of a channel.

use crate::protocol as p;
use crate::state::PeqBand;

/// One enabled filter line of a file.
#[derive(Clone, Debug, PartialEq)]
pub struct FileFilter {
    /// Type code as written (`PK`, `LS`, `HP`, …), upper case.
    pub kind: String,
    pub fc_hz: f64,
    pub gain_db: f64,
    /// Q if given (directly, or from a bandwidth in octaves).
    pub q: Option<f64>,
}

/// Number with a comma or a dot as decimal separator; "1,000" (a comma
/// followed by exactly three digits) is read as a thousands separator.
fn number(token: &str) -> Option<f64> {
    let t = token.trim();
    let t = if let Some((a, b)) = t.split_once(',') {
        if b.len() == 3 && b.chars().all(|c| c.is_ascii_digit()) && !a.contains('.') {
            format!("{a}{b}")
        } else {
            format!("{a}.{b}")
        }
    } else {
        t.to_string()
    };
    t.parse().ok().filter(|v: &f64| v.is_finite())
}

/// The enabled filters of a file, in file order; lines that are not filters,
/// switched off or of type `None` are skipped.
pub fn parse(text: &str) -> Vec<FileFilter> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.to_ascii_lowercase().starts_with("filter") {
            continue;
        }
        let Some((_, rest)) = line.split_once(':') else { continue };
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.first().map(|t| t.to_ascii_uppercase()) != Some("ON".into()) {
            continue;
        }
        let Some(kind) = tokens.get(1).map(|t| t.to_ascii_uppercase()) else { continue };
        if kind == "NONE" {
            continue;
        }
        let value_after = |key: &str| -> Option<(f64, Option<&str>)> {
            let i = tokens.iter().position(|t| t.eq_ignore_ascii_case(key))?;
            // "BW Oct 0.33": skip the unit word.
            let mut j = i + 1;
            while j < tokens.len() && number(tokens[j]).is_none() {
                j += 1;
            }
            Some((number(tokens.get(j)?)?, tokens.get(j + 1).copied()))
        };
        let Some((fc, unit)) = value_after("Fc") else { continue };
        let fc_hz = if unit.is_some_and(|u| u.eq_ignore_ascii_case("khz")) { fc * 1000.0 } else { fc };
        let gain_db = value_after("Gain").map_or(0.0, |(g, _)| g);
        let q = value_after("Q").map(|(q, _)| q).or_else(|| {
            value_after("BW").map(|(octaves, _)| {
                let n = 2f64.powf(octaves.max(0.01));
                n.sqrt() / (n - 1.0)
            })
        });
        out.push(FileFilter { kind, fc_hz, gain_db, q });
    }
    out
}

/// PEQ type for a file type code, or `None` if the DSP has no such filter.
/// Order of the DSP: Peak, Low Shelf, High Shelf, LP −6, LP −12, HP −6,
/// HP −12, Allpass 1, Allpass 2.
fn peq_type(kind: &str) -> Option<i64> {
    Some(match kind {
        "PK" | "PEQ" | "PA" | "MODAL" => 0,
        "LS" | "LSC" | "LSQ" | "LSF" => 1,
        "HS" | "HSC" | "HSQ" | "HSF" => 2,
        "LP1" | "LPF1" | "LPQ1" => 3,
        "LP" | "LPQ" | "LPF" => 4,
        "HP1" | "HPF1" | "HPQ1" => 5,
        "HP" | "HPQ" | "HPF" => 6,
        "AP" | "APQ" => 8,
        _ => return None,
    })
}

/// Result of [to_bands].
#[derive(Clone, Debug, PartialEq)]
pub struct FileBands {
    /// The new bands of the channel: one per filter, the rest flat (Peak at
    /// 0 dB, frequency and Q kept).
    pub bands: Vec<PeqBand>,
    /// Filters that went into bands.
    pub used: usize,
    /// What could not be taken over exactly.
    pub notes: Vec<String>,
}

/// Map [filters] onto a channel with the bands [current] (8 or 9).
pub fn to_bands(filters: &[FileFilter], current: &[PeqBand]) -> FileBands {
    let mut notes = Vec::new();
    let mut taken = Vec::new();
    for f in filters {
        let Some(t) = peq_type(&f.kind) else {
            notes.push(format!("{} at {:.0} Hz: filter type not available, skipped", f.kind, f.fc_hz));
            continue;
        };
        let gain = if t <= 2 { f.gain_db } else { 0.0 };
        let clamped = gain.clamp(-12.0, 12.0);
        if (clamped - gain).abs() > 0.05 {
            notes.push(format!("{} at {:.0} Hz: gain {:+.1} dB limited to {:+.0} dB", f.kind, f.fc_hz, gain, clamped));
        }
        if !(19.0..=20500.0).contains(&f.fc_hz) {
            notes.push(format!("{}: {:.0} Hz is outside 20 Hz – 20 kHz", f.kind, f.fc_hz));
        }
        // Without a Q: Butterworth for pass filters and shelves, 1.41 for peaks.
        let q = f.q.unwrap_or(if t == 0 { 1.41 } else { 0.71 });
        let max_q = p::max_q_raw_for_type(t);
        let q_raw = p::peq_q_to_raw(q);
        if q_raw > max_q {
            notes.push(format!("{} at {:.0} Hz: Q {q:.2} limited to {}", f.kind, f.fc_hz, p::format_q(max_q)));
        }
        taken.push(PeqBand {
            freq_raw: p::peq_hz_to_freq(f.fc_hz.clamp(20.0, 20000.0)),
            q_raw: q_raw.min(max_q),
            gain_db: (clamped * 10.0).round() / 10.0,
            peq_type: t,
            bypass: false,
        });
    }
    if taken.len() > current.len() {
        notes.push(format!("{} filters, the channel has {} bands: the last {} are left out", taken.len(), current.len(), taken.len() - current.len()));
        taken.truncate(current.len());
    }
    let used = taken.len();
    let bands = current
        .iter()
        .enumerate()
        .map(|(i, c)| taken.get(i).cloned().unwrap_or(PeqBand { gain_db: 0.0, peq_type: 0, bypass: false, ..c.clone() }))
        .collect();
    FileBands { bands, used, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "Filter Settings file\n\nRoom EQ V5.31\nNotes:\n\nEqualiser: Generic\n\
        Filter  1: ON  PK       Fc   63.0 Hz  Gain  -5.0 dB  Q  4.00\n\
        Filter  2: ON  LS       Fc   100 Hz  Gain   3.0 dB\n\
        Filter  3: OFF PK       Fc   200 Hz  Gain  -2.0 dB  Q  1.00\n\
        Filter  4: ON  None\n\
        Filter  5: ON  PK       Fc   1,250 Hz  Gain  -2,5 dB  BW Oct 0.333\n\
        Filter  6: ON  HP       Fc   2.5 kHz\n\
        Filter  7: ON  NO       Fc   3000 Hz\n\
        Filter: ON HS Fc 8000 Hz Gain 14 dB Q 5\n";

    #[test]
    fn parses_enabled_filters() {
        let f = parse(FILE);
        assert_eq!(f.len(), 6);
        assert_eq!(f[0], FileFilter { kind: "PK".into(), fc_hz: 63.0, gain_db: -5.0, q: Some(4.0) });
        assert_eq!(f[1].q, None);
        assert_eq!(f[2].fc_hz, 1250.0);
        assert_eq!(f[2].gain_db, -2.5);
        assert!((f[2].q.unwrap() - 4.32).abs() < 0.01, "1/3 octave");
        assert_eq!(f[3].fc_hz, 2500.0);
        assert_eq!(f[4].kind, "NO");
        assert_eq!(f[5].kind, "HS");
    }

    #[test]
    fn maps_onto_the_bands_with_notes() {
        let current = vec![PeqBand { peq_type: 6, bypass: true, ..PeqBand::default() }; 8];
        let r = to_bands(&parse(FILE), &current);
        assert_eq!(r.used, 5);
        assert_eq!(r.bands[0].peq_type, 0);
        assert_eq!(r.bands[0].gain_db, -5.0);
        assert_eq!(r.bands[1].peq_type, 1);
        assert_eq!(r.bands[1].q_raw, p::Q_RAW_TYPE_RESET);
        assert_eq!(r.bands[3].peq_type, 6, "HP = HP -12");
        assert_eq!(r.bands[3].gain_db, 0.0);
        assert_eq!(r.bands[4].gain_db, 12.0);
        assert_eq!(r.bands[4].q_raw, p::Q_RAW_LIMITED);
        // The rest is flat and active (a bypassed HP must not come back).
        assert_eq!(r.bands[7], PeqBand { peq_type: 0, gain_db: 0.0, bypass: false, ..current[7].clone() });
        assert_eq!(r.notes.len(), 3, "{:?}", r.notes);
    }

    #[test]
    fn more_filters_than_bands() {
        let text: String = (0..10).map(|i| format!("Filter {i}: ON PK Fc {} Hz Gain -1 dB Q 2\n", 100 * (i + 1))).collect();
        let r = to_bands(&parse(&text), &vec![PeqBand::default(); 9]);
        assert_eq!(r.used, 9);
        assert!(r.notes[0].contains("left out"));
    }
}
