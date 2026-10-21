//! History page: used space over time, with drops (cleanups) marked and explained.

use super::*;
use crate::store::{self, DiskSample};
use std::sync::OnceLock;

/// A drop in used space this large between two consecutive points counts as a cleanup.
const MIN_FREED: u64 = 1_000_000_000;
/// Per-location changes smaller than this are left out of the explanations.
const MIN_TARGET_DELTA: i64 = 200_000_000;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Range {
    Day,
    Week,
    Month,
    All,
}

impl Range {
    const ALL: [Range; 4] = [Range::Day, Range::Week, Range::Month, Range::All];
    fn label(self) -> &'static str {
        match self {
            Range::Day => "24 hours",
            Range::Week => "7 days",
            Range::Month => "30 days",
            Range::All => "All time",
        }
    }
    fn secs(self) -> Option<u64> {
        match self {
            Range::Day => Some(86_400),
            Range::Week => Some(7 * 86_400),
            Range::Month => Some(30 * 86_400),
            Range::All => None,
        }
    }
}

/// One point on the chart: (timestamp, used bytes, total bytes).
type Point = (u64, u64, u64);

struct Event {
    from: u64,
    ts: u64,
    used: u64,
    freed: u64,
    top: Vec<(&'static str, i64)>,
}

// ---------- local time formatting (no date crate needed) ----------

fn tz_offset() -> i64 {
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| {
        let out = Command::new("date").arg("+%z").output().ok();
        let s = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if s.len() != 5 {
            return 0;
        }
        let sign = if s.starts_with('-') { -1 } else { 1 };
        let (h, m) = (s[1..3].parse::<i64>().unwrap_or(0), s[3..5].parse::<i64>().unwrap_or(0));
        sign * (h * 3600 + m * 60)
    })
}

