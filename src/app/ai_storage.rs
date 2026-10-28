//! AI Storage page: what local models, assistants and ML caches take, and what contributes to it.

use super::*;
use crate::ai::{self, AiGroup, AiItem, AiScan, AiTool};

pub(super) enum AiState {
    Idle,
    Scanning,
    Done(AiScan),
}

fn group_color(g: AiGroup) -> Color32 {
    match g {
        AiGroup::Models => Color32::from_rgb(94, 92, 230),
        AiGroup::Media => Color32::from_rgb(255, 55, 95),
        AiGroup::Assistants => Color32::from_rgb(48, 176, 199),
        AiGroup::Frameworks => Color32::from_rgb(255, 149, 0),
    }
}

fn last_changed(modified: u64, now: u64) -> String {
    if modified == 0 { "unknown".into() } else { ago(now.saturating_sub(modified)) }
}

fn group_desc(g: AiGroup) -> &'static str {
    match g {
        AiGroup::Models => "Weights pulled by local model runners",
        AiGroup::Media => "Image and speech models, and what they generated",
        AiGroup::Assistants => "Transcripts, indexes and caches of AI coding tools and chat apps",
        AiGroup::Frameworks => "Pretrained weights and datasets cached by ML libraries",
    }
}

/// Below this width, side-by-side cards stack.
const WIDE: f32 = 820.0;
const GAP: f32 = 14.0;

/// One finding in the Insights card: a tinted glyph, a short title and a sentence of detail.
struct Insight {
    color: Color32,
    glyph: &'static str,
    title: String,
    detail: String,
}

struct BarItem<'a> {
    id: Option<&'static str>,
    name: &'a str,
    color: Color32,
    bytes: u64,
    count: usize,
}

/// Card heading: title, one line of description, optional figure on the right.
fn card_title(ui: &mut egui::Ui, p: &Pal, dot: Option<Color32>, title: &str, desc: &str, right: Option<String>) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                if let Some(c) = dot {
                    ui.label(RichText::new("●").size(10.0).color(c));
                }
                ui.label(RichText::new(title).size(15.0).strong().color(p.text));
            });
            ui.label(RichText::new(desc).size(12.0).color(p.muted));
        });
        if let Some(r) = right {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.label(RichText::new(r).monospace().size(13.0).color(p.text));
            });
        }
    });
    ui.add_space(10.0);
}

/// Big number with a coloured accent along the top edge.
fn stat_card(ui: &mut egui::Ui, p: &Pal, accent: Color32, value: String, label: &str, sub: String) {
    let r = egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(12.0)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(value).size(26.0).strong().color(p.text));
            caption(ui, p, label);
            ui.add(egui::Label::new(RichText::new(sub).size(11.0).color(p.muted)).truncate());
        })
        .response
        .rect;
    ui.painter().with_clip_rect(egui::Rect::from_min_max(r.min, Pos2::new(r.max.x, r.min.y + 3.0))).rect_filled(r, 12.0, accent);
}

/// Lay `n` equal-width cells out in rows of `cols`.
fn grid(ui: &mut egui::Ui, n: usize, cols: usize, mut cell: impl FnMut(&mut egui::Ui, usize)) {
    let w = (ui.available_width() - GAP * (cols - 1) as f32) / cols as f32;
    for row in (0..n).collect::<Vec<_>>().chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for &i in row {
                ui.allocate_ui_with_layout(Vec2::new(w, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(w);
                    cell(ui, i);
                });
            }
        });
        ui.add_space(GAP - ui.spacing().item_spacing.y);
    }
}

/// Two cards side by side at the same height (`frac` of the width to the left), stacked when narrow.
fn pair(ui: &mut egui::Ui, p: &Pal, frac: f32, left: impl FnOnce(&mut egui::Ui), right: impl FnOnce(&mut egui::Ui)) {
    if ui.available_width() < WIDE {
        card(ui, p, left);
        ui.add_space(GAP);
        card(ui, p, right);
        return;
    }
    // Heights are matched to the taller card of the previous frame.
    let id = ui.make_persistent_id("ai-pair");
    let min_h: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    let lw = (ui.available_width() - GAP) * frac;
    let rw = ui.available_width() - GAP - lw;
    let mut tallest = 0.0f32;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        for (w, add) in [(lw, Box::new(left) as Box<dyn FnOnce(&mut egui::Ui)>), (rw, Box::new(right))] {
            ui.allocate_ui_with_layout(Vec2::new(w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(w);
                let h = card(ui, p, |ui| {
                    add(ui);
                    let h = ui.min_rect().height();
                    ui.add_space((min_h - h).max(0.0));
                    h
                });
                tallest = tallest.max(h);
            });
        }
    });
    if (tallest - min_h).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, tallest));
        ui.ctx().request_repaint();
    }
}

