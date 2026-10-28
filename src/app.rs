mod ai_storage;
mod history;

use crate::ai;
use crate::scan::{self, ChildEntry, Progress, Snapshot};
use crate::store;
use crate::classify;
use crate::targets::{self, Group, Safety, Target};
use crate::theme::{self, Pal};
use crate::tray;
use eframe::egui::{self, Align, Color32, Layout, Pos2, RichText, Sense, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// Folders whose app is no longer installed.
const LEFTOVER: Color32 = Color32::from_rgb(175, 130, 255);
const AUTO_SCAN_AFTER_SECS: u64 = 6 * 3600;

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Overview,
    History,
    Ai,
    Explorer,
}

#[derive(PartialEq, Clone, Copy)]
enum Col {
    Name,
    Size,
    Growth,
}

enum ExploreState {
    Idle,
    Loading,
    Done(Result<Vec<ChildEntry>, String>),
}

#[derive(Default)]
pub struct Args {
    pub screenshot: Option<PathBuf>,
    pub page_explorer: bool,
    pub page_history: bool,
    pub page_ai: bool,
    pub size: Option<[f32; 2]>,
    pub search: Option<String>,
    pub select: Option<String>,
    pub theme: Option<egui::ThemePreference>,
    pub scan: bool,
    pub group: Option<usize>,
    /// Inspector tab: overview, inside or clear.
    pub tab: Option<String>,
}

/// Inspector sections.
#[derive(PartialEq, Clone, Copy)]
enum Tab {
    About,
    Inside,
    Clear,
}

pub struct App {
    history: Vec<Snapshot>,
    scanning: Option<Arc<Progress>>,
    scan_result: Arc<Mutex<Option<Snapshot>>>,
    page: Page,
    ai: Arc<Mutex<ai_storage::AiState>>,
    ai_open: Option<&'static str>,
    ai_show_all: bool,
    /// Scroll the open AI tool into view on the next frame.
    ai_scroll: bool,
    filter: Option<Safety>,
    group: Option<Group>,
    show_all_items: bool,
    range: history::Range,
    disk_log: Vec<store::DiskSample>,
    disk_log_at: u64,
    search: String,
    sort: (Col, bool), // (column, descending)
    selected: Option<&'static str>,
    tab: Tab,
    /// Folder expanded in the inspector's "Inside" list.
    open_item: Option<String>,
    fda_hidden: bool,
    explore_path: PathBuf,
    explore: Arc<Mutex<ExploreState>>,
    explore_started: bool,
    args: Args,
    frames: u32,
    shot_requested: bool,
    tray_on: bool,
    logo: egui::TextureHandle,
}

pub fn fmt_bytes(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < 4 {
        v /= 1000.0;
        i += 1;
    }
    // Three significant digits: 1.24 GB, 13.6 GB, 205 GB.
    match i {
        0 | 1 => format!("{:.0} {}", v, units[i]),
        _ if v < 9.995 => format!("{:.2} {}", v, units[i]),
        _ if v < 99.95 => format!("{:.1} {}", v, units[i]),
        _ => format!("{:.0} {}", v, units[i]),
    }
}

fn ago(secs: u64) -> String {
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

fn fmt_delta(d: i64, p: &Pal) -> (String, Color32) {
    if d.abs() < 10_000_000 {
        return ("~0".into(), p.muted);
    }
    let s = fmt_bytes(d.unsigned_abs());
    if d > 0 { (format!("+{s}"), p.bad) } else { (format!("−{s}"), p.good) }
}

fn tilde(path: &str) -> String {
    let home = format!("{}/", dirs::home_dir().unwrap_or_default().display());
    path.strip_prefix(&home).map(|r| format!("~/{r}")).unwrap_or_else(|| path.to_string())
}

fn reveal(path: &Path) {
    let _ = Command::new("open").arg("-R").arg(path).spawn();
}

// ---------- small widgets ----------

fn card<R>(ui: &mut egui::Ui, p: &Pal, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(12.0)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn caption(ui: &mut egui::Ui, p: &Pal, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(11.0).color(p.muted));
}

fn pill(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(11.5), fg);
    let size = galley.size() + Vec2::new(16.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, size.y / 2.0, bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

fn safety_pill(ui: &mut egui::Ui, s: Safety, p: &Pal) {
    let (fg, bg) = theme::badge_colors(s, p);
    pill(ui, s.label(), fg, bg).on_hover_text(s.explain());
}

fn bar(ui: &mut egui::Ui, p: &Pal, frac: f32, color: Color32) {
    let w = ui.available_width().max(20.0) - 8.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, p.track);
    let mut fill = rect;
    fill.set_width((rect.width() * frac.clamp(0.0, 1.0)).max(3.0));
    ui.painter().rect_filled(fill, 3.0, color);
}

/// Horizontal stacked bar with a legend: (color, bytes, label).
fn stacked(ui: &mut egui::Ui, p: &Pal, parts: &[(Color32, u64, &str)]) {
    let total: u64 = parts.iter().map(|x| x.1).sum::<u64>().max(1);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, p.track);
    let mut x = rect.left();
    for (c, b, _) in parts {
        let w = rect.width() * *b as f32 / total as f32;
        if w > 0.5 {
            ui.painter().rect_filled(egui::Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, rect.height())), 2.0, *c);
        }
        x += w;
    }
    ui.add_space(2.0);
    // Compact legend that wraps; exact share on hover.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (c, b, label) in parts.iter().filter(|x| x.1 > 0) {
            let mut job = egui::text::LayoutJob::default();
            let fmt = |c: Color32, mono: bool| egui::TextFormat { font_id: if mono { egui::FontId::monospace(11.0) } else { egui::FontId::proportional(11.5) }, color: c, ..Default::default() };
            job.append("● ", 0.0, fmt(*c, false));
            job.append(label, 0.0, fmt(p.muted, false));
            job.append(&format!(" {}", fmt_bytes(*b)), 0.0, fmt(p.text, true));
            ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend)).on_hover_text(format!("{:.0}%", *b as f32 * 100.0 / total as f32));
            ui.add_space(8.0);
        }
    });
}

/// A small "?" that shows `text` on hover: keeps explanations reachable without crowding the view.
fn help(ui: &mut egui::Ui, p: &Pal, text: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    let c = if resp.hovered() { p.text } else { p.muted };
    ui.painter().circle_stroke(rect.center(), 6.5, Stroke::new(1.0, c));
    ui.painter().text(rect.center() + Vec2::new(0.0, 0.5), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(10.0), c);
    resp.on_hover_text(text)
}

/// Status, size, reclaimable and 7-day columns; name and share take the rest.
const FIXED_COLS: f32 = 118.0 + 90.0 + 104.0 + 84.0;

struct Layout3 {
    wide: bool,
    name_w: f32,
    share_w: f32,
}

/// The overview table's columns, shared by the header and every category card.
fn columns<'a>(tb: TableBuilder<'a>, l: &Layout3) -> TableBuilder<'a> {
    let mut tb = tb
        .striped(false)
        .resizable(false)
        .vscroll(false)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(l.name_w).clip(true))
        .column(Column::exact(118.0))
        .column(Column::exact(90.0))
        .column(Column::exact(104.0));
    if l.wide {
        tb = tb.column(Column::exact(l.share_w));
    }
    tb.column(Column::exact(84.0))
}

