//! The chart views of the right panel. Every chart shows the children of one
//! folder, reports what was drawn where as `Hit`s, and shares the same
//! hover / click / double-click / right-click handling.

use crate::scan::{Kind, Tree};
use crate::{fmt_count, fmt_size, treemap};
use eframe::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, Vec2, pos2, vec2,
};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Chart {
    Treemap,
    Pie,
    Bars,
    Towers,
    Sunburst,
}

impl Chart {
    pub const ALL: [Chart; 5] = [Chart::Bars, Chart::Pie, Chart::Towers, Chart::Sunburst, Chart::Treemap];

    pub fn label(self) -> &'static str {
        match self {
            Chart::Treemap => "▦ Treemap",
            Chart::Pie => "◔ Pie",
            Chart::Bars => "☰ Bars",
            Chart::Towers => "▮ Towers",
            Chart::Sunburst => "◎ Sunburst",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Chart::Treemap => "Every box is a folder or file, sized by space used",
            Chart::Pie => "Share of this folder taken by each item",
            Chart::Bars => "Biggest items, ranked",
            Chart::Towers => "Biggest items as columns",
            Chart::Sunburst => "Rings show what's inside each folder, three levels deep",
        }
    }
}

/// Categorical palette (dark-surface steps). Hues follow the item's rank among the
/// folder's children, so an item keeps its color across every chart.
const SERIES: [Color32; 8] = [
    Color32::from_rgb(0x39, 0x87, 0xe5),
    Color32::from_rgb(0xd9, 0x59, 0x26),
    Color32::from_rgb(0x19, 0x9e, 0x70),
    Color32::from_rgb(0xc9, 0x85, 0x00),
    Color32::from_rgb(0xd5, 0x51, 0x81),
    Color32::from_rgb(0x00, 0x83, 0x00),
    Color32::from_rgb(0x90, 0x85, 0xe9),
    Color32::from_rgb(0xe6, 0x67, 0x67),
];
const OTHER: Color32 = Color32::from_rgb(0x6b, 0x6b, 0x68);
const BAR: Color32 = Color32::from_rgb(0x39, 0x87, 0xe5);
const TEXT: Color32 = Color32::from_rgb(0xee, 0xee, 0xea);
const TEXT_DIM: Color32 = Color32::from_rgb(0xa8, 0xa7, 0x9e);
const TOP_N: usize = 8;