/// Ranked bars sized against the largest item, labelled with size, share and item count. Returns the clicked id.
fn bar_list(ui: &mut egui::Ui, p: &Pal, items: &[BarItem], total: u64) -> Option<&'static str> {
    let max = items.iter().map(|i| i.bytes).max().unwrap_or(1).max(1);
    let mut picked = None;
    ui.spacing_mut().item_spacing.y = 4.0;
    for it in items {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), if it.id.is_some() { Sense::click() } else { Sense::hover() });
        if it.id.is_some() && resp.hovered() {
            ui.painter().rect_filled(rect.expand2(Vec2::new(6.0, 0.0)), 6.0, p.hover);
        }
        let text_y = rect.top() + 11.0;
        let mut job = egui::text::LayoutJob::default();
        let fmt = |c: Color32, mono: bool| egui::TextFormat { font_id: if mono { egui::FontId::monospace(12.0) } else { egui::FontId::proportional(11.5) }, color: c, ..Default::default() };
        job.append(&fmt_bytes(it.bytes), 0.0, fmt(p.text, true));
        let noun = if it.count == 1 { "item" } else { "items" };
        job.append(&format!("  ·  {:.0}%  ·  {} {noun}", it.bytes as f64 * 100.0 / total.max(1) as f64, it.count), 0.0, fmt(p.muted, false));
        let right = ui.painter().layout_job(job);
        let right_w = right.size().x;
        ui.painter().galley(Pos2::new(rect.right() - right_w, text_y - right.size().y / 2.0), right, p.text);
        ui.painter().circle_filled(Pos2::new(rect.left() + 4.0, text_y), 4.0, it.color);
        let mut job = egui::text::LayoutJob::single_section(it.name.to_string(), egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: p.text, ..Default::default() });
        job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - right_w - 30.0).max(40.0));
        let name = ui.painter().layout_job(job);
        ui.painter().galley(Pos2::new(rect.left() + 14.0, text_y - name.size().y / 2.0), name, p.text);
        let track = egui::Rect::from_min_size(Pos2::new(rect.left(), rect.bottom() - 9.0), Vec2::new(rect.width(), 6.0));
        ui.painter().rect_filled(track, 3.0, p.track);
        let mut fill = track;
        fill.set_width((track.width() * it.bytes as f32 / max as f32).max(3.0));
        ui.painter().rect_filled(fill, 3.0, it.color);
        if let Some(id) = it.id {
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Show details below").clicked() {
                picked = Some(id);
            }
        }
    }
    picked
}

fn insight_row(ui: &mut egui::Ui, p: &Pal, ins: &Insight) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 9.0, ins.color.gamma_multiply(if p.dark { 0.22 } else { 0.15 }));
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, ins.glyph, egui::FontId::proportional(11.5), ins.color);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(egui::Label::new(RichText::new(&ins.title).size(13.0).strong().color(p.text)).wrap());
            ui.add(egui::Label::new(RichText::new(&ins.detail).size(12.0).color(p.muted)).wrap());
        });
    });
}

fn group_tools(s: &AiScan, g: AiGroup) -> Vec<&AiTool> {
    s.tools.iter().filter(|t| ai::find(t.id).is_some_and(|d| d.group == g)).collect()
}

/// Items of 50 MB or more untouched for `ai::STALE_SECS`, largest first, labelled `Tool › item`.
fn stale_items(s: &AiScan) -> Vec<(String, &AiItem)> {
    let mut stale: Vec<(String, &AiItem)> = s
        .tools
        .iter()
        .flat_map(|t| t.items.iter().map(move |i| (ai::find(t.id).map_or(i.name.clone(), |d| format!("{} › {}", d.name, i.name)), i)))
        .chain(s.stray.iter().map(|i| (i.name.clone(), i)))
        .filter(|(_, i)| i.stale(s.ts) && i.bytes >= 50_000_000)
        .collect();
    stale.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes));
    stale
}

fn safety_bytes(s: &AiScan, sf: Safety) -> u64 {
    s.tools.iter().flat_map(|t| t.items.iter()).chain(s.stray.iter()).filter(|i| i.safety == sf).map(|i| i.bytes).sum()
}