/// Segmented tab control spanning the available width.
fn segmented(ui: &mut egui::Ui, p: &Pal, tabs: &[(Tab, &str)], cur: &mut Tab) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, p.window);
    let w = rect.width() / tabs.len() as f32;
    for (i, (tab, label)) in tabs.iter().enumerate() {
        let r = egui::Rect::from_min_size(rect.min + Vec2::new(w * i as f32, 0.0), Vec2::new(w, rect.height())).shrink(2.0);
        let resp = ui.interact(r, ui.scope_id().with(("tab", i)), Sense::click());
        let on = *cur == *tab;
        if on {
            ui.painter().rect(r, 6.0, p.card, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 6.0, p.hover);
        }
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(12.5), if on { p.text } else { p.muted });
        if resp.clicked() {
            *cur = *tab;
        }
    }
}

fn trend_color(pts: &[u64], p: &Pal) -> Color32 {
    if pts.last() > pts.first() { p.bad } else { p.good }
}

/// Larger area chart with min/max labels, for the inspector.
fn chart(ui: &mut egui::Ui, p: &Pal, pts: &[(u64, u64)]) {
    let size = Vec2::new(ui.available_width(), 90.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 8.0, p.window);
    if pts.len() < 2 {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Trend appears after two scans", egui::FontId::proportional(12.0), p.muted);
        return;
    }
    let inner = rect.shrink2(Vec2::new(10.0, 14.0));
    let ys: Vec<u64> = pts.iter().map(|x| x.1).collect();
    let (mn, mx) = (*ys.iter().min().unwrap() as f32, *ys.iter().max().unwrap() as f32);
    let rng = (mx - mn).max(1.0);
    let (t0, t1) = (pts[0].0 as f32, pts[pts.len() - 1].0 as f32);
    let tr = (t1 - t0).max(1.0);
    let line: Vec<Pos2> = pts
        .iter()
        .map(|(t, v)| egui::pos2(inner.left() + (*t as f32 - t0) / tr * inner.width(), inner.bottom() - (*v as f32 - mn) / rng * inner.height()))
        .collect();
    let col = trend_color(&ys, p);
    ui.painter().add(egui::Shape::line(line.clone(), Stroke::new(2.0, col)));
    if let Some(last) = line.last() {
        ui.painter().circle_filled(*last, 3.5, col);
    }
    let f = egui::FontId::proportional(10.5);
    ui.painter().text(rect.left_top() + Vec2::new(10.0, 3.0), egui::Align2::LEFT_TOP, fmt_bytes(mx as u64), f.clone(), p.muted);
    ui.painter().text(rect.left_bottom() + Vec2::new(10.0, -3.0), egui::Align2::LEFT_BOTTOM, fmt_bytes(mn as u64), f, p.muted);
}

/// One color per category, shared by the sidebar, the disk bar and section headers.
fn group_color(g: Group) -> Color32 {
    match g {
        Group::Caches => Color32::from_rgb(48, 176, 199),
        Group::Developer => Color32::from_rgb(94, 92, 230),
        Group::Apps => Color32::from_rgb(255, 149, 0),
        Group::Personal => Color32::from_rgb(255, 55, 95),
        Group::System => Color32::from_rgb(162, 132, 94),
    }
}

/// Size of each category in a snapshot (overlapping rows excluded).
fn group_sizes(latest: &Snapshot) -> [u64; 5] {
    let mut by = [0u64; 5];
    for r in &latest.targets {
        if let Some(t) = targets::find(&r.id).filter(|t| !t.nested) {
            by[Group::ALL.iter().position(|g| *g == t.group).unwrap_or(0)] += r.bytes;
        }
    }
    by
}

const OTHER_NOTE: &str = "\nNot itemized: macOS itself, Documents and Desktop, other folders, and anything this app can't read without Full Disk Access.";

/// Whole-disk bar: categories, everything else, free. Returns the category clicked, if any.
fn disk_bar(ui: &mut egui::Ui, p: &Pal, disk: &scan::Disk, by: &[u64; 5]) -> Option<Group> {
    let tracked: u64 = by.iter().sum();
    let other = disk.used().saturating_sub(tracked);
    let mut segs: Vec<(Option<Group>, &str, u64, Color32)> = Group::ALL.iter().zip(by).map(|(g, b)| (Some(*g), g.label(), *b, group_color(*g))).collect();
    segs.push((None, "Everything else", other, p.muted.gamma_multiply(0.55)));
    segs.push((None, "Free", disk.free, p.track));
    let total = disk.total.max(1) as f32;

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
    let mut x = rect.left();
    let mut hovered = None;
    for (i, (_, _, b, c)) in segs.iter().enumerate() {
        let w = rect.width() * *b as f32 / total;
        if w < 0.5 {
            continue;
        }
        let r = egui::Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, rect.height()));
        let paint = r.shrink2(Vec2::new(if w > 4.0 { 1.0 } else { 0.0 }, 0.0));
        ui.painter().rect_filled(paint, 4.0, *c);
        if resp.hover_pos().is_some_and(|h| r.contains(h)) {
            hovered = Some(i);
            ui.painter().rect_stroke(paint, 4.0, Stroke::new(2.0, p.text), egui::StrokeKind::Inside);
        }
        x += w;
    }
    let mut clicked = None;
    if let Some(i) = hovered {
        let (g, label, b, _) = segs[i];
        let extra = if g.is_some() { "\nClick to open" } else if label == "Everything else" { OTHER_NOTE } else { "" };
        let resp = resp.on_hover_text(format!("{label}: {} ({:.0}% of disk){extra}", fmt_bytes(b), b as f32 / total * 100.0));
        if resp.clicked() {
            clicked = g;
        }
        if g.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    ui.add_space(10.0);
    // Legend: two columns.
    let col_w = (ui.available_width() - 16.0) / 2.0;
    for pair in segs.chunks(2) {
        ui.horizontal(|ui| {
            for (j, (g, label, b, c)) in pair.iter().enumerate() {
                if j == 1 {
                    ui.add_space(16.0);
                }
                let (r, resp) = ui.allocate_exact_size(Vec2::new(col_w, 22.0), if g.is_some() { Sense::click() } else { Sense::hover() });
                if resp.hovered() && g.is_some() {
                    ui.painter().rect_filled(r.expand2(Vec2::new(4.0, 0.0)), 5.0, p.hover);
                }
                ui.painter().circle_filled(r.left_center() + Vec2::new(5.0, 0.0), 4.5, *c);
                ui.painter().text(r.left_center() + Vec2::new(16.0, 0.0), egui::Align2::LEFT_CENTER, *label, egui::FontId::proportional(12.5), p.text);
                ui.painter().text(r.right_center(), egui::Align2::RIGHT_CENTER, fmt_bytes(*b), egui::FontId::monospace(11.5), p.text);
                let resp = resp.on_hover_text(format!("{:.0}% of disk{}", *b as f32 / total * 100.0, if *label == "Everything else" { OTHER_NOTE } else { "" }));
                if resp.clicked() {
                    clicked = *g;
                }
            }
        });
    }
    clicked
}

struct Row {
    t: &'static Target,
    bytes: u64,
    note: Option<String>,
    d7: Option<i64>,
    reclaim: u64,
    sub: String,
    leftovers: usize,
}

