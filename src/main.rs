mod ai;
mod app;
mod classify;
mod logo;
mod scan;
mod store;
mod targets;
mod theme;
mod tray;

use eframe::egui;

fn parse_args() -> app::Args {
    let mut a = app::Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--screenshot" => a.screenshot = it.next().map(Into::into),
            "--explorer" => a.page_explorer = true,
            "--history" => a.page_history = true,
            "--ai" => a.page_ai = true,
            "--search" => a.search = it.next(),
            "--size" => a.size = it.next().and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))),
            "--select" => a.select = it.next(),
            "--light" => a.theme = Some(egui::ThemePreference::Light),
            "--scan" => a.scan = true,
            "--group" => a.group = it.next().and_then(|g| g.parse().ok()),
            "--tab" => a.tab = it.next(),
            "--dark" => a.theme = Some(egui::ThemePreference::Dark),
            _ => {}
        }
    }
    a
}

fn main() -> eframe::Result {
    // `disklens --export-icon icon.png [size]` writes the app icon and exits (used for the README / bundle).
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("--export-icon") {
        let path = argv.get(2).map_or("icon.png", String::as_str);
        let n: u32 = argv.get(3).and_then(|s| s.parse().ok()).unwrap_or(1024);
        image::save_buffer(path, &logo::icon_rgba(n), n, n, image::ColorType::Rgba8).expect("write icon");
        return Ok(());
    }
    let args = parse_args();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(args.size.unwrap_or([1280.0, 800.0]))
            .with_min_inner_size([980.0, 560.0])
            .with_title("DiskLens")
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: logo::icon_rgba(256), width: 256, height: 256 })),
        ..Default::default()
    };
    eframe::run_native("DiskLens", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, args)))))
}
