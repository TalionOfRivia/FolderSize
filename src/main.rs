#![windows_subsystem = "windows"]

mod scan;
mod charts;
mod sys;
mod treemap;

use eframe::egui::{self, Align2, Color32, FontId, Rect, RichText, Sense, Vec2};
use egui_extras::{Column, TableBuilder};
use scan::{Kind, NONE, Progress, Tree};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::time::Instant;

fn main() -> eframe::Result {
    let start = std::env::args_os().nth(1).map(|a| {
        let mut s = a.to_string_lossy().into_owned();
        if s.len() == 2 && s.ends_with(':') {
            s.push('\\');
        }
        PathBuf::from(s)
    });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FolderSize")
            .with_inner_size([1400.0, 850.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "FolderSize",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, start)))),
    )
}

struct Job {
    path: PathBuf,
    progress: Arc<Progress>,
    rx: mpsc::Receiver<Tree>,
    started: Instant,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Folders,
    Largest,
}

struct App {
    drives: Vec<sys::Drive>,
    admin: bool,
    job: Option<Job>,
    tree: Option<Tree>,
    scan_secs: f32,
    expanded: HashSet<u32>,
    selected: Option<u32>,
    map_root: u32,
    chart: charts::Chart,
    map_frac: f32,
    rows: Vec<(u32, u16)>,
    rows_dirty: bool,
    scroll_to: Option<usize>,
    tab: Tab,
    largest: Vec<u32>,
    confirm_delete: Option<u32>,
    message: Option<(String, bool)>,
}

impl App {
    fn new(cc: &eframe::CreationContext, start: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut app = App {
            drives: sys::drives(),
            admin: sys::is_admin(),
            job: None,
            tree: None,
            scan_secs: 0.0,
            expanded: HashSet::new(),
            selected: None,
            map_root: 0,
            chart: charts::Chart::Bars,
            map_frac: 0.42,
            rows: Vec::new(),
            rows_dirty: true,
            scroll_to: None,
            tab: Tab::Folders,
            largest: Vec::new(),
            confirm_delete: None,
            message: None,
        };
        if let Some(p) = start {
            app.start_scan(p);
        }
        app
    }

