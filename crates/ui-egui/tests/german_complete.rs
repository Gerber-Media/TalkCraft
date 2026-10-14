//! The German interface has a translation for every string the app shows: menus, windows,
//! dialogs, Settings tabs and tooltips are opened in German and every string that went through
//! `tr` without a German entry is reported.

use egui::{Pos2, RawInput, Rect, vec2};
use serde_json::json;
use soundcraft_ui_egui::{Services, SoundApp, i18n, menus};

fn frame(ctx: &egui::Context, app: &mut SoundApp) {
    let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))), ..Default::default() };
    ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    })
    .textures_delta
    .clear();
}

/// Strings shown untranslated on purpose: names, units, formats and technical terms.
const KEEP: &[&str] = &[];

#[test]
fn every_shown_string_has_a_german_translation() {
    i18n::record_missing();
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.ui.language = "de".into();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    // Every menu label, as the menu bars build them.
    fn walk(n: &menus::MenuNode) {
        let _ = i18n::tr(soundcraft_ui_egui::podcast::menu_label(&n.label));
        for c in &n.children {
            walk(c);
        }
    }
    for r in menus::tree() {
        walk(&r);
    }
    // Every window and dialog the menus open.
    let ids: Vec<String> = menus::UI_COMMANDS.iter().map(|c| c.0.to_string()).collect();
    for id in ids.iter().filter(|i| i.starts_with("window.")) {
        let _ = app.run(id, json!({"value": true}));
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        let _ = app.run(id, json!({"value": false}));
    }
    for root in menus::tree() {
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            if n.children.is_empty() {
                if let Some(cmd) = menus::command_for_path(&n.path, &menus::ui_aliases())
                    && app.dialogs.open_for_command(&app.engine, &cmd)
                {
                    frame(&ctx, &mut app);
                    frame(&ctx, &mut app);
                    app.dialogs.open = None;
                }
            } else {
                stack.extend(n.children);
            }
        }
    }
    for mode in ["record", "edit", "all"] {
        let _ = app.run("ui.workspace", json!({"mode": mode}));
        frame(&ctx, &mut app);
    }
    let missing: Vec<String> = i18n::missing().into_iter().filter(|s| !KEEP.contains(&s.as_str())).collect();
    for m in &missing {
        println!("MISSING {m:?}");
    }
    assert!(missing.is_empty(), "{} strings have no German translation", missing.len());
}

/// Every string the code sends through `tr` / `trf` (tooltips and messages too, which the walk
/// above may not show) has a German entry.
#[test]
fn every_translated_string_in_the_code_has_a_german_entry() {
    i18n::record_missing();
    i18n::set("de");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let src = src.split("#[cfg(test)]").next().unwrap_or("");
        for call in ["tr(\"", "trf(\""] {
            let mut rest = src;
            while let Some(i) = rest.find(call) {
                // Only calls of our functions, not e.g. `attr("`.
                let before = rest[..i].chars().last();
                rest = &rest[i + call.len()..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let mut key = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => key.push('\n'),
                            Some(o) => key.push(o),
                            None => {}
                        },
                        _ => key.push(c),
                    }
                }
                let _ = i18n::tr(&key);
                checked += 1;
            }
        }
    }
    let missing = i18n::missing();
    for m in &missing {
        println!("MISSING {m:?}");
    }
    assert!(checked > 300, "found {checked} strings");
    assert!(missing.is_empty(), "{} strings in the code have no German entry", missing.len());
}
