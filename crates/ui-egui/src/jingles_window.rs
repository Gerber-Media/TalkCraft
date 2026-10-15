//! The Jingles palette: pads for intro, outro and stingers. A pad holds an audio file (drop one on
//! it or choose it) and, if wanted, a picture shown on the pad; clicking the pad plays it. + and −
//! add and remove pads, like tracks; the grip sorts them; a right-click on a pad offers the rest.

use crate::SoundApp;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use egui::{Color32, RichText, vec2};
use serde_json::{Value, json};
use soundcraft_engine::cmd::podcast::{MAX_JINGLE_PADS, pad_count};
use std::collections::HashMap;

/// Pictures on the pads, loaded once per file (`None`: could not be read).
#[derive(Clone, Default)]
struct Pictures(HashMap<String, Option<egui::TextureHandle>>);

/// Decoded jingles (`None`: the file could not be read).
#[derive(Clone, Default)]
struct Sounds(HashMap<String, Option<std::sync::Arc<soundcraft_playback::Audition>>>);

impl std::fmt::Debug for Sounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sounds({})", self.0.len())
    }
}

impl std::fmt::Debug for Pictures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pictures({})", self.0.len())
    }
}

#[derive(Debug, Clone, Default)]
pub struct JinglesWindow {
    pictures: Pictures,
    sort: crate::reorder::Reorder,
    /// The pad being renamed, and the name typed so far.
    renaming: Option<(usize, String)>,
    /// Decoded pad sounds for pressing a pad while stopped, by file.
    sounds: Sounds,
    /// The "Selection as Jingle" form, while open.
    pub export: Option<ExportForm>,
}

/// `window.jingles`: opens or closes the window.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    crate::palettes::toggle(app, "jingles", p)
}

/// Stores `value` at pad `i` (0-based) in one of the session's lists (`jingles` or `jingle_images`).
fn set_slot(app: &mut SoundApp, key: &str, i: usize, value: &str) {
    let pod = &app.engine.session().podcast;
    let mut list = match key {
        "jingles" => pod.jingles.clone(),
        "jingle_names" => pod.jingle_names.clone(),
        _ => pod.jingle_images.clone(),
    };
    if list.len() <= i {
        list.resize(i + 1, String::new());
    }
    if let Some(slot) = list.get_mut(i) {
        *slot = value.trim().to_string();
    }
    while list.last().is_some_and(String::is_empty) {
        list.pop();
    }
    // The pad count stays: emptying the last pad must not remove it.
    let pads = pad_count(pod) as u32;
    if let Err(e) = app.run("podcast.set", json!({key: list, "jingle_pads": pads})) {
        app.ui.status = e;
    }
}

fn is_picture(path: &str) -> bool {
    let ext = std::path::Path::new(path).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
}

/// The picture for a pad as a texture (scaled down: pads are small).
fn picture(ctx: &egui::Context, cache: &mut Pictures, path: &str) -> Option<egui::TextureHandle> {
    if path.trim().is_empty() {
        return None;
    }
    cache
        .0
        .entry(path.to_string())
        .or_insert_with(|| {
            let img = image::open(path).ok()?.thumbnail(512, 512).to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            Some(ctx.load_texture(format!("jingle-picture:{path}"), color, egui::TextureOptions::LINEAR))
        })
        .clone()
}

/// Selection as Jingle: what to call it, which pad (0 = the first empty one), loudness on/off.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportForm {
    pub name: String,
    pub pad: usize,
    pub match_loudness: bool,
    pub note: String,
}

/// `window.jingle_export`: opens the form (`value: false` closes it).
pub fn toggle_export(app: &mut SoundApp, p: &Value) -> Value {
    let open = p.get("value").and_then(Value::as_bool).unwrap_or(app.extra.jingles.export.is_none());
    app.extra.jingles.export = open.then(|| ExportForm { name: String::new(), pad: 0, match_loudness: true, note: String::new() });
    json!({"value": open})
}

