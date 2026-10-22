//! Port of test/eq_math_test.dart (the EQ math moved from the Flutter client
//! into dsp-core).

use dsp_core::PeqBand;
use dsp_core::eq_math::{
    FitLimits, GEQ_CENTER_FREQUENCIES, filter_response_db, fit_peq_correction, free_peq_bands, peq_response_db,
};
use dsp_core::protocol as p;

fn slope(name: &str) -> i64 {
    p::CROSSOVER_SLOPE_NAMES.iter().position(|n| *n == name).unwrap() as i64
}

fn close(actual: f64, expected: f64, tolerance: f64) -> bool {
    (actual - expected).abs() <= tolerance
}

#[test]
fn peq_types() {
    assert!(close(peq_response_db(1000.0, 1000.0, 2.0, 6.0, 0), 6.0, 0.01), "peak gain at the centre");
    assert!(close(peq_response_db(20.0, 1000.0, 2.0, 6.0, 0), 0.0, 0.05));
    assert!(close(peq_response_db(20.0, 1000.0, 0.7, -9.0, 1), -9.0, 0.1), "low shelf below");
    assert!(close(peq_response_db(20000.0, 1000.0, 0.7, -9.0, 1), 0.0, 0.1));
    assert!(close(peq_response_db(20000.0, 1000.0, 0.7, 4.0, 2), 4.0, 0.1), "high shelf above");
    assert!(close(peq_response_db(1000.0, 1000.0, 1.0, 0.0, 3), -3.01, 0.01), "LP -6 at cut-off");
    // Bilinear at 48 kHz: the frequency axis is pre-warped.
    assert!(close(peq_response_db(4000.0, 1000.0, 0.707, 0.0, 4), -24.48, 0.05), "LP -12 two octaves up");
    assert!(close(peq_response_db(250.0, 1000.0, 0.707, 0.0, 6), -24.12, 0.05), "HP -12 two octaves down");
    assert_eq!(peq_response_db(300.0, 1000.0, 1.0, 0.0, 7), 0.0, "all pass");
}

#[test]
fn peaks_and_shelves_like_the_original_editor() {
    // A cut is the exact inverse of the boost.
    for f in [100.0, 700.0, 1000.0, 1400.0, 8000.0] {
        for t in [0, 1, 2] {
            assert!(close(peq_response_db(f, 1000.0, 2.0, -6.0, t), -peq_response_db(f, 1000.0, 2.0, 6.0, t), 1e-9));
        }
    }
    // Half an octave from the centre a +12 dB, Q 1 band has about 10.4 dB.
    assert!(close(peq_response_db(1414.0, 1000.0, 1.0, 12.0, 0), 10.37, 0.05));
    assert!(close(peq_response_db(1414.0, 1000.0, 1.0, -12.0, 0), -10.37, 0.05));
    // Shelves reach the full gain far from the corner.
    assert!(close(peq_response_db(20.0, 2000.0, 0.71, 6.0, 1), 6.0, 0.05));
    assert!(close(peq_response_db(20000.0, 200.0, 0.71, -6.0, 2), -6.0, 0.05));
}

#[test]
fn peq_fit_puts_corrections_into_free_bands_only() {
    // Wanted: -6 dB around 1 kHz (one 1/3-octave band), +4 dB at 100 Hz.
    let target: Vec<f64> = GEQ_CENTER_FREQUENCIES
        .iter()
        .map(|f| if *f == 1000.0 { -6.0 } else if *f == 100.0 { 4.0 } else { 0.0 })
        .collect();
    let bands = vec![
        PeqBand { peq_type: 1, gain_db: 3.0, ..PeqBand::default() }, // low shelf in use: must stay
        PeqBand::default(),
        PeqBand::default(),
        PeqBand::default(),
    ];
    let fitted = fit_peq_correction(&target, &GEQ_CENTER_FREQUENCIES, &bands, FitLimits { max_boost: 6.0, ..FitLimits::default() });
    assert_eq!(fitted[0], bands[0]);
    assert!(close(p::peq_freq_to_hz(fitted[1].freq_raw), 1000.0, 30.0));
    assert!(close(fitted[1].gain_db, -6.0, 0.5));
    assert!(close(p::peq_freq_to_hz(fitted[2].freq_raw), 100.0, 5.0));
    assert!(fitted[2].gain_db > 2.0);
    assert_eq!(free_peq_bands(&bands), [1, 2, 3]);

    // No boosts allowed: the +4 dB is left alone.
    let cuts_only =
        fit_peq_correction(&target, &GEQ_CENTER_FREQUENCIES, &bands, FitLimits { max_boost: 0.0, ..FitLimits::default() });
    assert!(cuts_only.iter().skip(1).all(|b| b.gain_db <= 0.0));
}

#[test]
fn crossover_slopes() {
    assert!(close(filter_response_db(1000.0, 1000.0, slope("BW -24"), false), -3.01, 0.01));
    assert!(close(filter_response_db(1000.0, 1000.0, slope("LK -24"), true), -6.02, 0.01));
    assert!(close(filter_response_db(1000.0, 1000.0, slope("BL -24"), false), -3.01, 0.05));
    assert!(close(filter_response_db(4000.0, 1000.0, slope("BW -24"), false), -48.2, 0.3), "24 dB/oct");
    assert!(close(filter_response_db(100.0, 1000.0, slope("BW -12"), false), 0.0, 0.01));
    assert_eq!(filter_response_db(100.0, 1000.0, 0, true), 0.0, "bypass");
}
