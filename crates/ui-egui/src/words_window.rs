//! The Filler Words window: whisper writes down every word with its time (hesitations kept), the
//! words appear as running text, fillers (ähm, äh, uh …) marked; select words and cut them out of
//! every track at once.

use crate::SoundApp;
use crate::theme::Tokens;
use egui::{Color32, RichText, vec2};
use serde_json::{Value, json};
use soundcraft_engine::transcribe::{self, Job};
use soundcraft_time::Samples;
use std::process::Child;

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub start: Samples,
    pub end: Samples,
    pub text: String,
    pub filler: bool,
    pub selected: bool,
}

#[derive(Debug, Default)]
pub struct WordsWindow {
    pub show: bool,
    pub words: Vec<Word>,
    task: Option<(Job, Child)>,
    /// The word clicked last: Shift-click selects from it.
    anchor: Option<usize>,
    /// The pointer is over the text: Delete cuts the selected words instead of the timeline.
    pub wants_keys: bool,
}

/// A copy never owns the running whisper.
impl Clone for WordsWindow {
    fn clone(&self) -> Self {
        Self { show: self.show, words: self.words.clone(), task: None, anchor: self.anchor, wants_keys: false }
    }
}

/// `window.words`: opens or closes the window.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    let w = &mut app.extra.words;
    w.show = p.get("value").and_then(Value::as_bool).unwrap_or(!w.show);
    json!({"value": w.show})
}