/// The form: what will be exported, name, pad, loudness, Export.
fn export_window(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(mut form) = app.extra.jingles.export.clone() else { return };
    let t = Tokens::current();
    let s = app.engine.session();
    let selected: Vec<String> = s.edit.selected_tracks.iter().filter_map(|id| s.track(*id)).map(|t| t.name.clone()).collect();
    let sel = s.edit.selection;
    let what = if selected.is_empty() { tr("every track").to_string() } else { selected.join(", ") };
    let span = if sel.is_empty() { tr("from the first to the last clip").to_string() } else { format!("{:.1} s", s.sample_rate.seconds(sel.len())) };
    let pod = s.podcast.clone();
    let count = pad_count(&pod);
    let mut open = true;
    let mut go = false;
    egui::Window::new(tr("Selection as Jingle")).open(&mut open).collapsible(false).resizable(false).default_pos([460.0, 200.0]).show(ctx, |ui| {
        ui.label(RichText::new(trf("{} · {}", &[&what, &span])).color(t.text_dim));
        ui.label(
            RichText::new(tr(
                "Drop an audio file into TalkCraft, cut it with the usual tools, select the range (or just its track), then export it here.",
            ))
            .small()
            .color(t.text_dim),
        );
        ui.add_space(4.0);
        egui::Grid::new("jingle_export").num_columns(2).show(ui, |ui| {
            ui.label(tr("Name"));
            ui.add(egui::TextEdit::singleline(&mut form.name).hint_text(tr("the clip's name")).desired_width(220.0));
            ui.end_row();
            ui.label(tr("Pad"));
            let label = |p: usize| {
                if p == 0 {
                    tr("first empty pad").to_string()
                } else {
                    let n = soundcraft_engine::cmd::podcast::jingle_name(&pod, p - 1);
                    if n.is_empty() { trf("Pad {}", &[&p]) } else { trf("Pad {} (replaces {})", &[&p, &n]) }
                }
            };
            egui::ComboBox::from_id_salt("jingle_export_pad").selected_text(label(form.pad)).width(220.0).show_ui(ui, |ui| {
                for p in 0..=count {
                    ui.selectable_value(&mut form.pad, p, label(p));
                }
            });
            ui.end_row();
        });
        ui.checkbox(&mut form.match_loudness, tr("Match the loudness (−16 LUFS, like the episode)"));
        ui.add_space(4.0);
        if ui.add(egui::Button::new(RichText::new(tr("Export as Jingle")).strong()).fill(t.accent)).clicked() {
            go = true;
        }
        if !form.note.is_empty() {
            ui.label(RichText::new(&form.note).small());
        }
    });
    if go {
        let mut p = json!({"name": form.name, "loudness_lufs": if form.match_loudness { json!(-16.0) } else { Value::Null }});
        if form.pad > 0 {
            p["pad"] = json!(form.pad);
        }
        form.note = match app.run("podcast.export_jingle", p) {
            Ok(v) => {
                crate::palettes::toggle(app, "jingles", &json!({"value": true}));
                trf("On pad {}: {}", &[&v["pad"], &v["path"].as_str().unwrap_or_default()])
            }
            Err(e) => e,
        };
    }
    app.extra.jingles.export = open.then_some(form);
}

