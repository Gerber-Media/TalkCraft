//! Modal dialogs opened by menu-style invocations.

use crate::SoundApp;
use crate::theme::{Tokens, bold};
use egui::{Align2, vec2};
use serde_json::{Value, json};
use soundcraft_engine::Engine;
use soundcraft_model::{ClipId, Session, SourceId, TrackId};
use soundcraft_time::Samples;

#[derive(Debug, Clone)]
pub enum Dialog {
    NewTracks {
        count: u32,
        format: String,
        kind: String,
        timebase: String,
        name: String,
    },
    NewSession {
        name: String,
        sample_rate: u32,
        template: String,
        speakers: u32,
    },
    Bounce {
        path: String,
        format: String,
        bit_depth: String,
        normalize: bool,
        loudness: bool,
    },
    RenameTrack {
        id: TrackId,
        name: String,
    },
    RenameClip {
        id: ClipId,
        name: String,
    },
    RenameMarker {
        id: soundcraft_model::MarkerId,
        name: String,
    },
    ClearClips {
        clips: Vec<ClipId>,
        sources: Vec<SourceId>,
        message: String,
    },
    PathPrompt {
        cmd: String,
        title: String,
        path: String,
        key: String,
    },
    Number {
        cmd: String,
        title: String,
        key: String,
        value: f64,
        suffix: String,
    },
    TempoChange {
        at: Samples,
        bpm: f64,
    }, // from double-clicking the tempo ruler
    Spot(crate::spot::Spot),
    Fades {
        shape: String,
    },
    StripSilence {
        threshold: f64,
        min_ms: f64,
        pre_ms: f64,
        post_ms: f64,
    },
    Group {
        name: String,
        edit: bool,
        mix: bool,
        members: Vec<(u64, String, bool)>,
    },
    Session {
        frame_rate: String,
        bit_depth: String,
    },
    ScoreSetup {
        title: String,
        composer: String,
        bars_per_system: u32,
        show_track_names: bool,
    },
    SaveChanges {
        then: (String, Value),
    },
    ExportEpisode {
        path: String,
        settings: Box<soundcraft_model::PodcastSettings>,
    },
    /// Setup > Preferences: episode details, export defaults, audio and transcription.
    Preferences {
        settings: Box<soundcraft_model::PodcastSettings>,
    },
    ShortenPauses {
        threshold: f64,
        longer_ms: f64,
        keep_ms: f64,
        preview: Option<String>,
    },
    /// A notice the user must see (e.g. the interface was unplugged during a recording).
    Message {
        title: String,
        text: String,
    },
    /// File > Export > Audiogram Video: where to, and square or portrait.
    Audiogram {
        path: String,
        portrait: bool,
    },
    /// Delete/Backspace: remove the selected section (`clips` empty) or the selected clips.
    DeleteSection {
        start: Samples,
        end: Samples,
        tracks: Vec<u64>,
        clips: Vec<u64>,
        summary: String,
        close_gap: bool,
    },
}

#[derive(Default)]
pub struct Dialogs {
    pub open: Option<Dialog>,
    /// Runs once the session has been saved from `SaveChanges`.
    after_save: Option<(String, Value)>,
}

impl Dialogs {
    pub fn open_name(&self) -> Option<&'static str> {
        self.open.as_ref().map(|d| match d {
            Dialog::NewTracks { .. } => "new_tracks",
            Dialog::NewSession { .. } => "new_session",
            Dialog::Bounce { .. } => "bounce",
            Dialog::RenameTrack { .. } => "rename_track",
            Dialog::RenameClip { .. } => "rename_clip",
            Dialog::RenameMarker { .. } => "rename_marker",
            Dialog::ClearClips { .. } => "clear_clips",
            Dialog::PathPrompt { .. } => "path",
            Dialog::Number { .. } => "number",
            Dialog::TempoChange { .. } => "tempo_change",
            Dialog::Spot(_) => "spot",
            Dialog::Fades { .. } => "fades",
            Dialog::StripSilence { .. } => "strip_silence",
            Dialog::Group { .. } => "group",
            Dialog::Session { .. } => "session",
            Dialog::ScoreSetup { .. } => "score_setup",
            Dialog::SaveChanges { .. } => "save_changes",
            Dialog::ExportEpisode { .. } => "export_episode",
            Dialog::Preferences { .. } => "preferences",
            Dialog::ShortenPauses { .. } => "shorten_pauses",
            Dialog::DeleteSection { .. } => "delete_section",
            Dialog::Audiogram { .. } => "audiogram",
            Dialog::Message { .. } => "message",
        })
    }

    /// Delete/Backspace: asks before removing the edit selection's section, or the selected
    /// clips. Err says what to select first.
    pub fn open_delete(&mut self, e: &Engine) -> Result<(), String> {
        let s = e.session();
        let sel = s.edit.selection;
        let secs = |n: Samples| s.sample_rate.seconds(n);
        let names = |ids: &[TrackId]| ids.iter().filter_map(|t| s.track(*t)).map(|t| t.name.clone()).collect::<Vec<_>>().join(", ");
        if !sel.is_empty() && !s.edit.selected_tracks.is_empty() {
            let tracks = s.edit.selected_tracks.clone();
            let summary = format!("Delete {:.1} s ({} – {}) on {}?", secs(sel.len()), clock(secs(sel.start)), clock(secs(sel.end)), names(&tracks));
            self.open = Some(Dialog::DeleteSection {
                start: sel.start,
                end: sel.end,
                tracks: tracks.iter().map(|t| t.0).collect(),
                clips: Vec::new(),
                summary,
                close_gap: true,
            });
            return Ok(());
        }
        let clips = s.edit.selected_clips.clone();
        if !clips.is_empty() {
            let summary = if clips.len() == 1 {
                "Delete the selected clip? Its place stays silent.".to_string()
            } else {
                format!("Delete {} selected clips? Their places stay silent.", clips.len())
            };
            self.open = Some(Dialog::DeleteSection {
                start: 0,
                end: 0,
                tracks: Vec::new(),
                clips: clips.iter().map(|c| c.0).collect(),
                summary,
                close_gap: false,
            });
            return Ok(());
        }
        Err("Nothing to delete: drag over the audio with the Selector, or click a clip with the Grabber".into())
    }

    /// File > Export > Audiogram Video.
    pub fn open_audiogram(&mut self, e: &Engine) {
        let name = &e.session().name;
        self.open = Some(Dialog::Audiogram { path: format!("{}/{name} Audiogram.mp4", default_dir()), portrait: false });
    }

    /// A quote clip: a portrait audiogram (stories, reels) of the edit selection.
    pub fn open_quote_clip(&mut self, e: &Engine, at_s: f64) {
        let name = &e.session().name;
        let at = at_s.max(0.0) as u64;
        let label = if crate::i18n::german() { "Zitat" } else { "Quote" };
        self.open = Some(Dialog::Audiogram { path: format!("{}/{name} {label} {}m{:02}s.mp4", default_dir(), at / 60, at % 60), portrait: true });
    }

    pub fn open_rename_track(&mut self, id: TrackId, name: &str) {
        self.open = Some(Dialog::RenameTrack { id, name: name.to_string() });
    }

    pub fn open_rename_clip(&mut self, id: ClipId, name: &str) {
        self.open = Some(Dialog::RenameClip { id, name: name.to_string() });
    }

    pub fn open_rename_marker(&mut self, id: soundcraft_model::MarkerId, name: &str) {
        self.open = Some(Dialog::RenameMarker { id, name: name.to_string() });
    }

    /// Clip List › Clear: confirm before clips and files leave the session. False when there is nothing to clear.
    pub fn open_clear(&mut self, e: &Engine, clips: Vec<ClipId>, sources: Vec<SourceId>) -> bool {
        let Some(message) = clear_message(e.session(), &clips, &sources) else { return false };
        self.open = Some(Dialog::ClearClips { clips, sources, message });
        true
    }

    /// Open the dialog for a command; false when the command has no dialog.
    pub fn open_for_command(&mut self, e: &Engine, id: &str) -> bool {
        self.after_save = None;
        let home = default_dir();
        let name = e.session().name.clone();
        self.open = Some(match id {
            "clip.clear" => {
                let ed = &e.session().edit;
                return self.open_clear(e, ed.selected_clips.clone(), ed.selected_sources.clone());
            }
            "track.new" => {
                Dialog::NewTracks { count: 1, format: "Mono".into(), kind: "audio".into(), timebase: "samples".into(), name: "Audio".into() }
            }
            "file.export_podcast" => {
                let settings = e.session().podcast.clone();
                let ext = if settings.export.format == "aiff" { "aif".to_string() } else { settings.export.format.clone() };
                Dialog::ExportEpisode { path: format!("{home}/{name}.{ext}"), settings: Box::new(settings) }
            }
            "setup.preferences" | "setup.transcription_settings" => Dialog::Preferences { settings: Box::new(e.session().podcast.clone()) },
            "session.new" => Dialog::NewSession { name: "Episode".into(), sample_rate: 48_000, template: "podcast".into(), speakers: 2 },
            "file.bounce_mix" => Dialog::Bounce {
                // Beside the session when it has been saved, else in the default folder.
                path: format!(
                    "{}/{name} Bounce.wav",
                    e.path
                        .as_deref()
                        .and_then(|p| std::path::Path::new(p).parent())
                        .filter(|d| !d.as_os_str().is_empty())
                        .map_or(home.clone(), |d| d.display().to_string())
                ),
                format: "wav".into(),
                bit_depth: "24".into(),
                normalize: false,
                loudness: true,
            },
            "file.import_audio" => Dialog::PathPrompt { cmd: id.into(), title: "Import Audio".into(), path: home, key: "path".into() },
            "file.import_midi" => Dialog::PathPrompt { cmd: id.into(), title: "Import MIDI".into(), path: home, key: "path".into() },
            "session.open" => Dialog::PathPrompt { cmd: id.into(), title: "Open Session".into(), path: home, key: "path".into() },
            "session.save" if e.path.is_some() => return false,
            "session.save" | "session.save_as" | "session.save_copy" => Dialog::PathPrompt {
                cmd: if id == "session.save_copy" { id } else { "session.save_as" }.into(),
                title: "Save Session As".into(),
                // The browser has no folders: the name becomes the bundle file's name.
                path: if home.is_empty() {
                    format!("{name}.{}", soundcraft_engine::io::BUNDLE_EXTENSION)
                } else {
                    format!("{home}/{name}/{name}.scraft")
                },
                key: "path".into(),
            },
            "edit.repeat" => Dialog::Number { cmd: id.into(), title: "Repeat".into(), key: "count".into(), value: 2.0, suffix: "times".into() },
            "edit.shift" => {
                Dialog::Number { cmd: id.into(), title: "Shift (seconds)".into(), key: "by_seconds".into(), value: 1.0, suffix: "s".into() }
            }
            "event.transpose" => {
                Dialog::Number { cmd: id.into(), title: "Transpose".into(), key: "semitones".into(), value: 12.0, suffix: "semitones".into() }
            }
            "event.quantize" => {
                Dialog::Number { cmd: id.into(), title: "Quantize strength".into(), key: "strength".into(), value: 100.0, suffix: "%".into() }
            }
            "edit.fades_create" => Dialog::Fades { shape: "equal power".into() },
            "edit.truncate_silence" => Dialog::ShortenPauses { threshold: -45.0, longer_ms: 1200.0, keep_ms: 600.0, preview: None },
            "edit.strip_silence" => Dialog::StripSilence { threshold: -48.0, min_ms: 50.0, pre_ms: 5.0, post_ms: 20.0 },
            "track.group" => {
                let s = e.session();
                let members = s.tracks.iter().map(|t| (t.id.0, t.name.clone(), s.edit.selected_tracks.contains(&t.id))).collect();
                Dialog::Group { name: format!("Group {}", s.groups.len() + 1), edit: true, mix: true, members }
            }
            "setup.session" => Dialog::Session { frame_rate: e.session().frame_rate.label().into(), bit_depth: "24".into() },
            "file.score_setup" => {
                let s = soundcraft_engine::score::ScoreSetup::from_session(e.session());
                Dialog::ScoreSetup { title: s.title, composer: s.composer, bars_per_system: s.bars_per_system, show_track_names: s.show_track_names }
            }
            "clip.rename" => {
                let target = soundcraft_engine::cmd::clip_ids_param(e, &json!({})).first().and_then(|id| e.session().find_clip(*id));
                let Some((_, clip)) = target else { return false };
                Dialog::RenameClip { id: clip.id, name: clip.name.clone() }
            }
            _ => match path_dialog(id, &home, &name) {
                Some(d) => d,
                None => return false,
            },
        });
        true
    }
}

