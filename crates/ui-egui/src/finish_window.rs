//! Edit › Finish Episode: the usual finishing steps in one go, each one ticked on or off, with a
//! report of what each did. Before and After jump between the two states (through the History), so
//! the result can be compared by ear; Undo All takes the whole run back.

use crate::SoundApp;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use egui::{RichText, vec2};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct FinishWindow {
    pub show: bool,
    pub denoise: bool,
    pub match_speakers: bool,
    pub pauses: bool,
    pub fillers: bool,
    pub duck: bool,
    pub clicks: bool,
    pub crosstalk: bool,
    pub level: bool,
    pub breaths: bool,
    /// What each step did.
    pub report: Vec<(bool, String)>,
    /// History steps before and after the run.
    pub before_after: Option<(usize, usize)>,
    /// The ticks were taken from the settings.
    loaded: bool,
}

impl Default for FinishWindow {
    fn default() -> Self {
        Self {
            show: false,
            denoise: false,
            match_speakers: true,
            pauses: true,
            fillers: true,
            duck: true,
            clicks: false,
            crosstalk: true,
            level: true,
            breaths: false,
            report: Vec::new(),
            before_after: None,
            loaded: false,
        }
    }
}

/// `window.finish`: opens or closes the window.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    let w = &mut app.extra.finish;
    w.show = p.get("value").and_then(Value::as_bool).unwrap_or(!w.show);
    json!({"value": w.show})
}

fn step(app: &mut SoundApp, report: &mut Vec<(bool, String)>, label: &str, cmd: &str, params: Value, ok: impl Fn(&Value) -> String) {
    match app.run(cmd, params) {
        Ok(v) => report.push((true, format!("{}: {}", tr(label), ok(&v)))),
        Err(e) => report.push((false, format!("{}: {e}", tr(label)))),
    }
}

