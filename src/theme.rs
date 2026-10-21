//! Look and feel: macOS-style palette, system fonts, spacing.

use crate::targets::Safety;
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub window: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub good: Color32,
    pub bad: Color32,
    pub warn: Color32,
    pub hover: Color32,
    pub track: Color32,
}

pub fn pal(dark: bool) -> Pal {
    let c = |r, g, b| Color32::from_rgb(r, g, b);
    if dark {
        Pal {
            dark,
            window: c(30, 30, 32),
            sidebar: c(38, 38, 41),
            card: c(44, 44, 47),
            border: c(62, 62, 66),
            text: c(240, 240, 242),
            muted: c(152, 152, 160),
            accent: c(10, 132, 255),
            good: c(50, 215, 100),
            bad: c(255, 105, 97),
            warn: c(255, 189, 68),
            hover: c(52, 52, 56),
            track: c(66, 66, 70),
        }
    } else {
        Pal {
            dark,
            window: c(246, 246, 247),
            sidebar: c(236, 236, 238),
            card: c(255, 255, 255),
            border: c(218, 218, 222),
            text: c(28, 28, 30),
            muted: c(110, 110, 118),
            accent: c(0, 122, 255),
            good: c(30, 150, 70),
            bad: c(215, 58, 45),
            warn: c(190, 115, 0),
            hover: c(234, 234, 238),
            track: c(224, 224, 228),
        }
    }
}

/// (text, background) of a safety badge.
pub fn badge_colors(s: Safety, p: &Pal) -> (Color32, Color32) {
    let base = match s {
        Safety::Safe => p.good,
        Safety::Review => p.warn,
        Safety::Careful => p.bad,
        Safety::System => p.muted,
    };
    (base, base.gamma_multiply(if p.dark { 0.2 } else { 0.14 }))
}

pub fn install(ctx: &egui::Context) {
    // System fonts (SF Pro / SF Mono) if available.
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, path: &str, fam: FontFamily| {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            fonts.families.entry(fam).or_default().insert(0, name.into());
        }
    };
    add("sf", "/System/Library/Fonts/SFNS.ttf", FontFamily::Proportional);
    add("sfmono", "/System/Library/Fonts/SFNSMono.ttf", FontFamily::Monospace);
    ctx.set_fonts(fonts);

    for dark in [true, false] {
        let p = pal(dark);
        let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.style_mut_of(theme, |s| {
            s.text_styles = [
                (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(12.0, 5.0);
            s.spacing.interact_size.y = 26.0;
            let v = &mut s.visuals;
            v.dark_mode = dark;
            v.panel_fill = p.window;
            v.window_fill = p.card;
            v.extreme_bg_color = p.track.gamma_multiply(0.6);
            v.faint_bg_color = p.hover;
            v.hyperlink_color = p.accent;
            v.override_text_color = Some(p.text);
            v.selection.bg_fill = p.accent.gamma_multiply(if dark { 0.35 } else { 0.18 });
            v.selection.stroke = egui::Stroke::new(1.0, p.accent);
            v.window_stroke = egui::Stroke::new(1.0, p.border);
            let r = egui::CornerRadius::same(7);
            for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
                w.corner_radius = r;
            }
            v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.border);
            v.widgets.inactive.weak_bg_fill = p.card;
            v.widgets.inactive.bg_fill = p.card;
            v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, p.border);
            v.widgets.hovered.weak_bg_fill = p.hover;
            v.widgets.hovered.bg_fill = p.hover;
            v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, p.muted);
            v.widgets.active.weak_bg_fill = p.hover;
            v.widgets.active.bg_stroke = egui::Stroke::new(1.0, p.accent);
        });
    }
}