/// When `id` would close a session with unsaved changes, ask first; true when the question is
/// open instead. A take in progress is stopped first, so it counts as a change.
pub fn ask_to_save(app: &mut SoundApp, id: &str, params: &Value) -> bool {
    if !matches!(id, "session.new" | "session.open" | "session.close" | "session.revert" | "app.quit")
        || soundcraft_engine::find_command(id).is_some_and(|c| (c.enabled)(&app.engine).is_err())
    {
        return false;
    }
    if app.engine.transport.recording {
        app.stop_play();
    }
    if !app.engine.is_dirty() {
        return false;
    }
    app.dialogs.open = Some(Dialog::SaveChanges { then: (id.into(), params.clone()) });
    true
}

/// Runs `id`, unless it asks to save first (`ask_to_save`).
pub fn run_or_ask(app: &mut SoundApp, id: &str, params: Value) -> Result<Value, String> {
    if ask_to_save(app, id, &params) {
        return Ok(Value::Null);
    }
    app.run(id, params)
}

/// A path prompt for any command whose parameters start with a required `path` or `dir` (plus
/// Print Score, whose path is optional), with a default file name from the command.
pub fn takes_path(id: &str) -> bool {
    path_key(id).is_some()
}

fn path_key(id: &str) -> Option<&'static str> {
    let spec = soundcraft_engine::command_specs().iter().find(|c| c.id == id)?;
    if spec.params.starts_with("{path?") && id != "file.print_score" {
        None
    } else if spec.params.starts_with("{path") {
        Some("path")
    } else if spec.params.starts_with("{dir") {
        Some("dir")
    } else {
        None
    }
}

/// What a path field's Browse button opens.
#[derive(Debug, Clone, Copy)]
pub enum Pick<'a> {
    /// An existing file: (purpose, extensions; empty = by purpose).
    Open(&'a str, &'a [&'a str]),
    /// A file to write (the dialog suggests the field's name and filters by its extension).
    Save(&'a str),
    Folder,
}

/// A path text field with a Browse… button next to it (when the host offers file dialogs).
/// True when the path changed through the dialog.
pub fn path_field(ui: &mut egui::Ui, services: &crate::Services, path: &mut String, width: f32, pick: Pick, hint: &str) -> bool {
    let mut picked = false;
    ui.horizontal(|ui| {
        let browse_w = 90.0;
        ui.add(egui::TextEdit::singleline(path).hint_text(hint).desired_width((width - browse_w).max(80.0)));
        let chosen = match pick {
            Pick::Open(purpose, exts) => {
                services.pick_open.as_ref().and_then(|f| ui.button(crate::i18n::tr("Browse…")).clicked().then(|| f(purpose, exts)).flatten())
            }
            Pick::Save(purpose) => {
                services.pick_save.as_ref().and_then(|f| ui.button(crate::i18n::tr("Browse…")).clicked().then(|| f(purpose, path)).flatten())
            }
            Pick::Folder => services.pick_folder.as_ref().and_then(|f| ui.button(crate::i18n::tr("Browse…")).clicked().then(|| f(path)).flatten()),
        };
        if let Some(p) = chosen {
            *path = p;
            picked = true;
        }
    });
    picked
}

fn path_dialog(id: &str, home: &str, name: &str) -> Option<Dialog> {
    let spec = soundcraft_engine::command_specs().iter().find(|c| c.id == id)?;
    let key = path_key(id)?;
    let importing = id.contains("import") || id.contains("open");
    let ext = match id {
        "file.export_midi" => ".mid",
        "file.export_sibelius" => ".musicxml",
        "file.print_score" => ".svg",
        "file.export_clip_groups" => ".scgrp",
        "file.export_session_text" => ".txt",
        "subtitle.export_text" => " Transcript.txt",
        "subtitle.export" => ".srt",
        _ if id.contains("session") => ".scraft",
        _ => "",
    };
    let path = if importing || key == "dir" { format!("{home}/") } else { format!("{home}/{name}{ext}") };
    let title = format!("{} — {}", spec.menu.last().copied().unwrap_or("File"), spec.label.trim_end_matches("..."));
    Some(Dialog::PathPrompt { cmd: id.into(), title, path, key: key.into() })
}

fn clear_message(s: &Session, clips: &[ClipId], sources: &[SourceId]) -> Option<String> {
    let listed: std::collections::BTreeSet<ClipId> = clips.iter().copied().collect();
    let names: Vec<&str> = s.tracks.iter().flat_map(|t| t.clips()).filter(|c| listed.contains(&c.id)).map(|c| c.name.as_str()).collect();
    let files: Vec<_> = sources.iter().filter_map(|id| s.source(*id)).collect();
    let count = |n: usize, what: &str| if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") };
    let what = match (names.as_slice(), files.as_slice()) {
        ([], []) => return None,
        ([name], []) => crate::i18n::trf("the clip \"{}\"", &[&name]),
        ([], [f]) => crate::i18n::trf("the file \"{}\"", &[&f.name]),
        (c, []) => count(c.len(), "clip"),
        ([], f) => count(f.len(), "file"),
        (c, f) => crate::i18n::trf("{} and {}", &[&(count(c.len(), "clip")), &(count(f.len(), "file"))]),
    };
    let mut text = format!("Remove {what} from the session?");
    let used: Vec<(&str, usize)> =
        files.iter().map(|f| (f.name.as_str(), s.clips_using(f.id).filter(|c| !listed.contains(&c.id)).count())).filter(|(_, n)| *n > 0).collect();
    let n: usize = used.iter().map(|u| u.1).sum();
    let users = if n == 1 {
        "a clip on the timeline, which is removed too.".to_string()
    } else {
        format!("{n} clips on the timeline, which are removed too.")
    };
    match used.as_slice() {
        [] => {}
        [_] if files.len() == 1 => text += &format!(" It is used by {users}"),
        u if u.len() == files.len() => text += &format!(" They are used by {users}"),
        [(name, _)] => text += &format!(" \"{name}\" is used by {users}"),
        many => text += &format!(" {} of the files are used by {users}", many.len()),
    }
    if !files.is_empty() && files.iter().all(|f| !f.unsaved) {
        text += if files.len() == 1 { " The file stays on disk." } else { " The files stay on disk." };
    }
    Some(text)
}

fn default_dir() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into())
    }
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
}