fn series(i: usize) -> Color32 {
    SERIES.get(i).copied().unwrap_or(OTHER)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

#[derive(Clone, Copy)]
enum Area {
    Rect(Rect),
    Sector { c: Pos2, r0: f32, r1: f32, a0: f32, a1: f32 },
}

impl Area {
    fn contains(&self, p: Pos2) -> bool {
        match *self {
            Area::Rect(r) => r.contains(p),
            Area::Sector { c, r0, r1, a0, a1 } => {
                let d = p - c;
                let r = d.length();
                let a = angle_of(d);
                r >= r0 && r <= r1 && a >= a0 && a <= a1
            }
        }
    }
}

/// Angle measured clockwise from 12 o'clock, in [0, TAU).
fn angle_of(d: Vec2) -> f32 {
    (d.y.atan2(d.x) + FRAC_PI_2).rem_euclid(TAU)
}

fn polar(c: Pos2, r: f32, a: f32) -> Pos2 {
    let t = a - FRAC_PI_2;
    c + vec2(t.cos(), t.sin()) * r
}

struct Hit {
    /// `None` for the folded "Other" slice.
    id: Option<u32>,
    area: Area,
}

#[derive(Default)]
pub struct Events {
    pub select: Option<u32>,
    pub zoom: Option<u32>,
    pub delete: Option<u32>,
}

/// One entry of a chart: a real child, or "Other" holding everything past the top N.
struct Item {
    id: Option<u32>,
    size: u64,
    color: Color32,
    label: String,
}

fn items(tree: &Tree, root: u32, top: usize) -> Vec<Item> {
    let kids: Vec<u32> = tree.nodes[root as usize]
        .children
        .iter()
        .copied()
        .filter(|&c| tree.nodes[c as usize].size > 0)
        .collect();
    let mut out: Vec<Item> = kids
        .iter()
        .take(top)
        .enumerate()
        .map(|(i, &c)| Item {
            id: Some(c),
            size: tree.nodes[c as usize].size,
            color: series(i),
            label: tree.nodes[c as usize].name.to_string(),
        })
        .collect();
    if kids.len() > top {
        let rest = &kids[top..];
        out.push(Item {
            id: None,
            size: rest.iter().map(|&c| tree.nodes[c as usize].size).sum(),
            color: OTHER,
            label: format!("Other ({} items)", fmt_count(rest.len() as u64)),
        });
    }
    out
}

pub fn show(ui: &mut egui::Ui, chart: Chart, tree: &Tree, root: u32, selected: Option<u32>) -> Events {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let painter = ui.painter_at(rect);
    let mut hits = Vec::new();
    let total = tree.nodes[root as usize].size;

    if total == 0 || tree.nodes[root as usize].children.is_empty() {
        painter.text(rect.center(), Align2::CENTER_CENTER, "Nothing to chart here", FontId::proportional(14.0), TEXT_DIM);
        return Events::default();
    }
    match chart {
        Chart::Treemap => draw_treemap(&painter, tree, root, rect.shrink(1.0), 0, None, &mut hits),
        Chart::Pie => draw_pie(&painter, tree, root, rect, &mut hits),
        Chart::Bars => draw_bars(&painter, tree, root, rect, &mut hits),
        Chart::Towers => draw_towers(&painter, tree, root, rect, &mut hits),
        Chart::Sunburst => draw_sunburst(&painter, tree, root, rect, &mut hits),
    }

    let hovered = resp.hover_pos().and_then(|p| hits.iter().rev().find(|h| h.area.contains(p)));
    if let Some(sel) = selected {
        // An item can appear twice (pie slice + legend row): mark both.
        for h in hits.iter().filter(|h| h.id == Some(sel)) {
            outline(&painter, h.area, Stroke::new(2.5, Color32::WHITE));
        }
    }
    let mut ev = Events::default();
    let Some(h) = hovered else { return ev };
    for other in hits.iter().filter(|o| o.id == h.id) {
        outline(&painter, other.area, Stroke::new(1.5, Color32::from_white_alpha(200)));
    }

    let id = h.id;
    resp.clone().on_hover_ui_at_pointer(|ui| match id {
        Some(id) => {
            let n = &tree.nodes[id as usize];
            ui.strong(n.name.to_string());
            ui.label(format!("{}  ({:.1}% of this folder)", fmt_size(n.size), pct(n.size, total)));
            if n.kind == Kind::Dir {
                ui.label(format!("{} files, {} folders", fmt_count(n.files), fmt_count(n.dirs)));
            }
        }
        None => {
            ui.strong("Other");
            ui.label("Smaller items, grouped. Switch to Bars or the folder tree to see them one by one.");
        }
    });
    if let Some(id) = id {
        if resp.clicked() {
            ev.select = Some(id);
        }
        if resp.double_clicked() && tree.nodes[id as usize].kind == Kind::Dir {
            ev.zoom = Some(id);
        }
        resp.context_menu(|ui| crate::context_items(ui, tree, id, &mut ev.delete));
    }
    ev
}

fn pct(part: u64, total: u64) -> f64 {
    part as f64 * 100.0 / total.max(1) as f64
}

fn outline(painter: &Painter, area: Area, stroke: Stroke) {
    match area {
        Area::Rect(r) => {
            painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
        }
        Area::Sector { c, r0, r1, a0, a1 } => {
            let n = arc_steps(a0, a1, r1);
            let mut pts: Vec<Pos2> = (0..=n).map(|i| polar(c, r1, a0 + (a1 - a0) * i as f32 / n as f32)).collect();
            pts.extend((0..=n).rev().map(|i| polar(c, r0, a0 + (a1 - a0) * i as f32 / n as f32)));
            painter.add(Shape::closed_line(pts, stroke));
        }
    }
}

fn arc_steps(a0: f32, a1: f32, r: f32) -> usize {
    (((a1 - a0) * r / 3.0).ceil() as usize).clamp(1, 512)
}

/// Fill a ring segment. `gap` px is trimmed off both radial edges to leave a surface gap.
fn sector(painter: &Painter, c: Pos2, r0: f32, r1: f32, a0: f32, a1: f32, gap: f32, color: Color32) {
    let trim0 = if r0 > 0.0 { gap / r0 } else { 0.0 };
    let trim1 = gap / r1;
    if a1 - a0 <= trim0.max(trim1) * 2.0 {
        return;
    }
    let n = arc_steps(a0, a1, r1);
    let mut mesh = Mesh::default();
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let ao = a0 + trim1 + (a1 - a0 - 2.0 * trim1) * t;
        let ai = a0 + trim0 + (a1 - a0 - 2.0 * trim0) * t;
        mesh.colored_vertex(polar(c, r1, ao), color);
        mesh.colored_vertex(polar(c, r0, ai), color);
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(Shape::mesh(mesh));
}

fn text(painter: &Painter, pos: Pos2, anchor: Align2, s: impl ToString, size: f32, color: Color32) -> Rect {
    painter.text(pos, anchor, s.to_string(), FontId::proportional(size), color)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

// ---------- legend (pie, sunburst) ----------

/// Legend rows: chip, name, size, share. Rows are hit targets too.
fn legend(painter: &Painter, items: &[Item], total: u64, rect: Rect, hits: &mut Vec<Hit>) {
    let row_h = 20.0;
    for (i, it) in items.iter().enumerate() {
        let y = rect.top() + i as f32 * row_h;
        if y + row_h > rect.bottom() {
            break;
        }
        let row = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), row_h));
        painter.rect_filled(
            Rect::from_center_size(pos2(row.left() + 8.0, row.center().y), vec2(11.0, 11.0)),
            2.0,
            it.color,
        );
        let pct_r = text(painter, pos2(row.right() - 4.0, row.center().y), Align2::RIGHT_CENTER, format!("{:.1}%", pct(it.size, total)), 12.0, TEXT_DIM);
        let size_r = text(painter, pos2(pct_r.left() - 14.0, row.center().y), Align2::RIGHT_CENTER, fmt_size(it.size), 12.0, TEXT);
        let name_w = (size_r.left() - row.left() - 30.0).max(20.0);
        let chars = (name_w / 6.5) as usize;
        text(painter, pos2(row.left() + 20.0, row.center().y), Align2::LEFT_CENTER, truncate(&it.label, chars), 12.0, TEXT);
        hits.push(Hit { id: it.id, area: Area::Rect(row) });
    }
}

