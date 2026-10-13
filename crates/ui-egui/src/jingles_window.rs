//! The Jingles window: eight pads for intro, outro and stingers. A pad holds an audio file (drop
//! one on it, or type its path); clicking the pad places the file on the jingle track at the
//! insertion point.

use crate::SoundApp;
use crate::theme::Tokens;
use egui::{RichText, vec2};
use serde_json::{Value, json};
use soundcraft_engine::cmd::podcast::JINGLE_PADS;

#[derive(Debug, Clone, Default)]
pub struct JinglesWindow {
    pub show: bool,
    /// Pad whose path is being edited, and the text so far.
    editing: Option<(usize, String)>,
}

/// `window.jingles`: opens or closes the window.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    let w = &mut app.extra.jingles;
    w.show = p.get("value").and_then(Value::as_bool).unwrap_or(!w.show);
    json!({"value": w.show})
}

/// Stores `path` on pad `i` (0-based) in the session's podcast settings.
fn set_pad(app: &mut SoundApp, i: usize, path: &str) {
    let mut pads = app.engine.session().podcast.jingles.clone();
    if pads.len() <= i {
        pads.resize(i + 1, String::new());
    }
    if let Some(slot) = pads.get_mut(i) {
        *slot = path.trim().to_string();
    }
    while pads.last().is_some_and(String::is_empty) {
        pads.pop();
    }
    if let Err(e) = app.run("podcast.set", json!({"jingles": pads})) {
        app.ui.status = e;
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_stem().and_then(|n| n.to_str()).unwrap_or(path).to_string()
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.extra.jingles.show {
        return;
    }
    let t = Tokens::current();
    let pads = app.engine.session().podcast.jingles.clone();
    let recording = app.engine.transport.recording;
    let mut open = true;
    let mut place: Option<usize> = None;
    let mut store: Option<(usize, String)> = None;
    let dropped: Vec<(Option<egui::Pos2>, String)> = ctx.input(|i| {
        i.raw.dropped_files.iter().map(|f| (i.pointer.hover_pos(), f.path().to_string_lossy().to_string())).filter(|(_, p)| !p.is_empty()).collect()
    });
    egui::Window::new("Jingles").open(&mut open).default_size(vec2(520.0, 230.0)).resizable(true).show(ctx, |ui| {
        let w = &mut app.extra.jingles;
        egui::Grid::new("jingle_pads").num_columns(4).spacing(vec2(8.0, 8.0)).show(ui, |ui| {
            for i in 0..JINGLE_PADS {
                let path = pads.get(i).cloned().unwrap_or_default();
                ui.vertical(|ui| {
                    let label = if path.is_empty() {
                        RichText::new(format!("Pad {}\nempty", i + 1)).color(t.text_dim)
                    } else {
                        RichText::new(format!("{}\n{}", i + 1, file_name(&path))).strong()
                    };
                    let r = ui
                        .add_enabled(!path.is_empty() && !recording, egui::Button::new(label).min_size(vec2(112.0, 54.0)))
                        .on_hover_text("Click: place on the jingle track at the insertion point · drop an audio file here to load the pad")
                        .on_disabled_hover_text(if recording { "Not while recording" } else { "Drop an audio file here, or set a path" });
                    if r.clicked() {
                        place = Some(i);
                    }
                    for (pos, file) in &dropped {
                        if pos.is_some_and(|p| r.rect.contains(p)) {
                            store = Some((i, file.clone()));
                        }
                    }
                    match &mut w.editing {
                        Some((e, text)) if *e == i => {
                            let te = ui.add(egui::TextEdit::singleline(text).desired_width(112.0).hint_text("/path/to/jingle.wav"));
                            if te.lost_focus() {
                                store = Some((i, text.clone()));
                            }
                        }
                        _ => {
                            if ui.small_button(if path.is_empty() { "Set file…" } else { "Change…" }).clicked() {
                                w.editing = Some((i, path.clone()));
                            }
                        }
                    }
                });
                if i % 4 == 3 {
                    ui.end_row();
                }
            }
        });
        ui.label(RichText::new("Pads are saved with the session. The jingle track is the one locked against recording.").small().color(t.text_dim));
    });
    app.extra.jingles.show = open;
    if let Some((i, path)) = store {
        app.extra.jingles.editing = None;
        set_pad(app, i, &path);
    }
    if let Some(i) = place {
        match app.run("podcast.place_jingle", json!({"pad": i + 1})) {
            Ok(_) => app.ui.status = format!("Placed {}", file_name(pads.get(i).map(String::as_str).unwrap_or(""))),
            Err(e) => app.ui.status = e,
        }
    }
}
