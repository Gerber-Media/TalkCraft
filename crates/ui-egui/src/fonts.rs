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

    #[test]
    fn every_symbol_in_the_interface_has_a_glyph() {
        // A character no font has shows as a box (it happened with ✓, ✎ and ✕).
        let defs = super::definitions();
        let fonts: Vec<fontdue::Font> = defs
            .font_data
            .values()
            .filter_map(|d| fontdue::Font::from_bytes(&d.font[..], fontdue::FontSettings { collection_index: d.index, ..Default::default() }).ok())
            .collect();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Only what is shown: string literals, not comments.
            for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
                let mut inside = false;
                let mut prev = ' ';
                for c in line.chars() {
                    if c == '"' && prev != '\\' {
                        inside = !inside;
                    } else if inside && !c.is_ascii() && !fonts.iter().any(|f| f.lookup_glyph_index(c) != 0) {
                        missing.insert((c, path.file_name().unwrap().to_string_lossy().to_string()));
                    }
                    prev = c;
                }
            }
        }
        assert!(missing.is_empty(), "no font has these characters: {missing:?}");
    }
}