    fn start_scan(&mut self, path: PathBuf) {
        if let Some(job) = &self.job {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let (p2, path2) = (progress.clone(), path.clone());
        std::thread::spawn(move || {
            let tree = scan::scan(&path2, p2);
            let _ = tx.send(tree);
        });
        self.job = Some(Job { path, progress, rx, started: Instant::now() });
        self.message = None;
    }

    fn poll_scan(&mut self) {
        let Some(job) = &self.job else { return };
        if let Ok(tree) = job.rx.try_recv() {
            let cancelled = job.progress.cancel.load(Ordering::Relaxed);
            self.scan_secs = job.started.elapsed().as_secs_f32();
            self.job = None;
            if cancelled {
                self.message = Some(("Scan cancelled.".into(), false));
                return;
            }
            self.expanded.clear();
            self.expanded.insert(0);
            self.selected = Some(0);
            self.map_root = 0;
            self.largest = tree.largest_files(500);
            self.tree = Some(tree);
            self.rows_dirty = true;
            self.drives = sys::drives();
        }
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        let Some(tree) = &self.tree else { return };
        let mut stack = vec![(0u32, 0u16)];
        while let Some((id, depth)) = stack.pop() {
            self.rows.push((id, depth));
            if self.expanded.contains(&id) {
                for &c in tree.nodes[id as usize].children.iter().rev() {
                    stack.push((c, depth + 1));
                }
            }
        }
        self.rows_dirty = false;
    }

    /// Select a node from outside the tree view: expand its ancestors and scroll to it.
    fn reveal(&mut self, id: u32) {
        let Some(tree) = &self.tree else { return };
        let mut p = tree.nodes[id as usize].parent;
        while p != NONE {
            self.expanded.insert(p);
            p = tree.nodes[p as usize].parent;
        }
        self.selected = Some(id);
        self.rebuild_rows();
        self.scroll_to = self.rows.iter().position(|r| r.0 == id);
    }

    fn select(&mut self, id: u32) {
        self.selected = Some(id);
        if let Some(tree) = &self.tree {
            let n = &tree.nodes[id as usize];
            self.map_root = if n.kind == Kind::Dir { id } else { n.parent.max(0) };
            if self.map_root == NONE {
                self.map_root = 0;
            }
        }
    }

    fn delete(&mut self, id: u32) {
        let Some(tree) = &mut self.tree else { return };
        let path = tree.path_of(id);
        match trash::delete(&path) {
            Ok(()) => {
                let freed = tree.nodes[id as usize].size;
                let parent = tree.nodes[id as usize].parent;
                tree.remove(id);
                self.largest.retain(|&x| {
                    let mut p = x;
                    while p != NONE {
                        if p == id {
                            return false;
                        }
                        p = tree.nodes[p as usize].parent;
                    }
                    true
                });
                if self.selected.is_some_and(|s| is_within(tree, s, id)) {
                    self.selected = Some(parent);
                }
                if is_within(tree, self.map_root, id) {
                    self.map_root = parent;
                }
                self.rows_dirty = true;
                self.drives = sys::drives();
                self.message = Some((
                    format!("Moved to Recycle Bin: {} ({}). Empty the bin to actually free the space.", path.display(), fmt_size(freed)),
                    false,
                ));
            }
            Err(e) => {
                self.message = Some((format!("Could not delete {}: {e}", path.display()), true));
            }
        }
    }

    fn export_csv(&mut self) {
        let Some(tree) = &self.tree else { return };
        let Some(dest) = rfd::FileDialog::new()
            .set_file_name("foldersize-report.csv")
            .add_filter("CSV", &["csv"])
            .save_file()
        else {
            return;
        };
        let mut out = String::from("Path,Size (bytes),On disk (bytes),Size (GB),Files,Folders,Modified\n");
        for (i, n) in tree.nodes.iter().enumerate() {
            if n.kind != Kind::Dir || n.removed {
                continue;
            }
            let p = tree.path_of(i as u32);
            out.push_str(&format!(
                "\"{}\",{},{},{:.3},{},{},{}\n",
                p.display().to_string().replace('"', "\"\""),
                n.size,
                n.disk,
                n.size as f64 / GB,
                n.files,
                n.dirs,
                fmt_date(n.modified)
            ));
        }
        self.message = Some(match std::fs::write(&dest, out) {
            Ok(()) => (format!("Report saved to {}", dest.display()), false),
            Err(e) => (format!("Could not save report: {e}"), true),
        });
    }
}

fn is_within(tree: &Tree, mut id: u32, ancestor: u32) -> bool {
    while id != NONE {
        if id == ancestor {
            return true;
        }
        id = tree.nodes[id as usize].parent;
    }
    false
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_scan();
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.rows_dirty {
            self.rebuild_rows();
        }

        self.top_bar(ui);
        self.status_bar(ui);

        if let Some(job) = &self.job {
            egui::CentralPanel::default().show(ui, |ui| scan_progress(ui, job));
        } else if self.tree.is_none() {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Pick a drive or folder above to see what's eating your space.").size(18.0));
                });
            });
        } else {
            egui::CentralPanel::default().show(ui, |ui| {
                // Tree on the left, chart on the right, split by a draggable divider that
                // keeps its proportion when the window is resized or maximized.
                let full = ui.available_rect_before_wrap();
                let split_x = full.left() + full.width() * (1.0 - self.map_frac);
                let left = Rect::from_min_max(full.min, egui::pos2(split_x - 5.0, full.bottom()));
                let right = Rect::from_min_max(egui::pos2(split_x + 5.0, full.top()), full.max);
                let handle = Rect::from_min_max(egui::pos2(split_x - 4.0, full.top()), egui::pos2(split_x + 4.0, full.bottom()));
                let h = ui.interact(handle, ui.id().with("split"), Sense::drag());
                if h.dragged() {
                    if let Some(p) = h.interact_pointer_pos() {
                        self.map_frac = ((full.right() - p.x) / full.width()).clamp(0.2, 0.75);
                    }
                }
                if h.hovered() || h.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                let line = if h.hovered() || h.dragged() { Color32::from_gray(140) } else { Color32::from_gray(60) };
                ui.painter().vline(split_x, full.y_range(), egui::Stroke::new(1.0, line));

                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                    self.summary(ui);
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.tab, Tab::Folders, "Folder tree");
                        ui.selectable_value(&mut self.tab, Tab::Largest, "Largest files");
                    });
                    ui.separator();
                    match self.tab {
                        Tab::Folders => self.tree_table(ui),
                        Tab::Largest => self.largest_table(ui),
                    }
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| self.map_panel(ui));
            });
        }

        self.delete_dialog(&ctx);
    }
}

