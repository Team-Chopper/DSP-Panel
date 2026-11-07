//! Mapping between a value and a fader position (0 = bottom/left, 1 = top/right).

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Taper {
    Linear(f64, f64),
    /// Logarithmic for values > 0 (times, frequencies).
    Log(f64, f64),
    /// Piecewise linear through (value, position) points, ascending.
    Piecewise(&'static [(f64, f64)]),
}

impl Taper {
    pub fn to_position(self, value: f64) -> f64 {
        match self {
            Taper::Linear(min, max) => ((value - min) / (max - min)).clamp(0.0, 1.0),
            Taper::Log(min, max) => ((value.clamp(min, max) / min).ln() / (max / min).ln()).clamp(0.0, 1.0),
            Taper::Piecewise(points) => {
                if value <= points[0].0 {
                    return points[0].1;
                }
                for w in points.windows(2) {
                    let ((v0, p0), (v1, p1)) = (w[0], w[1]);
                    if value <= v1 {
                        return p0 + (value - v0) / (v1 - v0) * (p1 - p0);
                    }
                }
                points[points.len() - 1].1
            }
        }
    }

    /// The value at a fader position.
    pub fn value_at(self, position: f64) -> f64 {
        let position = position.clamp(0.0, 1.0);
        match self {
            Taper::Linear(min, max) => min + position * (max - min),
            Taper::Log(min, max) => min * (max / min).powf(position),
            Taper::Piecewise(points) => {
                if position <= points[0].1 {
                    return points[0].0;
                }
                for w in points.windows(2) {
                    let ((v0, p0), (v1, p1)) = (w[0], w[1]);
                    if position <= p1 {
                        return v0 + (position - p0) / (p1 - p0) * (v1 - v0);
                    }
                }
                points[points.len() - 1].0
            }
        }
    }
}

/// Console taper for channel gain (−60 … +12 dB): most of the travel is spent
/// around 0 dB.
pub const GAIN_TAPER: Taper = Taper::Piecewise(&[
    (-60.0, 0.0),
    (-40.0, 0.14),
    (-30.0, 0.25),
    (-20.0, 0.39),
    (-12.0, 0.53),
    (-6.0, 0.65),
    (0.0, 0.78),
    (6.0, 0.89),
    (12.0, 1.0),
]);

pub const GAIN_MARKS: [f64; 9] = [12.0, 6.0, 0.0, -6.0, -12.0, -20.0, -30.0, -40.0, -60.0];