/// Show the open dialog, if any.
pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    audiosuite_window(app, ctx);
    crate::bounce::show(app, ctx);
    let Some(mut d) = app.dialogs.open.take() else { return };
    let mut keep = true;
    let mut action: Option<(String, Value)> = None;
    let mut save_first: Option<(String, Value)> = None;
    // Runs before `action` (the export dialog saves its settings into the session first).
    let mut before: Option<(String, Value)> = None;
    // Shorten Pauses › Preview: a dry run whose summary goes back into the dialog.
    let mut dry_run: Option<Value> = None;
    let confirmed = matches!(d, Dialog::SaveChanges { .. });
    let title = match &d {
        Dialog::NewTracks { .. } => "New Tracks",
        Dialog::NewSession { .. } => "New Session",
        Dialog::Bounce { .. } => "Bounce Mix",
        Dialog::RenameTrack { .. } => "Rename Track",
        Dialog::RenameClip { .. } => "Rename Clip",
        Dialog::RenameMarker { .. } => "Rename Marker",
        Dialog::ClearClips { .. } => "Clear",
        Dialog::PathPrompt { title, .. } | Dialog::Number { title, .. } => title.as_str(),
        Dialog::TempoChange { .. } => "Tempo Change (BPM)",
        Dialog::Spot(_) => "Spot",
        Dialog::Fades { .. } => "Fades",
        Dialog::StripSilence { .. } => "Strip Silence",
        Dialog::Group { .. } => "Create Group",
        Dialog::Session { .. } => "Session Setup",
        Dialog::ScoreSetup { .. } => "Score Setup",
        Dialog::SaveChanges { then } if then.0 == "session.revert" => "Revert to Saved",
        Dialog::SaveChanges { .. } => "Unsaved Changes",
        Dialog::ExportEpisode { .. } => "Export Episode",
        Dialog::Preferences { .. } => "Preferences",
        Dialog::ShortenPauses { .. } => "Shorten Pauses",
        Dialog::Audiogram { .. } => "Audiogram Video",
        Dialog::Message { title, .. } => title.as_str(),
        Dialog::DeleteSection { clips, .. } if !clips.is_empty() => "Delete Clips",
        Dialog::DeleteSection { .. } => "Delete Section",
    }
    .to_string();
    let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Window::new(egui::RichText::new(title.split(" — ").map(crate::i18n::tr).collect::<Vec<_>>().join(" — ")).font(bold(13.0)))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -80.0))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            match &mut d {
                Dialog::NewTracks { count, format, kind, timebase, name } => {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("Create"));
                        ui.add(egui::DragValue::new(count).range(1..=128));
                        ui.label(crate::i18n::tr("new"));
                        egui::ComboBox::from_id_salt("nt_fmt").selected_text(format.as_str()).show_ui(ui, |ui| {
                            for f in soundcraft_model::ChannelFormat::ALL {
                                ui.selectable_value(format, f.label().to_string(), f.label());
                            }
                        });
                        egui::ComboBox::from_id_salt("nt_kind")
                            .selected_text(soundcraft_model::TrackKind::from_id(kind).map_or("Audio Track", |k| k.label()))
                            .show_ui(ui, |ui| {
                                for k in soundcraft_model::TrackKind::ALL.into_iter().filter(|k| !crate::podcast::hidden_track_kind(*k)) {
                                    if ui.selectable_label(kind == k.id(), k.label()).clicked() {
                                        *kind = k.id().to_string();
                                        *name = k.default_name().to_string();
                                    }
                                }
                            });
                        ui.label("in");
                        egui::ComboBox::from_id_salt("nt_tb").selected_text(if timebase == "ticks" { "Ticks" } else { "Samples" }).show_ui(
                            ui,
                            |ui| {
                                ui.selectable_value(timebase, "samples".to_string(), crate::i18n::tr("Samples"));
                                ui.selectable_value(timebase, "ticks".to_string(), crate::i18n::tr("Ticks"));
                            },
                        );
                        ui.label(crate::i18n::tr("Name:"));
                        ui.add(egui::TextEdit::singleline(name).desired_width(140.0));
                    });
                    if buttons(ui, "Create", enter) {
                        action =
                            Some(("track.new".into(), json!({"count": count, "format": format, "kind": kind, "timebase": timebase, "name": name})));
                    }
                }
                Dialog::NewSession { name, sample_rate, template, speakers } => {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("Name"));
                        ui.text_edit_singleline(name);
                    });
                    egui::ComboBox::from_label(crate::i18n::tr("Sample Rate")).selected_text(format!("{sample_rate} Hz")).show_ui(ui, |ui| {
                        for r in soundcraft_time::SampleRate::COMMON {
                            ui.selectable_value(sample_rate, r, format!("{r} Hz"));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.radio_value(template, "podcast".to_string(), crate::i18n::tr("Podcast episode"));
                        ui.radio_value(template, "blank".to_string(), crate::i18n::tr("Empty session"));
                    });
                    if template == "podcast" {
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::tr("Speakers"));
                            ui.add(egui::DragValue::new(speakers).range(1..=8));
                        });
                        ui.label(
                            egui::RichText::new(crate::i18n::tr("One mono track per speaker with a voice chain (EQ, gate, compressor, de-esser), a jingle track and a limited master."))
                                .small()
                                .color(Tokens::current().text_dim),
                        );
                    }
                    if buttons(ui, "Create", enter) {
                        action = Some((
                            "session.new".into(),
                            json!({"name": name, "sample_rate": sample_rate, "template": template, "speakers": speakers}),
                        ));
                    }
                }
                Dialog::Bounce { path, format, bit_depth, normalize, loudness } => {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("File:").trim_end_matches(':'));
                        let before = path.clone();
                        path_field(ui, &app.services, path, 450.0, Pick::Save("file.bounce_mix"), "");
                        // A new file name sets the format from its extension (a picked .flac is FLAC).
                        if *path != before {
                            match std::path::Path::new(path.as_str()).extension().map(|x| x.to_string_lossy().to_lowercase()).as_deref() {
                                Some("wav") => *format = "wav".into(),
                                Some("aif" | "aiff") => *format = "aiff".into(),
                                Some("flac") => *format = "flac".into(),
                                _ => {}
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_label(crate::i18n::tr("Format")).selected_text(format.to_uppercase()).show_ui(ui, |ui| {
                            for f in ["wav", "aiff", "flac"] {
                                // A chosen format changes the file name's extension too.
                                if ui.selectable_value(format, f.to_string(), f.to_uppercase()).clicked() {
                                    let ext = if f == "aiff" { "aif" } else { f };
                                    *path = std::path::Path::new(path.as_str()).with_extension(ext).to_string_lossy().into_owned();
                                }
                            }
                        });
                        egui::ComboBox::from_label(crate::i18n::tr("Bit Depth")).selected_text(format!("{bit_depth}-bit")).show_ui(ui, |ui| {
                            for b in ["16", "24", "32f"] {
                                ui.selectable_value(bit_depth, b.to_string(), format!("{b}-bit"));
                            }
                        });
                    });
                    crate::widgets::checkbox(ui, loudness, crate::i18n::tr("Podcast loudness: -16 LUFS, -1 dBTP"));
                    ui.add_enabled_ui(!*loudness, |ui| crate::widgets::checkbox(ui, normalize, crate::i18n::tr("Normalize to -0.1 dBFS")));
                    ui.label(
                        egui::RichText::new(crate::i18n::tr("Bounces the edit selection, or the whole session when nothing is selected."))
                            .small()
                            .color(Tokens::current().text_dim),
                    );
                    if buttons(ui, "Bounce", enter) {
                        let p = std::path::Path::new(path.as_str()).with_extension(if format == "aiff" { "aif" } else { format.as_str() });
                        crate::bounce::start(
                            app,
                            &json!({"path": p.to_string_lossy(), "format": format, "bit_depth": bit_depth, "normalize": normalize, "loudness": loudness}),
                        );
                        keep = false;
                    }
                }
                Dialog::RenameTrack { id, name } => {
                    name_field(ui, name);
                    if buttons(ui, "OK", enter) {
                        action = Some(("track.rename".into(), json!({"track": id.0, "name": name})));
                    }
                }
                Dialog::RenameClip { id, name } => {
                    name_field(ui, name);
                    if buttons(ui, "OK", enter) {
                        action = Some(("clip.rename".into(), json!({"clip": id.0, "name": name})));
                    }
                }
                Dialog::RenameMarker { id, name } => {
                    name_field(ui, name);
                    if buttons(ui, "OK", enter) {
                        action = Some(("markers.edit".into(), json!({"id": id.0, "rename": name})));
                    }
                }
                Dialog::ClearClips { clips, sources, message } => {
                    ui.set_max_width(340.0);
                    ui.label(message.as_str());
                    if buttons(ui, "Remove", enter) {
                        action = Some(("clip.clear".into(), json!({"clips": clips, "sources": sources, "with_clips": true})));
                    }
                }
                Dialog::PathPrompt { cmd, path, key, .. } => {
                    let pick = if key == "dir" {
                        Pick::Folder
                    } else if cmd.contains("import") || cmd.contains("open") {
                        Pick::Open(cmd, &[])
                    } else {
                        Pick::Save(cmd)
                    };
                    path_field(ui, &app.services, path, 520.0, pick, "");
                    let file = soundcraft_engine::io::session_file(path);
                    let replaces = soundcraft_engine::io::replaces(&app.engine, cmd, &file);
                    if replaces {
                        ui.label(crate::i18n::trf("“{}” already exists. Do you want to replace it?", &[&file.file_name().unwrap_or_default().to_string_lossy()]));
                    }
                    if buttons(ui, if replaces { "Replace" } else { "OK" }, enter && !replaces) {
                        let mut v = json!({});
                        v[key.as_str()] = json!(path);
                        if replaces {
                            v["replace"] = json!(true);
                        }
                        action = Some((cmd.clone(), v));
                    }
                }
                Dialog::Number { cmd, key, value, suffix, .. } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(value).speed(0.1));
                        ui.label(suffix.as_str());
                    });
                    if buttons(ui, "OK", enter) {
                        let v = if key == "by_seconds" { json!({"by": {"seconds": value}}) } else { json!({key.as_str(): value}) };
                        action = Some((cmd.clone(), v));
                    }
                }
                Dialog::TempoChange { at, bpm } => {
                    ui.horizontal(|ui| {
                        // Same limits the engine enforces (time crate `valid_bpm`).
                        ui.add(egui::DragValue::new(bpm).speed(0.1).range(5.0..=1000.0));
                        ui.label("bpm");
                    });
                    if buttons(ui, "OK", enter) {
                        action = Some(tempo_change_action(*at, *bpm));
                    }
                }
                Dialog::Spot(d) => {
                    crate::spot::fields(ui, app.engine.session(), d);
                    if buttons(ui, "OK", enter) {
                        keep = false;
                        action = d.command(app.engine.session());
                    }
                }
                Dialog::Fades { shape } => {
                    for s in soundcraft_model::FadeShape::ALL {
                        ui.radio_value(shape, s.label().to_lowercase(), s.label());
                    }
                    if buttons(ui, "OK", enter) {
                        action = Some(("edit.fades_create".into(), json!({"shape": shape})));
                    }
                }
                Dialog::ShortenPauses { threshold, longer_ms, keep_ms, preview } => {
                    ui.add(egui::Slider::new(threshold, -80.0..=-20.0).text("Silence below dB"));
                    ui.add(egui::Slider::new(longer_ms, 300.0..=5000.0).text("Pauses longer than ms"));
                    *keep_ms = keep_ms.min(*longer_ms);
                    ui.add(egui::Slider::new(keep_ms, 0.0..=*longer_ms).text("Shorten them to ms"));
                    let params = json!({"threshold_db": threshold, "longer_than_ms": longer_ms, "keep_ms": keep_ms});
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("Preview")).clicked() {
                            let mut p = params.clone();
                            p["dry_run"] = json!(true);
                            dry_run = Some(p);
                        }
                        if let Some(text) = preview.as_deref() {
                            ui.label(egui::RichText::new(text).color(Tokens::current().text_dim));
                        }
                    });
                    ui.label(
                        egui::RichText::new(crate::i18n::tr("Works on all tracks at once (markers and subtitles move along). Applies to the edit selection, or the whole session; one undo step."))
                            .small()
                            .color(Tokens::current().text_dim),
                    );
                    if buttons(ui, "Shorten", enter) {
                        action = Some(("edit.truncate_silence".into(), params));
                    }
                }
                Dialog::Message { text, .. } => {
                    ui.label(text.as_str());
                    ui.separator();
                    if ui.button("OK").clicked() || enter {
                        keep = false;
                    }
                }
                Dialog::Audiogram { path, portrait } => {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("File:").trim_end_matches(':'));
                        path_field(ui, &app.services, path, 470.0, Pick::Save("audiogram"), "");
                    });
                    ui.horizontal(|ui| {
                        ui.radio_value(portrait, false, crate::i18n::tr("Square 1080 × 1080 (feed)"));
                        ui.radio_value(portrait, true, crate::i18n::tr("Portrait 1080 × 1920 (stories, reels)"));
                    });
                    let s = app.engine.session();
                    let r = crate::audiogram::range(s);
                    ui.label(
                        egui::RichText::new(crate::i18n::trf("{} s from {} — the edit selection, else the first minute. Cover, title and subtitles come from the episode.", &[&format!("{:.0}", s.sample_rate.seconds(r.len())), &clock(s.sample_rate.seconds(r.start))]))
                        .small()
                        .color(Tokens::current().text_dim),
                    );
                    if !crate::audiogram::available() {
                        ui.label(egui::RichText::new(crate::i18n::tr("Needs ffmpeg (macOS: brew install ffmpeg)")).color(Tokens::current().text_dim));
                    }
                    if buttons(ui, "Render", enter) && crate::audiogram::available() {
                        let p = std::path::Path::new(path.as_str()).with_extension("mp4").to_string_lossy().to_string();
                        if let Err(e) = start_audiogram(app, p, *portrait) {
                            app.ui.status = e;
                        }
                        keep = false;
                    }
                }
                Dialog::DeleteSection { start, end, tracks, clips, summary, close_gap } => {
                    ui.label(summary.as_str());
                    if clips.is_empty() {
                        ui.add_space(4.0);
                        ui.radio_value(close_gap, true, crate::i18n::tr("Close the gap on all tracks (later audio, markers and subtitles move up)"));
                        ui.radio_value(close_gap, false, crate::i18n::tr("Leave silence on the selected tracks"));
                    }
                    if buttons(ui, "Delete", enter) {
                        action = Some(if !clips.is_empty() {
                            before = Some(("edit.select".into(), json!({"clips": clips})));
                            ("edit.clear".into(), json!({}))
                        } else if *close_gap {
                            ("event.cut_time".into(), json!({"start": start, "end": end}))
                        } else {
                            ("edit.clear".into(), json!({"tracks": tracks, "start": start, "end": end}))
                        });
                    }
                }
                Dialog::StripSilence { threshold, min_ms, pre_ms, post_ms } => {
                    ui.add(egui::Slider::new(threshold, -96.0..=0.0).text("Threshold dB"));
                    ui.add(egui::Slider::new(min_ms, 0.0..=2000.0).text("Min strip duration ms"));
                    ui.add(egui::Slider::new(pre_ms, 0.0..=500.0).text("Clip start pad ms"));
                    ui.add(egui::Slider::new(post_ms, 0.0..=2000.0).text("Clip end pad ms"));
                    if buttons(ui, "Strip", enter) {
                        action = Some((
                            "edit.strip_silence".into(),
                            json!({"threshold_db": threshold, "min_length_ms": min_ms, "pad_before_ms": pre_ms, "pad_after_ms": post_ms}),
                        ));
                    }
                }
                Dialog::Group { name, edit, mix, members } => {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("Name"));
                        ui.text_edit_singleline(name);
                    });
                    ui.horizontal(|ui| {
                        crate::widgets::checkbox(ui, edit, crate::i18n::tr("Edit group"));
                        crate::widgets::checkbox(ui, mix, crate::i18n::tr("Mix group"));
                    });
                    ui.label(crate::i18n::tr("Tracks"));
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for (_, n, on) in members.iter_mut() {
                            crate::widgets::checkbox(ui, on, n.as_str());
                        }
                    });
                    if buttons(ui, "OK", enter) {
                        let ids: Vec<u64> = members.iter().filter(|m| m.2).map(|m| m.0).collect();
                        action = Some(("track.group".into(), json!({"name": name, "edit": edit, "mix": mix, "tracks": ids})));
                    }
                }
                Dialog::Session { frame_rate, bit_depth } => {
                    egui::ComboBox::from_label(crate::i18n::tr("Timecode Rate")).selected_text(frame_rate.as_str()).show_ui(ui, |ui| {
                        for r in soundcraft_time::FrameRate::ALL {
                            ui.selectable_value(frame_rate, r.label().to_string(), r.label());
                        }
                    });
                    egui::ComboBox::from_label(crate::i18n::tr("Bit Depth")).selected_text(bit_depth.as_str()).show_ui(ui, |ui| {
                        for b in ["16", "24", "32f"] {
                            ui.selectable_value(bit_depth, b.to_string(), b);
                        }
                    });
                    if buttons(ui, "OK", enter) {
                        action = Some(("setup.session".into(), json!({"frame_rate": frame_rate, "bit_depth": bit_depth})));
                    }
                }
                Dialog::Preferences { settings } => {
                    if preferences_ui(app, ui, settings, enter) {
                        action = Some(("podcast.set".into(), serde_json::to_value(settings.as_ref()).unwrap_or_default()));
                    }
                }
                Dialog::ExportEpisode { path, settings } => {
                    if export_episode_ui(ui, &app.services, Some(path), settings, enter) {
                        let s = serde_json::to_value(settings.as_ref()).unwrap_or_default();
                        before = Some(("podcast.set".into(), s));
                        action = Some(("file.export_podcast".into(), json!({"path": path})));
                    }
                }
                Dialog::ScoreSetup { title, composer, bars_per_system, show_track_names } => {
                    egui::Grid::new("score_setup").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                        ui.label(crate::i18n::tr("Title"));
                        ui.text_edit_singleline(title);
                        ui.end_row();
                        ui.label(crate::i18n::tr("Composer"));
                        ui.text_edit_singleline(composer);
                        ui.end_row();
                        ui.label(crate::i18n::tr("Bars per system"));
                        ui.add(egui::DragValue::new(bars_per_system).range(1..=16));
                        ui.end_row();
                    });
                    crate::widgets::checkbox(ui, show_track_names, crate::i18n::tr("Show track names"));
                    if buttons(ui, "OK", enter) {
                        action = Some((
                            "file.score_setup".into(),
                            json!({"title": title, "composer": composer, "bars_per_system": bars_per_system, "show_track_names": show_track_names}),
                        ));
                    }
                }
                Dialog::SaveChanges { then } => {
                    let name = &app.engine.session().name;
                    let revert = then.0 == "session.revert";
                    if revert || cfg!(target_arch = "wasm32") {
                        let (question, ok) = if revert {
                            (format!("Revert “{name}” to its last saved version?"), "Revert")
                        } else {
                            (format!("Close “{name}” without saving?"), "Don't Save")
                        };
                        ui.label(crate::i18n::trf("{} Your changes will be lost.", &[&question]));
                        if buttons(ui, ok, false) {
                            action = Some(then.clone());
                        }
                    } else {
                        ui.label(crate::i18n::trf("Save changes to “{}” before closing?", &[&name]));
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button(crate::i18n::tr("Don't Save")).clicked() {
                                action = Some(then.clone());
                            }
                            if buttons(ui, "Save", enter) {
                                save_first = Some(then.clone());
                            }
                        });
                    }
                }
            }
        });
    if esc || ctx.memory(|m| m.data.get_temp::<bool>(egui::Id::new("dlg_cancel")).unwrap_or(false)) {
        keep = false;
        app.dialogs.after_save = None;
        ctx.memory_mut(|m| m.data.remove::<bool>(egui::Id::new("dlg_cancel")));
    }
    if let Some(then) = save_first {
        keep = false;
        if !app.dialogs.open_for_command(&app.engine, "session.save") {
            action = Some(("session.save".into(), json!({})));
        }
        app.dialogs.after_save = Some(then);
    }
    if let Some((cmd, params)) = &action
        && !confirmed
        && ask_to_save(app, cmd, params)
    {
        keep = false;
        action = None;
    }
    if let Some((cmd, params)) = before
        && action.is_some()
        && let Err(e) = app.run(&cmd, params)
    {
        app.ui.status = e;
        action = None;
    }
    let saving = action.as_ref().is_some_and(|(cmd, _)| cmd.starts_with("session.save"));
    if let Some((cmd, params)) = action {
        keep = false;
        match app.run(&cmd, params) {
            Ok(v) if cmd == "edit.truncate_silence" => {
                app.ui.status = crate::i18n::trf(
                    "Shortened {} pause(s), {} s removed",
                    &[&(v["count"]), &format!("{:.1}", v["removed_s"].as_f64().unwrap_or(0.0))],
                );
            }
            Ok(v) if cmd == "file.export_podcast" => {
                app.ui.status = crate::i18n::trf(
                    "Exported {} ({} LUFS, {} dBTP, {} chapter(s))",
                    &[
                        &(v["path"].as_str().unwrap_or_default()),
                        &format!("{:.1}", v["lufs"].as_f64().unwrap_or(0.0)),
                        &format!("{:.1}", v["true_peak_db"].as_f64().unwrap_or(0.0)),
                        &(v["chapters"]),
                    ],
                );
            }
            Ok(v) if cmd == "file.export_clips" => {
                app.ui.status = crate::i18n::trf("Exported {} clip(s) to {}", &[&(v["written"]), &(v["dir"].as_str().unwrap_or_default())]);
            }
            Ok(_) if cmd == "edit.clear" && matches!(d, Dialog::DeleteSection { .. }) => {
                // Silence left in place: play on from the start of it rather than stopping at its end.
                let at = app.engine.session().edit.selection.start;
                app.engine.session_mut().edit.selection = soundcraft_time::Range::point(at);
            }
            Ok(_) => {}
            Err(e) => app.ui.status = e,
        }
    }
    if saving
        && let Some((then, params)) = app.dialogs.after_save.take()
        && !app.engine.is_dirty()
        && let Err(e) = app.run(&then, params)
    {
        app.ui.status = e;
    }
    if let Some(p) = dry_run
        && let Dialog::ShortenPauses { preview, .. } = &mut d
    {
        *preview = Some(match app.run("edit.truncate_silence", p) {
            Ok(v) => {
                crate::i18n::trf("{} pause(s), {} s would be removed", &[&(v["count"]), &format!("{:.1}", v["removed_s"].as_f64().unwrap_or(0.0))])
            }
            Err(e) => e,
        });
    }
    if keep {
        app.dialogs.open = Some(d);
    }
}