/// Bytes that can be cleared without losing anything: the whole location when it's Safe,
/// or for broken-down locations the folders classified Safe or Leftover.
fn reclaimable(t: &Target, r: &scan::TargetResult) -> u64 {
    if t.breakdown && !r.items.is_empty() {
        r.items
            .iter()
            .filter(|it| {
                let name = Path::new(&it.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let info = classify::classify(t.id, &name);
                info.leftover || info.safety == Safety::Safe
            })
            .map(|it| it.bytes)
            .sum()
    } else if t.safety == Safety::Safe {
        r.bytes
    } else {
        0
    }
}

fn leftover_count(t: &Target, r: &scan::TargetResult) -> usize {
    if !t.breakdown {
        return 0;
    }
    r.items.iter().filter(|it| classify::classify(t.id, &Path::new(&it.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).leftover).count()
}

/// Where a location is and what it holds (shown on hover).
fn subtitle(t: &Target, r: &scan::TargetResult) -> String {
    if t.breakdown && !r.items.is_empty() {
        let leftovers = r
            .items
            .iter()
            .filter(|it| classify::classify(t.id, &Path::new(&it.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).leftover)
            .count();
        let top: Vec<String> = r.items.iter().take(3).map(|it| Path::new(&it.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect();
        let unit = if t.kind == targets::Kind::Installers { "files" } else { "folders" };
        let mut s = format!("Contains {} {unit}, largest: {}", r.items.len(), top.join(", "));
        if leftovers > 0 {
            s = format!("{s}\n{leftovers} of them belong to apps that are no longer installed");
        }
        return s;
    }
    match t.kind {
        targets::Kind::Paths => {
            let found: Vec<&str> = t.paths.iter().copied().filter(|p| targets::expand(p).exists()).collect();
            match found.split_first() {
                Some((first, rest)) if rest.is_empty() => format!("Location: {}", tilde(&targets::expand(first).to_string_lossy())),
                Some(_) => format!("Locations:\n{}", found.iter().map(|p| tilde(&targets::expand(p).to_string_lossy())).collect::<Vec<_>>().join("\n")),
                None => String::new(),
            }
        }
        targets::Kind::Snapshots => r.note.clone().unwrap_or_default(),
        _ => format!("{} found in your projects", r.items.len()),
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        theme::install(&cc.egui_ctx);
        classify::warm_up();
        if let Some(t) = args.theme {
            cc.egui_ctx.options_mut(|o| o.theme_preference = t);
        }
        let history = store::load();
        let mut app = App {
            history,
            scanning: None,
            scan_result: Arc::new(Mutex::new(None)),
            page: if args.page_explorer { Page::Explorer } else if args.page_history { Page::History } else if args.page_ai { Page::Ai } else { Page::Overview },
            ai: Arc::new(Mutex::new(ai_storage::AiState::Idle)),
            ai_open: args.select.as_deref().and_then(ai::find).map(|t| t.id),
            ai_show_all: false,
            ai_scroll: false,
            filter: None,
            group: args.group.and_then(|i| Group::ALL.get(i).copied()),
            show_all_items: false,
            range: history::Range::All,
            disk_log: vec![],
            disk_log_at: 0,
            search: args.search.clone().unwrap_or_default(),
            sort: (Col::Size, true),
            selected: args.select.as_deref().and_then(|id| targets::find(id)).map(|t| t.id),
            tab: match args.tab.as_deref() {
                Some("inside") => Tab::Inside,
                Some("clear") => Tab::Clear,
                _ => Tab::About,
            },
            open_item: None,
            fda_hidden: false,
            explore_path: dirs::home_dir().unwrap_or_else(|| "/".into()),
            explore: Arc::new(Mutex::new(ExploreState::Idle)),
            explore_started: false,
            args,
            frames: 0,
            shot_requested: false,
            tray_on: false,
            logo: cc.egui_ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([128, 128], &crate::logo::icon_rgba(128)), egui::TextureOptions::LINEAR),
        };
        if app.args.screenshot.is_none() {
            app.tray_on = tray::init(&cc.egui_ctx);
        }
        let stale = app.history.last().is_none_or(|s| scan::now().saturating_sub(s.ts) > AUTO_SCAN_AFTER_SECS);
        if stale || app.args.scan {
            app.start_scan(&cc.egui_ctx);
        }
        app
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        if self.scanning.is_some() {
            return;
        }
        let progress = Arc::new(Progress::default());
        self.scanning = Some(progress.clone());
        let out = self.scan_result.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let snap = scan::run_scan(progress);
            *out.lock().unwrap() = Some(snap);
            ctx.request_repaint();
        });
    }

    fn start_explore(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.explore_path = path.clone();
        self.explore_started = true;
        *self.explore.lock().unwrap() = ExploreState::Loading;
        let out = self.explore.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = scan::explore(&path);
            *out.lock().unwrap() = ExploreState::Done(r);
            ctx.request_repaint();
        });
    }

    fn series_ts(&self, id: &str) -> Vec<(u64, u64)> {
        let start = self.history.len().saturating_sub(120);
        self.history[start..].iter().filter_map(|s| s.targets.iter().find(|t| t.id == id).map(|t| (s.ts, t.bytes))).collect()
    }

    /// (change vs previous scan, change vs ~7 days ago)
    fn deltas(&self, id: &str) -> (Option<i64>, Option<i64>) {
        let pts = self.series_ts(id);
        if pts.len() < 2 {
            return (None, None);
        }
        let last = pts[pts.len() - 1];
        let prev = pts[pts.len() - 2];
        let cutoff = last.0.saturating_sub(7 * 86400);
        let base = pts.iter().rev().find(|x| x.0 <= cutoff).copied().unwrap_or(pts[0]);
        (Some(last.1 as i64 - prev.1 as i64), Some(last.1 as i64 - base.1 as i64))
    }

    fn rows(&self, latest: &Snapshot) -> Vec<Row> {
        let q = self.search.to_lowercase();
        let mut rows: Vec<Row> = latest
            .targets
            .iter()
            .filter_map(|r| {
                let t = targets::find(&r.id)?;
                if self.group.is_some_and(|g| g != t.group) || self.filter.is_some_and(|f| f != t.safety) || !(r.bytes > 0 || r.note.is_some()) {
                    return None;
                }
                if !q.is_empty() && !t.name.to_lowercase().contains(&q) {
                    return None;
                }
                let (_, d7) = self.deltas(t.id);
                Some(Row { t, bytes: r.bytes, note: r.note.clone(), d7, reclaim: reclaimable(t, r), sub: subtitle(t, r), leftovers: leftover_count(t, r) })
            })
            .collect();
        let (col, desc) = self.sort;
        rows.sort_by(|a, b| {
            let o = match col {
                Col::Name => a.t.name.cmp(b.t.name),
                Col::Size => a.bytes.cmp(&b.bytes),
                Col::Growth => a.d7.unwrap_or(i64::MIN).cmp(&b.d7.unwrap_or(i64::MIN)),
            };
            if desc { o.reverse() } else { o }
        });
        rows
    }

    fn poll(&mut self) {
        if let Some(snap) = self.scan_result.lock().unwrap().take() {
            self.history.push(snap);
            store::save(&self.history);
            self.scanning = None;
        }
    }

    // ---------- chrome ----------

    fn sidebar(&mut self, ui: &mut egui::Ui, p: &Pal, disk: &scan::Disk, latest: Option<&Snapshot>) {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(34.0)));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(2.0);
                ui.label(RichText::new("DiskLens").size(15.0).strong());
                ui.label(RichText::new("by devground-labs").size(11.0).color(p.muted));
            });
        });
        ui.add_space(18.0);
        ui.label(RichText::new("  LIBRARY").size(10.5).color(p.muted));
        for (page, label) in [(Page::Overview, "Overview"), (Page::History, "History"), (Page::Ai, "AI Storage"), (Page::Explorer, "Explorer")] {
            let on = self.page == page;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
            if on {
                ui.painter().rect_filled(rect, 7.0, p.accent.gamma_multiply(if p.dark { 0.3 } else { 0.16 }));
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 7.0, p.hover);
            }
            ui.painter().text(rect.left_center() + Vec2::new(12.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.5), if on { p.accent } else { p.text });
            if resp.clicked() {
                self.page = page;
            }
        }
        ui.add_space(16.0);
        ui.label(RichText::new("  CATEGORIES").size(10.5).color(p.muted));
        let size_of = |g: Option<Group>| -> u64 {
            latest.map_or(0, |l| l.targets.iter().filter(|r| targets::find(&r.id).is_some_and(|t| !t.nested && g.is_none_or(|g| g == t.group))).map(|r| r.bytes).sum())
        };
        let entries: Vec<(Option<Group>, &str)> = std::iter::once((None, "All locations")).chain(Group::ALL.iter().map(|g| (Some(*g), g.label()))).collect();
        for (g, label) in entries {
            let on = self.page == Page::Overview && self.group == g;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
            if on {
                ui.painter().rect_filled(rect, 7.0, p.accent.gamma_multiply(if p.dark { 0.3 } else { 0.16 }));
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 7.0, p.hover);
            }
            let mut tx = 12.0;
            if let Some(g) = g {
                ui.painter().circle_filled(rect.left_center() + Vec2::new(15.0, 0.0), 4.0, group_color(g));
                tx = 26.0;
            }
            ui.painter().text(rect.left_center() + Vec2::new(tx, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), if on { p.accent } else { p.text });
            let sz = size_of(g);
            if sz > 0 {
                ui.painter().text(rect.right_center() - Vec2::new(8.0, 0.0), egui::Align2::RIGHT_CENTER, fmt_bytes(sz), egui::FontId::proportional(11.0), p.muted);
            }
            if resp.clicked() {
                self.page = Page::Overview;
                if self.group != g {
                    self.group = g;
                    self.filter = None;
                    self.selected = None;
                }
            }
        }
        ui.add_space((ui.available_height() - 74.0).max(12.0));
        ui.vertical(|ui| {
            let frac = disk.used() as f32 / disk.total.max(1) as f32;
            ui.label(RichText::new(format!("{} free", fmt_bytes(disk.free))).size(13.0).strong());
            let w = ui.available_width() - 14.0;
            let (r, _) = ui.allocate_exact_size(Vec2::new(w, 5.0), Sense::hover());
            ui.painter().rect_filled(r, 2.5, p.track);
            let mut f = r;
            f.set_width(r.width() * frac.clamp(0.0, 1.0));
            ui.painter().rect_filled(f, 2.5, if frac > 0.9 { p.bad } else { p.accent });
            ui.label(RichText::new(format!("{} of {} used", fmt_bytes(disk.used()), fmt_bytes(disk.total))).size(11.0).color(p.muted));
        });
    }

    fn statusbar(&self, ui: &mut egui::Ui, p: &Pal) {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            match (&self.scanning, self.history.last()) {
                (Some(pr), _) => {
                    let (d, t) = (pr.done.load(Ordering::Relaxed), pr.total.load(Ordering::Relaxed));
                    ui.spinner();
                    ui.label(RichText::new(format!("Scanning {} ({d}/{t})", pr.current.lock().unwrap())).size(12.0).color(p.muted));
                }
                (None, Some(s)) => {
                    ui.label(RichText::new(format!("Last scan {} · took {}s · {} snapshots saved", ago(scan::now().saturating_sub(s.ts)), s.secs, self.history.len())).size(12.0).color(p.muted));
                }
                _ => {}
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, p: &Pal, title: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(22.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let on_ai = self.page == Page::Ai;
                let scanning = if on_ai { self.ai_scanning() } else { self.scanning.is_some() };
                let btn = egui::Button::new(RichText::new(if scanning { "Scanning…" } else { "Scan now" }).color(Color32::WHITE)).fill(p.accent).stroke(Stroke::NONE);
                if ui.add_enabled(!scanning, btn).clicked() {
                    let ctx = ui.ctx().clone();
                    if on_ai {
                        self.start_ai_scan(&ctx);
                    } else {
                        self.start_scan(&ctx);
                    }
                }
                if on_ai {
                    if let Some(when) = self.ai_scanned_label() {
                        ui.label(RichText::new(when).size(11.5).color(p.muted));
                    }
                }
                if self.page == Page::Overview {
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search locations").desired_width(200.0));
                }
            });
        });
    }

    // ---------- overview ----------

    fn summary(&mut self, ui: &mut egui::Ui, p: &Pal, latest: &Snapshot, disk: &scan::Disk) {
        let safe: u64 = latest.targets.iter().filter_map(|r| targets::find(&r.id).filter(|t| !t.nested).map(|t| reclaimable(t, r))).sum();
        let growth: Option<i64> = {
            let mut any = false;
            let mut g = 0i64;
            for r in latest.targets.iter().filter(|r| !targets::find(&r.id).is_some_and(|t| t.nested)) {
                if let (Some(d), _) = self.deltas(&r.id) {
                    g += d;
                    any = true;
                }
            }
            any.then_some(g)
        };
        let denied = latest.targets.iter().any(|t| t.denied);
        let by = group_sizes(latest);
        let mut open: Option<Group> = None;

        ui.horizontal_top(|ui| {
            let total_w = ui.available_width();
            let gap = ui.spacing().item_spacing.x;
            let left_w = (total_w * 0.52).max(380.0);
            let right_w = total_w - left_w - gap;
            ui.allocate_ui(Vec2::new(left_w, 0.0), |ui| {
                card(ui, p, |ui| ui.vertical(|ui| {
                    ui.set_min_height(172.0);
                    ui.horizontal(|ui| {
                        caption(ui, p, "Macintosh HD · Data");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let pct = disk.used() as f32 / disk.total.max(1) as f32 * 100.0;
                            ui.label(RichText::new(format!("{pct:.0}% full")).size(12.0).color(if pct > 90.0 { p.bad } else { p.muted }));
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} used", fmt_bytes(disk.used()))).size(22.0).strong());
                        ui.label(RichText::new(format!("of {} · {} free", fmt_bytes(disk.total), fmt_bytes(disk.free))).color(p.muted));
                    });
                    ui.add_space(8.0);
                    open = disk_bar(ui, p, disk, &by);
                }));
            });
            ui.allocate_ui(Vec2::new(right_w, 0.0), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    let titled = |ui: &mut egui::Ui, title: &str, tip: &str| {
                        ui.horizontal(|ui| {
                            caption(ui, p, title);
                            help(ui, p, tip);
                        });
                    };
                    card(ui, p, |ui| {
                        ui.set_min_height(64.0);
                        titled(ui, "Likely safe to reclaim", "Caches, installers, leftovers from removed apps and build output: things that rebuild themselves or aren't needed any more.");
                        ui.label(RichText::new(fmt_bytes(safe)).size(26.0).strong().color(p.good));
                    });
                    card(ui, p, |ui| {
                        ui.set_min_height(64.0);
                        titled(ui, "Change since last scan", "Growth or shrinkage across all tracked locations since the previous scan.");
                        match growth {
                            Some(g) => {
                                let (s, c) = fmt_delta(g, p);
                                ui.label(RichText::new(s).size(26.0).strong().color(c));
                            }
                            None => {
                                ui.label(RichText::new("No baseline yet").size(18.0).strong().color(p.muted)).on_hover_text("Run another scan later to see growth.");
                            }
                        }
                    });
                });
            });
        });
        if denied && !self.fda_hidden {
            ui.add_space(4.0);
            egui::Frame::new().fill(p.warn.gamma_multiply(0.12)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 5)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Some folders couldn't be read, so sizes may be low.").size(12.0).color(p.warn));
                    help(ui, p, "macOS blocks some folders unless DiskLens has Full Disk Access.\nGrant it in System Settings › Privacy & Security › Full Disk Access, then scan again.");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                        let c = if resp.hovered() { p.text } else { p.warn };
                        let k = r.shrink(5.0);
                        ui.painter().line_segment([k.left_top(), k.right_bottom()], Stroke::new(1.4, c));
                        ui.painter().line_segment([k.right_top(), k.left_bottom()], Stroke::new(1.4, c));
                        if resp.on_hover_text("Hide until next launch").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.fda_hidden = true;
                        }
                    });
                });
            });
        }
        if let Some(g) = open {
            self.group = Some(g);
            self.filter = None;
            self.selected = None;
        }
    }

    fn group_header(&self, ui: &mut egui::Ui, p: &Pal, latest: &Snapshot, g: Group) {
        let mut by = [0u64; 4];
        let mut safe = 0;
        for r in &latest.targets {
            if let Some(t) = targets::find(&r.id).filter(|t| t.group == g && !t.nested) {
                by[Safety::ALL.iter().position(|s| *s == t.safety).unwrap_or(0)] += r.bytes;
                safe += reclaimable(t, r);
            }
        }
        let total: u64 = by.iter().sum();
        card(ui, p, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        caption(ui, p, "Total in this category");
                        help(ui, p, g.blurb());
                    });
                    ui.label(RichText::new(fmt_bytes(total)).size(24.0).strong());
                });
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    caption(ui, p, "Safe to reclaim");
                    ui.label(RichText::new(fmt_bytes(safe)).size(24.0).strong().color(p.good));
                });
            });
            ui.add_space(10.0);
            stacked(ui, p, &Safety::ALL.iter().zip(by).map(|(s, b)| (theme::badge_colors(*s, p).0, b, s.label())).collect::<Vec<_>>());
        });
    }

    fn table(&mut self, ui: &mut egui::Ui, p: &Pal, latest: &Snapshot) {
        // Filter chips. What each status means lives on hover and behind the "?".
        ui.horizontal(|ui| {
            let chip = |ui: &mut egui::Ui, on: bool, text: String, color: Option<Color32>| -> egui::Response {
                let (fg, bg) = if on { (Color32::WHITE, p.accent) } else { (p.text, p.hover) };
                let r = ui.add(egui::Button::new(RichText::new(text).size(12.5).color(fg)).fill(bg).stroke(Stroke::NONE).corner_radius(14.0).min_size(Vec2::new(0.0, 24.0)));
                if let (Some(c), false) = (color, on) {
                    let pos = r.rect.left_center() + Vec2::new(8.0, 0.0);
                    ui.painter().circle_filled(pos, 3.0, c);
                }
                r
            };
            let in_group = |id: &str| targets::find(id).is_some_and(|t| self.group.is_none_or(|g| g == t.group));
            let all = latest.targets.iter().filter(|r| in_group(&r.id)).count();
            if chip(ui, self.filter.is_none(), format!("All  {all}"), None).clicked() {
                self.filter = None;
            }
            for s in Safety::ALL {
                let n = latest.targets.iter().filter(|r| in_group(&r.id) && targets::find(&r.id).is_some_and(|t| t.safety == s)).count();
                if n > 0 {
                    let dotc = theme::badge_colors(s, p).0;
                    if chip(ui, self.filter == Some(s), format!("   {}  {n}", s.label()), Some(dotc)).on_hover_text(s.explain()).clicked() {
                        self.filter = Some(s);
                    }
                }
            }
            ui.add_space(2.0);
            let legend: Vec<String> = Safety::ALL.iter().map(|s| format!("{}: {}", s.label(), s.explain())).collect();
            help(ui, p, &format!("What the statuses mean\n\n{}", legend.join("\n\n")));
        });
        ui.add_space(10.0);

        let rows = self.rows(latest);
        let max = rows.iter().map(|r| r.bytes).max().unwrap_or(1).max(1);
        let used = latest.disk.used().max(1);
        let mut clicked: Option<&'static str> = None;
        let mut new_sort = self.sort;
        let selected = self.selected;
        let sort = self.sort;

        // In the all-locations view, each category gets its own card (each keeps the chosen sort).
        let mut sections: Vec<(Option<(Group, u64, u64)>, Vec<usize>)> = vec![];
        if self.group.is_none() && sort.0 != Col::Name {
            for g in Group::ALL {
                let idx: Vec<usize> = (0..rows.len()).filter(|i| rows[*i].t.group == g).collect();
                if idx.is_empty() {
                    continue;
                }
                let solid = idx.iter().filter(|i| !rows[**i].t.nested);
                let (tot, rec) = solid.fold((0, 0), |(a, b), i| (a + rows[*i].bytes, b + rows[*i].reclaim));
                sections.push((Some((g, tot, rec)), idx));
            }
        } else {
            sections.push((None, (0..rows.len()).collect()));
        }
        let mut open_group: Option<Group> = None;

        let sort_hdr = |ui: &mut egui::Ui, label: &str, col: Col, right: bool, pad: f32, new_sort: &mut (Col, bool)| {
            let active = sort.0 == col;
            let arrow = if active { if sort.1 { " ↓" } else { " ↑" } } else { "" };
            let text = RichText::new(format!("{label}{arrow}")).size(11.0).color(if active { p.text } else { p.muted });
            let go = |ui: &mut egui::Ui| ui.add(egui::Label::new(text).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            let hit = if right {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(pad);
                    go(ui)
                })
                .inner
            } else {
                go(ui)
            };
            if hit {
                *new_sort = if active { (col, !sort.1) } else { (col, true) };
            }
        };
        let head = |ui: &mut egui::Ui, text: &str, right: bool, tip: &str| {
            let l = RichText::new(text).size(11.0).color(p.muted);
            if right {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(l).on_hover_text(tip));
            } else {
                ui.label(l).on_hover_text(tip);
            }
        };
        let quiet_delta = |ui: &mut egui::Ui, d: Option<i64>| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                match d.map(|d| fmt_delta(d, p)) {
                    Some((s, c)) if s != "~0" => ui.label(RichText::new(s).monospace().size(12.0).color(c)),
                    _ => ui.label(RichText::new("—").color(p.muted.gamma_multiply(0.6))),
                }
            });
        };
        // Cards and the header share one frame shape so the columns line up.
        let frame = |fill: Color32, stroke: Color32| egui::Frame::new().fill(fill).stroke(Stroke::new(1.0, stroke)).corner_radius(12.0).inner_margin(egui::Margin::symmetric(4, 2));
        let layout = |avail: f32| -> Layout3 {
            let wide = avail > 880.0; // room for the share bar
            let rest = (avail - FIXED_COLS).max(220.0);
            let share_w = if wide { (rest * 0.25).clamp(90.0, 200.0) } else { 0.0 };
            Layout3 { wide, name_w: rest - share_w, share_w }
        };

        // Column headers, once for all categories.
        frame(Color32::TRANSPARENT, Color32::TRANSPARENT).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let l = layout(ui.available_width() - 2.0);
            ui.spacing_mut().item_spacing.x = 0.0;
            columns(TableBuilder::new(ui).id_salt("tracked-head"), &l).header(22.0, |mut h| {
                h.col(|ui| {
                    ui.add_space(10.0);
                    sort_hdr(ui, "LOCATION", Col::Name, false, 0.0, &mut new_sort)
                });
                h.col(|ui| head(ui, "STATUS", false, "Can you clear it? Hover a status for what it means."));
                h.col(|ui| sort_hdr(ui, "SIZE", Col::Size, true, 0.0, &mut new_sort));
                h.col(|ui| head(ui, "RECLAIMABLE", true, "Space you can get back without losing anything: rebuilds itself, or left over from an uninstalled app."));
                if l.wide {
                    h.col(|ui| {
                        ui.add_space(18.0);
                        head(ui, "SHARE", false, "Size compared with the largest row.");
                    });
                }
                h.col(|ui| sort_hdr(ui, "7 DAYS", Col::Growth, true, 12.0, &mut new_sort));
            })
            .body(|_| {});
        });

        for (si, (head_info, idx)) in sections.iter().enumerate() {
        if let Some((g, tot, rec)) = head_info {
            ui.add_space(if si == 0 { 2.0 } else { 18.0 });
            ui.horizontal(|ui| {
                let (bar_r, _) = ui.allocate_exact_size(Vec2::new(4.0, 18.0), Sense::hover());
                ui.painter().rect_filled(bar_r, 2.0, group_color(*g));
                ui.add_space(4.0);
                ui.label(RichText::new(g.label()).size(15.0).strong());
                help(ui, p, g.blurb());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Open ›").on_hover_text("Show only this category").clicked() {
                        open_group = Some(*g);
                    }
                    ui.add_space(10.0);
                    if *rec > 0 {
                        ui.label(RichText::new(format!("{} safe to clear", fmt_bytes(*rec))).size(12.0).color(p.good));
                        ui.label(RichText::new("·").color(p.muted));
                    }
                    ui.label(RichText::new(fmt_bytes(*tot)).size(13.5).strong());
                });
            });
            ui.add_space(6.0);
        }
        frame(p.card, p.border).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let l = layout(ui.available_width() - 2.0);
            ui.spacing_mut().item_spacing.x = 0.0;
            columns(TableBuilder::new(ui).id_salt(("tracked", si)).sense(Sense::click()), &l).body(|mut body| {
                    for i in idx {
                            body.row(40.0, |mut row| {
                                let r = &rows[*i];
                                row.set_selected(selected == Some(r.t.id));
                                row.col(|ui| {
                                    ui.add_space(10.0);
                                    ui.add(egui::Label::new(RichText::new(r.t.name).size(13.5)).truncate());
                                    if r.leftovers > 0 {
                                        ui.add_space(8.0);
                                        ui.label(RichText::new(format!("{} leftover", r.leftovers)).size(11.0).color(LEFTOVER))
                                            .on_hover_text(format!("{} folders here belong to apps that are no longer installed.", r.leftovers));
                                    }
                                });
                                row.col(|ui| safety_pill(ui, r.t.safety, p));
                                row.col(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let txt = match (&r.note, r.bytes) {
                                            (Some(n), 0) => n.split_whitespace().next().map(|c| format!("{c} snap.")).unwrap_or_default(),
                                            _ => fmt_bytes(r.bytes),
                                        };
                                        ui.label(RichText::new(txt).monospace().size(13.0));
                                    });
                                });
                                row.col(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if r.reclaim > 0 {
                                            ui.label(RichText::new(fmt_bytes(r.reclaim)).monospace().size(13.0).color(p.good));
                                        } else {
                                            ui.label(RichText::new("—").color(p.muted.gamma_multiply(0.6)));
                                        }
                                    });
                                });
                                if l.wide { row.col(|ui| {
                                    ui.add_space(18.0);
                                    bar(ui, p, r.bytes as f32 / max as f32, if r.t.nested { p.muted } else { p.accent.gamma_multiply(0.8) });
                                }); }
                                row.col(|ui| quiet_delta(ui, r.d7));
                                let resp = row.response();
                                if resp.clicked() {
                                    clicked = Some(r.t.id);
                                }
                                let pct = r.bytes as f64 * 100.0 / used as f64;
                                let share = if pct >= 1.0 { format!("{pct:.0}% of used space") } else { "under 1% of used space".into() };
                                let mut tip = format!("{}\n{}", targets::blurb(r.t.id), share);
                                if r.t.nested {
                                    tip += "\nAlready counted inside another row, so not added to totals.";
                                }
                                if !r.sub.is_empty() {
                                    tip += &format!("\n\n{}", r.sub);
                                }
                                resp.on_hover_text(tip + "\n\nClick for details and how to clear it.");
                            });
                    }
                });
        });
        }
        if let Some(g) = open_group {
            self.group = Some(g);
            self.filter = None;
            self.selected = None;
        }
        self.sort = new_sort;
        if let Some(id) = clicked {
            self.selected = if self.selected == Some(id) { None } else { Some(id) };
            self.show_all_items = false;
            self.open_item = None;
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("DiskLens never deletes anything.").size(11.5).color(p.muted));
            help(ui, p, "Sizes are allocated blocks, like `du`.\nGrey bars overlap other rows (e.g. Application Support contains the app rows) and aren't counted in totals.\nReclaimable = rebuilds itself, or left over from an uninstalled app.");
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui, p: &Pal, latest: &Snapshot) {
        let Some(t) = self.selected.and_then(targets::find) else {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No selection").size(15.0).strong());
                ui.add_space(2.0);
                ui.label(RichText::new("Pick a location to see what it is\nand how to clear it.").color(p.muted));
            });
            return;
        };
        let res = latest.targets.iter().find(|r| r.id == t.id);
        let series = self.series_ts(t.id);
        let (d1, d7) = self.deltas(t.id);
        let has_items = res.is_some_and(|r| r.items.iter().any(|it| it.bytes > 0 || !t.breakdown));
        if self.tab == Tab::Inside && !has_items {
            self.tab = Tab::About;
        }
        let mut explore_to: Option<PathBuf> = None;

        // Fixed header: what it is, how big, can I clear it.
        ui.add_space(14.0);
        ui.add(egui::Label::new(RichText::new(t.name).size(17.0).strong()).truncate());
        ui.add_space(2.0);
        safety_pill(ui, t.safety, p);
        ui.horizontal(|ui| {
            ui.label(RichText::new(fmt_bytes(res.map_or(0, |r| r.bytes))).size(28.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if let Some(d) = d7 {
                    let (s, c) = fmt_delta(d, p);
                    ui.label(RichText::new("7 days").size(11.5).color(p.muted));
                    ui.label(RichText::new(s).size(12.5).color(c)).on_hover_text(match d1 {
                        Some(d1) => format!("{} since the last scan", fmt_delta(d1, p).0),
                        None => String::new(),
                    });
                }
            });
        });
        if res.is_some_and(|r| r.denied) {
            ui.label(RichText::new("Real size is larger").size(12.0).color(p.warn)).on_hover_text("Part of this folder couldn't be read without Full Disk Access.");
        }
        ui.add_space(8.0);
        let mut tabs = vec![(Tab::About, "Overview")];
        if has_items {
            tabs.push((Tab::Inside, if t.breakdown { "Inside" } else { "Largest" }));
        }
        tabs.push((Tab::Clear, "How to clear"));
        segmented(ui, p, &tabs, &mut self.tab);
        ui.add_space(10.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            match self.tab {
                Tab::About => {
                    chart(ui, p, &series);
                    ui.add_space(12.0);
                    ui.label(RichText::new(t.desc).size(13.0).color(p.text));
                    let found: Vec<PathBuf> = t.paths.iter().map(|x| targets::expand(x)).filter(|x| x.exists()).collect();
                    if !found.is_empty() {
                        ui.add_space(14.0);
                        caption(ui, p, "Location");
                        for full in &found {
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.small_button("Reveal").on_hover_text("Show in Finder").clicked() {
                                        reveal(full);
                                    }
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add(egui::Label::new(RichText::new(tilde(&full.to_string_lossy())).monospace().size(11.5).color(p.muted)).truncate());
                                    });
                                });
                            });
                        }
                    }
                    if let Some(first) = t.paths.first() {
                        ui.add_space(10.0);
                        if ui.button("Explore folder").clicked() {
                            explore_to = Some(targets::expand(first));
                        }
                    }
                }
                Tab::Inside => {
                    if let Some(r) = res {
                        if t.breakdown {
                            self.breakdown(ui, p, t, r);
                        } else {
                            for it in r.items.iter().take(12) {
                                ui.horizontal(|ui| {
                                    if it.bytes > 0 {
                                        ui.label(RichText::new(fmt_bytes(it.bytes)).monospace().size(11.5));
                                        if ui.add(egui::Label::new(RichText::new(tilde(&it.path)).size(11.5).color(p.accent)).truncate().sense(Sense::click())).on_hover_text("Reveal in Finder").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                            reveal(Path::new(&it.path));
                                        }
                                    } else {
                                        ui.label(RichText::new(&it.path).monospace().size(11.0).color(p.muted));
                                    }
                                });
                            }
                        }
                    }
                }
                Tab::Clear => {
                    let (fg, bg) = theme::badge_colors(t.safety, p);
                    egui::Frame::new().fill(bg).corner_radius(8.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(t.safety.label()).size(12.5).strong().color(fg));
                        ui.label(RichText::new(t.safety.explain()).size(12.5));
                    });
                    if let Some(cmd) = t.cmd {
                        ui.add_space(14.0);
                        caption(ui, p, "Command");
                        egui::Frame::new().fill(p.window).stroke(Stroke::new(1.0, p.border)).corner_radius(8.0).inner_margin(10).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(cmd).monospace().size(11.5));
                        });
                        ui.add_space(2.0);
                        if ui.small_button("Copy command").clicked() {
                            ui.ctx().copy_text(cmd.to_string());
                        }
                    }
                    if t.breakdown && has_items {
                        ui.add_space(10.0);
                        ui.label(RichText::new("Each folder inside has its own advice. See the Inside tab.").size(12.0).color(p.muted));
                    }
                    ui.add_space(14.0);
                    ui.label(RichText::new("DiskLens never deletes anything. You stay in control.").size(11.5).color(p.muted));
                }
            }
            ui.add_space(14.0);
        });
        if let Some(path) = explore_to {
            let ctx = ui.ctx().clone();
            self.page = Page::Explorer;
            self.start_explore(&ctx, path);
        }
    }

    // ---------- explorer ----------

    /// "What's inside": each subfolder as a compact row; click one for its owner, advice and command.
    fn breakdown(&mut self, ui: &mut egui::Ui, p: &Pal, t: &Target, r: &scan::TargetResult) {
        let rows: Vec<(&scan::Item, classify::Info)> = r
            .items
            .iter()
            .filter(|it| it.bytes > 0)
            .map(|it| (it, classify::classify(t.id, Path::new(&it.path).file_name().map(|n| n.to_string_lossy()).as_deref().unwrap_or(""))))
            .collect();
        let mut by = [0u64; 4];
        let mut leftover = 0u64;
        for (it, info) in &rows {
            if info.leftover {
                leftover += it.bytes;
            } else {
                by[Safety::ALL.iter().position(|s| *s == info.safety).unwrap_or(0)] += it.bytes;
            }
        }
        let mut parts: Vec<(Color32, u64, &str)> = Safety::ALL.iter().zip(by).map(|(s, b)| (theme::badge_colors(*s, p).0, b, s.label())).collect();
        parts.insert(1, (LEFTOVER, leftover, "Leftover"));
        stacked(ui, p, &parts);
        ui.add_space(8.0);
        let limit = if self.show_all_items { rows.len() } else { 15 };
        for (it, info) in rows.iter().take(limit) {
            let name = Path::new(&it.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let open = self.open_item.as_deref() == Some(it.path.as_str());
            let color = if info.leftover { LEFTOVER } else { theme::badge_colors(info.safety, p).0 };
            let status = if info.leftover { "Leftover" } else { info.safety.label() };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
            if open || resp.hovered() {
                ui.painter().rect_filled(rect, 6.0, p.hover);
            }
            ui.painter().circle_filled(rect.left_center() + Vec2::new(9.0, 0.0), 4.0, color);
            let size = ui.painter().layout_no_wrap(fmt_bytes(it.bytes), egui::FontId::monospace(11.5), p.text);
            let size_w = size.size().x;
            ui.painter().galley(rect.right_center() - Vec2::new(8.0 + size_w, size.size().y / 2.0), size, p.text);
            let mut job = egui::text::LayoutJob::single_section(name.clone(), egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: p.text, ..Default::default() });
            job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 22.0 - size_w - 16.0);
            let g = ui.painter().layout_job(job);
            ui.painter().galley(rect.left_center() + Vec2::new(20.0, -g.size().y / 2.0), g, p.text);
            let app = if info.app.is_empty() { String::new() } else { format!(" · {}", info.app) };
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("{name}\n{status}{app}\n{}\n\nClick for details", info.note));
            if resp.clicked() {
                self.open_item = if open { None } else { Some(it.path.clone()) };
            }
            if open {
                egui::Frame::new().fill(p.window).corner_radius(8.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal(|ui| {
                        if info.leftover {
                            pill(ui, "Leftover", LEFTOVER, LEFTOVER.gamma_multiply(0.2)).on_hover_text("Belongs to an app that is no longer installed.");
                        } else {
                            safety_pill(ui, info.safety, p);
                        }
                        if !info.app.is_empty() {
                            ui.label(RichText::new(&info.app).size(11.5).color(p.muted));
                        }
                    });
                    ui.label(RichText::new(info.note).size(12.0).color(p.muted));
                    if let Some(cmd) = info.cmd {
                        ui.label(RichText::new(cmd).monospace().size(11.0));
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("Reveal").on_hover_text("Show in Finder").clicked() {
                            reveal(Path::new(&it.path));
                        }
                        if let Some(cmd) = info.cmd {
                            if ui.small_button("Copy command").clicked() {
                                ui.ctx().copy_text(cmd.to_string());
                            }
                        }
                    });
                });
                ui.add_space(4.0);
            }
        }
        if rows.len() > 15 {
            ui.add_space(4.0);
            let label = if self.show_all_items { "Show fewer".to_string() } else { format!("Show all {} folders", rows.len()) };
            if ui.small_button(label).clicked() {
                self.show_all_items = !self.show_all_items;
            }
        }
    }

    fn explorer(&mut self, ui: &mut egui::Ui, p: &Pal) {
        if !self.explore_started {
            let path = self.explore_path.clone();
            let ctx = ui.ctx().clone();
            self.start_explore(&ctx, path);
        }
        let path = self.explore_path.clone();
        let mut go: Option<PathBuf> = None;
        // Breadcrumbs
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut acc = PathBuf::new();
            let comps: Vec<_> = path.components().collect();
            for (i, c) in comps.iter().enumerate() {
                acc.push(c);
                let label = match c {
                    std::path::Component::RootDir => "Macintosh HD".to_string(),
                    other => other.as_os_str().to_string_lossy().into_owned(),
                };
                let last = i + 1 == comps.len();
                let r = ui.add(egui::Button::new(RichText::new(label).size(13.0).color(if last { p.text } else { p.accent })).frame(false));
                if r.clicked() && !last {
                    go = Some(acc.clone());
                }
                if !last {
                    ui.label(RichText::new("›").color(p.muted));
                }
            }
            ui.add_space(8.0);
            if ui.small_button("Reveal in Finder").clicked() {
                reveal(&path);
            }
        });
        ui.add_space(6.0);
        let state = self.explore.lock().unwrap();
        match &*state {
            ExploreState::Idle | ExploreState::Loading => {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Measuring… large folders can take a moment").color(p.muted));
                });
            }
            ExploreState::Done(Err(e)) => {
                ui.label(RichText::new(e).color(p.warn));
            }
            ExploreState::Done(Ok(items)) => {
                let total: u64 = items.iter().map(|i| i.bytes).sum();
                let max = items.first().map_or(1, |i| i.bytes).max(1);
                ui.label(RichText::new(format!("{} in {} items", fmt_bytes(total), items.len())).color(p.muted));
                ui.add_space(4.0);
                egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(12.0).inner_margin(egui::Margin::symmetric(4, 2)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.x = 0.0;
                    TableBuilder::new(ui)
                        .id_salt("explorer")
                        .resizable(false)
                        .vscroll(false)
                        .sense(Sense::click())
                        .cell_layout(Layout::left_to_right(Align::Center))
                        .column(Column::remainder().at_least(200.0).clip(true))
                        .column(Column::exact(90.0))
                        .column(Column::exact(180.0))
                        .column(Column::exact(70.0))
                        .header(28.0, |mut h| {
                            for (i, t) in ["NAME", "SIZE", "SHARE", ""].iter().enumerate() {
                                h.col(|ui| {
                                    if i == 1 {
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(*t).size(11.5).color(p.muted)));
                                    } else {
                                        ui.add_space(if i == 0 { 8.0 } else { 14.0 });
                                        ui.label(RichText::new(*t).size(11.5).color(p.muted));
                                    }
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(32.0, items.len().min(400), |mut row| {
                                let it = &items[row.index()];
                                row.col(|ui| {
                                    ui.add_space(8.0);
                                    let (r, _) = ui.allocate_exact_size(Vec2::new(16.0, 12.0), Sense::hover());
                                    let c = if it.is_dir { p.accent } else { p.muted };
                                    ui.painter().rect_filled(r, 3.0, c.gamma_multiply(if it.is_dir { 0.9 } else { 0.5 }));
                                    ui.add_space(8.0);
                                    ui.label(RichText::new(&it.name).size(13.5));
                                    if it.denied {
                                        ui.add_space(8.0);
                                        ui.label(RichText::new("partly unreadable").size(11.0).color(p.warn));
                                    }
                                });
                                row.col(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(fmt_bytes(it.bytes)).monospace().size(13.0)));
                                });
                                row.col(|ui| {
                                    ui.add_space(14.0);
                                    bar(ui, p, it.bytes as f32 / max as f32, p.accent);
                                });
                                row.col(|ui| {
                                    if ui.small_button("Reveal").clicked() {
                                        reveal(&it.path);
                                    }
                                });
                                if it.is_dir && row.response().clicked() {
                                    go = Some(it.path.clone());
                                }
                            });
                        });
                });
            }
        }
        drop(state);
        if let Some(pth) = go {
            let ctx = ui.ctx().clone();
            self.start_explore(&ctx, pth);
        }
    }

    // ---------- screenshot mode (used for visual checks) ----------

    fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some(path) = self.args.screenshot.clone() else { return };
        self.frames += 1;
        let ready = self.scanning.is_none() && !self.history.is_empty() && (self.page != Page::Explorer || matches!(&*self.explore.lock().unwrap(), ExploreState::Done(_))) && (self.page != Page::Ai || self.ai_ready());
        if ready && self.frames > 8 && !self.shot_requested {
            self.shot_requested = true;
            let c2 = ctx.clone();
            ctx.request_screenshot(move |img| {
                let _ = image::save_buffer(&path, img.as_raw(), img.width() as u32, img.height() as u32, image::ColorType::Rgba8);
                c2.send_viewport_cmd(egui::ViewportCommand::Close);
            });
        }
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();
        self.screenshot_tick(&ctx);
        // Closing the window keeps the app alive in the menu bar.
        if self.tray_on && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if self.scanning.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        let p = theme::pal(ctx.global_style().visuals.dark_mode);
        let latest = self.history.last().cloned();
        let live = scan::disk_info();
        let disk = if live.total > 0 { live } else { latest.as_ref().map(|l| l.disk.clone()).unwrap_or_default() };

        egui::Panel::left("nav").exact_size(206.0).resizable(false).frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, p.border))).show(ui, |ui| {
            self.sidebar(ui, &p, &disk, latest.as_ref());
        });
        egui::Panel::bottom("status").exact_size(26.0).resizable(false).frame(egui::Frame::new().fill(p.sidebar).stroke(Stroke::new(1.0, p.border))).show(ui, |ui| {
            self.statusbar(ui, &p);
        });
        if self.page == Page::Overview {
            egui::Panel::right("inspector").exact_size(330.0).resizable(false).frame(egui::Frame::new().fill(p.card).inner_margin(egui::Margin::symmetric(16, 0)).stroke(Stroke::new(1.0, p.border))).show(ui, |ui| {
                match &latest {
                    Some(l) => self.inspector(ui, &p, l),
                    None => {}
                }
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.window).inner_margin(egui::Margin::symmetric(20, 16))).show(ui, |ui| {
            let title = match (self.page, self.group) {
                (Page::Explorer, _) => "Explorer",
                (Page::History, _) => "History",
                (Page::Ai, _) => "AI Storage",
                (Page::Overview, Some(g)) => g.label(),
                (Page::Overview, None) => "Overview",
            };
            self.toolbar(ui, &p, title);
            ui.add_space(8.0);
            match (self.page, &latest) {
                (Page::Overview, Some(l)) => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        match self.group {
                            None => self.summary(ui, &p, l, &disk),
                            Some(g) => self.group_header(ui, &p, l, g),
                        }
                        ui.add_space(10.0);
                        self.table(ui, &p, l);
                    });
                }
                (Page::Overview, None) => {
                    ui.add_space(80.0);
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Running your first scan… this takes a few seconds").color(p.muted));
                    });
                }
                (Page::Explorer, _) => self.explorer(ui, &p),
                (Page::History, _) => self.history_page(ui, &p),
                (Page::Ai, _) => self.ai_page(ui, &p, &disk),
            }
        });
    }
}