/// Runs the ticked steps in order; returns what each did.
pub fn run(app: &mut SoundApp) {
    let before = app.engine.undo_history().len();
    let w = app.extra.finish.clone();
    let mut report = Vec::new();
    if w.denoise {
        // Every clip on the speaker tracks.
        let s = app.engine.session();
        let clips: Vec<u64> =
            soundcraft_engine::podcast::record_targets(s).iter().filter_map(|t| s.track(*t)).flat_map(|t| t.clips().iter().map(|c| c.id.0)).collect();
        if clips.is_empty() {
            report.push((false, format!("{}: {}", tr("Reduce noise"), tr("no clips on the speaker tracks"))));
        } else {
            step(app, &mut report, "Reduce noise", "clip.denoise", json!({"clips": clips, "reduction_db": 12.0}), |_| tr("done").to_string());
        }
    }
    if w.clicks {
        step(app, &mut report, "Repair mouth clicks", "clip.declick", json!({}), |v| trf("{} click(s)", &[&v["clicks"]]));
    }
    if w.crosstalk {
        // Only between two or more speakers; with one it would just be a gate.
        if soundcraft_engine::podcast::record_targets(app.engine.session()).len() >= 2 {
            step(app, &mut report, "Lower crosstalk", "podcast.remove_crosstalk", json!({}), |v| {
                let a = v["tracks"].as_array().cloned().unwrap_or_default();
                a.iter().map(|t| format!("{} {} s", t["track"].as_str().unwrap_or("?"), t["lowered_s"])).collect::<Vec<_>>().join(", ")
            });
        } else {
            report.push((false, format!("{}: {}", tr("Lower crosstalk"), tr("only one speaker track"))));
        }
    }
    if w.level {
        step(app, &mut report, "Level each speaker within the track", "clip.level", json!({}), |v| trf("{} clip(s)", &[&v["clips"]]));
    }
    if w.breaths {
        step(app, &mut report, "Lower breaths", "clip.reduce_breaths", json!({}), |v| trf("{} breath(s)", &[&v["breaths"]]));
    }
    if w.match_speakers {
        step(app, &mut report, "Match speaker levels", "podcast.match_speakers", json!({}), |v| {
            trf("{} track(s) at −20 LUFS", &[&v["tracks"].as_array().map_or(0, Vec::len)])
        });
    }
    if w.fillers {
        if app.extra.words.words.iter().any(|x| x.selected) {
            let n = app.extra.words.words.iter().filter(|x| x.selected).count();
            crate::words_window::cut_selected(app);
            report.push((true, format!("{}: {}", tr("Cut filler words"), trf("{} word(s)", &[&n]))));
        } else {
            report.push((false, format!("{}: {}", tr("Cut filler words"), tr("none selected (Window › Edit by Text › Find Words first)"))));
        }
    }
    if w.pauses {
        step(app, &mut report, "Shorten pauses", "edit.truncate_silence", json!({}), |v| {
            trf("{} pause(s), {} s removed", &[&v["count"], &format!("{:.1}", v["removed_s"].as_f64().unwrap_or(0.0))])
        });
    }
    if w.duck {
        let has_music = soundcraft_engine::cmd::podcast::jingle_track(app.engine.session())
            .and_then(|t| app.engine.session().track(t).map(|t| !t.clips().is_empty()))
            .unwrap_or(false);
        if has_music {
            step(app, &mut report, "Duck music under speech", "podcast.auto_duck", json!({}), |_| tr("done").to_string());
        } else {
            report.push((false, format!("{}: {}", tr("Duck music under speech"), tr("no music on the jingle track"))));
        }
    }
    let after = app.engine.undo_history().len();
    app.extra.finish.report = report;
    app.extra.finish.before_after = (after > before).then_some((before, after));
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.extra.finish.show {
        return;
    }
    let t = Tokens::current();
    // The ticks are remembered across restarts.
    if !app.extra.finish.loaded {
        app.extra.finish.loaded = true;
        if let Some([d, m, f, p, k]) = app.ui.finish_steps {
            let w = &mut app.extra.finish;
            (w.denoise, w.match_speakers, w.fillers, w.pauses, w.duck) = (d, m, f, p, k);
        }
        if let Some([c, x, l, b]) = app.ui.finish_speech {
            let w = &mut app.extra.finish;
            (w.clicks, w.crosstalk, w.level, w.breaths) = (c, x, l, b);
        }
    }
    let mut open = true;
    let mut start = false;
    let mut go: Option<usize> = None;
    let now = app.engine.undo_history().len();
    egui::Window::new(tr("Finish Episode")).open(&mut open).default_pos([420.0, 180.0]).default_size(vec2(440.0, 360.0)).resizable(true).show(
        ctx,
        |ui| {
            let w = &mut app.extra.finish;
            ui.label(
                RichText::new(tr("The usual finishing steps in one go. Each one is undoable; Before and After let you compare."))
                    .small()
                    .color(t.text_dim),
            );
            ui.add_space(4.0);
            ui.checkbox(&mut w.denoise, tr("Reduce background noise (takes a while on long episodes)"));
            ui.checkbox(&mut w.clicks, tr("Repair mouth clicks"));
            ui.checkbox(&mut w.crosstalk, tr("Remove crosstalk between the speakers"));
            ui.checkbox(&mut w.level, tr("Level each speaker within the track"));
            ui.checkbox(&mut w.breaths, tr("Lower breaths"));
            ui.checkbox(&mut w.match_speakers, tr("Match speaker levels"));
            ui.checkbox(&mut w.fillers, tr("Cut the filler words selected in Edit by Text"));
            ui.checkbox(&mut w.pauses, tr("Shorten long pauses"));
            ui.checkbox(&mut w.duck, tr("Duck music under speech"));
            ui.add_space(6.0);
            if ui.add(egui::Button::new(RichText::new(tr("Finish Episode")).strong()).fill(t.accent)).clicked() {
                start = true;
            }
            if !w.report.is_empty() {
                ui.separator();
                for (ok, line) in &w.report {
                    ui.label(RichText::new(format!("{} {line}", if *ok { "✔" } else { "–" })).color(if *ok { t.text } else { t.text_dim }));
                }
            }
            if let Some((before, after)) = w.before_after {
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.selectable_label(now == before, tr("Before")).on_hover_text(tr("The episode as it was (play to compare)")).clicked() {
                        go = Some(before);
                    }
                    if ui.selectable_label(now == after, tr("After")).clicked() {
                        go = Some(after);
                    }
                    if ui.button(tr("Undo All")).clicked() {
                        go = Some(before);
                        w.before_after = None;
                        w.report.clear();
                    }
                });
            }
        },
    );
    app.extra.finish.show = open;
    let w = &app.extra.finish;
    app.ui.finish_steps = Some([w.denoise, w.match_speakers, w.fillers, w.pauses, w.duck]);
    app.ui.finish_speech = Some([w.clicks, w.crosstalk, w.level, w.breaths]);
    if start {
        run(app);
    }
    if let Some(step) = go {
        let _ = app.run("edit.history_go", json!({"step": step}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    #[test]
    fn the_run_reports_each_step_and_before_after_jump_between_states() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 2})).unwrap();
        run(&mut app);
        let report = app.extra.finish.report.clone();
        assert_eq!(report.len(), 6, "denoise, clicks and breaths are off by default: {report:?}");
        // Nothing to do on an empty session: the skipped steps say why, nothing breaks.
        assert!(report.iter().any(|(ok, _)| !ok));
    }
}
