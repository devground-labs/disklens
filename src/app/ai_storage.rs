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

/// One dot, one sentence.
fn insight(ui: &mut egui::Ui, p: &Pal, color: Color32, text: String) {
    ui.horizontal_top(|ui| {
        ui.label(RichText::new("●").size(10.0).color(color));
        ui.add(egui::Label::new(RichText::new(text).size(13.0).color(p.text)).wrap());
    });
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
            self.ai_summary(ui, p, &s, disk);
            ui.add_space(14.0);
            for g in AiGroup::ALL {
                let tools: Vec<&AiTool> = s.tools.iter().filter(|t| ai::find(t.id).is_some_and(|d| d.group == g)).collect();
                if tools.is_empty() {
                    continue;
                }
                let total: u64 = tools.iter().map(|t| t.bytes).sum();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").size(10.0).color(group_color(g)));
                    caption(ui, p, g.label());
                    ui.label(RichText::new(fmt_bytes(total)).size(11.0).color(p.muted));
                });
                ui.add_space(4.0);
                for t in tools {
                    self.ai_tool_card(ui, p, t, s.ts);
                    ui.add_space(8.0);
                }
                ui.add_space(8.0);
            }
            if !s.stray.is_empty() {
                self.ai_stray(ui, p, &s);
            }
            ui.add_space(8.0);
            ui.label(RichText::new(format!("Scanned {} in {}s. Sizes are what the files take on disk. \"Changed\" is the newest file change inside, not when you last used it.", ago(scan::now().saturating_sub(s.ts)), s.secs)).size(11.5).color(p.muted));
        });
    }

    fn ai_summary(&self, ui: &mut egui::Ui, p: &Pal, s: &AiScan, disk: &scan::Disk) {
        let total: u64 = s.tools.iter().map(|t| t.bytes).sum::<u64>() + s.stray.iter().map(|i| i.bytes).sum::<u64>();
        let items = || s.tools.iter().flat_map(|t| t.items.iter()).chain(s.stray.iter());
        card(ui, p, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    caption(ui, p, "AI storage");
                    ui.label(RichText::new(fmt_bytes(total)).size(30.0).strong());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.1}% of your disk", total as f64 * 100.0 / disk.total.max(1) as f64)).size(13.0).color(p.muted));
                });
            });
            ui.add_space(8.0);
            let mut parts: Vec<(Color32, u64, &str)> = AiGroup::ALL
                .iter()
                .map(|g| (group_color(*g), s.tools.iter().filter(|t| ai::find(t.id).is_some_and(|d| d.group == *g)).map(|t| t.bytes).sum(), g.label()))
                .collect();
            parts.push((p.muted, s.stray.iter().map(|i| i.bytes).sum(), "Loose model files"));
            stacked(ui, p, &parts);
        });
        ui.add_space(10.0);
        card(ui, p, |ui| {
            ui.horizontal(|ui| {
                caption(ui, p, "What stands out");
                help(ui, p, "Worked out from the last scan. Nothing is ever deleted by DiskLens.");
            });
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            if let Some((t, d)) = s.tools.first().and_then(|t| ai::find(t.id).map(|d| (t, d))) {
                let share = t.bytes as f64 * 100.0 / total.max(1) as f64;
                // Folder names like "User" only make sense for tools that hold named models.
                let lead = match t.items.first() {
                    Some(i) if d.group != AiGroup::Assistants && t.items.len() > 1 && i.bytes * 2 > t.bytes => format!(", mostly {} ({})", i.name, fmt_bytes(i.bytes)),
                    _ => String::new(),
                };
                insight(ui, p, p.accent, format!("{} is the biggest: {} ({share:.0}% of AI storage){lead}.", d.name, fmt_bytes(t.bytes)));
            }
            let by = |sf: Safety| -> u64 { items().filter(|i| i.safety == sf).map(|i| i.bytes).sum() };
            let (safe, review, careful) = (by(Safety::Safe), by(Safety::Review), by(Safety::Careful));
            if safe > 0 {
                insight(ui, p, p.good, format!("{} is cache that rebuilds itself. Clearing it loses nothing.", fmt_bytes(safe)));
            }
            if review > 0 {
                insight(ui, p, p.warn, format!("{} is models and app data you can fetch or recreate, but slowly. Check each one before removing it.", fmt_bytes(review)));
            }
            if careful > 0 {
                insight(ui, p, p.bad, format!("{} is your own history and files, such as chat transcripts. Only you can decide on these.", fmt_bytes(careful)));
            }
            let mut stale: Vec<(String, &AiItem)> = s
                .tools
                .iter()
                .flat_map(|t| t.items.iter().map(move |i| (ai::find(t.id).map_or(i.name.clone(), |d| format!("{} › {}", d.name, i.name)), i)))
                .chain(s.stray.iter().map(|i| (i.name.clone(), i)))
                .filter(|(_, i)| i.stale(s.ts) && i.bytes >= 50_000_000)
                .collect();
            stale.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes));
            if !stale.is_empty() {
                let sum: u64 = stale.iter().map(|(_, i)| i.bytes).sum();
                let names: Vec<String> = stale.iter().take(3).map(|(n, i)| format!("{n} ({})", fmt_bytes(i.bytes))).collect();
                insight(ui, p, LEFTOVER, format!("{} hasn't changed in {}+ days, including {}. Worth a look if you need space.", fmt_bytes(sum), ai::STALE_SECS / 86_400, names.join(", ")));
            }
            if !s.stray.is_empty() {
                insight(ui, p, p.warn, format!("{} model file(s) ({}) sit outside the usual AI folders. See \"Loose model files\" below.", s.stray.len(), fmt_bytes(s.stray.iter().map(|i| i.bytes).sum())));
            }
            if s.tools.iter().any(|t| t.denied) {
                insight(ui, p, p.muted, "Some folders couldn't be read, so their sizes are undercounted. Grant Full Disk Access to see everything.".into());
            }
        });
    }

    fn ai_tool_card(&mut self, ui: &mut egui::Ui, p: &Pal, t: &AiTool, now: u64) {
        let Some(def) = ai::find(t.id) else { return };
        let open = self.ai_open == Some(def.id);
        card(ui, p, |ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.hovered() {
                ui.painter().rect_filled(rect.expand2(Vec2::new(8.0, 2.0)), 6.0, p.hover);
            }
            ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, if open { "▾" } else { "▸" }, egui::FontId::proportional(12.0), p.muted);
            ui.painter().text(rect.left_center() + Vec2::new(18.0, 0.0), egui::Align2::LEFT_CENTER, def.name, egui::FontId::proportional(14.5), p.text);
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
        });
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

    fn ai_stray(&mut self, ui: &mut egui::Ui, p: &Pal, s: &AiScan) {
        ui.horizontal(|ui| {
            caption(ui, p, "Loose model files");
            help(ui, p, "Model weights (.gguf, .safetensors, .ckpt, .pth, .onnx) of 200 MB or more found in your folders, outside the tools above.");
        });
        ui.add_space(4.0);
        let total: u64 = s.stray.iter().map(|i| i.bytes).sum();
        card(ui, p, |ui| {
            self.ai_items(ui, p, &s.stray, total, s.ts);
        });
    }
}
