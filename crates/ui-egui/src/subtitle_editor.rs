//! The Subtitles window: every cue as an editable row (times, speaker, text), the cue under the
//! playhead highlighted and followed, and tools to add cues at the playhead, lay out empty cues
//! along the speech, import and export SRT/WebVTT.

use crate::SoundApp;
use crate::theme::{Tokens, mono};
use egui::{Color32, RichText, vec2};
use serde_json::{Value, json};
use soundcraft_model::SubtitleCue;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct SubtitleEditor {
    pub show: bool,
    /// Scroll the row under the playhead into view while playing.
    pub follow: bool,
    /// Typing in progress per cue id. Written to the session when the field loses focus, so a
    /// sentence is one undo step rather than one per key.
    drafts: BTreeMap<u64, Draft>,
    filter: String,
    /// The cue clicked last (the ruler highlights it too).
    pub selected: Option<u64>,
    initialized: bool,
}

#[derive(Debug, Clone, Default)]
struct Draft {
    /// The cue's speaker and text when the draft was taken; a change behind it (undo, import)
    /// replaces the draft.
    base: (String, String),
    speaker: String,
    text: String,
}

impl Draft {
    fn of(c: &SubtitleCue) -> Self {
        Self { base: (c.speaker.clone(), c.text.clone()), speaker: c.speaker.clone(), text: c.text.clone() }
    }
}

/// `m:ss.d` for compact time cells.
fn short_time(secs: f64) -> String {
    let secs = secs.max(0.0);
    let m = (secs / 60.0).floor();
    format!("{}:{:04.1}", m as u64, secs - m * 60.0)
}