/// What a click or menu on the pads asks for.
enum Action {
    Fire(usize),
    Audio(usize, String),
    Picture(usize, String),
    RemovePad(usize),
    AddPad,
    Place(usize),
    ExportForm,
    Move(usize, usize),
    Rename(usize, String),
    Color(usize, Option<[u8; 3]>),
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    export_window(app, ctx);
    if !crate::palettes::is_open(app, "jingles") {
        return;
    }
    let t = Tokens::current();
    let pod = app.engine.session().podcast.clone();
    let count = pad_count(&pod);
    let mut actions: Vec<Action> = Vec::new();
    crate::palettes::show(app, ctx, "jingles", tr("Jingles"), vec2(560.0, 260.0), |app, ui, own_window| {
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
        let rows = count.div_ceil(4).max(1) as f32;
        let pad = if own_window {
            let w = ((ui.available_width() - 3.0 * 8.0) / 4.0).max(112.0);
            let h = ((ui.available_height() - rows * 38.0 - 50.0) / rows).clamp(54.0, 220.0);
            vec2(w, h)
        } else {
            vec2(112.0, 54.0)
        };
        let ctx = ui.ctx().clone();
        let w = &mut app.extra.jingles;
        egui::Grid::new("jingle_pads").num_columns(4).spacing(vec2(8.0, 8.0)).show(ui, |ui| {
            w.sort.places_for(count);
            let order = w.sort.order(count);
            for (k, &i) in order.iter().enumerate() {
                let path = pod.jingles.get(i).cloned().unwrap_or_default();
                let pic_path = pod.jingle_images.get(i).cloned().unwrap_or_default();
                let pic = picture(&ctx, &mut w.pictures, &pic_path);
                ui.vertical(|ui| {
                    let name = soundcraft_engine::cmd::podcast::jingle_name(&pod, i);
                    let text = if path.is_empty() && name.is_empty() {
                        trf("Pad {}\nempty", &[&(i + 1)])
                    } else {
                        format!("{}\n{}", i + 1, name)
                    };
                    let size = if own_window { (pad.y / 4.0).clamp(14.0, 28.0) } else { 13.0 };
                    // With a picture the label is drawn over it; the button itself stays empty.
                    let color = pod.jingle_colors.get(i).copied().flatten();
                    let fill = color.map(crate::theme::rgb);
                    let label = if pic.is_some() {
                        RichText::new("")
                    } else if let Some(f) = fill {
                        // The text follows the pad's colour: dark on light, light on dark.
                        RichText::new(&text).size(size).strong().color(crate::colors::ink_on(f))
                    } else if path.is_empty() {
                        RichText::new(&text).size(size).color(t.text_dim)
                    } else {
                        RichText::new(&text).size(size).strong()
                    };
                    let mut button = egui::Button::new(label).min_size(pad);
                    if let Some(f) = fill {
                        button = button.fill(if path.is_empty() { f.gamma_multiply(0.5) } else { f });
                    }
                    let r = ui
                        .add_enabled(!path.is_empty() || pic.is_some(), button)
                        .on_hover_text(tr("Click: play it now (while recording or playing it also goes into the episode at the playhead) · drop an audio file or a picture here · right-click for more"))
                        .on_disabled_hover_text(tr("Drop an audio file here, or set a path"));
                    if let Some(tex) = &pic {
                        let inner = r.rect.shrink(2.0);
                        // The picture fills the pad (cropped to its shape), the label sits on a dark band.
                        let tex_size = tex.size_vec2();
                        let scale = (inner.width() / tex_size.x).max(inner.height() / tex_size.y);
                        let shown = tex_size * scale;
                        let uv_w = (inner.width() / shown.x).min(1.0);
                        let uv_h = (inner.height() / shown.y).min(1.0);
                        let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), vec2(uv_w, uv_h));
                        let tint = if path.is_empty() { Color32::from_gray(120) } else { Color32::WHITE };
                        ui.painter().with_clip_rect(inner).image(tex.id(), inner, uv, tint);
                        let band = egui::Rect::from_min_max(egui::pos2(inner.min.x, inner.max.y - size * 1.6), inner.max);
                        ui.painter().rect_filled(band, 0.0, Color32::from_black_alpha(150));
                        let one_line = text.replace('\n', " · ");
                        ui.painter().with_clip_rect(band).text(
                            band.left_center() + vec2(6.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            one_line,
                            egui::FontId::proportional(size * 0.8),
                            Color32::WHITE,
                        );
                    }
                    w.sort.place(k, r.rect);
                    w.sort.mark(ui, i, r.rect);
                    // The grip in the pad's corner sorts it; the rest of the pad plays it.
                    let grip = egui::Rect::from_min_size(egui::pos2(r.rect.max.x - 18.0, r.rect.min.y + 3.0), crate::reorder::GRIP);
                    if count > 1 && pic.is_some() {
                        // On a picture the grip needs its own dark ground to be seen.
                        ui.painter().rect_filled(grip.expand(1.0), 3.0, Color32::from_black_alpha(150));
                    }
                    if count > 1
                        && let Some((from, to)) = w.sort.grip(ui, grip, i, "jingle")
                    {
                        actions.push(Action::Move(from, to));
                    }
                    if r.clicked() && !path.is_empty() {
                        actions.push(Action::Fire(i));
                    }
                    r.context_menu(|ui| {
                        if ui.button(tr("Rename…")).clicked() {
                            w.renaming = Some((i, name.clone()));
                            ui.close();
                        }
                        ui.menu_button(tr("Colour"), |ui| match crate::colors::menu(ui, color) {
                            Some(crate::colors::Choice::Color(c)) => {
                                actions.push(Action::Color(i, Some(c)));
                                ui.close();
                            }
                            Some(crate::colors::Choice::Reset) => {
                                actions.push(Action::Color(i, None));
                                ui.close();
                            }
                            None => {}
                        });
                        if let Some(pick) = &app.services.pick_open
                            && ui.button(tr("Choose audio file…")).clicked()
                        {
                            if let Some(p) = pick("jingle", &[]) {
                                actions.push(Action::Audio(i, p));
                            }
                            ui.close();
                        }
                        if let Some(pick) = &app.services.pick_open
                            && ui.button(tr("Choose picture…")).clicked()
                        {
                            if let Some(p) = pick("image", &["png", "jpg", "jpeg", "webp"]) {
                                actions.push(Action::Picture(i, p));
                            }
                            ui.close();
                        }
                        if !pic_path.is_empty() && ui.button(tr("Remove picture")).clicked() {
                            actions.push(Action::Picture(i, String::new()));
                            ui.close();
                        }
                        if !path.is_empty() && ui.button(tr("Place at the insertion point")).clicked() {
                            actions.push(Action::Place(i));
                            ui.close();
                        }
                        if !path.is_empty() && ui.button(tr("Empty pad")).clicked() {
                            actions.push(Action::Audio(i, String::new()));
                            ui.close();
                        }
                        ui.separator();
                        if count > 1 && ui.button(tr("Remove pad")).clicked() {
                            actions.push(Action::RemovePad(i));
                            ui.close();
                        }
                    });
                    for (pos, file) in &dropped {
                        if pos.is_some_and(|p| r.rect.contains(p)) {
                            actions.push(if is_picture(file) { Action::Picture(i, file.clone()) } else { Action::Audio(i, file.clone()) });
                        }
                    }
                    if let Some((at, typed)) = &mut w.renaming
                        && *at == i
                    {
                        // The name field where the buttons were; Enter or clicking elsewhere keeps it.
                        let te = ui.add(egui::TextEdit::singleline(typed).desired_width(pad.x).hint_text(tr("Name on the pad")));
                        if !te.has_focus() && !te.lost_focus() {
                            te.request_focus();
                        }
                        if te.lost_focus() {
                            let escape = ui.input(|inp| inp.key_pressed(egui::Key::Escape));
                            if !escape {
                                actions.push(Action::Rename(i, typed.clone()));
                            }
                            actions.push(Action::Rename(usize::MAX, String::new()));
                        }
                    } else {
                    ui.horizontal(|ui| {
                        if let Some(pick) = &app.services.pick_open
                            && ui.small_button(tr("Browse…")).on_hover_text(tr("Choose an audio file")).clicked()
                            && let Some(p) = pick("jingle", &[])
                        {
                            actions.push(Action::Audio(i, p));
                        }
                        if let Some(pick) = &app.services.pick_open
                            && ui.small_button(tr("Picture")).on_hover_text(tr("Show a picture on the pad (PNG or JPEG)")).clicked()
                            && let Some(p) = pick("image", &["png", "jpg", "jpeg", "webp"])
                        {
                            actions.push(Action::Picture(i, p));
                        }
                    });
                    }
                });
                if k % 4 == 3 {
                    ui.end_row();
                }
            }
        });
        ui.horizontal(|ui| {
            // Like the track list: + adds a pad, − removes the last one.
            if ui.add_enabled(count < MAX_JINGLE_PADS, egui::Button::new(" + ")).on_hover_text(tr("Add a pad")).clicked() {
                actions.push(Action::AddPad);
            }
            if ui.add_enabled(count > 1, egui::Button::new(" − ")).on_hover_text(tr("Remove the last pad")).clicked() {
                actions.push(Action::RemovePad(count - 1));
            }
            if ui.button(tr("From Selection…")).on_hover_text(tr("Cut a jingle in the timeline and put it on a pad")).clicked() {
                actions.push(Action::ExportForm);
            }
            ui.label(
                RichText::new(tr(
                    "Pads are saved with the session. Keys 1–8 fire the first eight. The jingle track is the one locked against recording.",
                ))
                .small()
                .color(t.text_dim),
            );
        });
    });
    for a in actions {
        match a {
            Action::Fire(i) => {
                fire(app, i + 1);
            }
            Action::Audio(i, p) => set_slot(app, "jingles", i, &p),
            Action::Picture(i, p) => set_slot(app, "jingle_images", i, &p),
            Action::ExportForm => {
                toggle_export(app, &json!({"value": true}));
            }
            Action::Place(i) => place(app, i + 1, app.engine.session().edit.selection.start),
            Action::AddPad => {
                if let Err(e) = app.run("podcast.jingle_pad", json!({"add": true})) {
                    app.ui.status = e;
                }
            }
            // usize::MAX: the name field closed.
            Action::Rename(usize::MAX, _) => app.extra.jingles.renaming = None,
            Action::Rename(i, name) => set_slot(app, "jingle_names", i, &name),
            Action::Color(i, c) => {
                let pod = &app.engine.session().podcast;
                let mut colors = pod.jingle_colors.clone();
                colors.resize(colors.len().max(i + 1), None);
                if let Some(slot) = colors.get_mut(i) {
                    *slot = c;
                }
                while colors.last().is_some_and(Option::is_none) {
                    colors.pop();
                }
                let pads = pad_count(pod) as u32;
                if let Err(e) = app.run("podcast.set", json!({"jingle_colors": colors, "jingle_pads": pads})) {
                    app.ui.status = e;
                }
            }
            Action::Move(from, to) => {
                if let Err(e) = app.run("podcast.jingle_move", json!({"from": from + 1, "to": to + 1})) {
                    app.ui.status = e;
                }
            }
            Action::RemovePad(i) => {
                if let Err(e) = app.run("podcast.jingle_pad", json!({"remove": i + 1})) {
                    app.ui.status = e;
                }
            }
        }
    }
}