/// (month 1-12, day, hour, minute) in local time.
fn civil(ts: u64) -> (usize, i64, i64, i64) {
    let t = ts as i64 + tz_offset();
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Howard Hinnant's days -> civil date.
    let z = days + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (m as usize, d, secs / 3600, secs % 3600 / 60)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn fmt_day(ts: u64) -> String {
    let (m, d, _, _) = civil(ts);
    format!("{} {d}", MONTHS[m - 1])
}

fn fmt_time(ts: u64) -> String {
    let (m, d, h, mi) = civil(ts);
    format!("{} {d}, {h:02}:{mi:02}", MONTHS[m - 1])
}

fn fmt_clock(ts: u64) -> String {
    let (_, _, h, mi) = civil(ts);
    format!("{h:02}:{mi:02}")
}

// ---------- widgets ----------

fn chip(ui: &mut egui::Ui, p: &Pal, on: bool, text: &str) -> bool {
    let (fg, bg) = if on { (Color32::WHITE, p.accent) } else { (p.text, p.hover) };
    ui.add(egui::Button::new(RichText::new(text).size(12.5).color(fg)).fill(bg).stroke(Stroke::NONE).corner_radius(14.0).min_size(Vec2::new(0.0, 24.0))).clicked()
}

fn stat(ui: &mut egui::Ui, p: &Pal, title: &str, value: String, color: Color32, sub: String) {
    card(ui, p, |ui| {
        caption(ui, p, title);
        ui.label(RichText::new(value).size(24.0).strong().color(color));
        ui.label(RichText::new(sub).size(12.0).color(p.muted));
    });
}

/// Used space over time. Drops of 1 GB+ get a green marker; markers closer than ~48 px are labeled together.
fn usage_chart(ui: &mut egui::Ui, p: &Pal, pts: &[Point], events: &[Event]) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 300.0), Sense::hover());
    let painter = ui.painter_at(rect);
    if pts.len() < 2 {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Not enough data yet. Points are added by scans and by the menu-bar monitor.", egui::FontId::proportional(13.0), p.muted);
        return;
    }
    let plot = egui::Rect::from_min_max(rect.min + Vec2::new(70.0, 34.0), rect.max - Vec2::new(18.0, 30.0));
    let (mn, mx) = pts.iter().fold((u64::MAX, 0), |(a, b), x| (a.min(x.1), b.max(x.1)));
    let pad = ((mx - mn) as f64 * 0.18).max(1e9);
    let (lo, hi) = (mn as f64 - pad, mx as f64 + pad);
    // Long stretches with no data (Mac asleep, app not running) are shortened to `cap`
    // so short bursts like a cleanup session stay readable. Shortened gaps are marked on the axis.
    let real_span = (pts[pts.len() - 1].0 - pts[0].0).max(1) as f64;
    let cap = (real_span / 20.0).max(1800.0);
    let mut vt = vec![0.0f64]; // virtual time of each point
    let mut breaks = vec![]; // indices where a gap was shortened
    for (i, w) in pts.windows(2).enumerate() {
        let dt = (w[1].0 - w[0].0) as f64;
        if dt > cap {
            breaks.push(i + 1);
        }
        vt.push(vt[i] + dt.min(cap));
    }
    let span = vt.last().copied().unwrap_or(1.0).max(1.0);
    let vx = |v: f64| plot.left() + (v / span) as f32 * plot.width();
    // Map a real timestamp onto the shortened axis (interpolating within its segment).
    let x = |t: u64| {
        let i = pts.partition_point(|pt| pt.0 <= t).clamp(1, pts.len() - 1);
        let (a, b) = (pts[i - 1].0 as f64, pts[i].0 as f64);
        let f = if b > a { ((t as f64 - a) / (b - a)).clamp(0.0, 1.0) } else { 1.0 };
        vx(vt[i - 1] + (vt[i] - vt[i - 1]) * f)
    };
    let y = |v: u64| plot.bottom() - ((v as f64 - lo) / (hi - lo)) as f32 * plot.height();
    let small = egui::FontId::proportional(11.0);

    // Grid + y labels
    for i in 0..=4 {
        let v = lo + (hi - lo) * i as f64 / 4.0;
        let yy = y(v as u64);
        painter.line_segment([Pos2::new(plot.left(), yy), Pos2::new(plot.right(), yy)], Stroke::new(1.0, p.border));
        painter.text(Pos2::new(plot.left() - 10.0, yy), egui::Align2::RIGHT_CENTER, fmt_bytes(v.max(0.0) as u64), small.clone(), p.muted);
    }
    // X labels at real data times: the point nearest each of five evenly spaced ticks.
    let mut last_label_x = f32::NEG_INFINITY;
    for i in 0..=4 {
        let target = span * i as f64 / 4.0;
        let k = vt.iter().enumerate().min_by(|a, b| (a.1 - target).abs().total_cmp(&(b.1 - target).abs())).map(|(k, _)| k).unwrap_or(0);
        let px = vx(vt[k]);
        if px - last_label_x < 90.0 {
            continue;
        }
        last_label_x = px;
        let label = if real_span <= 2.0 * 86_400.0 { fmt_time(pts[k].0) } else { fmt_day(pts[k].0) };
        let align = if px < plot.left() + 40.0 {
            egui::Align2::LEFT_TOP
        } else if px > plot.right() - 40.0 {
            egui::Align2::RIGHT_TOP
        } else {
            egui::Align2::CENTER_TOP
        };
        painter.text(Pos2::new(px, plot.bottom() + 8.0), align, label, small.clone(), p.muted);
    }
    // Axis-break marks where a long gap was shortened.
    for &k in &breaks {
        let mid = (vx(vt[k - 1]) + vx(vt[k])) / 2.0;
        for dx in [-3.0, 3.0] {
            painter.line_segment([Pos2::new(mid + dx - 3.0, plot.bottom() + 5.0), Pos2::new(mid + dx + 3.0, plot.bottom() - 5.0)], Stroke::new(1.5, p.muted));
        }
        let gap = pts[k].0 - pts[k - 1].0;
        let txt = if gap >= 86_400 { format!("{} d gap", gap / 86_400) } else { format!("{} h gap", gap / 3600) };
        painter.text(Pos2::new(mid, plot.bottom() - 9.0), egui::Align2::CENTER_BOTTOM, txt, egui::FontId::proportional(10.0), p.muted);
    }

    // Area (as trapezoids, which are convex) + line
    let line: Vec<Pos2> = pts.iter().zip(&vt).map(|(pt, v)| Pos2::new(vx(*v), y(pt.1))).collect();
    let fill = p.accent.gamma_multiply(if p.dark { 0.16 } else { 0.10 });
    for w in line.windows(2) {
        painter.add(egui::Shape::convex_polygon(vec![w[0], w[1], Pos2::new(w[1].x, plot.bottom()), Pos2::new(w[0].x, plot.bottom())], fill, Stroke::NONE));
    }
    painter.add(egui::Shape::line(line.clone(), Stroke::new(2.0, p.accent)));

    // Cleanup markers, clustered so labels don't collide
    let mut clusters: Vec<(f32, Pos2, u64)> = vec![]; // (x, marker pos, freed)
    for e in events {
        let pos = Pos2::new(x(e.ts), y(e.used));
        match clusters.last_mut() {
            Some(c) if (pos.x - c.0).abs() < 48.0 => {
                c.1 = pos;
                c.2 += e.freed;
            }
            _ => clusters.push((pos.x, pos, e.freed)),
        }
    }
    for (_, pos, freed) in &clusters {
        painter.circle(*pos, 5.0, p.good, Stroke::new(2.0, p.card));
        let label = format!("−{} freed", fmt_bytes(*freed));
        let galley = painter.layout_no_wrap(label, egui::FontId::proportional(11.5), Color32::WHITE);
        let mut r = egui::Rect::from_center_size(Pos2::new(pos.x, pos.y - 20.0), galley.size() + Vec2::new(12.0, 6.0));
        r = r.translate(Vec2::new((plot.left() - r.left()).max(0.0) + (plot.right() - r.right()).min(0.0), (rect.top() + 2.0 - r.top()).max(0.0)));
        painter.rect_filled(r, 5.0, p.good);
        painter.galley(r.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }

    // Hover crosshair + readout
    if let Some(hp) = resp.hover_pos().filter(|h| plot.expand2(Vec2::new(8.0, 0.0)).contains(*h)) {
        let i = line.iter().enumerate().min_by(|a, b| (a.1.x - hp.x).abs().total_cmp(&(b.1.x - hp.x).abs())).map(|(i, _)| i).unwrap_or(0);
        let (pt, pos) = (pts[i], line[i]);
        painter.line_segment([Pos2::new(pos.x, plot.top()), Pos2::new(pos.x, plot.bottom())], Stroke::new(1.0, p.muted.gamma_multiply(0.6)));
        painter.circle(pos, 4.5, p.accent, Stroke::new(2.0, p.card));
        let text = format!("{}\n{} used · {} free", fmt_time(pt.0), fmt_bytes(pt.1), fmt_bytes(pt.2.saturating_sub(pt.1)));
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), p.text);
        let size = galley.size() + Vec2::new(18.0, 12.0);
        let mut left = pos.x + 12.0;
        if left + size.x > rect.right() {
            left = pos.x - 12.0 - size.x;
        }
        let r = egui::Rect::from_min_size(Pos2::new(left, plot.top() + 4.0), size);
        painter.rect(r, 7.0, p.card, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        painter.galley(r.min + Vec2::new(9.0, 6.0), galley, p.text);
    }
}

