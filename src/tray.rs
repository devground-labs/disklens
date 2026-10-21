//! Menu-bar icon, free-space readout and low-space notifications.

use crate::app::fmt_bytes;
use crate::{logo, scan, store};
use eframe::egui;
use std::sync::atomic::{AtomicU64, Ordering};
use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const THRESHOLDS: [u64; 6] = [0, 5, 10, 20, 30, 50];
const CHECK_EVERY: Duration = Duration::from_secs(60);
const REALERT_AFTER_SECS: u64 = 6 * 3600;
/// Record a free-space sample this often, or sooner when free space moves by SAMPLE_DELTA.
const SAMPLE_EVERY_SECS: u64 = 15 * 60;
const SAMPLE_DELTA: u64 = 500_000_000;

thread_local! {
    /// The tray icon must stay on the main thread; other threads reach it via the main dispatch queue.
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

/// State shared between the menu callbacks, the monitor thread and the UI thread.
pub struct Shared {
    pub free: AtomicU64,
    pub threshold_gb: AtomicU64,
}

pub struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    checks: Vec<(u64, CheckMenuItem)>,
    shared: Arc<Shared>,
    shown: (u64, u64, u64), // (free GB tenths shown, threshold shown, alert state)
}

/// The DiskLens mark as a template (monochrome) menu-bar glyph.
fn glyph() -> Icon {
    const N: u32 = 44;
    Icon::from_rgba(logo::glyph_rgba(N), N, N).expect("icon")
}

fn notify(title: &str, body: &str) {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"{}\" sound name \"Funk\"", esc(body), esc(title));
    let _ = std::process::Command::new("osascript").args(["-e", &script]).output();
}

fn monitor(shared: Arc<Shared>) {
    let mut last_alert: Option<(u64, u64)> = None; // (timestamp, free at alert)
    let mut last_sample = store::load_disk_log().last().map(|s| (s.ts, s.free));
    loop {
        let disk = scan::disk_info();
        let free = disk.free;
        if free > 0 {
            shared.free.store(free, Ordering::Relaxed);
            let now = scan::now();
            let due = last_sample.is_none_or(|(ts, f)| now.saturating_sub(ts) >= SAMPLE_EVERY_SECS || f.abs_diff(free) >= SAMPLE_DELTA);
            if due {
                store::append_disk_sample(store::DiskSample { ts: now, total: disk.total, free });
                last_sample = Some((now, free));
            }
        }
        dispatch2::DispatchQueue::main().exec_async(refresh);
        let thr = shared.threshold_gb.load(Ordering::Relaxed) * 1_000_000_000;
        if thr == 0 || free >= thr + 1_000_000_000 {
            last_alert = None; // recovered (with 1 GB hysteresis)
        } else if free > 0 && free < thr {
            let now = scan::now();
            let again = match last_alert {
                None => true,
                Some((t, f)) => now.saturating_sub(t) > REALERT_AFTER_SECS || free + 2_000_000_000 < f,
            };
            if again {
                last_alert = Some((now, free));
                let safe: u64 = store::load()
                    .last()
                    .map(|s| s.targets.iter().filter(|r| crate::targets::find(&r.id).is_some_and(|t| t.safety == crate::targets::Safety::Safe && !t.nested)).map(|r| r.bytes).sum())
                    .unwrap_or(0);
                let hint = if safe > 0 { format!(" About {} looks safe to reclaim.", fmt_bytes(safe)) } else { String::new() };
                notify("DiskLens: low disk space", &format!("Only {} free (alert level {} GB).{hint}", fmt_bytes(free), thr / 1_000_000_000));
            }
        }
        std::thread::sleep(CHECK_EVERY);
    }
}

impl Tray {
    fn new(ctx: &egui::Context) -> Option<Tray> {
        let settings = store::load_settings();
        let shared = Arc::new(Shared { free: AtomicU64::new(scan::disk_info().free), threshold_gb: AtomicU64::new(settings.threshold_gb) });

        let menu = Menu::new();
        let status = MenuItem::new("Free space: …", false, None);
        let open = MenuItem::new("Open DiskLens", true, None);
        let quit = MenuItem::new("Quit DiskLens", true, None);
        let about = MenuItem::new(format!("DiskLens {} · by devground-labs", env!("CARGO_PKG_VERSION")), false, None);
        let sub = Submenu::new("Alert when free space below", true);
        let mut checks = vec![];
        for gb in THRESHOLDS {
            let label = if gb == 0 { "Off".to_string() } else { format!("{gb} GB") };
            let c = CheckMenuItem::new(label, true, gb == settings.threshold_gb, None);
            let _ = sub.append(&c);
            checks.push((gb, c));
        }
        let _ = menu.append_items(&[&status, &PredefinedMenuItem::separator(), &open, &sub, &PredefinedMenuItem::separator(), &about, &quit]);

        let icon = TrayIconBuilder::new().with_menu(Box::new(menu)).with_icon_templated(glyph()).with_tooltip("DiskLens").build().ok()?;

        let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
        let ids: Vec<_> = checks.iter().map(|(gb, c)| (c.id().clone(), *gb)).collect();
        let (sh, ctx2) = (shared.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
            if ev.id == open_id {
                ctx2.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx2.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if ev.id == quit_id {
                std::process::exit(0);
            } else if let Some((_, gb)) = ids.iter().find(|(id, _)| *id == ev.id) {
                sh.threshold_gb.store(*gb, Ordering::Relaxed);
                store::save_settings(&store::Settings { threshold_gb: *gb });
            }
            refresh();
            ctx2.request_repaint();
        }));

        let sh = shared.clone();
        std::thread::spawn(move || monitor(sh));
        Some(Tray { icon, status, checks, shared, shown: (u64::MAX, u64::MAX, 0) })
    }

    fn sync(&mut self) {
        let free = self.shared.free.load(Ordering::Relaxed);
        let thr = self.shared.threshold_gb.load(Ordering::Relaxed);
        let low = (thr > 0 && free < thr * 1_000_000_000) as u64;
        let key = (free / 100_000_000, thr, low);
        if key == self.shown {
            return;
        }
        self.shown = key;
        let gb = format!("{:.0} GB", free as f64 / 1e9);
        self.icon.set_title(Some(if low == 1 { format!("⚠ {gb}") } else { gb }));
        self.status.set_text(format!("Free space: {}{}", fmt_bytes(free), if low == 1 { "  (low)" } else { "" }));
        for (g, c) in &self.checks {
            c.set_checked(*g == thr);
        }
    }
}

/// Create the menu-bar item (must be called on the main thread). Returns false if it couldn't be created.
pub fn init(ctx: &egui::Context) -> bool {
    match Tray::new(ctx) {
        Some(t) => {
            TRAY.with(|c| *c.borrow_mut() = Some(t));
            refresh();
            true
        }
        None => false,
    }
}

/// Update the title and menu from the latest shared state (main thread only).
pub fn refresh() {
    TRAY.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut() {
            if let Some(t) = c.as_mut() {
                t.sync();
            }
        }
    });
}