/// Plays pad `pad` (1-based) now. During a take or playback it goes into the episode at the
/// playhead (and sounds from there); while stopped it is only heard, nothing changes. Remote guests
/// hear it too. False when the pad is empty.
pub fn fire(app: &mut SoundApp, pad: usize) -> bool {
    let path = app.engine.session().podcast.jingles.get(pad.wrapping_sub(1)).cloned().unwrap_or_default();
    if path.trim().is_empty() {
        return false;
    }
    if app.is_playing() || app.engine.transport.recording {
        let at = app.position();
        place(app, pad, at);
    } else {
        audition(app, pad, &path);
        crate::remote::announce_jingle(app, pad);
    }
    true
}

/// The jingle track's volume as a gain (the pad sounds as it will in the episode).
fn jingle_gain(app: &SoundApp) -> f32 {
    let s = app.engine.session();
    let db = soundcraft_engine::cmd::podcast::jingle_track(s).and_then(|t| s.track(t)).map_or(0.0, |t| t.mixer.volume_db);
    10f32.powf(db / 20.0)
}

/// Plays the pad's file now, beside the transport.
fn audition(app: &mut SoundApp, pad: usize, path: &str) {
    let name = soundcraft_engine::cmd::podcast::jingle_name(&app.engine.session().podcast, pad - 1);
    let gain = jingle_gain(app);
    let sound = app
        .extra
        .jingles
        .sounds
        .0
        .entry(path.to_string())
        .or_insert_with(|| {
            let bytes = std::fs::read(path).ok()?;
            let ext = std::path::Path::new(path).extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
            let (_, buf) = soundcraft_audio_io::decode(&bytes, ext.as_deref()).ok()?;
            Some(std::sync::Arc::new(soundcraft_playback::Audition { channels: buf.channels, sample_rate: buf.sample_rate, gain: 1.0 }))
        })
        .clone();
    match (sound, &app.player) {
        (Some(s), Some(p)) => {
            // The gain follows the jingle track's fader, so a copy with today's gain.
            let s = if (s.gain - gain).abs() > 1e-4 {
                let fresh = std::sync::Arc::new(soundcraft_playback::Audition { channels: s.channels.clone(), sample_rate: s.sample_rate, gain });
                app.extra.jingles.sounds.0.insert(path.to_string(), Some(fresh.clone()));
                fresh
            } else {
                s
            };
            p.audition(s);
            app.ui.status = trf("Playing {}", &[&name]);
        }
        (None, _) => app.ui.status = trf("Cannot read {}", &[&path]),
        (_, None) => app.ui.status = tr("No audio output").to_string(),
    }
}