// ---------- page ----------

impl App {
    /// Per-location size changes between two scans, largest first (non-overlapping locations only).
    fn target_deltas(a: &Snapshot, b: &Snapshot) -> Vec<(&'static str, i64)> {
        let mut v: Vec<(&'static str, i64)> = b
            .targets
            .iter()
            .filter_map(|tb| {
                let t = targets::find(&tb.id).filter(|t| !t.nested)?;
                let ta = a.targets.iter().find(|x| x.id == tb.id)?;
                Some((t.name, tb.bytes as i64 - ta.bytes as i64))
            })
            .filter(|(_, d)| d.abs() >= MIN_TARGET_DELTA)
            .collect();
        v.sort_by_key(|(_, d)| -d.abs());
        v
    }

    /// Drops of MIN_FREED or more between consecutive points, with the locations that shrank
    /// (when scans bracket the drop closely enough to tell).
    fn events(&self, pts: &[Point]) -> Vec<Event> {
        let mut out = vec![];
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a.1 < b.1 + MIN_FREED {
                continue;
            }
            let before = self.history.iter().rev().find(|s| s.ts <= a.0 + 60);
            let after = self.history.iter().find(|s| s.ts + 60 >= b.0);
            let top = match (before, after) {
                (Some(x), Some(y)) if y.ts > x.ts && a.0.saturating_sub(x.ts) < 6 * 3600 && y.ts.saturating_sub(b.0) < 6 * 3600 => {
                    Self::target_deltas(x, y).into_iter().filter(|(_, d)| *d < 0).take(3).collect()
                }
                _ => vec![],
            };
            out.push(Event { from: a.0, ts: b.0, used: b.1, freed: a.1 - b.1, top });
        }
        out
    }