// ---------- panels ----------

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let busy = self.job.is_some();
                let mut go = None;
                for d in &self.drives {
                    let used = d.total.saturating_sub(d.free);
                    let frac = if d.total > 0 { used as f32 / d.total as f32 } else { 0.0 };
                    let text = format!(
                        "{}  {} free of {}",
                        d.root.display(),
                        fmt_size(d.free),
                        fmt_size(d.total)
                    );
                    let resp = ui
                        .add_enabled(!busy, egui::Button::new(text).min_size(Vec2::new(0.0, 30.0)))
                        .on_hover_text(format!("Scan {}", d.root.display()));
                    // Thin usage bar under each drive button.
                    let r = resp.rect;
                    let bar = Rect::from_min_size(r.left_bottom() + Vec2::new(4.0, -5.0), Vec2::new(r.width() - 8.0, 3.0));
                    ui.painter().rect_filled(bar, 1.0, Color32::from_gray(60));
                    let fill = Rect::from_min_size(bar.min, Vec2::new(bar.width() * frac, 3.0));
                    let c = if frac > 0.9 { Color32::from_rgb(230, 80, 70) } else { Color32::from_rgb(80, 160, 230) };
                    ui.painter().rect_filled(fill, 1.0, c);
                    if resp.clicked() {
                        go = Some(d.root.clone());
                    }
                }
                ui.separator();
                if ui.add_enabled(!busy, egui::Button::new("📂 Folder…")).clicked() {
                    go = rfd::FileDialog::new().pick_folder();
                }
                if let Some(tree) = &self.tree {
                    if ui.add_enabled(!busy, egui::Button::new("⟳ Rescan")).clicked() {
                        go = Some(tree.root_path.clone());
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Export CSV")).clicked() {
                        self.export_csv();
                    }
                }
                if let Some(p) = go {
                    self.start_scan(p);
                }
            });
            ui.add_space(2.0);
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some((msg, err)) = &self.message {
                    let c = if *err { Color32::from_rgb(240, 110, 100) } else { Color32::from_rgb(130, 200, 130) };
                    ui.colored_label(c, msg);
                } else if let Some(tree) = &self.tree {
                    let r = &tree.nodes[0];
                    ui.label(format!(
                        "{} files, {} folders scanned in {:.1}s",
                        fmt_count(r.files),
                        fmt_count(r.dirs),
                        self.scan_secs
                    ));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.admin {
                        ui.label("Running as administrator");
                    } else {
                        let root = self.tree.as_ref().map(|t| t.root_path.clone());
                        if ui
                            .button("🛡 Run as administrator")
                            .on_hover_text("Some system folders can only be read by an administrator. Restart elevated to include them.")
                            .clicked()
                            && sys::relaunch_as_admin(root.as_deref())
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
            });
        });
    }

    fn summary(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = &self.tree else { return };
        let r = &tree.nodes[0];
        ui.horizontal(|ui| {
            ui.heading(tree.root_path.display().to_string());
            ui.label(RichText::new(fmt_size(r.size)).strong().size(18.0));
        });
        // For a whole drive, show what the scan couldn't see (system files, no-access folders).
        if tree.root_path.parent().is_none() {
            if let Some((total, free)) = sys::disk_space(&tree.root_path) {
                let used = total.saturating_sub(free);
                let note = if used > r.disk {
                    format!(
                        "Not visible to the scan (system files, no access, restore points): {}",
                        fmt_size(used - r.disk)
                    )
                } else {
                    "Files found add up to more than the used space: hard-linked files are counted once per link.".into()
                };
                ui.label(format!("Drive: {} used, {} free.  {note}", fmt_size(used), fmt_size(free)));
            }
        }
    }

    fn tree_table(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = &self.tree else { return };
        let mut toggle = None;
        let mut clicked = None;
        let mut ask_delete = None;
        let selected = self.selected;
        let rows = &self.rows;
        let expanded = &self.expanded;

        let mut table = TableBuilder::new(ui)
            .auto_shrink([false, false])
            .striped(true)
            .resizable(false)
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::remainder().at_least(250.0).clip(true))
            .column(Column::exact(85.0))
            .column(Column::exact(140.0))
            .column(Column::exact(80.0))
            .column(Column::exact(70.0))
            .column(Column::exact(85.0));
        if let Some(i) = self.scroll_to.take() {
            table = table.scroll_to_row(i, Some(egui::Align::Center));
        }
        table
            .header(22.0, |mut h| {
                for t in ["Name", "Size", "% of parent", "Files", "Folders", "Modified"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, rows.len(), |mut row| {
                    let (id, depth) = rows[row.index()];
                    let n = &tree.nodes[id as usize];
                    row.set_selected(selected == Some(id));
                    let parent_size = if n.parent == NONE { n.size } else { tree.nodes[n.parent as usize].size };
                    let frac = if parent_size > 0 { n.size as f32 / parent_size as f32 } else { 0.0 };
                    row.col(|ui| {
                        ui.add_space(depth as f32 * 16.0);
                        if n.kind == Kind::Dir && !n.children.is_empty() {
                            let open = expanded.contains(&id);
                            if ui.add(egui::Button::new(if open { "⏷" } else { "⏵" }).frame(false).small()).clicked() {
                                toggle = Some(id);
                            }
                        } else {
                            ui.add_space(18.0);
                        }
                        let icon = match n.kind {
                            Kind::Dir => "🗀",
                            Kind::File => "🗋",
                            Kind::Link => "⮫",
                        };
                        let name = if n.parent == NONE { tree.root_path.display().to_string() } else { n.name.to_string() };
                        let mut text = RichText::new(format!("{icon} {name}"));
                        if n.kind == Kind::Link {
                            text = text.weak().italics();
                        }
                        let l = ui.add(egui::Label::new(text).selectable(false).truncate());
                        if n.denied {
                            ui.colored_label(Color32::from_rgb(230, 170, 60), "⚠")
                                .on_hover_text("Some contents could not be read (access denied). Run as administrator to include them.");
                        }
                        if n.kind == Kind::Link {
                            l.on_hover_text("Junction / link to another location. Not followed, so it isn't counted twice.");
                        }
                    });
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(fmt_size(n.size)).on_hover_text(format!(
                                "{} bytes\nOn disk: {}",
                                fmt_count(n.size),
                                fmt_size(n.disk)
                            ));
                        });
                    });
                    row.col(|ui| percent_bar(ui, frac));
                    row.col(|ui| {
                        if n.kind == Kind::Dir {
                            ui.label(fmt_count(n.files));
                        }
                    });
                    row.col(|ui| {
                        if n.kind == Kind::Dir {
                            ui.label(fmt_count(n.dirs));
                        }
                    });
                    row.col(|ui| {
                        ui.label(fmt_date(n.modified));
                    });
                    let resp = row.response();
                    if resp.clicked() {
                        clicked = Some(id);
                    }
                    if resp.double_clicked() && n.kind == Kind::Dir {
                        toggle = Some(id);
                    }
                    resp.context_menu(|ui| {
                        context_items(ui, tree, id, &mut ask_delete);
                    });
                });
            });

        if let Some(id) = toggle {
            if !self.expanded.remove(&id) {
                self.expanded.insert(id);
            }
            self.rows_dirty = true;
        }
        if let Some(id) = clicked {
            self.select(id);
        }
        if ask_delete.is_some() {
            self.confirm_delete = ask_delete;
        }
    }

    fn largest_table(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = &self.tree else { return };
        let mut clicked = None;
        let mut ask_delete = None;
        let selected = self.selected;
        let list = &self.largest;
        TableBuilder::new(ui)
            .auto_shrink([false, false])
            .striped(true)
            .resizable(false)
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(85.0))
            .column(Column::exact(85.0))
            .column(Column::remainder().clip(true))
            .header(22.0, |mut h| {
                for t in ["Size", "Modified", "Path"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, list.len(), |mut row| {
                    let id = list[row.index()];
                    let n = &tree.nodes[id as usize];
                    row.set_selected(selected == Some(id));
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(fmt_size(n.size));
                        });
                    });
                    row.col(|ui| {
                        ui.label(fmt_date(n.modified));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(tree.path_of(id).display().to_string()).selectable(false).truncate());
                    });
                    let resp = row.response();
                    if resp.clicked() {
                        clicked = Some(id);
                    }
                    resp.context_menu(|ui| context_items(ui, tree, id, &mut ask_delete));
                });
            });
        if let Some(id) = clicked {
            self.select(id);
        }
        if ask_delete.is_some() {
            self.confirm_delete = ask_delete;
        }
    }

    fn map_panel(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = &self.tree else { return };
        let root = self.map_root;
        ui.horizontal(|ui| {
            let parent = tree.nodes[root as usize].parent;
            if ui.add_enabled(parent != NONE, egui::Button::new("⬆ Up")).clicked() {
                self.map_root = parent;
            }
            let p = tree.path_of(root);
            ui.add(egui::Label::new(RichText::new(p.display().to_string()).strong()).truncate());
        });
        ui.horizontal_wrapped(|ui| {
            for c in charts::Chart::ALL {
                ui.selectable_value(&mut self.chart, c, c.label()).on_hover_text(c.hint());
            }
        });
        ui.label(
            RichText::new(format!(
                "{} · click to select · double-click a folder to zoom in · right-click for actions",
                self.chart.hint()
            ))
            .weak()
            .small(),
        );
        ui.add_space(4.0);
        let ev = charts::show(ui, self.chart, tree, root, self.selected);
        if let Some(id) = ev.select {
            self.reveal(id);
        }
        if let Some(id) = ev.zoom {
            self.map_root = id;
        }
        if ev.delete.is_some() {
            self.confirm_delete = ev.delete;
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_delete else { return };
        let Some(tree) = &self.tree else { return };
        let n = &tree.nodes[id as usize];
        let path = tree.path_of(id);
        let mut decision = None;
        egui::Window::new("Send to Recycle Bin?")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(RichText::new(path.display().to_string()).strong());
                let what = if n.kind == Kind::Dir {
                    format!("{} — {} files, {} folders", fmt_size(n.size), fmt_count(n.files), fmt_count(n.dirs))
                } else {
                    fmt_size(n.size)
                };
                ui.label(what);
                ui.add_space(6.0);
                ui.label("It goes to the Recycle Bin, so you can still restore it.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Send to Recycle Bin").color(Color32::from_rgb(240, 120, 110))).clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.confirm_delete = None;
                self.delete(id);
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }
}

