//! The Subtitles window: every cue as an editable row (times, speaker, text), the cue under the
//! playhead highlighted and followed, and tools to add cues at the playhead, lay out empty cues
//! along the speech, transcribe it with whisper.cpp, import and export SRT/WebVTT.

use crate::SoundApp;
use crate::theme::{Tokens, mono};
use egui::{Color32, RichText, vec2};
use serde_json::{Value, json};
use soundcraft_engine::transcribe::{self, Job};
use soundcraft_model::SubtitleCue;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// A background program the window waits for.
#[derive(Debug)]
enum Task {
    /// whisper-cli transcribing the mix.
    Transcribe { job: Job, child: Child, start: Option<i64>, end: Option<i64> },
    /// curl downloading the speech model to `part`, renamed to `to` when done.
    Download { child: Child, part: PathBuf, to: PathBuf },
}

#[derive(Debug, Default)]
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
    task: Option<Task>,
}

/// A copy never owns the running program.
impl Clone for SubtitleEditor {
    fn clone(&self) -> Self {
        Self {
            show: self.show,
            follow: self.follow,
            drafts: self.drafts.clone(),
            filter: self.filter.clone(),
            selected: self.selected,
            initialized: self.initialized,
            task: None,
        }
    }
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

/// Starts transcribing the edit selection (or the whole session) in the background.
fn start_transcription(app: &mut SoundApp) -> Result<(), String> {
    let s = app.engine.session();
    let sel = s.edit.selection;
    let (start, end) = if sel.is_empty() { (None, None) } else { (Some(sel.start), Some(sel.end)) };
    let r = if sel.is_empty() { soundcraft_time::Range::new(0, s.content_end().max(1)) } else { sel };
    let language = soundcraft_engine::cmd::subtitles::transcribe_language(&app.engine, &json!({}));
    let job = transcribe::prepare(&app.engine, r, &language, None, "subtitle.transcribe").map_err(|e| e.to_string())?;
    match job.spawn() {
        Ok(child) => {
            app.extra.subtitles.task = Some(Task::Transcribe { job, child, start, end });
            Ok(())
        }
        Err(e) => {
            job.cleanup();
            Err(e.to_string())
        }
    }
}

/// Starts downloading the default speech model with curl.
fn start_download() -> Result<Task, String> {
    let dir = transcribe::default_model_dir().ok_or("no settings folder for the model")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let to = dir.join(transcribe::DEFAULT_MODEL);
    let part = dir.join(format!("{}.part", transcribe::DEFAULT_MODEL));
    let child = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&part)
        .arg(transcribe::DEFAULT_MODEL_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    Ok(Task::Download { child, part, to })
}

/// Size of the default model, for download progress.
const MODEL_BYTES: f32 = 574_041_195.0;

/// Checks the background task; applies a finished transcription.
fn poll(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(task) = app.extra.subtitles.task.as_mut() else { return };
    let child = match task {
        Task::Transcribe { child, .. } | Task::Download { child, .. } => child,
    };
    let done = match child.try_wait() {
        Ok(Some(status)) => Some(status.success()),
        Ok(None) => None,
        Err(_) => Some(false),
    };
    let Some(ok) = done else {
        ctx.request_repaint_after_secs(0.5);
        return;
    };
    match app.extra.subtitles.task.take() {
        Some(Task::Transcribe { job, start, end, .. }) => {
            let result = job.result(ok).map_err(|e| e.to_string()).and_then(|_| {
                let mut p = json!({"srt": job.srt().to_string_lossy(), "replace": true});
                if let (Some(a), Some(b)) = (start, end) {
                    p["start"] = json!(a);
                    p["end"] = json!(b);
                }
                app.run("subtitle.transcribe", p)
            });
            job.cleanup();
            app.ui.status = match result {
                Ok(v) => format!("Transcribed {} cue(s), {} with a speaker", v["created"], v["named"]),
                Err(e) => e,
            };
        }
        Some(Task::Download { part, to, .. }) => {
            app.ui.status = if ok && std::fs::rename(&part, &to).is_ok() {
                "Speech model downloaded — ready to transcribe".into()
            } else {
                let _ = std::fs::remove_file(&part);
                "The speech model could not be downloaded".into()
            };
        }
        None => {}
    }
}

/// The transcription row: start, progress and cancel, or what is missing.
fn transcription_ui(ui: &mut egui::Ui, ed: &mut SubtitleEditor, t: &Tokens, start: &mut bool, error: &mut Option<String>) {
    match &mut ed.task {
        Some(Task::Transcribe { job, .. }) => {
            let p = job.progress();
            ui.add(egui::ProgressBar::new(p).desired_width(200.0).text(format!(
                "Transcribing {:.0} min of audio… {:.0}%",
                job.seconds / 60.0,
                p * 100.0
            )));
        }
        Some(Task::Download { part, .. }) => {
            let got = std::fs::metadata(&*part).map_or(0, |m| m.len()) as f32;
            ui.add(
                egui::ProgressBar::new((got / MODEL_BYTES).clamp(0.0, 1.0))
                    .desired_width(200.0)
                    .text(format!("Downloading speech model… {:.0} MB", got / 1e6)),
            );
        }
        None => {}
    }
    if let Some(task) = &mut ed.task {
        if ui.button(crate::i18n::tr("Cancel")).clicked() {
            match task {
                Task::Transcribe { child, job, .. } => {
                    let _ = child.kill();
                    let _ = child.wait();
                    job.cleanup();
                }
                Task::Download { child, part, .. } => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(&*part);
                }
            }
            ed.task = None;
        }
        return;
    }
    if transcribe::find_cli().is_none() {
        ui.label(RichText::new(transcribe::install_hint()).color(t.text_dim));
    } else if transcribe::find_model(None).is_none() {
        if ui.button(format!("Download speech model (~{:.0} MB)", MODEL_BYTES / 1e6)).on_hover_text(transcribe::DEFAULT_MODEL_URL).clicked() {
            match start_download() {
                Ok(task) => ed.task = Some(task),
                Err(e) => *error = Some(e),
            }
        }
        ui.label(RichText::new("needed once for automatic transcription").color(t.text_dim));
    } else if ui
        .button(crate::i18n::tr("Transcribe"))
        .on_hover_text("Fill in subtitles from the speech with whisper.cpp — the edit selection, or the whole episode. Replaces the cues there (undoable); speakers come from the tracks.")
        .clicked()
    {
        *start = true;
    }
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    poll(app, ctx);
    if !app.extra.subtitles.show {
        return;
    }
    let mut transcribe_now = false;
    let mut error = None;
    let mut open = true;
    let mut run: Vec<(String, Value)> = Vec::new();
    let mut dialog: Option<&'static str> = None;
    let t = Tokens::current();
    let sr = app.engine.session().sample_rate;
    let now = app.position();
    let cues: Vec<SubtitleCue> = app.engine.session().subtitles.clone();
    let playing = app.engine.transport.playing;
    let screen = ctx.content_rect();
    egui::Window::new(crate::i18n::tr("Subtitles"))
        .open(&mut open)
        .default_size(vec2(760.0, 420.0))
        .default_pos(egui::pos2((screen.max.x - 800.0).max(16.0), (screen.max.y - 480.0).max(80.0)))
        .resizable(true)
        .show(ctx, |ui| {
            let ed = &mut app.extra.subtitles;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(crate::i18n::tr("+ Add at playhead"))
                    .on_hover_text("A 3 s cue at the playhead, or over the edit selection (Cmd+Ctrl+U)")
                    .clicked()
                {
                    run.push(("subtitle.add".into(), json!({})));
                }
                if ui
                    .button(crate::i18n::tr("Create cues from speech"))
                    .on_hover_text("Empty cues along the speech in the mix, ready to type into")
                    .clicked()
                {
                    run.push(("subtitle.segment".into(), json!({})));
                }
                if ui.button(crate::i18n::tr("Import…")).clicked() {
                    dialog = Some("subtitle.import");
                }
                if ui.add_enabled(!cues.is_empty(), egui::Button::new(crate::i18n::tr("Export…"))).clicked() {
                    dialog = Some("subtitle.export");
                }
                if ui
                    .add_enabled(!cues.is_empty(), egui::Button::new(crate::i18n::tr("Export Text…")))
                    .on_hover_text("The transcript as readable text (.txt) or Markdown (.md)")
                    .clicked()
                {
                    dialog = Some("subtitle.export_text");
                }
                ui.separator();
                ui.checkbox(&mut ed.follow, crate::i18n::tr("Follow playhead"));
                ui.add(egui::TextEdit::singleline(&mut ed.filter).hint_text("Search").desired_width(140.0));
                ui.label(RichText::new(format!("{} cue(s)", cues.len())).color(t.text_dim));
            });
            ui.horizontal(|ui| transcription_ui(ui, ed, &t, &mut transcribe_now, &mut error));
            ui.separator();
            if cues.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(crate::i18n::tr("No subtitles yet")).strong());
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
                            let num = if current {
                                num.color(Color32::from_rgb(250, 210, 90)).strong()
                            } else if selected {
                                num.color(ui.visuals().selection.stroke.color)
                            } else {
                                num.color(t.text_dim)
                            };
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
    if let Some(e) = error {
        app.ui.status = e;
    }
    if transcribe_now && let Err(e) = start_transcription(app) {
        app.ui.status = e;
    }
}
