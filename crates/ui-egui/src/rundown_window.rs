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
    sort: crate::reorder::Reorder,
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
    let mut reordered: Option<(usize, usize)> = None;
    let mut go: Option<usize> = None;
    crate::palettes::show(app, ctx, "rundown", tr("Run Sheet"), vec2(420.0, 380.0), |app, ui, _| {
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
        let mut moved: Option<(usize, usize)> = None;
        let sort = &mut app.extra.rundown.sort;
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            let n = items.len();
            sort.places_for(n);
            for (k, i) in sort.order(n).into_iter().enumerate() {
                let Some(item) = items.get_mut(i) else { continue };
                let row = ui.horizontal(|ui| {
                    // The grip sorts the topic (drag it up or down).
                    let (g, _) = ui.allocate_exact_size(crate::reorder::GRIP, egui::Sense::hover());
                    if n > 1
                        && let Some(m) = sort.grip(ui, g, i, "rundown")
                    {
                        moved = Some(m);
                    }
                    let here = current.is_some_and(|(c, _)| c == i);
                    ui.label(RichText::new(format!("{}.", i + 1)).color(if here { Color32::from_rgb(210, 60, 55) } else { t.text_dim }));
                    changed |= ui.add(egui::TextEdit::singleline(&mut item.title).desired_width(200.0).hint_text(tr("Topic"))).lost_focus();
                    changed |= ui.add(egui::DragValue::new(&mut item.minutes).range(0.0..=600.0).speed(0.5).suffix(" min")).changed();
                    if ui.small_button("▶").on_hover_text(tr("Start this topic now")).clicked() {
                        go = Some(i);
                    }
                    if ui.small_button("×").on_hover_text(tr("Remove this topic")).clicked() {
                        remove = Some(i);
                    }
                });
                sort.place(k, row.response.rect);
                sort.mark(ui, i, row.response.rect);
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
        if let Some((from, to)) = moved {
            let item = items.remove(from);
            items.insert(to, item);
            reordered = Some((from, to));
            changed = true;
        }
        if !live {
            ui.label(RichText::new(tr("The plan is saved with the session.")).small().color(t.text_dim));
        }
    });
    // The topic being talked about stays the current one wherever it moved.
    if let (Some((from, to)), Some((c, at))) = (reordered, app.extra.rundown.current) {
        let c = if c == from {
            to
        } else if from < c && c <= to {
            c - 1
        } else if to <= c && c < from {
            c + 1
        } else {
            c
        };
        app.extra.rundown.current = Some((c, at));
    }
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
/// `rundown.start {topic}` (1-based) starts a topic; `rundown.next` the next one. Each sets a
/// chapter marker named after the topic.
pub fn command(app: &mut SoundApp, id: &str, p: &Value) -> Result<Value, String> {
    let n = app.engine.session().podcast.rundown.len();
    if id == "rundown.next" {
        if !next_topic(app) {
            return Err("no next topic in the run sheet".into());
        }
    } else {
        let t = p
            .get("topic")
            .and_then(Value::as_u64)
            .map(|t| t as usize)
            .filter(|t| (1..=n).contains(t))
            .ok_or_else(|| format!("`topic` is 1 to {n}"))?;
        start_topic(app, t - 1);
    }
    Ok(json!({"topic": app.extra.rundown.current.map(|(i, _)| i + 1)}))
}

/// The topic being talked about: title, seconds spent on it, seconds planned (0 = open).
pub fn current_topic(app: &SoundApp) -> Option<(String, f64, f64)> {
    let (i, since) = app.extra.rundown.current?;
    let item = app.engine.session().podcast.rundown.get(i)?;
    let spent = app.engine.session().sample_rate.seconds((app.position() - since).max(0));
    let title = if item.title.trim().is_empty() { trf("Topic {}", &[&(i + 1)]) } else { item.title.clone() };
    Some((title, spent, f64::from(item.minutes) * 60.0))
}

/// The next topic's title, if the plan has one.
pub fn next_title(app: &SoundApp) -> Option<String> {
    let next = app.extra.rundown.current.map_or(0, |(i, _)| i + 1);
    let item = app.engine.session().podcast.rundown.get(next)?;
    Some(if item.title.trim().is_empty() { trf("Topic {}", &[&(next + 1)]) } else { item.title.clone() })
}

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