/// Opens or closes the window (`window.subtitles`).
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    let ed = &mut app.extra.subtitles;
    ed.show = p.get("value").and_then(Value::as_bool).unwrap_or(!ed.show);
    if !ed.initialized {
        ed.follow = true;
        ed.initialized = true;
    }
    json!({"value": ed.show})
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.extra.subtitles.show {
        return;
    }
    let mut open = true;
    let mut run: Vec<(String, Value)> = Vec::new();
    let mut dialog: Option<&'static str> = None;
    let t = Tokens::current();
    let sr = app.engine.session().sample_rate;
    let now = app.position();
    let cues: Vec<SubtitleCue> = app.engine.session().subtitles.clone();
    let playing = app.engine.transport.playing;
    let screen = ctx.content_rect();
    egui::Window::new("Subtitles")
        .open(&mut open)
        .default_size(vec2(760.0, 420.0))
        .default_pos(egui::pos2((screen.max.x - 800.0).max(16.0), (screen.max.y - 480.0).max(80.0)))
        .resizable(true)
        .show(ctx, |ui| {
            let ed = &mut app.extra.subtitles;
            ui.horizontal_wrapped(|ui| {
                if ui.button("+ Add at playhead").on_hover_text("A 3 s cue at the playhead, or over the edit selection (Cmd+Ctrl+U)").clicked() {
                    run.push(("subtitle.add".into(), json!({})));
                }
                if ui.button("Create cues from speech").on_hover_text("Empty cues along the speech in the mix, ready to type into").clicked() {
                    run.push(("subtitle.segment".into(), json!({})));
                }
                if ui.button("Import…").clicked() {
                    dialog = Some("subtitle.import");
                }
                if ui.add_enabled(!cues.is_empty(), egui::Button::new("Export…")).clicked() {
                    dialog = Some("subtitle.export");
                }
                ui.separator();
                ui.checkbox(&mut ed.follow, "Follow playhead");
                ui.add(egui::TextEdit::singleline(&mut ed.filter).hint_text("Search").desired_width(140.0));
                ui.label(RichText::new(format!("{} cue(s)", cues.len())).color(t.text_dim));
            });
            ui.separator();
            if cues.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("No subtitles yet").strong());
                    ui.label(
                        RichText::new("Add a cue at the playhead, create empty cues from the speech, or import an .srt / .vtt file.")
                            .color(t.text_dim),
                    );
                });
                return;
            }
            let filter = ed.filter.to_lowercase();
            // Fixed columns; the text column takes what is left.
            const NUM_W: f32 = 30.0;
            const TIME_W: f32 = 62.0;
            const SPEAKER_W: f32 = 90.0;
            const DEL_W: f32 = 26.0;
            ui.horizontal(|ui| {
                for (label, w) in [("#", NUM_W), ("Start", TIME_W), ("End", TIME_W), ("Speaker", SPEAKER_W), ("Text", 0.0)] {
                    let r = RichText::new(label).color(t.text_dim);
                    if w > 0.0 {
                        ui.add_sized([w, 16.0], egui::Label::new(r));
                    } else {
                        ui.label(r);
                    }
                }
            });
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 22.0).show(ui, |ui| {
                for (n, c) in cues.iter().enumerate() {
                    if !filter.is_empty() && !c.text.to_lowercase().contains(&filter) && !c.speaker.to_lowercase().contains(&filter) {
                        continue;
                    }
                    let current = now >= c.start && now < c.end;
                    let selected = ed.selected == Some(c.id);
                    let frame = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(2, 3)).corner_radius(4.0).fill(if current {
                        Color32::from_rgba_unmultiplied(200, 160, 60, 40)
                    } else if selected {
                        Color32::from_rgba_unmultiplied(70, 120, 200, 45)
                    } else {
                        Color32::TRANSPARENT
                    });
                    frame.show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            let num = RichText::new(format!("{}", n + 1)).font(mono(11.0));
                            let num = if current { num.color(Color32::from_rgb(250, 210, 90)).strong() } else { num.color(t.text_dim) };
                            let row = ui.add_sized([NUM_W, 22.0], egui::Button::selectable(selected, num));
                            if row.clicked() {
                                ed.selected = Some(c.id);
                                run.push(("transport.locate".into(), json!({"at": c.start})));
                            }
                            if current && playing && ed.follow {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            for (at, key) in [(c.start, "start"), (c.end, "end")] {
                                let b = ui
                                    .add_sized([TIME_W, 22.0], egui::Button::new(RichText::new(short_time(sr.seconds(at))).font(mono(11.0))))
                                    .on_hover_text(format!("Click: go here. Shift-click: set the {key} to the playhead"));
                                if b.clicked() {
                                    if ui.input(|i| i.modifiers.shift) {
                                        run.push(("subtitle.set".into(), json!({"id": c.id, key: now})));
                                    } else {
                                        run.push(("transport.locate".into(), json!({"at": at})));
                                    }
                                }
                            }
                            let draft = ed.drafts.entry(c.id).or_insert_with(|| Draft::of(c));
                            if draft.base.0 != c.speaker || draft.base.1 != c.text {
                                *draft = Draft::of(c);
                            }
                            let sp = ui.add(
                                egui::TextEdit::singleline(&mut draft.speaker)
                                    .id_salt(("sub_speaker", c.id))
                                    .hint_text("Speaker")
                                    .desired_width(SPEAKER_W),
                            );
                            let text_w = (ui.available_width() - DEL_W - 8.0).max(120.0);
                            let tx = ui.add(
                                egui::TextEdit::multiline(&mut draft.text)
                                    .id_salt(("sub_text", c.id))
                                    .hint_text("Type what is said…")
                                    .desired_rows(1)
                                    .desired_width(text_w),
                            );
                            if (sp.lost_focus() || tx.lost_focus()) && (draft.speaker != c.speaker || draft.text != c.text) {
                                run.push(("subtitle.set".into(), json!({"id": c.id, "speaker": draft.speaker, "text": draft.text})));
                            }
                            if tx.gained_focus() {
                                ed.selected = Some(c.id);
                            }
                            if ui.add_sized([DEL_W, 22.0], egui::Button::new("🗑")).on_hover_text("Delete this cue").clicked() {
                                run.push(("subtitle.delete".into(), json!({"id": c.id})));
                            }
                        });
                    });
                }
            });
            ui.label(
                RichText::new(
                    "Text is saved when you leave the field. Shift-click a time to set it to the playhead. Space plays when no field is focused.",
                )
                .small()
                .color(t.text_dim),
            );
        });
    app.extra.subtitles.show = open;
    // Forget drafts of deleted cues and drafts without changes (they are rebuilt from the session).
    app.extra.subtitles.drafts.retain(|id, d| cues.iter().any(|c| c.id == *id) && (d.speaker != d.base.0 || d.text != d.base.1));
    for (id, params) in run {
        match app.run(&id, params) {
            Ok(v) if id == "subtitle.segment" => app.ui.status = format!("Created {} empty cue(s) along the speech", v["created"]),
            Ok(v) if id == "subtitle.add" => app.extra.subtitles.selected = v["id"].as_u64(),
            Ok(_) => {}
            Err(e) => app.ui.status = e,
        }
    }
    if let Some(id) = dialog {
        let _ = app.dialogs.open_for_command(&app.engine, id);
    }
}