/// Cancel/OK row; returns true when OK was pressed (or Enter).
/// Gathers the audio, cues, title and cover, and starts rendering the audiogram in the background.
fn start_audiogram(app: &mut SoundApp, out: String, portrait: bool) -> Result<(), String> {
    let r = crate::audiogram::range(app.engine.session());
    let ch = soundcraft_engine::io::render_mix(&app.engine, r, "audiogram").map_err(|e| e.to_string())?;
    let n = ch.iter().map(Vec::len).min().unwrap_or(0);
    let audio: Vec<f32> = (0..n).map(|i| ch.iter().filter_map(|c| c.get(i)).sum::<f32>() / ch.len().max(1) as f32).collect();
    let s = app.engine.session();
    let ep = &s.podcast.episode;
    let title = match (ep.show.trim(), ep.title.trim()) {
        ("", "") => s.name.clone(),
        ("", t) => t.to_string(),
        (show, "") => show.to_string(),
        (show, t) => format!("{show} · {t}"),
    };
    let cover = Some(ep.cover.trim().to_string()).filter(|c| !c.is_empty());
    let cues = soundcraft_engine::subtitles::file_cues(s, r);
    let req = crate::audiogram::Request { out, portrait, title, cover, audio, rate: s.sample_rate.hz(), cues };
    app.audiogram = Some(crate::audiogram::start(req)?);
    app.ui.status = "Rendering audiogram…".into();
    Ok(())
}

