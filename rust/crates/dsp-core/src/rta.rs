//! Real-time analyser math, shared by the RTA page, the GEQ overlay and the
//! Auto-EQ: 16384-point FFT (≈ 2.9 Hz per bin at 48 kHz) with 75 % overlap
//! (≈ 12 spectra per second), shown in 1/24-octave bands with selectable
//! averaging and peak hold. A measurement accumulates the power spectrum over
//! several seconds and reduces it to 1/3-octave levels for the Auto-EQ.

use std::f64::consts::PI;
use std::sync::OnceLock;

pub const FFT_SIZE: usize = 16384;
/// 75 % overlap.
pub const HOP: usize = FFT_SIZE / 4;
pub const PREFERRED_SAMPLE_RATE: f64 = 48000.0;
pub const BANDS_PER_OCTAVE: usize = 24;
pub const DISPLAY_MIN_HZ: f64 = 20.0;
pub const DISPLAY_MAX_HZ: f64 = 20000.0;
/// round(log2(20000 / 20) · 24)
pub const BAND_COUNT: usize = 239;
pub const MIN_DB: f64 = -140.0;
pub const MAX_DB: f64 = 10.0;
pub const DB_RANGE: f64 = MAX_DB - MIN_DB;
pub const PEAK_DECAY_DB_PER_SEC: f64 = 6.0;

/// Log-spaced band edges: band i spans `edges[i]` to `edges[i + 1]`.
pub fn band_edges() -> &'static [f64] {
    static EDGES: OnceLock<Vec<f64>> = OnceLock::new();
    EDGES.get_or_init(|| {
        let (lo, hi) = (DISPLAY_MIN_HZ.ln(), DISPLAY_MAX_HZ.ln());
        (0..=BAND_COUNT).map(|i| (lo + (hi - lo) * i as f64 / BAND_COUNT as f64).exp()).collect()
    })
}

/// Averaging of the displayed spectrum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Averaging {
    Off,
    Fast,
    Slow,
    Long,
}

impl Averaging {
    pub const ALL: [Averaging; 4] = [Averaging::Off, Averaging::Fast, Averaging::Slow, Averaging::Long];

    pub fn label(self) -> &'static str {
        match self {
            Averaging::Off => "Off",
            Averaging::Fast => "Fast",
            Averaging::Slow => "Slow",
            Averaging::Long => "Long",
        }
    }

    /// Time constant in seconds (0 = none).
    pub fn seconds(self) -> f64 {
        match self {
            Averaging::Off => 0.0,
            Averaging::Fast => 0.125,
            Averaging::Slow => 1.0,
            Averaging::Long => 4.0,
        }
    }
}

// ─── FFT ───

/// Real FFT of a power-of-two size: a complex FFT of half the size over the
/// even/odd samples, then split into the spectrum of the real signal.
pub struct RealFft {
    n: usize,
    /// e^(−2πik/(n/2)) for k < n/4.
    tw_re: Vec<f64>,
    tw_im: Vec<f64>,
    /// e^(−2πik/n) for k ≤ n/2.
    post_re: Vec<f64>,
    post_im: Vec<f64>,
    rev: Vec<usize>,
    re: Vec<f64>,
    im: Vec<f64>,
}