/// Puts pad `pad`'s file into the episode at `at` (on the jingle track).
fn place(app: &mut SoundApp, pad: usize, at: soundcraft_time::Samples) {
    match app.run("podcast.place_jingle", json!({"pad": pad, "at": at})) {
        Ok(_) => {
            crate::remote::announce_jingle(app, pad);
            let name = soundcraft_engine::cmd::podcast::jingle_name(&app.engine.session().podcast, pad - 1);
            app.ui.status = trf("Placed {}", &[&name]);
        }
        Err(e) => app.ui.status = e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    #[test]
    fn pads_keep_their_count_when_the_last_one_is_emptied_and_take_pictures() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.engine.execute("session.new", &json!({"name": "J", "template": "podcast", "speakers": 1})).unwrap();
        app.run("podcast.jingle_pad", json!({"add": true})).unwrap();
        set_slot(&mut app, "jingles", 8, "/tmp/outro.wav");
        set_slot(&mut app, "jingle_images", 8, "/tmp/outro.png");
        set_slot(&mut app, "jingles", 8, "");
        let pod = &app.engine.session().podcast;
        assert_eq!(pad_count(pod), 9, "emptying pad 9 keeps it");
        assert_eq!(pod.jingle_images.get(8).map(String::as_str), Some("/tmp/outro.png"));
        assert!(is_picture("a/B.JPG") && !is_picture("a/b.wav"));
        // A pad gets its own name; the file stays.
        set_slot(&mut app, "jingles", 0, "/tmp/intro-v3-final.wav");
        set_slot(&mut app, "jingle_names", 0, "Intro");
        let pod = &app.engine.session().podcast;
        assert_eq!(soundcraft_engine::cmd::podcast::jingle_name(pod, 0), "Intro");
        assert_eq!(pod.jingles[0], "/tmp/intro-v3-final.wav");
        // Pressed while stopped: heard only, the episode stays as it was.
        let clips = |app: &SoundApp| app.engine.session().tracks.iter().map(|t| t.clips().len()).sum::<usize>();
        let before = clips(&app);
        let undo = app.engine.undo_history().len();
        assert!(app.run("jingles.fire", json!({"pad": 1})).is_ok());
        assert_eq!((clips(&app), app.engine.undo_history().len()), (before, undo), "nothing placed while stopped");
        assert!(app.run("jingles.fire", json!({"pad": 7})).is_err(), "an empty pad");
        // File › Export › Selection as Jingle opens the form.
        app.run("window.jingle_export", json!({})).unwrap();
        assert!(app.extra.jingles.export.as_ref().is_some_and(|f| f.match_loudness && f.pad == 0));
    }

    fn frame(ctx: &egui::Context, app: &mut SoundApp, events: Vec<egui::Event>) {
        let raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1000.0))), events, ..Default::default() };
        ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        })
        .textures_delta
        .clear();
    }

    #[test]
    fn dragging_a_pads_grip_sorts_it_and_the_others_make_room() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.engine.execute("session.new", &json!({"name": "J", "template": "podcast", "speakers": 1})).unwrap();
        app.engine.execute("podcast.set", &json!({"jingles": ["a.wav", "b.wav", "c.wav"]})).unwrap();
        app.run("window.jingles", json!({"value": true})).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        let place = |app: &SoundApp, k: usize| app.extra.jingles.sort.place_rect(k).unwrap();
        let pad3 = place(&app, 2);
        let grip = egui::pos2(pad3.max.x - 11.0, pad3.min.y + 12.0);
        let target = place(&app, 0).center();
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(grip)]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton { pos: grip, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
        );
        for step in 1..=6 {
            let at = grip + (target - grip) * (step as f32 / 6.0);
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)]);
        }
        // While dragging the pads already show the new order.
        assert_eq!(app.extra.jingles.sort.order(8)[0], 2, "pad 3 is shown first while dragged");
        assert_eq!(app.engine.session().podcast.jingles[0], "a.wav", "nothing changed before letting go");
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }],
        );
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.engine.session().podcast.jingles, vec!["c.wav", "a.wav", "b.wav"]);
    }
}
