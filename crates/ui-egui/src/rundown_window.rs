//! Window › Run Sheet: the episode's topics with their planned length. While recording it shows
//! the current topic large with the time spent on it; "Next topic" moves on and sets a chapter
//! marker named after the topic, so the chapters come out of the plan.

use crate::SoundApp;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use egui::{Color32, RichText, vec2};
use serde_json::{Value, json};
use soundcraft_model::RundownItem;

#[derive(Debug, Clone, Default)]
pub struct RundownWindow {
    /// The topic being talked about, and the timeline position where it began.
    current: Option<(usize, soundcraft_time::Samples)>,
}

/// `window.rundown`: opens or closes the window.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    crate::palettes::toggle(app, "rundown", p)
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn save(app: &mut SoundApp, items: &[RundownItem]) {
    let list: Vec<Value> = items.iter().map(|i| json!({"title": i.title, "minutes": i.minutes})).collect();
    if let Err(e) = app.run("podcast.set", json!({"rundown": list})) {
        app.ui.status = e;
    }
}

/// Moves to topic `i`: a chapter marker named after it at the playhead (or the insertion point).
pub fn start_topic(app: &mut SoundApp, i: usize) {
    let Some(item) = app.engine.session().podcast.rundown.get(i).cloned() else { return };
    let at = if app.is_playing() { app.position() } else { app.engine.session().edit.selection.start };
    let title = if item.title.trim().is_empty() { trf("Topic {}", &[&(i + 1)]) } else { item.title.clone() };
    match app.run("markers.add", json!({"at": at, "name": title})) {
        Ok(_) => app.extra.rundown.current = Some((i, at)),
        Err(e) => app.ui.status = e,
    }
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !crate::palettes::is_open(app, "rundown") {
        return;
    }
    let t = Tokens::current();
    let mut items = app.engine.session().podcast.rundown.clone();
    let sr = app.engine.session().sample_rate;
    let now = app.position();
    let live = app.engine.transport.recording || app.is_playing();
    let current = app.extra.rundown.current.filter(|(i, _)| *i < items.len());
    let mut changed = false;
    let mut go: Option<usize> = None;
    crate::palettes::show(app, ctx, "rundown", tr("Run Sheet"), vec2(420.0, 380.0), |_app, ui, _| {
        // Now: the current topic large, its time against the plan, and the button on.
        if let Some((i, since)) = current {
            let item = items.get(i).cloned().unwrap_or_default();
            let spent = sr.seconds((now - since).max(0));
            let planned = f64::from(item.minutes) * 60.0;
            let over = planned > 0.0 && spent > planned;
            ui.label(RichText::new(tr("Now")).small().color(t.text_dim));
            ui.label(RichText::new(&item.title).size(20.0).strong());
            let time = if planned > 0.0 { format!("{} / {}", clock(spent), clock(planned)) } else { clock(spent) };
            ui.label(RichText::new(time).monospace().size(16.0).color(if over { Color32::from_rgb(210, 60, 55) } else { t.text }));
            if over {
                ui.label(RichText::new(tr("Over the planned time")).small().color(Color32::from_rgb(210, 60, 55)));
            }
        } else {
            ui.label(RichText::new(tr("Start the first topic when the episode begins; each topic becomes a chapter.")).small().color(t.text_dim));
        }
        ui.horizontal(|ui| {
            let next = current.map_or(0, |(i, _)| i + 1);
            let label = items.get(next).map_or_else(|| tr("End of the plan").to_string(), |n| trf("Next topic ▶ {}", &[&n.title]));
            if ui
                .add_enabled(next < items.len(), egui::Button::new(label))
                .on_hover_text(tr("Sets a chapter marker named after the topic (Tab while recording)"))
                .clicked()
            {
                go = Some(next);
            }
        });
        ui.separator();
        // The plan: editable, with the total.
        let total: f32 = items.iter().map(|i| i.minutes).sum();
        ui.label(RichText::new(trf("Plan: {} topics, {} min", &[&items.len(), &format!("{total:.0}")])).strong());
        let mut remove: Option<usize> = None;
        let mut swap: Option<(usize, usize)> = None;
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            let n = items.len();
            for (i, item) in items.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let here = current.is_some_and(|(c, _)| c == i);
                    ui.label(RichText::new(format!("{}.", i + 1)).color(if here { Color32::from_rgb(210, 60, 55) } else { t.text_dim }));
                    changed |= ui.add(egui::TextEdit::singleline(&mut item.title).desired_width(200.0).hint_text(tr("Topic"))).lost_focus();
                    changed |= ui.add(egui::DragValue::new(&mut item.minutes).range(0.0..=600.0).speed(0.5).suffix(" min")).changed();
                    if ui.small_button("▶").on_hover_text(tr("Start this topic now")).clicked() {
                        go = Some(i);
                    }
                    if i > 0 && ui.small_button("↑").clicked() {
                        swap = Some((i, i - 1));
                    }
                    if i + 1 < n && ui.small_button("↓").clicked() {
                        swap = Some((i, i + 1));
                    }
                    if ui.small_button("✕").on_hover_text(tr("Remove this topic")).clicked() {
                        remove = Some(i);
                    }
                });
            }
        });
        if ui.button(tr("+ Topic")).clicked() {
            items.push(RundownItem { title: String::new(), minutes: 5.0 });
            changed = true;
        }
        if let Some(i) = remove {
            items.remove(i);
            changed = true;
        }
        if let Some((a, b)) = swap {
            items.swap(a, b);
            changed = true;
        }
        if !live {
            ui.label(RichText::new(tr("The plan is saved with the session.")).small().color(t.text_dim));
        }
    });
    if changed {
        save(app, &items);
    }
    if let Some(i) = go {
        start_topic(app, i);
    }
    if live && current.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// Tab while recording: the next topic.
pub fn next_topic(app: &mut SoundApp) -> bool {
    let n = app.engine.session().podcast.rundown.len();
    let next = app.extra.rundown.current.map_or(0, |(i, _)| i + 1);
    if n == 0 || next >= n {
        return false;
    }
    start_topic(app, next);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    #[test]
    fn topics_are_saved_and_each_next_topic_becomes_a_named_chapter() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 1})).unwrap();
        save(&mut app, &[RundownItem { title: "Begrüßung".into(), minutes: 2.0 }, RundownItem { title: "Energie".into(), minutes: 20.0 }]);
        assert_eq!(app.engine.session().podcast.rundown.len(), 2);
        assert!(app.run("podcast.set", json!({"rundown": [{"title": "x", "minutes": -1.0}]})).is_err(), "no negative plan");
        assert!(next_topic(&mut app));
        app.engine.execute("edit.select", &json!({"start": 48_000 * 60})).unwrap();
        assert!(next_topic(&mut app));
        assert!(!next_topic(&mut app), "end of the plan");
        let names: Vec<String> = app.engine.session().markers.iter().map(|m| m.name.clone()).collect();
        assert!(names.contains(&"Begrüßung".to_string()) && names.contains(&"Energie".to_string()), "{names:?}");
    }
}
