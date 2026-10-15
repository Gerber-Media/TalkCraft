//! Colours the user gives things to tell them apart (track name plates, jingle pads, markers): a
//! row of swatches, a free colour picker, and text that stays readable on any of them.

use crate::i18n::tr;
use egui::{Color32, Sense, Stroke, vec2};

/// Swatches offered first: clear, distinct colours that read well in light and dark themes.
pub const SWATCHES: [[u8; 3]; 12] = [
    [214, 69, 65],
    [235, 132, 50],
    [240, 196, 64],
    [128, 186, 72],
    [47, 158, 99],
    [52, 170, 168],
    [59, 130, 196],
    [92, 99, 210],
    [150, 86, 196],
    [214, 90, 160],
    [128, 128, 128],
    [40, 40, 44],
];

/// Relative luminance (sRGB, WCAG).
fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// Text colour for a background: dark on light colours, light on dark ones (whichever contrasts more).
pub fn ink_on(bg: Color32) -> Color32 {
    let l = luminance(bg);
    // Contrast against near-black (0.012) and white (1.0). On mid tones both are close: white
    // then, as on most coloured buttons; dark only where it clearly reads better (yellow, light).
    let dark = (l + 0.05) / (0.012 + 0.05);
    let light = 1.05 / (l + 0.05);
    if dark > light * 1.25 { Color32::from_rgb(18, 18, 20) } else { Color32::WHITE }
}

/// What the colour menu chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Color([u8; 3]),
    /// Back to the standard look.
    Reset,
}

/// Swatches, a free picker and (when `current` is set) "Standard". Use inside a menu.
pub fn menu(ui: &mut egui::Ui, current: Option<[u8; 3]>) -> Option<Choice> {
    let mut out = None;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(6.0 * 26.0);
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for c in SWATCHES {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            let col = Color32::from_rgb(c[0], c[1], c[2]);
            ui.painter().rect_filled(r, 4.0, col);
            if current == Some(c) {
                ui.painter().rect_stroke(r.expand(1.5), 5.0, Stroke::new(2.0, ink_on(ui.visuals().panel_fill)), egui::StrokeKind::Outside);
            }
            if resp.hovered() {
                ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, ink_on(col)), egui::StrokeKind::Inside);
            }
            if resp.clicked() {
                out = Some(Choice::Color(c));
            }
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(tr("Own colour"));
        let mut c = current.unwrap_or([128, 128, 128]);
        if egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
            out = Some(Choice::Color(c));
        }
    });
    if current.is_some() && ui.button(tr("Standard colour")).clicked() {
        out = Some(Choice::Reset);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_stays_readable_on_any_colour() {
        assert_eq!(ink_on(Color32::from_rgb(240, 196, 64)), Color32::from_rgb(18, 18, 20), "dark on yellow");
        assert_eq!(ink_on(Color32::from_rgb(40, 40, 44)), Color32::WHITE, "light on near black");
        assert_eq!(ink_on(Color32::from_rgb(59, 130, 196)), Color32::WHITE, "light on blue");
        assert_eq!(ink_on(Color32::WHITE), Color32::from_rgb(18, 18, 20));
        // Every swatch gets the text colour with at least 3:1 contrast (WCAG AA for bold labels).
        for c in SWATCHES {
            let bg = Color32::from_rgb(c[0], c[1], c[2]);
            let (a, b) = (luminance(bg), luminance(ink_on(bg)));
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(ratio >= 3.0, "{c:?}: {ratio}");
        }
    }
}
