//! Fonts: Inter (SIL Open Font License 1.1, by Rasmus Andersson), bundled so the interface looks the
//! same everywhere. It is drawn for screens and stays straight at small sizes, where system fonts
//! (Helvetica Neue on macOS) came out wavy in egui. egui's own fonts stay behind it for symbols and
//! emoji. A "bold" family is always defined so the UI can use it.

use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");

/// Bytes and face index of the bold interface font (for drawing text outside egui, e.g. into
/// audiogram frames).
pub fn bold_font_bytes() -> Option<(Vec<u8>, u32)> {
    Some((INTER_SEMIBOLD.to_vec(), 0))
}

pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), Arc::new(FontData::from_static(INTER_REGULAR)));
    fonts.font_data.insert("inter-semibold".into(), Arc::new(FontData::from_static(INTER_SEMIBOLD)));
    let base: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(f) = fonts.families.get_mut(&FontFamily::Proportional) {
        f.insert(0, "inter".into());
    }
    let mut bold = base;
    bold.insert(0, "inter-semibold".into());
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    fonts
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

#[cfg(test)]
mod tests {
    #[test]
    fn inter_is_the_interface_font_and_has_the_german_letters() {
        let defs = super::definitions();
        assert_eq!(defs.families[&egui::FontFamily::Proportional][0], "inter");
        assert_eq!(defs.families[&egui::FontFamily::Name("bold".into())][0], "inter-semibold");
        let face = fontdue::Font::from_bytes(super::INTER_REGULAR, fontdue::FontSettings::default()).unwrap();
        for c in "äöüÄÖÜß€–…„“".chars() {
            assert_ne!(face.lookup_glyph_index(c), 0, "{c}");
        }
    }
}