    pub(super) fn history_page(&mut self, ui: &mut egui::Ui, p: &Pal) {
        let now = scan::now();
        if now.saturating_sub(self.disk_log_at) >= 60 {
            self.disk_log = store::load_disk_log();
            self.disk_log_at = now;
        }
        let cutoff = self.range.secs().map_or(0, |s| now.saturating_sub(s));
        let live = scan::disk_info();
        let mut pts: Vec<Point> = self
            .history
            .iter()
            .map(|s| (s.ts, s.disk.used(), s.disk.total))
            .chain(self.disk_log.iter().map(|d: &DiskSample| (d.ts, d.total.saturating_sub(d.free), d.total)))
            .chain((live.total > 0).then(|| (now, live.used(), live.total)))
            .filter(|pt| pt.0 >= cutoff && pt.1 > 0)
            .collect();
        pts.sort_by_key(|pt| pt.0);
        pts.dedup_by_key(|pt| pt.0);
        let events = self.events(&pts);

        ui.horizontal(|ui| {
            for r in Range::ALL {
                if chip(ui, p, self.range == r, r.label()) {
                    self.range = r;
                }
            }
        });
        ui.add_space(8.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let freed: u64 = events.iter().map(|e| e.freed).sum();
            let (first, last) = (pts.first().copied(), pts.last().copied());
            ui.columns(3, |c| {
                if let Some(l) = last {
                    stat(&mut c[0], p, "Used now", fmt_bytes(l.1), p.text, format!("of {} · {} free", fmt_bytes(l.2), fmt_bytes(l.2.saturating_sub(l.1))));
                }
                stat(&mut c[1], p, "Freed by cleanups", fmt_bytes(freed), p.good, format!("{} drop{} of 1 GB or more", events.len(), if events.len() == 1 { "" } else { "s" }));
                match (first, last) {
                    (Some(f), Some(l)) if pts.len() > 1 => {
                        let (s, c2) = fmt_delta(l.1 as i64 - f.1 as i64, p);
                        stat(&mut c[2], p, "Net change in used", s, c2, format!("since {}", fmt_time(f.0)));
                    }
                    _ => stat(&mut c[2], p, "Net change in used", "–".into(), p.muted, "Needs two data points".into()),
                }
            });
            ui.add_space(12.0);
            card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    caption(ui, p, "Used space");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} points · hover for details", pts.len())).size(11.5).color(p.muted));
                    });
                });
                usage_chart(ui, p, &pts, &events);
            });
            ui.add_space(12.0);
            ui.columns(2, |c| {
                card(&mut c[0], p, |ui| {
                    caption(ui, p, "Space freed");
                    ui.add_space(4.0);
                    if events.is_empty() {
                        ui.label(RichText::new("No drops of 1 GB or more in this range.").color(p.muted));
                    }
                    for e in events.iter().rev() {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("−{}", fmt_bytes(e.freed))).size(16.0).strong().color(p.good));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(format!("{} → {}", fmt_time(e.from), fmt_clock(e.ts))).size(11.5).color(p.muted));
                            });
                        });
                        let why = if e.top.is_empty() {
                            "No scan right before and after, so the source isn't known.".to_string()
                        } else {
                            e.top.iter().map(|(n, d)| format!("{n} −{}", fmt_bytes(d.unsigned_abs()))).collect::<Vec<_>>().join(" · ")
                        };
                        ui.label(RichText::new(why).size(12.0).color(p.muted));
                        ui.add_space(4.0);
                        ui.separator();
                    }
                });
                card(&mut c[1], p, |ui| {
                    caption(ui, p, "Biggest changes by location");
                    ui.add_space(4.0);
                    let scans: Vec<&Snapshot> = self.history.iter().filter(|s| s.ts >= cutoff).collect();
                    match (scans.first(), scans.last()) {
                        (Some(a), Some(b)) if scans.len() > 1 => {
                            ui.label(RichText::new(format!("{} → {} ({} scans)", fmt_time(a.ts), fmt_time(b.ts), scans.len())).size(11.5).color(p.muted));
                            ui.add_space(4.0);
                            let deltas = Self::target_deltas(a, b);
                            let max = deltas.iter().map(|d| d.1.unsigned_abs()).max().unwrap_or(1).max(1);
                            if deltas.is_empty() {
                                ui.label(RichText::new("No location changed by more than 200 MB.").color(p.muted));
                            }
                            for (name, d) in deltas.iter().take(10) {
                                let (s, col) = fmt_delta(*d, p);
                                ui.horizontal(|ui| {
                                    ui.allocate_ui_with_layout(Vec2::new(170.0, 18.0), Layout::left_to_right(Align::Center), |ui| {
                                        ui.set_width(170.0);
                                        ui.add(egui::Label::new(RichText::new(*name).size(12.5)).truncate());
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.add_sized(Vec2::new(80.0, 18.0), egui::Label::new(RichText::new(s).monospace().size(12.0).color(col)));
                                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| bar(ui, p, d.unsigned_abs() as f32 / max as f32, col));
                                    });
                                });
                            }
                        }
                        _ => {
                            ui.label(RichText::new("Needs two scans in this range. Use Scan now, or pick a longer range.").color(p.muted));
                        }
                    }
                });
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Points come from full scans and from free-space samples the menu-bar monitor takes every 15 minutes. Used space also moves with swap, caches and Time Machine snapshots, so not every drop is a cleanup.").size(11.5).color(p.muted));
        });
    }
}