/// `m:ss.s` for dialog texts.
fn clock(secs: f64) -> String {
    let secs = secs.max(0.0);
    let m = (secs / 60.0).floor();
    format!("{}:{:04.1}", m as u64, secs - m * 60.0)
}

fn buttons(ui: &mut egui::Ui, ok: &str, enter: bool) -> bool {
    let mut pressed = enter;
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(crate::i18n::tr("Cancel")).clicked() {
            ui.ctx().memory_mut(|m| m.data.insert_temp(egui::Id::new("dlg_cancel"), true));
        }
        if ui.add(egui::Button::new(egui::RichText::new(crate::i18n::tr(ok)).strong()).fill(Tokens::current().accent)).clicked() {
            pressed = true;
        }
    });
    pressed
}

/// A name field that takes the focus with its text selected, so typing replaces the old name.
fn name_field(ui: &mut egui::Ui, name: &mut String) {
    let mut out = egui::TextEdit::singleline(name).show(ui);
    if !out.response.has_focus() {
        out.response.request_focus();
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()));
        out.state.cursor.set_char_range(Some(all));
        out.state.store(ui.ctx(), out.response.id);
    }
}

/// The command the tempo-change dialog runs on OK.
fn tempo_change_action(at: Samples, bpm: f64) -> (String, Value) {
    ("event.tempo".into(), json!({"bpm": bpm, "at": at}))
}