impl RealFft {
    pub fn new(n: usize) -> RealFft {
        assert!(n.is_power_of_two() && n >= 4, "FFT size must be a power of two");
        let m = n / 2;
        let bits = m.trailing_zeros();
        let rev = (0..m).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) }).collect();
        let tw = |k: usize, size: usize| {
            let a = -2.0 * PI * k as f64 / size as f64;
            (a.cos(), a.sin())
        };
        let (tw_re, tw_im) = (0..m / 2).map(|k| tw(k, m)).unzip();
        let (post_re, post_im) = (0..=m).map(|k| tw(k, n)).unzip();
        RealFft { n, tw_re, tw_im, post_re, post_im, rev, re: vec![0.0; m], im: vec![0.0; m] }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// |X[k]| for k = 0 ..= n/2 of the real signal [input] (length n).
    pub fn magnitudes(&mut self, input: &[f64], out: &mut [f64]) {
        let m = self.n / 2;
        debug_assert!(input.len() == self.n && out.len() == m + 1);
        for i in 0..m {
            let j = self.rev[i];
            self.re[j] = input[2 * i];
            self.im[j] = input[2 * i + 1];
        }
        let (re, im) = (&mut self.re, &mut self.im);
        let mut len = 2;
        while len <= m {
            let half = len / 2;
            let step = m / len;
            for start in (0..m).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = (self.tw_re[k * step], self.tw_im[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
        for (k, o) in out.iter_mut().enumerate() {
            let (zr, zi) = (re[k % m], im[k % m]);
            // conj(Z[m − k])
            let (cr, ci) = (re[(m - k) % m], -im[(m - k) % m]);
            let (er, ei) = ((zr + cr) / 2.0, (zi + ci) / 2.0);
            // (Z − conj) / 2i
            let (or, oi) = ((zi - ci) / 2.0, -(zr - cr) / 2.0);
            let (wr, wi) = (self.post_re[k], self.post_im[k]);
            let xr = er + wr * or - wi * oi;
            let xi = ei + wr * oi + wi * or;
            *o = xr.hypot(xi);
        }
    }
}

// ─── Analyser ───

fn power_db(power: f64) -> f64 {
    if power <= 0.0 { MIN_DB } else { (10.0 * power.log10()).clamp(MIN_DB, MAX_DB) }
}

/// Spectrum analyser fed with mono samples.
pub struct Analyzer {
    pub sample_rate: f64,
    pub averaging: Averaging,
    /// Displayed level per band in dB.
    pub magnitudes: Vec<f64>,
    /// Peak hold per band in dB.
    pub peaks: Vec<f64>,
    fft: RealFft,
    window: Vec<f64>,
    samples: Vec<f64>,
    windowed: Vec<f64>,
    mags: Vec<f64>,
    power: Vec<f64>,
    band_power: Option<Vec<f64>>,
    measure_sum: Option<Vec<f64>>,
    measure_count: usize,
}

impl Analyzer {
    pub fn new(sample_rate: f64) -> Analyzer {
        let window = (0..FFT_SIZE).map(|i| 0.5 * (1.0 - (2.0 * PI * i as f64 / (FFT_SIZE - 1) as f64).cos())).collect();
        Analyzer {
            sample_rate,
            averaging: Averaging::Fast,
            magnitudes: vec![MIN_DB; BAND_COUNT],
            peaks: vec![MIN_DB; BAND_COUNT],
            fft: RealFft::new(FFT_SIZE),
            window,
            samples: Vec::with_capacity(FFT_SIZE + HOP),
            windowed: vec![0.0; FFT_SIZE],
            mags: vec![0.0; FFT_SIZE / 2 + 1],
            power: vec![0.0; FFT_SIZE / 2 + 1],
            band_power: None,
            measure_sum: None,
            measure_count: 0,
        }
    }

    /// Add samples; returns the number of new spectra.
    pub fn push(&mut self, input: &[f32]) -> usize {
        self.samples.extend(input.iter().map(|s| *s as f64));
        let mut spectra = 0;
        while self.samples.len() >= FFT_SIZE {
            self.process();
            self.samples.drain(..HOP);
            spectra += 1;
        }
        spectra
    }

    fn process(&mut self) {
        for ((w, s), win) in self.windowed.iter_mut().zip(&self.samples).zip(&self.window) {
            *w = s * win;
        }
        self.fft.magnitudes(&self.windowed, &mut self.mags);
        // Power per bin, normalised so that a full-scale sine reads ~0 dB
        // (Hann window coherent gain 0.5).
        let norm = 2.0 / (FFT_SIZE as f64 * 0.5);
        for (p, m) in self.power.iter_mut().zip(&self.mags) {
            let v = m * norm;
            *p = v * v;
        }
        if let Some(sum) = &mut self.measure_sum {
            for (s, p) in sum.iter_mut().zip(&self.power) {
                *s += p;
            }
            self.measure_count += 1;
        }

        // Bins into log-spaced bands (power sum; bands narrower than a bin
        // take the power density of the nearest bin).
        let bin_hz = self.sample_rate / FFT_SIZE as f64;
        let edges = band_edges();
        let last = self.power.len() - 1;
        let mut bands = vec![0.0; BAND_COUNT];
        for (b, band) in bands.iter_mut().enumerate() {
            let lo = edges[b] / bin_hz;
            let hi = edges[b + 1] / bin_hz;
            let lo_bin = (lo.ceil() as usize).clamp(1, last);
            let hi_bin = (hi.floor() as usize).clamp(1, last);
            *band = if hi_bin < lo_bin {
                self.power[(((lo + hi) / 2.0).round() as usize).clamp(1, last)] * (hi - lo)
            } else {
                self.power[lo_bin..=hi_bin].iter().sum()
            };
        }

        // Exponential averaging of the band power.
        let tau = self.averaging.seconds();
        match &mut self.band_power {
            Some(previous) if tau > 0.0 => {
                let alpha = 1.0 - (-(HOP as f64) / self.sample_rate / tau).exp();
                for (p, b) in previous.iter_mut().zip(&bands) {
                    *p += alpha * (b - *p);
                }
            }
            _ => self.band_power = Some(bands),
        }

        let decay = PEAK_DECAY_DB_PER_SEC * HOP as f64 / self.sample_rate;
        let band_power = self.band_power.as_ref().expect("set above");
        for ((m, peak), p) in self.magnitudes.iter_mut().zip(&mut self.peaks).zip(band_power) {
            *m = power_db(*p);
            *peak = m.max(*peak - decay).clamp(MIN_DB, MAX_DB);
        }
    }

    /// Clear the peak hold and the averaging.
    pub fn reset_peaks(&mut self) {
        self.peaks.fill(MIN_DB);
        self.band_power = None;
    }

    /// Back to silence (capture stopped).
    pub fn clear(&mut self) {
        self.magnitudes.fill(MIN_DB);
        self.peaks.fill(MIN_DB);
        self.band_power = None;
        self.samples.clear();
        self.measure_sum = None;
    }

    // ─── Measurement (Auto-EQ) ───

    pub fn start_measurement(&mut self) {
        self.measure_sum = Some(vec![0.0; FFT_SIZE / 2 + 1]);
        self.measure_count = 0;
    }

    pub fn is_measuring(&self) -> bool {
        self.measure_sum.is_some()
    }

    /// Seconds of audio in the running measurement.
    pub fn measured_seconds(&self) -> f64 {
        self.measure_count as f64 * HOP as f64 / self.sample_rate
    }

    /// Stop the measurement; the level in dB of the 1/3-octave band around
    /// each of [centers], or None if nothing was measured.
    pub fn stop_measurement(&mut self, centers: &[f64]) -> Option<Vec<f64>> {
        let sum = self.measure_sum.take()?;
        let count = self.measure_count;
        if count == 0 {
            return None;
        }
        let bin_hz = self.sample_rate / FFT_SIZE as f64;
        let factor = 2f64.powf(1.0 / 6.0);
        let last = sum.len() - 1;
        Some(
            centers
                .iter()
                .map(|fc| {
                    let lo = ((fc / factor / bin_hz).ceil() as usize).clamp(1, last);
                    let hi = ((fc * factor / bin_hz).floor() as usize).clamp(lo, last);
                    let p: f64 = sum[lo..=hi].iter().sum();
                    10.0 * (p / count as f64).max(1e-20).log10()
                })
                .collect(),
        )
    }

    pub fn cancel_measurement(&mut self) {
        self.measure_sum = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_count_matches_the_definition() {
        let n = ((DISPLAY_MAX_HZ / DISPLAY_MIN_HZ).log2() * BANDS_PER_OCTAVE as f64).round() as usize;
        assert_eq!(n, BAND_COUNT);
        assert_eq!(band_edges().len(), BAND_COUNT + 1);
        assert!((band_edges()[BAND_COUNT] - DISPLAY_MAX_HZ).abs() < 1e-6);
    }

    #[test]
    fn real_fft_matches_a_plain_dft() {
        let n = 64;
        let input: Vec<f64> = (0..n).map(|i| ((i * 7919) % 101) as f64 / 50.0 - 1.0).collect();
        let mut fft = RealFft::new(n);
        let mut out = vec![0.0; n / 2 + 1];
        fft.magnitudes(&input, &mut out);
        for (k, got) in out.iter().enumerate() {
            let (mut re, mut im) = (0.0, 0.0);
            for (t, x) in input.iter().enumerate() {
                let a = -2.0 * PI * (k * t) as f64 / n as f64;
                re += x * a.cos();
                im += x * a.sin();
            }
            assert!((re.hypot(im) - got).abs() < 1e-9, "bin {k}: {got} vs {}", re.hypot(im));
        }
    }

    #[test]
    fn full_scale_sine_reads_about_zero_db() {
        let mut a = Analyzer::new(48000.0);
        a.averaging = Averaging::Off;
        let samples: Vec<f32> = (0..FFT_SIZE).map(|i| (2.0 * PI * 1000.0 * i as f64 / 48000.0).sin() as f32).collect();
        assert_eq!(a.push(&samples), 1);
        let edges = band_edges();
        let band = (0..BAND_COUNT).find(|b| edges[*b] <= 1000.0 && 1000.0 < edges[b + 1]).unwrap();
        assert!(a.magnitudes[band] > -1.0 && a.magnitudes[band] < 3.0, "{}", a.magnitudes[band]);
        assert!(a.magnitudes[0] < -60.0);
        assert_eq!(a.peaks[band], a.magnitudes[band]);
    }

    #[test]
    fn measurement_reports_third_octave_levels() {
        let mut a = Analyzer::new(48000.0);
        a.start_measurement();
        let samples: Vec<f32> = (0..FFT_SIZE + 3 * HOP).map(|i| (2.0 * PI * 1000.0 * i as f64 / 48000.0).sin() as f32).collect();
        assert_eq!(a.push(&samples), 4);
        assert!((a.measured_seconds() - 4.0 * HOP as f64 / 48000.0).abs() < 1e-12);
        let levels = a.stop_measurement(&[100.0, 1000.0]).unwrap();
        assert!(levels[1] > -1.0 && levels[1] < 3.0, "{levels:?}");
        assert!(levels[0] < -60.0);
        assert!(!a.is_measuring());
        assert!(a.stop_measurement(&[1000.0]).is_none());
    }
}
