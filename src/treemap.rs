//! Squarified treemap layout (Bruls, Huizing, van Wijk).

use eframe::egui::{Rect, pos2};

/// Lay out `sizes` (sorted biggest first, all > 0) inside `area`.
/// Returns one rect per input, in the same order.
pub fn squarify(sizes: &[f64], area: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(sizes.len());
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 || area.width() <= 0.0 || area.height() <= 0.0 {
        return vec![Rect::NOTHING; sizes.len()];
    }
    let scale = (area.width() as f64 * area.height() as f64) / total;
    let areas: Vec<f64> = sizes.iter().map(|s| s * scale).collect();

    let mut rect = area;
    let mut i = 0;
    while i < areas.len() {
        let short = rect.width().min(rect.height()) as f64;
        // Grow the row while it improves the worst aspect ratio.
        let mut j = i + 1;
        let mut best = worst(&areas[i..j], short);
        while j < areas.len() {
            let w = worst(&areas[i..j + 1], short);
            if w > best {
                break;
            }
            best = w;
            j += 1;
        }
        let row = &areas[i..j];
        let row_sum: f64 = row.iter().sum();
        let horizontal = rect.width() >= rect.height();
        if horizontal {
            // Column on the left side.
            let w = (row_sum / rect.height() as f64) as f32;
            let mut y = rect.top();
            for a in row {
                let h = (a / w as f64) as f32;
                out.push(Rect::from_min_max(pos2(rect.left(), y), pos2(rect.left() + w, y + h)));
                y += h;
            }
            rect = Rect::from_min_max(pos2(rect.left() + w, rect.top()), rect.max);
        } else {
            // Row along the top.
            let h = (row_sum / rect.width() as f64) as f32;
            let mut x = rect.left();
            for a in row {
                let w = (a / h as f64) as f32;
                out.push(Rect::from_min_max(pos2(x, rect.top()), pos2(x + w, rect.top() + h)));
                x += w;
            }
            rect = Rect::from_min_max(pos2(rect.left(), rect.top() + h), rect.max);
        }
        i = j;
    }
    out
}

fn worst(row: &[f64], short: f64) -> f64 {
    let sum: f64 = row.iter().sum();
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let s2 = sum * sum;
    let w2 = short * short;
    (w2 * max / s2).max(s2 / (w2 * min))
}
