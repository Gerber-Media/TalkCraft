//! Palettes: small tool windows (Jingles, Run Sheet, Remote Guests) that live inside the main
//! window or, detached, in a window of their own that can go to a second screen or a touchscreen.
//! Whether each is open, detached and where it was is kept in the settings; Setup › Preferences ›
//! General resets them all.

use crate::SoundApp;
use crate::i18n::tr;
use egui::{RichText, vec2};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PaletteState {
    pub open: bool,
    /// In a window of its own.
    pub detached: bool,
    /// Inside the main window: position and content size (points).
    pub rect: Option<[f32; 4]>,
    /// Its own window: position on the desktop and inner size.
    pub outer: Option<[f32; 4]>,
}

/// The palettes, with their command and title.
pub const PALETTES: &[(&str, &str)] =
    &[("jingles", "Jingles"), ("rundown", "Run Sheet"), ("remote", "Remote Guests"), ("onair", "On Air"), ("remote_control", "Remote Control")];

pub fn state(app: &SoundApp, id: &str) -> PaletteState {
    app.ui.palettes.get(id).cloned().unwrap_or_default()
}

fn state_mut<'a>(app: &'a mut SoundApp, id: &str) -> &'a mut PaletteState {
    app.ui.palettes.entry(id.to_string()).or_default()
}

pub fn is_open(app: &SoundApp, id: &str) -> bool {
    app.ui.palettes.get(id).is_some_and(|s| s.open)
}

/// `window.<id>`: opens or closes a palette (`value` sets it).
pub fn toggle(app: &mut SoundApp, id: &str, p: &Value) -> Value {
    let s = state_mut(app, id);
    s.open = p.get("value").and_then(Value::as_bool).unwrap_or(!s.open);
    json!({"value": s.open})
}

/// `ui.palette {palette, detached?}`: puts a palette in a window of its own (or back).
/// `ui.palette {reset: true}`: every palette back into the main window, at its default place.
pub fn command(app: &mut SoundApp, p: &Value) -> Result<Value, String> {
    if p.get("reset").and_then(Value::as_bool) == Some(true) {
        reset(app);
        return Ok(json!({"reset": true}));
    }
    let id = p.get("palette").and_then(Value::as_str).ok_or("`palette` required (jingles, rundown, remote, onair, remote_control)")?;
    if !PALETTES.iter().any(|(p, _)| *p == id) {
        return Err(format!("unknown palette `{id}` (jingles, rundown, remote, onair, remote_control)"));
    }
    let s = state_mut(app, id);
    if let Some(d) = p.get("detached").and_then(Value::as_bool) {
        s.detached = d;
        s.open = true;
    }
    Ok(serde_json::to_value(&*s).unwrap_or(Value::Null))
}

/// Every palette back into the main window at its default place and size (open ones stay open).
pub fn reset(app: &mut SoundApp) {
    for s in app.ui.palettes.values_mut() {
        *s = PaletteState { open: s.open, ..Default::default() };
    }
    app.ui.palette_layout = app.ui.palette_layout.wrapping_add(1);
    app.extra.palette_start.clear();
}

/// Shows palette `id` where it lives; `body` draws its contents (it gets the whole app, so it can
/// act directly). `detached` tells the body it has a window of its own (bigger buttons fit).
pub fn show(
    app: &mut SoundApp,
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    size: egui::Vec2,
    mut body: impl FnMut(&mut SoundApp, &mut egui::Ui, bool),
) {
    let st = state(app, id);
    if !st.open {
        return;
    }
    if st.detached && !ctx.embed_viewports() {
        detached(app, ctx, id, title, size, &mut body);
        return;
    }
    let mut open = true;
    let mut win =
        egui::Window::new(title).id(egui::Id::new(("palette", id, app.ui.palette_layout))).open(&mut open).resizable(true).default_size(size);
    // Each palette has its own first place, so they do not open on top of each other.
    let slot = PALETTES.iter().position(|(p, _)| *p == id).unwrap_or(0) as f32;
    win = win.default_pos([260.0 + slot * 48.0, 140.0 + slot * 40.0]);
    if let Some([x, y, w, h]) = st.rect {
        // Never larger than the screen, and never off it: a palette that once grew too big
        // comes back at a size that can be handled again.
        let screen = ctx.content_rect();
        let (w, h) = (w.min(screen.width() * 0.9).max(160.0), h.min(screen.height() * 0.9).max(80.0));
        let x = x.clamp(screen.min.x, (screen.max.x - 120.0).max(screen.min.x));
        let y = y.clamp(screen.min.y, (screen.max.y - 60.0).max(screen.min.y));
        win = win.default_pos([x, y]).default_size([w, h]);
    }
    let mut content = None;
    let shown = win.show(ctx, |ui| {
        content = Some(ui.max_rect().size());
        header(app, ui, id, false);
        body(app, ui, false);
    });
    if let (Some(r), Some(c)) = (shown, content) {
        let at = r.response.rect.min;
        state_mut(app, id).rect = Some([at.x, at.y, c.x, c.y]);
    }
    if !open {
        state_mut(app, id).open = false;
    }
}