fn legend_height(n: usize) -> f32 {
    n as f32 * 20.0 + 8.0
}

// ---------- pie ----------

fn draw_pie(painter: &Painter, tree: &Tree, root: u32, rect: Rect, hits: &mut Vec<Hit>) {
    let items = items(tree, root, TOP_N);
    let total = tree.nodes[root as usize].size;
    let lh = legend_height(items.len());
    let chart_rect = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - lh));
    let r1 = (chart_rect.width().min(chart_rect.height()) / 2.0 - 8.0).max(20.0);
    let r0 = r1 * 0.55;
    let c = chart_rect.center();

    let mut a = 0.0f32;
    for it in &items {
        let sweep = it.size as f32 / total.max(1) as f32 * TAU;
        sector(painter, c, r0, r1, a, a + sweep, 1.0, it.color);
        hits.push(Hit { id: it.id, area: Area::Sector { c, r0, r1, a0: a, a1: a + sweep } });
        // Direct label on slices big enough to hold it.
        if sweep > 0.35 {
            let mid = polar(c, (r0 + r1) / 2.0, a + sweep / 2.0);
            text(painter, mid, Align2::CENTER_CENTER, format!("{:.0}%", pct(it.size, total)), 13.0, Color32::WHITE);
        }
        a += sweep;
    }
    let n = &tree.nodes[root as usize];
    text(painter, c - vec2(0.0, 10.0), Align2::CENTER_CENTER, fmt_size(n.size), 20.0, TEXT);
    text(painter, c + vec2(0.0, 14.0), Align2::CENTER_CENTER, truncate(&n.name, (r0 / 4.0) as usize), 12.0, TEXT_DIM);

    let lrect = Rect::from_min_max(pos2(rect.left() + 8.0, rect.bottom() - lh + 8.0), pos2(rect.right() - 8.0, rect.bottom()));
    legend(painter, &items, total, lrect, hits);
}