fn insights(p: &Pal, s: &AiScan, total: u64) -> Vec<Insight> {
    let mut out = Vec::new();
    let mut add = |color, glyph, title: String, detail: String| out.push(Insight { color, glyph, title, detail });
    if let Some((t, d)) = s.tools.first().and_then(|t| ai::find(t.id).map(|d| (t, d))) {
        let share = t.bytes as f64 * 100.0 / total.max(1) as f64;
        // Folder names like "User" only make sense for tools that hold named models.
        let lead = match t.items.first() {
            Some(i) if d.group != AiGroup::Assistants && t.items.len() > 1 && i.bytes * 2 > t.bytes => format!(", mostly {} ({})", i.name, fmt_bytes(i.bytes)),
            _ => String::new(),
        };
        add(p.accent, "i", format!("{} is the biggest", d.name), format!("{}, {share:.0}% of your AI storage{lead}.", fmt_bytes(t.bytes)));
    }
    let (safe, review, careful) = (safety_bytes(s, Safety::Safe), safety_bytes(s, Safety::Review), safety_bytes(s, Safety::Careful));
    if safe > 0 {
        add(p.good, "✓", format!("{} is safe to clear", fmt_bytes(safe)), "Cache that rebuilds itself. Clearing it loses nothing.".into());
    }
    if review > 0 {
        add(p.warn, "!", format!("{} needs a check first", fmt_bytes(review)), "Models and app data you can fetch or recreate, but slowly. Check each one before removing it.".into());
    }
    if careful > 0 {
        add(p.bad, "!", format!("{} is your own data", fmt_bytes(careful)), "History and files such as chat transcripts. Only you can decide on these.".into());
    }
    let stale = stale_items(s);
    if !stale.is_empty() {
        let sum: u64 = stale.iter().map(|(_, i)| i.bytes).sum();
        let names: Vec<String> = stale.iter().take(3).map(|(n, i)| format!("{n} ({})", fmt_bytes(i.bytes))).collect();
        add(LEFTOVER, "z", format!("{} untouched for {}+ days", fmt_bytes(sum), ai::STALE_SECS / 86_400), format!("Including {}. Worth a look if you need space.", names.join(", ")));
    }
    if !s.stray.is_empty() {
        add(p.warn, "!", format!("{} loose model file(s)", s.stray.len()), format!("{} outside the usual AI folders. See Loose model files below.", fmt_bytes(s.stray.iter().map(|i| i.bytes).sum())));
    }
    if s.tools.iter().any(|t| t.denied) {
        add(p.muted, "i", "Some folders couldn't be read".into(), "Their sizes are undercounted. Grant Full Disk Access to see everything.".into());
    }
    out
}