fn context_items(ui: &mut egui::Ui, tree: &Tree, id: u32, ask_delete: &mut Option<u32>) {
    let path = tree.path_of(id);
    if ui.button("Show in Explorer").clicked() {
        sys::show_in_explorer(&path);
        ui.close();
    }
    if ui.button("Copy path").clicked() {
        ui.ctx().copy_text(path.display().to_string());
        ui.close();
    }
    if tree.nodes[id as usize].parent != NONE {
        ui.separator();
        if ui.button("Send to Recycle Bin…").clicked() {
            *ask_delete = Some(id);
            ui.close();
        }
    }
}

fn scan_progress(ui: &mut egui::Ui, job: &Job) {
    let p = &job.progress;
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.heading(format!("Scanning {}", job.path.display()));
        ui.add_space(12.0);
        ui.spinner();
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!(
                "{}   ·   {} files   ·   {} folders   ·   {:.0}s",
                fmt_size(p.bytes.load(Ordering::Relaxed)),
                fmt_count(p.files.load(Ordering::Relaxed)),
                fmt_count(p.dirs.load(Ordering::Relaxed)),
                job.started.elapsed().as_secs_f32()
            ))
            .size(16.0),
        );
        ui.add_space(6.0);
        if let Ok(cur) = p.current.try_lock() {
            ui.add(egui::Label::new(RichText::new(cur.as_str()).weak()).truncate());
        }
        ui.add_space(12.0);
        if ui.button("Cancel").clicked() {
            p.cancel.store(true, Ordering::Relaxed);
        }
    });
}

fn percent_bar(ui: &mut egui::Ui, frac: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width() - 4.0, 14.0), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, Color32::from_gray(45));
    let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * frac.clamp(0.0, 1.0), rect.height()));
    let c = Color32::from_rgb(70 + (150.0 * frac) as u8, 140, 220 - (150.0 * frac) as u8);
    ui.painter().rect_filled(fill, 2.0, c);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("{:.1}%", frac * 100.0),
        FontId::proportional(11.0),
        Color32::WHITE,
    );
}

// ---------- formatting ----------

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

fn fmt_size(b: u64) -> String {
    let b = b as f64;
    if b >= 1024.0 * GB {
        format!("{:.2} TB", b / (1024.0 * GB))
    } else if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{b:.0} B")
    }
}

fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// FILETIME (100 ns ticks since 1601) to YYYY-MM-DD, UTC.
fn fmt_date(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let secs = (ft / 10_000_000) as i64 - 11_644_473_600;
    let days = secs.div_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}
