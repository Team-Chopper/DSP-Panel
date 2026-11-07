//! Canvas drawing of an RTA spectrum (1/24-octave bars with peak hold). Only
//! the bars are redrawn per spectrum; grids and labels live in SVG layers that
//! change with the size only.

use dsp_core::rta::{BAND_COUNT, DB_RANGE, MIN_DB, band_edges};
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

/// Size [canvas] to (w, h) CSS pixels at the device pixel ratio and return a
/// cleared context in CSS pixel coordinates.
pub fn prepare(canvas: &HtmlCanvasElement, w: f64, h: f64) -> Option<CanvasRenderingContext2d> {
    let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio()).max(1.0);
    let (pw, ph) = ((w * dpr).round().max(1.0) as u32, (h * dpr).round().max(1.0) as u32);
    if canvas.width() != pw {
        canvas.set_width(pw);
    }
    if canvas.height() != ph {
        canvas.set_height(ph);
    }
    let ctx: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    ctx.clear_rect(0.0, 0.0, w, h);
    Some(ctx)
}

/// Look of the bars.
pub struct BarStyle {
    pub bar: &'static str,
    pub peak: &'static str,
    pub peak_width: f64,
    /// Half-pixel gap between bars wider than 3 px.
    pub gaps: bool,
    /// Skip peaks at the bottom of the range.
    pub hide_silent_peaks: bool,
}

/// Bars of [levels] / [peaks] (dB per RTA band) between [left] and [right]
/// (x from Hz via [x_of]) in the vertical range [top] .. [top] + [height]
/// (−140 … +10 dB).
#[allow(clippy::too_many_arguments)]
pub fn draw_bars(
    ctx: &CanvasRenderingContext2d,
    levels: &[f64],
    peaks: &[f64],
    x_of: impl Fn(f64) -> f64,
    left: f64,
    right: f64,
    top: f64,
    height: f64,
    style: &BarStyle,
) {
    let edges = band_edges();
    let bottom = top + height;
    let y_of = |db: f64| top + height * (1.0 - ((db - MIN_DB) / DB_RANGE).clamp(0.0, 1.0));
    ctx.set_fill_style_str(style.bar);
    let mut segments = Vec::with_capacity(BAND_COUNT);
    for b in 0..BAND_COUNT.min(levels.len()) {
        let (x0, x1) = (x_of(edges[b]), x_of(edges[b + 1]));
        if x1 <= left || x0 >= right {
            continue;
        }
        let gap = if style.gaps && x1 - x0 > 3.0 { 0.5 } else { 0.0 };
        let (x0, x1) = (x0.clamp(left, right) + gap, x1.clamp(left, right) - gap);
        let y = y_of(levels[b]);
        ctx.fill_rect(x0, y, x1 - x0, bottom - y);
        if let Some(peak) = peaks.get(b)
            && (!style.hide_silent_peaks || *peak > MIN_DB) {
                segments.push((x0, x1, y_of(*peak)));
            }
    }
    ctx.set_stroke_style_str(style.peak);
    ctx.set_line_width(style.peak_width);
    ctx.begin_path();
    for (x0, x1, y) in segments {
        ctx.move_to(x0, y);
        ctx.line_to(x1, y);
    }
    ctx.stroke();
}