/// The detach/attach switch at the top of a palette.
fn header(app: &mut SoundApp, ui: &mut egui::Ui, id: &str, detached: bool) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (label, tip) = if detached {
                ("⬋ ".to_string() + tr("Back to the main window"), tr("Puts the palette back into TalkCraft's window"))
            } else {
                ("⬈ ".to_string() + tr("Own window"), tr("A window of its own, e.g. for a second screen or a touchscreen"))
            };
            if ui.small_button(RichText::new(label).small()).on_hover_text(tip).clicked() {
                state_mut(app, id).detached = !detached;
            }
        });
    });
}

fn detached(
    app: &mut SoundApp,
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    size: egui::Vec2,
    body: &mut impl FnMut(&mut SoundApp, &mut egui::Ui, bool),
) {
    // The window is placed where it was only when it is created: afterwards it is the user's.
    let saved = state(app, id).outer;
    let start = *app.extra.palette_start.entry(id).or_insert(saved);
    let mut b = egui::ViewportBuilder::default().with_title(format!("{title} — TalkCraft")).with_min_inner_size([220.0, 140.0]).with_inner_size(size);
    if let Some([x, y, w, h]) = start {
        b = b.with_position([x, y]).with_inner_size([w, h]);
    }
    let vid = egui::ViewportId::from_hash_of(("palette", id));
    ctx.show_viewport_immediate(vid, b, |ui, _class| {
        let (close, outer, inner) = ui.ctx().input(|i| (i.viewport().close_requested(), i.viewport().outer_rect, i.viewport().inner_rect));
        egui::CentralPanel::default().show(ui, |ui| {
            header(app, ui, id, true);
            body(app, ui, true);
        });
        let s = state_mut(app, id);
        if let (Some(o), Some(i)) = (outer, inner) {
            s.outer = Some([o.min.x, o.min.y, i.width(), i.height()]);
        }
        if close {
            // Closing the window closes the palette; it opens in its own window again next time.
            s.open = false;
        }
    });
    if !state(app, id).open || !state(app, id).detached {
        app.extra.palette_start.remove(id);
    }
}

/// Setup › Preferences › General: the reset button.
pub fn preferences(app: &mut SoundApp, ui: &mut egui::Ui) {
    ui.label(RichText::new(tr("Palettes")).strong());
    ui.horizontal(|ui| {
        if ui
            .button(tr("Reset palettes to default"))
            .on_hover_text(tr(
                "Jingles, Run Sheet, Remote Guests, On Air and Remote Control go back into the main window, at their default place and size",
            ))
            .clicked()
        {
            reset(app);
        }
    });
    let detached: Vec<String> = PALETTES.iter().filter(|(id, _)| state(app, id).detached).map(|(_, t)| tr(t).to_string()).collect();
    if !detached.is_empty() {
        ui.label(
            RichText::new(crate::i18n::trf("In their own window: {}", &[&detached.join(", ")]))
                .small()
                .color(crate::theme::Tokens::current().text_dim),
        );
    }
    ui.add_space(vec2(0.0, 2.0).y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    #[test]
    fn palettes_detach_are_remembered_and_reset() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.run("window.jingles", json!({"value": true})).unwrap();
        app.run("ui.palette", json!({"palette": "jingles", "detached": true})).unwrap();
        state_mut(&mut app, "jingles").outer = Some([1900.0, 100.0, 600.0, 300.0]);
        assert!(app.run("ui.palette", json!({"palette": "mixer"})).is_err());
        // Kept in the settings file.
        let ui: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(ui.palettes.get("jingles"), app.ui.palettes.get("jingles"));
        assert!(ui.palettes["jingles"].detached && ui.palettes["jingles"].open);
        app.run("ui.palette", json!({"reset": true})).unwrap();
        let s = state(&app, "jingles");
        assert!(s.open && !s.detached && s.outer.is_none() && s.rect.is_none());
        assert_eq!(app.ui.palette_layout, 1);
    }
}