// ---------- sunburst ----------

fn draw_sunburst(painter: &Painter, tree: &Tree, root: u32, rect: Rect, hits: &mut Vec<Hit>) {
    let items = items(tree, root, TOP_N);
    let total = tree.nodes[root as usize].size;
    let lh = legend_height(items.len());
    let chart_rect = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - lh));
    let radius = (chart_rect.width().min(chart_rect.height()) / 2.0 - 8.0).max(30.0);
    let c = chart_rect.center();
    let hole = radius * 0.28;
    let ring = (radius - hole) / 3.0;

    let kids: Vec<u32> = tree.nodes[root as usize].children.clone();
    let mut a = 0.0f32;
    for (i, &k) in kids.iter().enumerate() {
        let sweep = tree.nodes[k as usize].size as f32 / total.max(1) as f32 * TAU;
        if sweep * radius < 1.0 {
            break; // children are sorted biggest first
        }
        let color = if i < TOP_N { series(i) } else { OTHER };
        ring_segment(painter, tree, k, c, hole, ring, 0, a, a + sweep, color, hits);
        a += sweep;
    }
    let n = &tree.nodes[root as usize];
    text(painter, c - vec2(0.0, 8.0), Align2::CENTER_CENTER, fmt_size(n.size), 16.0, TEXT);
    text(painter, c + vec2(0.0, 11.0), Align2::CENTER_CENTER, truncate(&n.name, (hole / 4.0) as usize), 11.0, TEXT_DIM);

    let lrect = Rect::from_min_max(pos2(rect.left() + 8.0, rect.bottom() - lh + 8.0), pos2(rect.right() - 8.0, rect.bottom()));
    legend(painter, &items, total, lrect, hits);
}

#[allow(clippy::too_many_arguments)]
fn ring_segment(
    painter: &Painter,
    tree: &Tree,
    id: u32,
    c: Pos2,
    hole: f32,
    ring: f32,
    depth: u32,
    a0: f32,
    a1: f32,
    color: Color32,
    hits: &mut Vec<Hit>,
) {
    let r0 = hole + ring * depth as f32;
    let r1 = r0 + ring;
    let n = &tree.nodes[id as usize];
    let fill = if n.kind == Kind::File { mix(color, Color32::from_gray(30), 0.35) } else { color };
    sector(painter, c, r0 + 1.0, r1 - 1.0, a0, a1, 1.0, fill);
    hits.push(Hit { id: Some(id), area: Area::Sector { c, r0, r1, a0, a1 } });
    if depth >= 2 || n.size == 0 {
        return;
    }
    let child_color = mix(color, Color32::from_gray(20), 0.28);
    let mut a = a0;
    for &k in &n.children {
        let sweep = tree.nodes[k as usize].size as f32 / n.size as f32 * (a1 - a0);
        if sweep * (r1 + ring) < 1.5 {
            break;
        }
        ring_segment(painter, tree, k, c, hole, ring, depth + 1, a, a + sweep, child_color, hits);
        a += sweep;
    }
}

// ---------- bars ----------