fn audiosuite_window(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(process) = app.ui.audiosuite.clone() else { return };
    let mut open = true;
    let title = soundcraft_dsp::plugin_info(&process).map_or_else(|| process.replace('_', " "), |p| p.name.to_string());
    egui::Window::new(crate::i18n::trf("AudioSuite · {}", &[&title])).open(&mut open).default_width(360.0).show(ctx, |ui| {
        let key = egui::Id::new(("as_params", process.clone()));
        let mut params: std::collections::BTreeMap<String, f32> = ui.ctx().memory(|m| m.data.get_temp(key)).unwrap_or_default();
        if let Some(info) = soundcraft_dsp::plugin_info(&process) {
            for p in info.params {
                let v = params.entry(p.id.to_string()).or_insert(p.default);
                ui.add(egui::Slider::new(v, p.min..=p.max).text(p.name));
            }
        } else {
            match process.as_str() {
                "normalize" => {
                    let v = params.entry("target_db".into()).or_insert(-0.1);
                    ui.add(egui::Slider::new(v, -24.0..=0.0).text("Peak dB"));
                }
                "time_stretch" => {
                    let v = params.entry("ratio".into()).or_insert(1.0);
                    ui.add(egui::Slider::new(v, 0.25..=4.0).text("Length ratio"));
                }
                "pitch_shift" => {
                    let v = params.entry("semitones".into()).or_insert(0.0);
                    ui.add(egui::Slider::new(v, -24.0..=24.0).text("Semitones"));
                }
                "varispeed" => {
                    let v = params.entry("speed".into()).or_insert(1.0);
                    ui.add(egui::Slider::new(v, 0.25..=4.0).text("Speed"));
                }
                _ => {
                    ui.label(crate::i18n::tr("No parameters."));
                }
            }
        }
        ui.ctx().memory_mut(|m| m.data.insert_temp(key, params.clone()));
        let hint = if soundcraft_dsp::is_generator(&process) {
            "Fills the selection on the selected tracks with a new clip."
        } else {
            "Processes the selected clips and replaces them with rendered audio."
        };
        ui.label(egui::RichText::new(crate::i18n::tr(hint)).small());
        if ui.button(egui::RichText::new(crate::i18n::tr("Render")).strong()).clicked() {
            let p: serde_json::Map<String, Value> = params.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            let _ = app.run("audiosuite.process", json!({"process": process, "params": p}));
        }
    });
    if !open {
        app.ui.audiosuite = None;
    }
}

/// Loudness targets offered by the export dialog: (label, LUFS).
const LOUDNESS_PRESETS: [(&str, Option<f32>); 5] = [
    ("-16 LUFS (stereo standard: Apple, Spotify)", Some(-16.0)),
    ("-19 LUFS (mono standard)", Some(-19.0)),
    ("-14 LUFS (louder)", Some(-14.0)),
    ("-23 LUFS (EBU R128 broadcast)", Some(-23.0)),
    ("Off (keep the mix level)", None),
];

/// The Export Episode dialog: format, quality, loudness and ID3 metadata. True when Export is pressed.
/// The Export Episode dialog's body and its Export button.
fn export_episode_ui(
    ui: &mut egui::Ui,
    services: &crate::Services,
    path: Option<&mut String>,
    s: &mut soundcraft_model::PodcastSettings,
    enter: bool,
) -> bool {
    let dim = Tokens::current().text_dim;
    export_settings_ui(ui, services, path, s, false);
    ui.label(
        egui::RichText::new(crate::i18n::tr("Exports the edit selection, or the whole session. Settings are saved with the session."))
            .small()
            .color(dim),
    );
    buttons(ui, "Export", enter && !ui.ctx().egui_wants_keyboard_input())
}

/// Setup > Preferences: everything a podcast session keeps, plus where audio and transcription
/// stand. True when Save was pressed.
/// The Settings' tabs: the episode (saved with the session) and the app's own settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefTab {
    Episode,
    General,
    Appearance,
    Audio,
    Transcription,
}

