//! The DiskLens mark: a magnifying glass over a disk-usage pie, drawn procedurally
//! so the app icon, sidebar logo and menu-bar glyph always match.

use std::f32::consts::TAU;

fn rounded_box(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (dx, dy) = ((x - cx).abs() - (hw - r), (y - cy).abs() - (hh - r));
    (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - r
}

fn segment(x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (px, py, vx, vy) = (x - ax, y - ay, bx - ax, by - ay);
    let t = ((px * vx + py * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    ((px - vx * t).powi(2) + (py - vy * t).powi(2)).sqrt()
}

/// Coverage (0..1) of each layer at a point in unit coordinates.
struct Layers {
    lens: f32,      // ring + handle
    pie_used: f32,  // the "used" wedge inside the lens
    pie_rest: f32,  // the rest of the pie
}

const LENS: (f32, f32, f32) = (0.44, 0.44, 0.215); // centre x, centre y, radius
const PIE_R: f32 = 0.135;
const USED: f32 = 0.72; // fraction of the pie drawn as "used"

fn layers(x: f32, y: f32, px: f32) -> Layers {
    let aa = |d: f32| (0.5 - d / px).clamp(0.0, 1.0);
    let (cx, cy, r) = LENS;
    let dist = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    let ring = (dist - r).abs() - 0.034;
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let handle = segment(x, y, cx + (r + 0.03) * k, cy + (r + 0.03) * k, 0.76, 0.76) - 0.05;
    let lens = aa(ring.min(handle));

    // Pie: angle measured clockwise from 12 o'clock.
    let pie = aa(dist - PIE_R);
    let ang = ((x - cx).atan2(-(y - cy)) + TAU) % TAU;
    let gap = 0.035 / PIE_R.max(dist); // thin gap between wedges
    let in_used = ang > gap && ang < USED * TAU - gap;
    let in_rest = ang > USED * TAU + gap && ang < TAU - gap;
    Layers { lens, pie_used: if in_used { pie } else { 0.0 }, pie_rest: if in_rest { pie } else { 0.0 } }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Full-color app icon (RGBA, `n`×`n`), on a macOS-style rounded square with margin.
pub fn icon_rgba(n: u32) -> Vec<u8> {
    let px = 1.0 / n as f32;
    let mut out = vec![0u8; (n * n * 4) as usize];
    let (top, bottom) = ([0.04, 0.52, 1.0], [0.37, 0.36, 0.90]); // #0A84FF -> #5E5CE6
    for j in 0..n {
        for i in 0..n {
            let (x, y) = ((i as f32 + 0.5) * px, (j as f32 + 0.5) * px);
            let bg = (0.5 - rounded_box(x, y, 0.5, 0.5, 0.41, 0.41, 0.095) / px).clamp(0.0, 1.0);
            let base = mix(top, bottom, ((x + y) / 2.0).clamp(0.0, 1.0));
            let l = layers(x, y, px);
            let mut c = base;
            c = mix(c, [1.0, 1.0, 1.0], l.pie_rest * 0.38);
            c = mix(c, [1.0, 1.0, 1.0], l.pie_used);
            c = mix(c, [1.0, 1.0, 1.0], l.lens);
            let k = ((j * n + i) * 4) as usize;
            out[k] = (c[0] * 255.0) as u8;
            out[k + 1] = (c[1] * 255.0) as u8;
            out[k + 2] = (c[2] * 255.0) as u8;
            out[k + 3] = (bg * 255.0) as u8;
        }
    }
    out
}

/// Monochrome template glyph for the menu bar (alpha only; macOS tints it).
pub fn glyph_rgba(n: u32) -> Vec<u8> {
    let px = 1.0 / n as f32;
    let mut out = vec![0u8; (n * n * 4) as usize];
    for j in 0..n {
        for i in 0..n {
            // Scale the mark up a little: the menu bar has no background square.
            let (x, y) = (0.06 + (i as f32 + 0.5) * px * 0.86, 0.06 + (j as f32 + 0.5) * px * 0.86);
            let l = layers(x, y, px * 0.86);
            let a = l.lens.max(l.pie_used).max(l.pie_rest * 0.45);
            out[((j * n + i) * 4 + 3) as usize] = (a * 255.0) as u8;
        }
    }
    out
}