fn draw_bars(painter: &Painter, tree: &Tree, root: u32, rect: Rect, hits: &mut Vec<Hit>) {
    let row_h = 26.0;
    let fit = ((rect.height() - 8.0) / row_h).floor().max(1.0) as usize;
    let all = tree.nodes[root as usize].children.iter().filter(|&&c| tree.nodes[c as usize].size > 0).count();
    // Leave the last row for "Other" when everything doesn't fit.
    let top = if all > fit { fit.saturating_sub(1).max(1) } else { fit };
    let mut items = items(tree, root, top);
    for it in &mut items {
        if it.id.is_some() {
            it.color = BAR;
        }
    }
    let total = tree.nodes[root as usize].size;
    let max = items.iter().map(|i| i.size).max().unwrap_or(1).max(1);
    let label_w = (rect.width() * 0.36).clamp(90.0, 260.0);
    let value_w = 110.0;
    let bar_x = rect.left() + label_w + 8.0;
    let bar_max = (rect.right() - value_w - bar_x).max(20.0);

    for (i, it) in items.iter().enumerate() {
        let y = rect.top() + 4.0 + i as f32 * row_h;
        let row = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), row_h));
        let icon = match it.id.map(|id| tree.nodes[id as usize].kind) {
            Some(Kind::Dir) => "🗀 ",
            Some(Kind::File) => "🗋 ",
            _ => "",
        };
        let chars = (label_w / 6.8) as usize;
        text(painter, pos2(rect.left() + label_w, row.center().y), Align2::RIGHT_CENTER, truncate(&format!("{icon}{}", it.label), chars), 12.5, TEXT);
        let w = (it.size as f32 / max as f32 * bar_max).max(2.0);
        let bar = Rect::from_min_size(pos2(bar_x, y + 4.0), vec2(w, row_h - 8.0));
        painter.rect_filled(bar, egui::CornerRadius { nw: 0, sw: 0, ne: 4, se: 4 }, it.color);
        text(
            painter,
            pos2(bar.right() + 8.0, row.center().y),
            Align2::LEFT_CENTER,
            format!("{}   {:.1}%", fmt_size(it.size), pct(it.size, total)),
            12.0,
            TEXT_DIM,
        );
        hits.push(Hit { id: it.id, area: Area::Rect(row) });
    }
    // Baseline.
    let bottom = rect.top() + 4.0 + items.len() as f32 * row_h;
    painter.line_segment([pos2(bar_x, rect.top() + 2.0), pos2(bar_x, bottom)], Stroke::new(1.0, Color32::from_gray(90)));
}

// ---------- towers (vertical columns) ----------

