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
    /// Pad whose path is being edited, and the text so far.
    editing: Option<(usize, String)>,
}

/// `window.jingles`: opens or closes the window.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    crate::palettes::toggle(app, "jingles", p)
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
    if !crate::palettes::is_open(app, "jingles") {
        return;
    }
    let t = Tokens::current();
    let pads = app.engine.session().podcast.jingles.clone();
    let mut place: Option<usize> = None;
    let mut store: Option<(usize, String)> = None;
    crate::palettes::show(app, ctx, "jingles", crate::i18n::tr("Jingles"), vec2(520.0, 230.0), |app, ui, own_window| {
        // Files dropped on this window (in its own window, the drop arrives there).
        let dropped: Vec<(Option<egui::Pos2>, String)> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| (i.pointer.hover_pos(), f.path().to_string_lossy().to_string()))
                .filter(|(_, p)| !p.is_empty())
                .collect()
        });
        // In a window of its own the pads fill it: big enough for a finger on a touchscreen.
        let pad = if own_window {
            let w = ((ui.available_width() - 3.0 * 8.0) / 4.0).max(112.0);
            let h = ((ui.available_height() - 2.0 * 30.0 - 40.0) / 2.0).clamp(54.0, 220.0);
            vec2(w, h)
        } else {
            vec2(112.0, 54.0)
        };
        let w = &mut app.extra.jingles;
        egui::Grid::new("jingle_pads").num_columns(4).spacing(vec2(8.0, 8.0)).show(ui, |ui| {
            for i in 0..JINGLE_PADS {
                let path = pads.get(i).cloned().unwrap_or_default();
                ui.vertical(|ui| {
                    let label = if path.is_empty() {
                        RichText::new(crate::i18n::trf("Pad {}\nempty", &[&(i + 1)])).color(t.text_dim)
                    } else {
                        RichText::new(format!("{}\n{}", i + 1, file_name(&path))).strong()
                    };
                    let label = if own_window { label.size((pad.y / 4.0).clamp(14.0, 28.0)) } else { label };
                    let r = ui
                        .add_enabled(!path.is_empty(), egui::Button::new(label).min_size(pad))
                        .on_hover_text(crate::i18n::tr(
                            "Click: play it now (while recording or playing at the playhead, else at the insertion point) · drop an audio file here to load the pad",
                        ))
                        .on_disabled_hover_text(crate::i18n::tr("Drop an audio file here, or set a path"));
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
                            let te = ui.add(egui::TextEdit::singleline(text).desired_width(112.0).hint_text(crate::i18n::tr("/path/to/jingle.wav")));
                            if te.lost_focus() {
                                store = Some((i, text.clone()));
                            }
                        }
                        _ => {
                            ui.horizontal(|ui| {
                                if let Some(pick) = &app.services.pick_open
                                    && ui.small_button(crate::i18n::tr("Browse…")).on_hover_text(crate::i18n::tr("Choose an audio file")).clicked()
                                    && let Some(p) = pick("jingle", &[])
                                {
                                    store = Some((i, p));
                                }
                                if ui.small_button(crate::i18n::tr("Path")).on_hover_text(crate::i18n::tr("Type or paste a path")).clicked() {
                                    w.editing = Some((i, path.clone()));
                                }
                            });
                        }
                    }
                });
                if i % 4 == 3 {
                    ui.end_row();
                }
            }
        });
        ui.label(
            RichText::new(crate::i18n::tr("Pads are saved with the session. The jingle track is the one locked against recording."))
                .small()
                .color(t.text_dim),
        );
    });
    if let Some((i, path)) = store {
        app.extra.jingles.editing = None;
        set_pad(app, i, &path);
    }
    if let Some(i) = place {
        fire(app, i + 1);
    }
}

/// Plays pad `pad` (1..8): during a take or playback at the playhead, so it sounds now and is in
/// the episode, else at the insertion point. Remote guests hear it too. False when the pad is empty.
pub fn fire(app: &mut SoundApp, pad: usize) -> bool {
    let path = app.engine.session().podcast.jingles.get(pad.wrapping_sub(1)).cloned().unwrap_or_default();
    if path.trim().is_empty() {
        return false;
    }
    let at = if app.is_playing() { app.position() } else { app.engine.session().edit.selection.start };
    match app.run("podcast.place_jingle", json!({"pad": pad, "at": at})) {
        Ok(_) => {
            crate::remote::announce_jingle(app, pad);
            app.ui.status = crate::i18n::trf("Placed {}", &[&file_name(&path)]);
        }
        Err(e) => app.ui.status = e,
    }
    true
}