impl App {
    pub(super) fn start_ai_scan(&mut self, ctx: &egui::Context) {
        {
            let mut st = self.ai.lock().unwrap();
            if matches!(*st, AiState::Scanning) {
                return;
            }
            *st = AiState::Scanning;
        }
        let out = self.ai.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = ai::run_scan();
            *out.lock().unwrap() = AiState::Done(result);
            ctx.request_repaint();
        });
    }

    pub(super) fn ai_ready(&self) -> bool {
        matches!(&*self.ai.lock().unwrap(), AiState::Done(_))
    }

    pub(super) fn ai_scanning(&self) -> bool {
        matches!(&*self.ai.lock().unwrap(), AiState::Scanning)
    }

    /// "Scanned 3 min ago · 2s" for the toolbar, once a scan has finished.
    pub(super) fn ai_scanned_label(&self) -> Option<String> {
        match &*self.ai.lock().unwrap() {
            AiState::Done(s) => Some(format!("Scanned {} · {}s", ago(scan::now().saturating_sub(s.ts)), s.secs)),
            _ => None,
        }
    }

    pub(super) fn ai_page(&mut self, ui: &mut egui::Ui, p: &Pal, disk: &scan::Disk) {
        let snapshot = match &*self.ai.lock().unwrap() {
            AiState::Done(s) => Some(s.clone()),
            _ => None,
        };
        let Some(s) = snapshot else {
            if !self.ai_scanning() {
                let ctx = ui.ctx().clone();
                self.start_ai_scan(&ctx);
            }
            ui.add_space(80.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(RichText::new("Looking for AI models, caches and chat history…").color(p.muted));
            });
            return;
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if s.tools.is_empty() && s.stray.is_empty() {
                card(ui, p, |ui| {
                    ui.label(RichText::new("No AI storage found").size(16.0).strong());
                    ui.label(RichText::new("No local models, assistant data or ML caches turned up in the usual places.").color(p.muted));
                });
                return;
            }
            let stray_bytes: u64 = s.stray.iter().map(|i| i.bytes).sum();
            let total: u64 = s.tools.iter().map(|t| t.bytes).sum::<u64>() + stray_bytes;
            self.ai_stats(ui, p, &s, disk, total);

            let mut bars: Vec<BarItem> = s
                .tools
                .iter()
                .filter_map(|t| ai::find(t.id).map(|d| BarItem { id: Some(d.id), name: d.name, color: group_color(d.group), bytes: t.bytes, count: t.items.len() }))
                .collect();
            if stray_bytes > 0 {
                bars.push(BarItem { id: None, name: "Loose model files", color: p.muted, bytes: stray_bytes, count: s.stray.len() });
            }
            bars.sort_by(|a, b| b.bytes.cmp(&a.bytes));
            let found = insights(p, &s, total);
            let mut picked = None;
            pair(
                ui,
                p,
                0.58,
                |ui| {
                    card_title(ui, p, None, "Where the space goes", "Storage by tool, largest first. Click one for details.", None);
                    picked = bar_list(ui, p, &bars, total);
                },
                |ui| {
                    card_title(ui, p, None, "Insights", "What stands out from the last scan", None);
                    for (i, ins) in found.iter().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        insight_row(ui, p, ins);
                    }
                },
            );
            if let Some(id) = picked {
                self.ai_open = Some(id);
                self.ai_show_all = false;
                self.open_item = None;
                self.ai_scroll = true;
            }
            ui.add_space(GAP);

            for g in AiGroup::ALL {
                let tools = group_tools(&s, g);
                if tools.is_empty() {
                    continue;
                }
                let sum: u64 = tools.iter().map(|t| t.bytes).sum();
                card(ui, p, |ui| {
                    card_title(ui, p, Some(group_color(g)), g.label(), group_desc(g), Some(fmt_bytes(sum)));
                    for (i, t) in tools.iter().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        self.ai_tool_row(ui, p, t, s.ts);
                    }
                });
                ui.add_space(GAP);
            }
            if !s.stray.is_empty() {
                card(ui, p, |ui| {
                    card_title(ui, p, Some(p.muted), "Loose model files", "Model weights of 200 MB or more (.gguf, .safetensors, .ckpt, .pth, .onnx) outside the tools above", Some(fmt_bytes(stray_bytes)));
                    self.ai_items(ui, p, &s.stray, stray_bytes, s.ts);
                });
                ui.add_space(GAP);
            }
            ui.label(RichText::new("Sizes are what the files take on disk. \"Changed\" is the newest file change inside, not when you last used it. DiskLens never deletes anything.").size(11.5).color(p.muted));
        });
    }

    fn ai_stats(&self, ui: &mut egui::Ui, p: &Pal, s: &AiScan, disk: &scan::Disk, total: u64) {
        let stale = stale_items(s);
        let stale_bytes: u64 = stale.iter().map(|(_, i)| i.bytes).sum();
        let tools = s.tools.len();
        let cells = [
            (p.accent, fmt_bytes(total), "AI storage", format!("{:.1}% of your disk · {tools} tool{}", total as f64 * 100.0 / disk.total.max(1) as f64, if tools == 1 { "" } else { "s" })),
            (p.good, fmt_bytes(safety_bytes(s, Safety::Safe)), "Safe to clear", "Cache that rebuilds itself".into()),
            (p.warn, fmt_bytes(safety_bytes(s, Safety::Review)), "Check first", "Models and app data, slow to recreate".into()),
            (LEFTOVER, fmt_bytes(stale_bytes), "Untouched 90d+", match stale.len() {
                0 => "Nothing big has gone stale".into(),
                n => format!("{n} item{} of 50 MB or more", if n == 1 { "" } else { "s" }),
            }),
        ];
        let cols = if ui.available_width() < WIDE { 2 } else { 4 };
        grid(ui, cells.len(), cols, |ui, i| {
            let (c, v, l, sub) = &cells[i];
            stat_card(ui, p, *c, v.clone(), l, sub.clone());
        });
    }

    fn ai_tool_row(&mut self, ui: &mut egui::Ui, p: &Pal, t: &AiTool, now: u64) {
        let Some(def) = ai::find(t.id) else { return };
        let open = self.ai_open == Some(def.id);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
        if open && self.ai_scroll {
            resp.scroll_to_me(Some(Align::TOP));
            self.ai_scroll = false;
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.hovered() {
            ui.painter().rect_filled(rect.expand2(Vec2::new(8.0, 2.0)), 6.0, p.hover);
        }
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, if open { "▾" } else { "▸" }, egui::FontId::proportional(12.0), p.muted);
        ui.painter().text(rect.left_center() + Vec2::new(18.0, 0.0), egui::Align2::LEFT_CENTER, def.name, egui::FontId::proportional(14.0), p.text);
        ui.painter().text(rect.right_center(), egui::Align2::RIGHT_CENTER, fmt_bytes(t.bytes), egui::FontId::monospace(13.0), p.text);
        ui.painter().text(rect.right_center() - Vec2::new(86.0, 0.0), egui::Align2::RIGHT_CENTER, format!("changed {}", last_changed(t.modified, now)), egui::FontId::proportional(11.5), p.muted);
        if resp.clicked() {
            self.ai_open = if open { None } else { Some(def.id) };
            self.ai_show_all = false;
            self.open_item = None;
        }
        if !open {
            return;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            safety_pill(ui, def.safety, p);
            if t.denied {
                pill(ui, "Partly unreadable", p.warn, p.warn.gamma_multiply(0.2)).on_hover_text("Grant Full Disk Access to count everything.");
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new(def.desc).size(12.5).color(p.muted));
        if let Some(cmd) = def.cmd {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(cmd).monospace().size(11.5));
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(cmd.to_string());
                }
            });
        }
        ui.add_space(8.0);
        self.ai_items(ui, p, &t.items, t.bytes, now);
        ui.add_space(4.0);
    }

    fn ai_items(&mut self, ui: &mut egui::Ui, p: &Pal, items: &[AiItem], total: u64, now: u64) {
        let limit = if self.ai_show_all { items.len() } else { 12 };
        for it in items.iter().take(limit) {
            let open = self.open_item.as_deref() == Some(it.path.as_str());
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
            if open || resp.hovered() {
                ui.painter().rect_filled(rect, 6.0, p.hover);
            }
            ui.painter().circle_filled(rect.left_center() + Vec2::new(9.0, 0.0), 4.0, theme::badge_colors(it.safety, p).0);
            let size = ui.painter().layout_no_wrap(fmt_bytes(it.bytes), egui::FontId::monospace(11.5), p.text);
            let size_w = size.size().x;
            ui.painter().galley(rect.right_center() - Vec2::new(8.0 + size_w, size.size().y / 2.0), size, p.text);
            let stale = it.stale(now);
            let tag_w = if stale { 44.0 } else { 0.0 };
            if stale {
                ui.painter().text(rect.right_center() - Vec2::new(18.0 + size_w, 0.0), egui::Align2::RIGHT_CENTER, "stale", egui::FontId::proportional(11.0), LEFTOVER);
            }
            let mut job = egui::text::LayoutJob::single_section(it.name.clone(), egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: p.text, ..Default::default() });
            job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 22.0 - size_w - 16.0 - tag_w).max(40.0));
            let g = ui.painter().layout_job(job);
            ui.painter().galley(rect.left_center() + Vec2::new(20.0, -g.size().y / 2.0), g, p.text);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("{}\n{:.0}% of this location\nChanged {}\n\nClick for details", tilde(&it.path), it.bytes as f64 * 100.0 / total.max(1) as f64, last_changed(it.modified, now)));
            if resp.clicked() {
                self.open_item = if open { None } else { Some(it.path.clone()) };
            }
            if open {
                egui::Frame::new().fill(p.window).corner_radius(8.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 4.0;
                    safety_pill(ui, it.safety, p);
                    if !it.note.is_empty() {
                        ui.label(RichText::new(&it.note).size(12.0).color(p.muted));
                    }
                    ui.label(RichText::new(tilde(&it.path)).monospace().size(11.0).color(p.muted));
                    if let Some(cmd) = &it.cmd {
                        ui.label(RichText::new(cmd).monospace().size(11.0));
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("Reveal").on_hover_text("Show in Finder").clicked() {
                            reveal(Path::new(&it.path));
                        }
                        if let Some(cmd) = &it.cmd {
                            if ui.small_button("Copy command").clicked() {
                                ui.ctx().copy_text(cmd.clone());
                            }
                        }
                    });
                });
                ui.add_space(4.0);
            }
        }
        if items.len() > 12 {
            ui.add_space(4.0);
            let label = if self.ai_show_all { "Show fewer".to_string() } else { format!("Show all {} items", items.len()) };
            if ui.small_button(label).clicked() {
                self.ai_show_all = !self.ai_show_all;
            }
        }
    }
}