fn draw_towers(painter: &Painter, tree: &Tree, root: u32, rect: Rect, hits: &mut Vec<Hit>) {
    let label_h = 120.0;
    let left_pad = 70.0;
    let plot = Rect::from_min_max(pos2(rect.left() + left_pad, rect.top() + 64.0), pos2(rect.right() - 12.0, rect.bottom() - label_h));
    let fit = ((plot.width() / 34.0).floor() as usize).clamp(3, 24);
    let mut items = items(tree, root, fit - 1);
    for it in &mut items {
        if it.id.is_some() {
            it.color = BAR;
        }
    }
    let max = items.iter().map(|i| i.size).max().unwrap_or(1).max(1);
    let slot = plot.width() / items.len() as f32;
    let col_w = (slot * 0.7).min(70.0);

    // Recessive grid: quarter lines with size labels.
    for q in 1..=4 {
        let v = max as f64 * q as f64 / 4.0;
        let y = plot.bottom() - plot.height() * q as f32 / 4.0;
        painter.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, Color32::from_gray(48)));
        text(painter, pos2(plot.left() - 6.0, y), Align2::RIGHT_CENTER, fmt_size(v as u64), 10.5, TEXT_DIM);
    }

    for (i, it) in items.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let h = (it.size as f32 / max as f32 * plot.height()).max(2.0);
        let col = Rect::from_min_max(pos2(cx - col_w / 2.0, plot.bottom() - h), pos2(cx + col_w / 2.0, plot.bottom()));
        painter.rect_filled(col, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, it.color);

        // Value above the column; turned sideways when the column is narrow.
        let value = fmt_size(it.size);
        let g = painter.layout_no_wrap(value, FontId::proportional(11.0), TEXT);
        if g.size().x + 4.0 <= slot {
            painter.galley(pos2(cx - g.size().x / 2.0, col.top() - g.size().y - 2.0), g, TEXT);
        } else {
            let pos = pos2(cx - g.size().y / 2.0, col.top() - 4.0);
            painter.add(egui::epaint::TextShape::new(pos, g, TEXT).with_angle(-FRAC_PI_2));
        }

        // Name under the column, slanted so long names stay readable.
        let name = truncate(&it.label, 20);
        let g = painter.layout_no_wrap(name, FontId::proportional(11.5), TEXT);
        let w = g.size().x;
        let anchor = pos2(cx + 4.0, plot.bottom() + 6.0);
        let dir = vec2(FRAC_PI_4.cos(), -FRAC_PI_4.sin());
        let pos = anchor - dir * w + vec2(-g.size().y * 0.7, g.size().y * 0.7);
        painter.add(egui::epaint::TextShape::new(pos, g, TEXT).with_angle(-FRAC_PI_4));

        let hit = Rect::from_min_max(pos2(cx - slot / 2.0, plot.top()), pos2(cx + slot / 2.0, plot.bottom()));
        hits.push(Hit { id: it.id, area: Area::Rect(hit) });
    }
    painter.line_segment([plot.left_bottom(), plot.right_bottom()], Stroke::new(1.0, Color32::from_gray(110)));
}

// ---------- treemap ----------

const MAX_ITEMS: usize = 400;
const HEADER: f32 = 15.0;

fn draw_treemap(painter: &Painter, tree: &Tree, id: u32, rect: Rect, depth: u32, color: Option<Color32>, hits: &mut Vec<Hit>) {
    let node = &tree.nodes[id as usize];
    let kids: Vec<u32> = node
        .children
        .iter()
        .copied()
        .filter(|&c| tree.nodes[c as usize].size > 0)
        .take(MAX_ITEMS)
        .collect();
    let sizes: Vec<f64> = kids.iter().map(|&c| tree.nodes[c as usize].size as f64).collect();
    let rects = treemap::squarify(&sizes, rect);
    for (i, (&c, r)) in kids.iter().zip(rects).enumerate() {
        if r.width() < 1.5 || r.height() < 1.5 {
            continue;
        }
        let n = &tree.nodes[c as usize];
        let base = color.unwrap_or_else(|| series(i));
        let shade = mix(base, Color32::from_gray(18), (depth.min(4) as f32) * 0.14);
        let fill = if n.kind == Kind::File { mix(shade, Color32::from_gray(40), 0.4) } else { shade };
        painter.rect_filled(r, 0.0, fill);
        painter.rect_stroke(r, 0.0, Stroke::new(0.5, Color32::from_black_alpha(170)), egui::StrokeKind::Inside);
        hits.push(Hit { id: Some(c), area: Area::Rect(r) });

        if r.width() > 50.0 && r.height() > 14.0 {
            let label = format!("{}  {}", n.name, fmt_size(n.size));
            let galley = painter.layout_no_wrap(label, FontId::proportional(11.0), Color32::WHITE);
            let clip = Rect::from_min_size(r.min + vec2(3.0, 1.0), vec2(r.width() - 6.0, HEADER - 2.0));
            painter.with_clip_rect(clip.intersect(painter.clip_rect())).galley(clip.min, galley, Color32::WHITE);
        }
        let nest = n.kind == Kind::Dir && !n.children.is_empty() && depth < 6 && r.width() > 40.0 && r.height() > 40.0;
        if nest {
            let inner = Rect::from_min_max(r.min + vec2(2.0, HEADER), r.max - vec2(2.0, 2.0));
            draw_treemap(painter, tree, c, inner, depth + 1, Some(base), hits);
        }
    }
}