fn preferences_ui(app: &mut SoundApp, ui: &mut egui::Ui, s: &mut soundcraft_model::PodcastSettings, enter: bool) -> bool {
    let dim = Tokens::current().text_dim;
    let tr = crate::i18n::tr;
    let id = egui::Id::new("preferences_tab");
    let mut tab = ui.data(|d| d.get_temp::<PrefTab>(id)).unwrap_or(PrefTab::Episode);
    // One size for every tab, so the dialog does not jump when switching.
    let height = (ui.ctx().content_rect().height() * 0.6).min(560.0);
    ui.set_min_width(520.0);
    ui.horizontal(|ui| {
        for (t, label) in [
            (PrefTab::Episode, "Episode"),
            (PrefTab::General, "General"),
            (PrefTab::Appearance, "Appearance"),
            (PrefTab::Audio, "Audio"),
            (PrefTab::Transcription, "Transcription"),
        ] {
            if ui.selectable_label(tab == t, tr(label)).clicked() {
                tab = t;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(id, tab));
    ui.separator();
    egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show(ui, |ui| match tab {
        PrefTab::Episode => {
            episode_ui(ui, &app.services, &mut s.episode);
            ui.separator();
            export_settings_ui(ui, &app.services, None, s, true);
            ui.label(egui::RichText::new(tr("Saved with the session.")).small().color(dim));
        }
        PrefTab::General => general_prefs(app, ui),
        PrefTab::Appearance => {
            crate::extra_windows::appearance_ui(app, ui);
            ui.separator();
            option_groups(app, ui, APPEARANCE_OPTIONS);
            session_note(ui);
        }
        PrefTab::Audio => audio_prefs(app, ui),
        PrefTab::Transcription => transcription_prefs(ui),
    });
    buttons(ui, "Save", enter && !ui.ctx().egui_wants_keyboard_input())
}

/// How a former menu option's state is read.
#[derive(Clone, Copy)]
enum Opt {
    /// The menu's own check state (the `options.*` switches).
    Menu,
    /// An on/off view flag of the session.
    Flag(&'static str),
    /// One of the Edit window scrolling modes.
    Scroll(&'static str),
    /// One of two styles: chosen while the flag is on (`true`) or off (`false`).
    Radio(&'static str, bool),
    /// A flag whose menu item only turns it on; `off` is the menu item that turns it off.
    Switch(&'static str, &'static str),
}

type OptGroup = (&'static str, &'static [(&'static str, Opt)]);

/// Menu options that moved to the Settings. Only the ones that change something are listed; the
/// rest of the incumbent's switches (they only stored a value nothing read) are just hidden.
const GENERAL_OPTIONS: &[OptGroup] = &[
    (
        "Edit Window Scrolling",
        &[
            ("Options > Edit Window Scrolling > No Scrolling", Opt::Scroll("none")),
            ("Options > Edit Window Scrolling > After Playback", Opt::Scroll("after_playback")),
            ("Options > Edit Window Scrolling > Page", Opt::Scroll("page")),
            ("Options > Edit Window Scrolling > Continuous", Opt::Scroll("continuous")),
            ("Options > Edit Window Scrolling > Center Playhead", Opt::Scroll("center")),
        ],
    ),
    (
        "Editing",
        &[
            ("Options > Insertion Follows Playback", Opt::Menu),
            ("Options > Link Timeline and Edit Selection", Opt::Menu),
            ("Options > Automation Follows Edit", Opt::Menu),
        ],
    ),
];
const APPEARANCE_OPTIONS: &[OptGroup] = &[
    (
        "Waveforms",
        &[
            ("View > Waveforms > Peak", Opt::Radio("waveform.power", false)),
            ("View > Waveforms > Power", Opt::Radio("waveform.power", true)),
            ("View > Waveforms > Rectified", Opt::Flag("waveform.rectified")),
            ("View > Waveforms > Outlines", Opt::Flag("waveform.outlines")),
        ],
    ),
    (
        "Clip",
        &[
            ("View > Clip > Name", Opt::Flag("view.clip.name")),
            ("View > Clip > Clip Gain Line", Opt::Flag("view.clip.gain_line")),
            ("View > Clip > Clip Gain Info", Opt::Flag("view.clip.gain_info")),
            ("View > Clip > Overlap Shadows", Opt::Flag("view.clip.overlap_shadows")),
            ("View > Clip > Transparency", Opt::Flag("view.clip.transparency")),
            ("View > Clip > Clip Effects Status", Opt::Flag("view.clip.effects_status")),
            ("View > Clip > Rating", Opt::Flag("view.clip.rating")),
            ("View > Clip > Current Time", Opt::Switch("view.clip.time.current", "View > Clip > No Time")),
        ],
    ),
    ("Tracks", &[("View > Track Number", Opt::Flag("edit_view.track_number"))]),
    ("Marker Displays", &[("View > Marker Displays > Ruler Lines", Opt::Flag("view.marker.ruler_lines"))]),
    (
        "Transport",
        &[
            ("View > Transport > Counters", Opt::Flag("view.transport.counters")),
            ("View > Transport > Output Meters", Opt::Flag("view.transport.output_meters")),
            ("View > Transport > Expanded", Opt::Flag("view.transport.expanded")),
        ],
    ),
];
const AUDIO_OPTIONS: &[OptGroup] = &[
    ("Recording", &[("Options > Pre/Post-Roll", Opt::Menu), ("Options > Loop Record", Opt::Menu)]),
    ("Mix", &[("Options > Delay Compensation", Opt::Menu)]),
];

/// Menu items that are neither in the menus nor in the Settings: switches of the incumbent that
/// TalkCraft never acts on (they only stored a value).
const WITHOUT_EFFECT: &[&str] = &[
    "Options > Link Track and Edit Selection",
    "Options > Markers Follow Edit",
    "Options > Marker Target Follows First Selected Track",
    "Options > Auto-Spot Clips",
    "Options > Pre-Fader Metering",
    "Options > Low Latency Monitoring",
    "Options > Solo Mode",
    "View > Waveforms > Overlapped Crossfades",
    "View > Clip > Channel Name",
    "View > Clip > Overwrite Indicator",
    "View > Clip > Copy/Move Indicators",
    "View > Clip > Processing State",
    "View > Clip > No Time",
    "View > Clip > Display on All Channels",
    "View > Track Transcription Lane",
    "View > Marker Displays",
];

/// Whether a catalog path now lives in the Settings (or, doing nothing, nowhere) instead of the menus.
pub fn in_settings(path: &str) -> bool {
    WITHOUT_EFFECT.iter().any(|p| path.starts_with(p))
        || [GENERAL_OPTIONS, APPEARANCE_OPTIONS, AUDIO_OPTIONS].iter().flat_map(|g| g.iter()).any(|(_, opts)| opts.iter().any(|(p, _)| *p == path))
}

/// Whether an option is on (or, for a choice, chosen).
fn option_state(app: &SoundApp, path: &str, how: Opt) -> bool {
    let cmd = crate::menus::command_for_path(path, &crate::menus::ui_aliases());
    let edit = &app.engine.session().edit;
    match how {
        Opt::Menu => crate::menus::item_state(app, path, cmd.as_deref()).1,
        Opt::Flag(f) | Opt::Switch(f, _) => edit.flag(f),
        Opt::Scroll(m) => edit.scrolling == m,
        Opt::Radio(f, on) => edit.flag(f) == on,
    }
}

/// Flips an on/off option, or chooses a choice, through the menu command it replaced.
fn option_change(app: &mut SoundApp, path: &str, how: Opt) {
    let aliases = crate::menus::ui_aliases();
    let target = match how {
        Opt::Switch(_, off) if option_state(app, path, how) => off,
        _ => path,
    };
    if let Some(cmd) = crate::menus::command_for_path(target, &aliases) {
        crate::menus::invoke_menu(app, &cmd, target);
    }
}

/// The groups as check boxes. Each runs the menu command it replaced, so behaviour is exactly the
/// menu's (these options are saved with the session).
fn option_groups(app: &mut SoundApp, ui: &mut egui::Ui, groups: &[OptGroup]) {
    let aliases = crate::menus::ui_aliases();
    for (heading, opts) in groups {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(crate::i18n::tr(heading)).strong());
        for (path, how) in *opts {
            if crate::menus::command_for_path(path, &aliases).is_none() {
                continue;
            }
            let checked = option_state(app, path, *how);
            let leaf = crate::i18n::tr(crate::podcast::menu_label(path.rsplit(" > ").next().unwrap_or(path)));
            let changed = if matches!(how, Opt::Scroll(_) | Opt::Radio(..)) {
                ui.add(egui::RadioButton::new(checked, leaf)).clicked() && !checked
            } else {
                let mut on = checked;
                ui.add(egui::Checkbox::new(&mut on, leaf)).changed()
            };
            if changed {
                option_change(app, path, *how);
            }
        }
    }
}

fn session_note(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(crate::i18n::tr("These options are saved with the session.")).small().color(Tokens::current().text_dim));
}

fn audio_prefs(app: &mut SoundApp, ui: &mut egui::Ui) {
    let dim = Tokens::current().text_dim;
    {
        ui.label(egui::RichText::new(crate::i18n::tr("Devices")).strong());
        ui.label(
            egui::RichText::new(crate::i18n::tr(
                "Record (Cmd+Space) records every speaker track except locked ones; a track's own record button records just that speaker.",
            ))
            .color(dim),
        );
        ui.horizontal(|ui| {
            if ui.button(crate::i18n::tr("Audio devices and buffer…")).clicked() {
                let _ = app.run("window.playback_engine", json!({"value": true}));
            }
            if ui.button(crate::i18n::tr("I/O…")).clicked() {
                let _ = app.run("window.io_setup", json!({"value": true}));
            }
        });
        ui.checkbox(&mut app.ui.countdown, crate::i18n::tr("Count 3-2-1 before recording starts"));
        ui.separator();
        option_groups(app, ui, AUDIO_OPTIONS);
        session_note(ui);
    }
}

fn general_prefs(app: &mut SoundApp, ui: &mut egui::Ui) {
    let dim = Tokens::current().text_dim;
    {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Interface language")).strong());
            let label = |v: &str| match v {
                "de" => "Deutsch",
                "en" => "English",
                _ => "Automatisch / Automatic",
            };
            egui::ComboBox::from_id_salt("ui_language").selected_text(label(&app.ui.language)).show_ui(ui, |ui| {
                for v in ["auto", "de", "en"] {
                    ui.selectable_value(&mut app.ui.language, v.to_string(), label(v));
                }
            });
        });
        ui.label(egui::RichText::new(crate::i18n::tr("The macOS menu bar follows after a restart.")).small().color(dim));
        ui.checkbox(&mut app.ui.ask_workspace, crate::i18n::tr("Ask at start what I want to do (otherwise start with everything)"));
        ui.separator();
        crate::palettes::preferences(app, ui);
        if ui.button(crate::i18n::tr("Keyboard Shortcuts…")).clicked() {
            let _ = app.run("window.shortcuts", json!({"value": true}));
        }
        ui.separator();
        option_groups(app, ui, GENERAL_OPTIONS);
        session_note(ui);
    }
}

fn transcription_prefs(ui: &mut egui::Ui) {
    let dim = Tokens::current().text_dim;
    {
        use soundcraft_engine::transcribe;
        match transcribe::find_cli() {
            Some(cli) => ui.label(crate::i18n::trf("whisper.cpp: {}", &[&cli.display()])),
            None => ui.label(egui::RichText::new(transcribe::install_hint()).color(dim)),
        };
        match transcribe::find_model(None) {
            Some(m) => ui.label(format!("{} {}", crate::i18n::tr("Speech model:"), m.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())),
            None => ui.label(egui::RichText::new(crate::i18n::tr("No speech model yet: Window > Subtitles offers to download one.")).color(dim)),
        };
        ui.label(egui::RichText::new(crate::i18n::tr("Transcription listens for the episode's language.")).small().color(dim));
    }
}

/// Format, loudness and side files; with `path`, its extension follows the format. The episode
/// tags follow for MP3 unless `episode_shown` (Preferences shows them on their own).
fn export_settings_ui(
    ui: &mut egui::Ui,
    services: &crate::Services,
    mut path: Option<&mut String>,
    s: &mut soundcraft_model::PodcastSettings,
    episode_shown: bool,
) {
    let x = &mut s.export;
    if let Some(path) = path.as_mut() {
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("File:").trim_end_matches(':'));
            path_field(ui, services, path, 470.0, Pick::Save("file.export_podcast"), "");
        });
    }
    ui.label(egui::RichText::new(crate::i18n::tr("Format")).strong());
    egui::Grid::new("ep_fmt").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        ui.label(crate::i18n::tr("Format"));
        let before = x.format.clone();
        egui::ComboBox::from_id_salt("ep_format").selected_text(x.format.to_uppercase()).show_ui(ui, |ui| {
            for f in soundcraft_engine::cmd::podcast::EXPORT_FORMATS {
                let aac = soundcraft_engine::aac::find_encoder().is_some();
                let label = match f {
                    "m4a" if aac => "M4A (AAC) – recommended".to_string(),
                    "m4a" => "M4A (needs ffmpeg)".to_string(),
                    "m4b" if aac => "M4B (audiobook, chapters)".to_string(),
                    "m4b" => "M4B (needs ffmpeg)".to_string(),
                    "mp3" if !soundcraft_audio_io::mp3::mp3_supported() => "MP3 (not in this build)".to_string(),
                    "mp3" => "MP3 (most compatible)".to_string(),
                    _ => f.to_uppercase(),
                };
                ui.selectable_value(&mut x.format, f.to_string(), label);
            }
        });
        if x.format != before
            && let Some(path) = path.as_mut()
        {
            let ext = if x.format == "aiff" { "aif" } else { x.format.as_str() };
            **path = std::path::Path::new(path.as_str()).with_extension(ext).to_string_lossy().to_string();
        }
        ui.end_row();
        if matches!(x.format.as_str(), "mp3" | "m4a" | "m4b") {
            ui.label(crate::i18n::tr("Bitrate"));
            egui::ComboBox::from_id_salt("ep_bitrate").selected_text(format!("{} kbit/s", x.bitrate_kbps)).show_ui(ui, |ui| {
                for b in soundcraft_audio_io::mp3::MP3_BITRATES {
                    let hint = match b {
                        64 => "  speech, mono",
                        96 => "  speech, good",
                        128 => "  standard",
                        192 => "  music",
                        _ => "",
                    };
                    ui.selectable_value(&mut x.bitrate_kbps, b, format!("{b} kbit/s{hint}"));
                }
            });
        } else {
            ui.label(crate::i18n::tr("Bit depth"));
            egui::ComboBox::from_id_salt("ep_bits").selected_text(format!("{}-bit", x.bit_depth)).show_ui(ui, |ui| {
                for b in ["16", "24", "32f"] {
                    ui.selectable_value(&mut x.bit_depth, b.to_string(), format!("{b}-bit"));
                }
            });
        }
        ui.end_row();
        ui.label(crate::i18n::tr("Sample rate"));
        egui::ComboBox::from_id_salt("ep_sr").selected_text(format!("{} Hz", x.sample_rate)).show_ui(ui, |ui| {
            for r in soundcraft_audio_io::mp3::MP3_SAMPLE_RATES {
                ui.selectable_value(&mut x.sample_rate, r, format!("{r} Hz"));
            }
        });
        ui.end_row();
        ui.label(crate::i18n::tr("Channels"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut x.mono, false, crate::i18n::tr("Stereo"));
            ui.radio_value(&mut x.mono, true, crate::i18n::tr("Mono"));
        });
        ui.end_row();
        ui.label(crate::i18n::tr("Loudness"));
        let current = LOUDNESS_PRESETS
            .iter()
            .find(|(_, v)| *v == x.loudness_lufs)
            .map_or_else(|| x.loudness_lufs.map_or_else(|| "Off".to_string(), |l| format!("{l:.0} LUFS")), |(l, _)| (*l).to_string());
        egui::ComboBox::from_id_salt("ep_lufs").width(300.0).selected_text(current).show_ui(ui, |ui| {
            for (label, v) in LOUDNESS_PRESETS {
                ui.selectable_value(&mut x.loudness_lufs, v, label);
            }
        });
        ui.end_row();
        if matches!(x.format.as_str(), "mp3" | "m4a" | "m4b") {
            ui.label(crate::i18n::tr("Chapters"));
            ui.checkbox(&mut x.chapters, crate::i18n::tr("Write markers as chapters"));
            ui.end_row();
        }
        ui.label(crate::i18n::tr("Also write"));
        ui.vertical(|ui| {
            ui.checkbox(&mut x.shownotes, crate::i18n::tr("Shownotes (.md) and chapters (.chapters.json)"));
            ui.checkbox(&mut x.subtitles, crate::i18n::tr("Subtitles (.srt and .vtt)"));
        });
        ui.end_row();
    });
    if matches!(x.format.as_str(), "mp3" | "m4a" | "m4b") && !episode_shown {
        ui.separator();
        episode_ui(ui, services, &mut s.episode);
    }
}