/// The words of a `transcript.words` result.
pub fn parse_words(v: &Value) -> Vec<Word> {
    v["words"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|w| {
                    Some(Word {
                        start: w["start"].as_i64()?,
                        end: w["end"].as_i64()?,
                        text: w["text"].as_str()?.to_string(),
                        filler: w["filler"].as_bool().unwrap_or(false),
                        selected: w["filler"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The words left after cutting `cut` (sorted, merged) out of the timeline: cut words go, later
/// ones move earlier by what was removed before them.
pub fn after_cut(words: &[Word], cut: &[(Samples, Samples)]) -> Vec<Word> {
    words
        .iter()
        .filter(|w| !cut.iter().any(|(a, b)| w.start >= *a && w.end <= *b))
        .map(|w| {
            let before: Samples = cut.iter().filter(|(_, b)| *b <= w.start).map(|(a, b)| b - a).sum();
            Word { start: w.start - before, end: w.end - before, ..w.clone() }
        })
        .collect()
}

fn start(app: &mut SoundApp) -> Result<(), String> {
    let s = app.engine.session();
    let r = soundcraft_time::Range::new(0, s.content_end().max(1));
    let language = soundcraft_engine::cmd::subtitles::transcribe_language(&app.engine, &json!({}));
    let mut job = transcribe::prepare(&app.engine, r, &language, None, "transcript.words").map_err(|e| e.to_string())?;
    job.words = true;
    match job.spawn() {
        Ok(child) => {
            app.extra.words.task = Some((job, child));
            Ok(())
        }
        Err(e) => {
            job.cleanup();
            Err(e.to_string())
        }
    }
}

fn poll(app: &mut SoundApp, ctx: &egui::Context) {
    let Some((_, child)) = app.extra.words.task.as_mut() else { return };
    let done = match child.try_wait() {
        Ok(Some(st)) => Some(st.success()),
        Ok(None) => None,
        Err(_) => Some(false),
    };
    let Some(ok) = done else {
        ctx.request_repaint_after_secs(0.5);
        return;
    };
    let Some((job, _)) = app.extra.words.task.take() else { return };
    let result = job.result(ok).map_err(|e| e.to_string()).and_then(|_| app.run("transcript.words", json!({"srt": job.srt().to_string_lossy()})));
    job.cleanup();
    match result {
        Ok(v) => {
            app.extra.words.words = parse_words(&v);
            let n = app.extra.words.words.iter().filter(|w| w.filler).count();
            app.ui.status = crate::i18n::trf("Found {} words, {} filler(s) selected", &[&app.extra.words.words.len(), &n]);
        }
        Err(e) => app.ui.status = e,
    }
}

/// While playing, the text follows the spoken word (unless the pointer is scrolling the text).
fn app_follow(ui: &egui::Ui) -> bool {
    !ui.input(|i| i.pointer.any_down()) && ui.input(|i| i.smooth_scroll_delta.y == 0.0)
}

/// Cuts the selected words (padded a little into the pauses around them) from every track.
pub(crate) fn cut_selected(app: &mut SoundApp) {
    let sr = app.engine.session().sample_rate;
    let pad = sr.samples(0.02);
    let mut ranges: Vec<(Samples, Samples)> =
        app.extra.words.words.iter().filter(|w| w.selected).map(|w| ((w.start - pad).max(0), w.end + pad)).collect();
    if ranges.is_empty() {
        return;
    }
    ranges.sort_unstable();
    let mut merged: Vec<(Samples, Samples)> = Vec::new();
    for (a, b) in ranges {
        match merged.last_mut() {
            Some(m) if a <= m.1 => m.1 = m.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let params = json!({"ranges": merged.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>()});
    match app.run("edit.cut_ranges", params) {
        Ok(v) => {
            app.extra.words.words = after_cut(&app.extra.words.words, &merged);
            app.ui.status = crate::i18n::trf(
                "Cut {} word(s), {} s removed (undo with Cmd+Z)",
                &[&merged.len(), &format!("{:.1}", sr.seconds(v["removed"].as_i64().unwrap_or(0)))],
            );
        }
        Err(e) => app.ui.status = e,
    }
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    poll(app, ctx);
    if !app.extra.words.show {
        return;
    }
    let t = Tokens::current();
    let mut open = true;
    let mut find = false;
    let mut cut = false;
    let mut locate: Option<Samples> = None;
    let sr = app.engine.session().sample_rate;
    let screen = ctx.content_rect();
    let playing_at = app.is_playing().then(|| app.position());
    let mut hovered = false;
    let mut delete = false;
    egui::Window::new(crate::i18n::tr("Edit by Text"))
        .open(&mut open)
        .default_size(vec2(640.0, 380.0))
        .default_pos(egui::pos2((screen.max.x - 680.0).max(16.0), 140.0))
        .resizable(true)
        .show(ctx, |ui| {
            let w = &mut app.extra.words;
            ui.horizontal_wrapped(|ui| {
                if let Some((job, child)) = &mut w.task {
                    let p = job.progress();
                    ui.add(egui::ProgressBar::new(p).desired_width(220.0).text(format!("Listening for every word… {:.0}%", p * 100.0)));
                    if ui.button(crate::i18n::tr("Cancel")).clicked() {
                        let _ = child.kill();
                        let _ = child.wait();
                        job.cleanup();
                        w.task = None;
                    }
                } else if transcribe::missing(None).is_some() {
                    ui.label(RichText::new(transcribe::missing(None).unwrap_or_default()).color(t.text_dim));
                } else if ui.button(crate::i18n::tr("Find Words")).on_hover_text(crate::i18n::tr("Transcribes the episode word by word, keeping the ähms and uhs")).clicked() {
                    find = true;
                }
                let fillers = w.words.iter().filter(|x| x.filler).count();
                if ui.add_enabled(fillers > 0, egui::Button::new(crate::i18n::trf("Select all fillers ({})", &[&fillers]))).clicked() {
                    w.words.iter_mut().for_each(|x| x.selected = x.filler);
                }
                if ui.add_enabled(w.words.iter().any(|x| x.selected), egui::Button::new(crate::i18n::tr("Select none"))).clicked() {
                    w.words.iter_mut().for_each(|x| x.selected = false);
                }
                let n = w.words.iter().filter(|x| x.selected).count();
                let secs: f64 = w.words.iter().filter(|x| x.selected).map(|x| sr.seconds(x.end - x.start)).sum();
                let label = RichText::new(crate::i18n::trf("Cut {} word(s) · {} s", &[&n, &format!("{:.1}", secs)])).strong();
                if ui.add_enabled(n > 0, egui::Button::new(label).fill(t.accent)).on_hover_text(crate::i18n::tr("Removes them from every track at once; one undo step")).clicked() {
                    cut = true;
                }
            });
            ui.separator();
            if w.words.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(crate::i18n::tr("No words yet")).strong());
                    ui.label(RichText::new(crate::i18n::tr("Find Words writes down every word with its time. Hesitations are marked and preselected; click a word to change that, right-click to hear it.")).color(t.text_dim));
                });
                return;
            }
            // The word being spoken while playing.
            let speaking = playing_at.and_then(|at| w.words.iter().rposition(|x| x.start <= at && at < x.end + sr.samples(0.3)));
            let shift = ui.input(|i| i.modifiers.shift);
            let mut range: Option<(usize, usize)> = None;
            let area = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(3.0, 4.0);
                    for (i, word) in w.words.iter_mut().enumerate() {
                        let mut text = RichText::new(&word.text);
                        if word.filler {
                            text = text.color(Color32::from_rgb(240, 170, 60));
                        }
                        if word.selected {
                            text = text.strikethrough().color(Color32::from_rgb(255, 120, 120));
                        }
                        if speaking == Some(i) {
                            text = text.underline().strong();
                        }
                        let r = ui.add(egui::Button::new(text).frame(word.selected).fill(Color32::from_rgba_unmultiplied(160, 40, 40, 60)));
                        if speaking == Some(i) && app_follow(ui) {
                            r.scroll_to_me(Some(egui::Align::Center));
                        }
                        if r.clicked() {
                            match w.anchor {
                                Some(a) if shift => range = Some((a.min(i), a.max(i))),
                                _ => word.selected = !word.selected,
                            }
                            w.anchor = Some(i);
                        }
                        if r.secondary_clicked() {
                            locate = Some(word.start);
                        }
                        r.on_hover_text(crate::i18n::trf(
                            "{} s · click: select for cutting · Shift-click: select up to here · right-click: listen · Delete: cut",
                            &[&format!("{:.2}", sr.seconds(word.start))],
                        ));
                    }
                });
            });
            if let Some((a, b)) = range {
                for x in w.words.iter_mut().skip(a).take(b - a + 1) {
                    x.selected = true;
                }
            }
            hovered = area.inner_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()));
            delete = hovered && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        });
    app.extra.words.wants_keys = hovered && app.extra.words.words.iter().any(|x| x.selected);
    if delete {
        cut = true;
    }
    app.extra.words.show = open;
    if find && let Err(e) = start(app) {
        app.ui.status = e;
    }
    if cut {
        cut_selected(app);
    }
    if let Some(at) = locate {
        let from = (at - sr.samples(1.0)).max(0);
        // Play starts at the insertion point: put it a second before the word.
        let _ = app.run("transport.stop", json!({}));
        app.engine.session_mut().edit.selection = soundcraft_time::Range::point(from);
        let _ = app.run("transport.play", json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(start: Samples, end: Samples, text: &str) -> Word {
        Word { start, end, text: text.into(), filler: false, selected: false }
    }

    #[test]
    fn words_after_a_cut_move_up_and_cut_words_go() {
        let words = vec![w(0, 10, "a"), w(10, 20, "ähm"), w(20, 30, "b"), w(50, 60, "c")];
        let got = after_cut(&words, &[(10, 20), (40, 45)]);
        let spans: Vec<(Samples, Samples, &str)> = got.iter().map(|x| (x.start, x.end, x.text.as_str())).collect();
        assert_eq!(spans, [(0, 10, "a"), (10, 20, "b"), (35, 45, "c")]);
    }

    #[test]
    fn parsed_fillers_start_selected() {
        let v = json!({"words": [{"start": 0, "end": 5, "text": "Hallo", "filler": false}, {"start": 5, "end": 9, "text": "ähm", "filler": true}]});
        let words = parse_words(&v);
        assert_eq!(words.iter().map(|x| x.selected).collect::<Vec<_>>(), [false, true]);
    }
}