/// Episode details: ID3 tags in MP3s, shownotes, transcription language.
fn episode_ui(ui: &mut egui::Ui, services: &crate::Services, e: &mut soundcraft_model::EpisodeInfo) {
    {
        ui.label(egui::RichText::new(crate::i18n::tr("Episode")).strong());
        egui::Grid::new("ep_meta").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            for (label, field) in [("Title", &mut e.title), ("Podcast", &mut e.show), ("Author", &mut e.author), ("Genre", &mut e.genre)] {
                ui.label(crate::i18n::tr(label));
                ui.add(egui::TextEdit::singleline(field).desired_width(300.0));
                ui.end_row();
            }
            ui.label(crate::i18n::tr("Episode"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut e.episode).range(0..=99_999));
                ui.label(crate::i18n::tr("Year"));
                ui.add(egui::DragValue::new(&mut e.year).range(0..=2200));
                ui.label(crate::i18n::tr("Language"));
                egui::ComboBox::from_id_salt("ep_lang").selected_text(e.language.as_str()).show_ui(ui, |ui| {
                    for (code, name) in [("deu", "Deutsch"), ("eng", "English"), ("dan", "Dansk"), ("fra", "Français"), ("spa", "Español")] {
                        ui.selectable_value(&mut e.language, code.to_string(), format!("{code}  {name}"));
                    }
                });
            });
            ui.end_row();
            ui.label(crate::i18n::tr("Cover"));
            path_field(
                ui,
                services,
                &mut e.cover,
                300.0,
                Pick::Open("image", &["jpg", "jpeg", "png"]),
                crate::i18n::tr("path to a square JPEG or PNG"),
            );
            ui.end_row();
            ui.label(crate::i18n::tr("Description"));
            ui.add(egui::TextEdit::multiline(&mut e.description).desired_rows(3).desired_width(300.0));
            ui.end_row();
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_option_in_the_settings_switches_and_shows_its_state() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, crate::Services::default());
        for group in [GENERAL_OPTIONS, APPEARANCE_OPTIONS, AUDIO_OPTIONS] {
            for (heading, opts) in group {
                for (path, how) in *opts {
                    assert!(crate::menus::command_for_path(path, &crate::menus::ui_aliases()).is_some(), "{path} has a command");
                    assert!(crate::podcast::hidden_menu(path), "{path} left the menu");
                    let before = option_state(&app, path, *how);
                    option_change(&mut app, path, *how);
                    match how {
                        Opt::Scroll(_) | Opt::Radio(..) => assert!(option_state(&app, path, *how), "{path} is chosen"),
                        _ => {
                            assert_ne!(option_state(&app, path, *how), before, "{path} switches");
                            option_change(&mut app, path, *how);
                            assert_eq!(option_state(&app, path, *how), before, "{path} switches back");
                        }
                    }
                }
                // A choice shows exactly one chosen entry.
                let chosen = opts.iter().filter(|(p, h)| matches!(h, Opt::Scroll(_) | Opt::Radio(..)) && option_state(&app, p, *h)).count();
                let choices = opts.iter().filter(|(_, h)| matches!(h, Opt::Scroll(_) | Opt::Radio(..))).count();
                assert!(choices == 0 || chosen == 1, "{heading}: {chosen} of {choices} chosen");
            }
        }
    }

    use super::*;

    #[test]
    fn file_commands_get_path_prompts_with_sensible_names() {
        let e = soundcraft_engine::demo::demo_engine();
        let mut d = Dialogs::default();
        for (id, ext) in [("file.export_sibelius", ".musicxml"), ("file.print_score", ".svg"), ("file.export_clip_groups", ".scgrp")] {
            assert!(takes_path(id), "{id}");
            assert!(d.open_for_command(&e, id), "{id}");
            match d.open.take() {
                Some(Dialog::PathPrompt { cmd, path, key, .. }) => {
                    assert_eq!(cmd, id);
                    assert_eq!(key, "path");
                    assert!(path.ends_with(ext), "{path}");
                }
                other => panic!("{id}: {other:?}"),
            }
        }
        // Optional-path commands run directly.
        assert!(!takes_path("file.send_to_sibelius"));
        assert!(!takes_path("edit.copy"));
        assert!(d.open_for_command(&e, "file.score_setup"));
        assert!(matches!(d.open, Some(Dialog::ScoreSetup { bars_per_system: 4, .. })));
    }

    #[test]
    fn clip_rename_opens_the_rename_dialog_for_the_selected_clip() {
        let mut e = soundcraft_engine::demo::demo_engine();
        let mut d = Dialogs::default();
        e.execute("edit.select_none", &json!({})).unwrap();
        assert!(!d.open_for_command(&e, "clip.rename"));
        let clip = e.session().track_by_name("Kick").unwrap().clips()[0].clone();
        e.execute("edit.select", &json!({"clips": [clip.id.0]})).unwrap();
        assert!(d.open_for_command(&e, "clip.rename"));
        assert!(matches!(d.open, Some(Dialog::RenameClip { id, ref name }) if id == clip.id && *name == clip.name));
    }

    #[test]
    fn tempo_change_dialog_runs_a_real_command_at_its_position() {
        let (cmd, params) = tempo_change_action(96_000, 133.5);
        assert!(soundcraft_engine::command_specs().iter().any(|c| c.id == cmd), "{cmd} is not a command");
        assert_eq!(params["at"], 96_000);
        assert_eq!(params["bpm"], 133.5);
    }

    #[test]
    fn clear_always_asks_and_names_the_timeline_clips_that_go_too() {
        let mut e = soundcraft_engine::demo::demo_engine();
        let file = |e: &Engine, name: &str| e.session().sources.iter().find(|f| f.name == name).map(|f| f.id).unwrap();
        let (kick, hats) = (file(&e, "Kick"), file(&e, "Hats"));
        let kick_clips: Vec<ClipId> = e.session().clips_using(kick).map(|c| c.id).collect();
        let snare = e.session().track_by_name("Snare").unwrap().clips()[0].id;
        let message = |d: &Dialogs| match &d.open {
            Some(Dialog::ClearClips { message, .. }) => message.clone(),
            other => panic!("{other:?}"),
        };
        let mut d = Dialogs::default();
        assert!(!d.open_for_command(&e, "clip.clear"));
        e.execute("edit.select", &json!({"clips": [snare]})).unwrap();
        assert!(d.open_for_command(&e, "clip.clear"));
        assert_eq!(message(&d), "Remove the clip \"Snare\" from the session?");
        e.execute("edit.select", &json!({"sources": [kick]})).unwrap();
        assert!(d.open_for_command(&e, "clip.clear"));
        assert!(matches!(&d.open, Some(Dialog::ClearClips { clips, sources, .. }) if clips.is_empty() && *sources == vec![kick]));
        assert_eq!(message(&d), "Remove the file \"Kick\" from the session? It is used by 3 clips on the timeline, which are removed too.");
        assert!(d.open_clear(&e, kick_clips.clone(), vec![kick]));
        assert_eq!(message(&d), "Remove 3 clips and 1 file from the session?");
        assert!(d.open_clear(&e, kick_clips, vec![kick, hats]));
        assert_eq!(message(&d), "Remove 3 clips and 2 files from the session? \"Hats\" is used by a clip on the timeline, which is removed too.");
        e.session_mut().sources.iter_mut().for_each(|f| f.unsaved = false);
        assert!(d.open_clear(&e, vec![], vec![kick, hats]));
        assert_eq!(
            message(&d),
            "Remove 2 files from the session? They are used by 4 clips on the timeline, which are removed too. The files stay on disk."
        );
    }
}
