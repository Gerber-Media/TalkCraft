//! File › Publish Episode: a checklist of what listeners and directories expect, the target (the
//! own RSS feed on storage the user controls, or a podcast host's API), and one button that
//! exports the episode and publishes it in the background with a log.
//!
//! Keys and passwords never go into the session or `ui.json`: they live in `publish.json` in the
//! settings folder, readable only by the user. Each user enters their own; TalkCraft has none.

use crate::SoundApp;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use egui::{RichText, vec2};
use serde_json::{Value, json};
use soundcraft_engine::publish::hosts::Host;
use soundcraft_engine::publish::job::{self, Prepared, Settings, Target};
use soundcraft_engine::publish::storage::StorageKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// What a background publish reports.
#[derive(Debug, Default)]
pub struct Progress {
    pub log: Vec<String>,
    pub done: Option<Result<String, String>>,
}

#[derive(Debug, Clone, Default)]
pub struct PublishWindow {
    pub show: bool,
    /// Loaded from `publish.json` the first time it is needed.
    pub settings: Option<Settings>,
    saved: Option<Settings>,
    changed_at: Option<f64>,
    pub run: Option<Arc<Mutex<Progress>>>,
    /// Where the last export went (for the social package).
    pub exported: Option<Prepared>,
    pub note: String,
    /// Another folder than the settings folder's `publish` (tests).
    pub dir: Option<PathBuf>,
}

/// The folder for everything publishing keeps (keys, feed state, exports, Git checkouts).
pub fn base_dir(app: &SoundApp) -> Option<PathBuf> {
    app.extra.publish.dir.clone().or_else(|| soundcraft_engine::transcribe::settings_dir().map(|d| d.join("publish")))
}

pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join("publish.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Writes the settings readable only by the user (they hold keys and passwords).
pub fn save(dir: &Path, s: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join("publish.json");
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn settings(app: &mut SoundApp) -> &mut Settings {
    let dir = base_dir(app);
    let w = &mut app.extra.publish;
    if w.settings.is_none() {
        let s = dir.map(|d| load(&d)).unwrap_or_default();
        w.saved = Some(s.clone());
        w.settings = Some(s);
    }
    w.settings.get_or_insert_with(Settings::default)
}

fn store(app: &mut SoundApp) {
    let dir = base_dir(app);
    let w = &mut app.extra.publish;
    if let (Some(s), Some(dir)) = (&w.settings, dir)
        && w.saved.as_ref() != Some(s)
    {
        match save(&dir, s) {
            Ok(()) => w.saved = Some(s.clone()),
            Err(e) => w.note = e,
        }
    }
    w.changed_at = None;
}

/// One line of the checklist: `Some(true)` done, `Some(false)` missing, `None` a hint.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub label: &'static str,
    pub ok: Option<bool>,
    pub detail: String,
}

/// What a finished episode should have before it goes out.
pub fn checklist(app: &SoundApp) -> Vec<Check> {
    let s = app.engine.session();
    let ep = &s.podcast.episode;
    let mut out = Vec::new();
    let title = ep.title.trim();
    out.push(Check {
        label: "Title",
        ok: Some(!title.is_empty()),
        detail: if title.is_empty() { tr("set it under Settings › Episode").into() } else { title.into() },
    });
    let show = ep.show.trim();
    out.push(Check { label: "Podcast name", ok: Some(!show.is_empty()), detail: show.into() });
    let words = ep.description.split_whitespace().count();
    out.push(Check { label: "Description", ok: Some(words >= 5), detail: trf("{} word(s)", &[&words]) });
    out.push(cover_check(&ep.cover));
    let chapters =
        s.markers.iter().filter(|m| m.kind == soundcraft_model::MarkerKind::Marker && !soundcraft_engine::podcast::is_cut_marker(m)).count();
    out.push(Check { label: "Chapters", ok: if chapters > 0 { Some(true) } else { None }, detail: trf("{} chapter(s)", &[&chapters]) });
    let open_cuts = s.markers.iter().filter(|m| soundcraft_engine::podcast::is_cut_marker(m)).count();
    out.push(Check { label: "Cut markers", ok: Some(open_cuts == 0), detail: trf("{} still open", &[&open_cuts]) });
    let lufs = s.podcast.export.loudness_lufs;
    out.push(Check {
        label: "Loudness",
        ok: Some(lufs.is_some()),
        detail: lufs.map_or_else(|| tr("not normalized").to_string(), |l| format!("{l:.0} LUFS")),
    });
    let secs = s.sample_rate.seconds(s.content_end());
    out.push(Check { label: "Length", ok: Some(secs >= 1.0), detail: format!("{}:{:02}", (secs / 60.0) as u64, (secs % 60.0) as u64) });
    out.push(Check {
        label: "Transcript",
        ok: if s.subtitles.is_empty() { None } else { Some(true) },
        detail: trf("{} subtitle(s)", &[&s.subtitles.len()]),
    });
    out
}

fn cover_check(path: &str) -> Check {
    let path = path.trim();
    if path.is_empty() {
        return Check { label: "Cover", ok: Some(false), detail: tr("none (square JPEG or PNG, 1400–3000 px)").into() };
    }
    match image::image_dimensions(path) {
        Ok((w, h)) => {
            let ok = w == h && (1400..=3000).contains(&w);
            let detail = if ok { format!("{w} × {h} px") } else { trf("{} × {} px: needs square, 1400–3000 px", &[&w, &h]) };
            Check { label: "Cover", ok: Some(ok), detail }
        }
        Err(e) => Check { label: "Cover", ok: Some(false), detail: e.to_string() },
    }
}

/// ISO 639-2 (`deu`) to the two-letter code feeds use (`de`).
fn feed_language(code: &str) -> String {
    let two = match code {
        "deu" | "ger" => "de",
        "eng" => "en",
        "dan" => "da",
        "fra" | "fre" => "fr",
        "spa" => "es",
        "ita" => "it",
        "nld" | "dut" => "nl",
        "swe" => "sv",
        "nor" | "nob" => "no",
        "pol" => "pl",
        "por" => "pt",
        _ => code,
    };
    two.to_string()
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "m4a" => "audio/x-m4a",
        "m4b" => "audio/x-m4b",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "aif" | "aiff" => "audio/aiff",
        _ => "application/octet-stream",
    }
}

/// Exports the episode into `dir/export` with shownotes, chapters and subtitles.
pub fn prepare(app: &mut SoundApp, dir: &Path) -> Result<Prepared, String> {
    let ep = app.engine.session().podcast.episode.clone();
    let name = if ep.title.trim().is_empty() { app.engine.session().name.clone() } else { ep.title.clone() };
    let stem = if ep.episode > 0 { format!("{:03}-{}", ep.episode, job::slug(&name)) } else { job::slug(&name) };
    let out = dir.join("export").join(job::slug(if ep.show.trim().is_empty() { &name } else { &ep.show }));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let v = app.run("file.export_podcast", json!({"path": out.join(&stem).to_string_lossy(), "shownotes": true, "subtitles": true}))?;
    let audio = PathBuf::from(v["path"].as_str().unwrap_or_default());
    let also: Vec<PathBuf> =
        v["also_written"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect()).unwrap_or_default();
    let find = |suffix: &str| also.iter().find(|p| p.to_string_lossy().ends_with(suffix)).cloned();
    let cover = Some(PathBuf::from(ep.cover.trim())).filter(|p| !ep.cover.trim().is_empty() && p.is_file());
    Ok(Prepared {
        audio_mime: mime(&audio).into(),
        audio,
        duration_s: v["seconds"].as_f64().unwrap_or(0.0).round().max(0.0) as u64,
        show: ep.show.clone(),
        author: ep.author.clone(),
        language: feed_language(&ep.language),
        title: name,
        description: ep.description.clone(),
        number: ep.episode,
        season: 0,
        cover,
        chapters: find(".chapters.json"),
        transcript: find(".vtt"),
    })
}

/// Exports, then publishes in the background; progress lands in `app.extra.publish.run`.
pub fn start(app: &mut SoundApp) -> Result<(), String> {
    if app.extra.publish.run.as_ref().is_some_and(|r| r.lock().map(|p| p.done.is_none()).unwrap_or(false)) {
        return Err(tr("A publish is still running").into());
    }
    let dir = base_dir(app).ok_or_else(|| tr("No settings folder").to_string())?;
    let settings = settings(app).clone();
    store(app);
    let prepared = prepare(app, &dir)?;
    app.extra.publish.exported = Some(prepared.clone());
    let feed_state = dir.join("feeds").join(format!("{}.json", job::slug(if prepared.show.is_empty() { &prepared.title } else { &prepared.show })));
    let work = dir.join("storage").join(job::slug(&settings.storage.url));
    let progress = Arc::new(Mutex::new(Progress { log: vec![trf("Exported {}", &[&prepared.audio.display()])], done: None }));
    app.extra.publish.run = Some(progress.clone());
    std::thread::spawn(move || {
        let p = progress.clone();
        let mut log = move |line: String| {
            if let Ok(mut g) = p.lock() {
                g.log.push(line);
            }
        };
        let mut t = soundcraft_engine::publish::http::Curl;
        let r = job::run(&settings, &prepared, &feed_state, &work, &mut t, &mut log);
        if let Ok(mut g) = progress.lock() {
            g.done = Some(r);
        }
    });
    Ok(())
}

/// A folder for social posts: the text (title, description, chapters, link), the cover and the
/// shownotes. The audiogram (video) is made from the Audiogram window.
pub fn social_package(app: &SoundApp, link: &str) -> Result<PathBuf, String> {
    let p = app.extra.publish.exported.as_ref().ok_or_else(|| tr("Publish or export first").to_string())?;
    let dir = p.audio.with_extension("").with_extension("social");
    let dir = PathBuf::from(format!("{}-social", dir.to_string_lossy().trim_end_matches(".social")));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let de = p.language == "de";
    let mut text = format!("{}\n\n", if p.number > 0 { format!("#{} {}", p.number, p.title) } else { p.title.clone() });
    if !p.description.trim().is_empty() {
        text += &format!("{}\n\n", p.description.trim());
    }
    let md = p.audio.with_extension("md");
    if let Ok(notes) = std::fs::read_to_string(&md) {
        let chapters: Vec<&str> = notes.lines().filter(|l| l.starts_with("- ")).collect();
        if !chapters.is_empty() {
            text += &format!("{}\n{}\n\n", if de { "Kapitel:" } else { "Chapters:" }, chapters.join("\n"));
        }
        std::fs::write(dir.join("shownotes.md"), notes).map_err(|e| e.to_string())?;
    }
    if !link.is_empty() {
        text += &format!("{} {link}\n", if de { "Jetzt hören:" } else { "Listen now:" });
    }
    if !p.show.is_empty() {
        text += &format!("\n#podcast #{}\n", job::slug(&p.show).replace('-', ""));
    }
    std::fs::write(dir.join("post.txt"), text).map_err(|e| e.to_string())?;
    if let Some(c) = &p.cover
        && let Some(name) = c.file_name()
    {
        std::fs::copy(c, dir.join(name)).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// `window.publish`, `publish.checklist`, `publish.start`, `publish.status`.
pub fn command(app: &mut SoundApp, id: &str, p: &Value) -> Result<Value, String> {
    match id {
        "window.publish" => {
            let w = &mut app.extra.publish;
            w.show = p.get("value").and_then(Value::as_bool).unwrap_or(!w.show);
            if !w.show {
                store(app);
            }
            Ok(json!({"value": app.extra.publish.show}))
        }
        "publish.checklist" => {
            Ok(Value::Array(checklist(app).into_iter().map(|c| json!({"item": c.label, "ok": c.ok, "detail": c.detail})).collect()))
        }
        "publish.start" => {
            start(app)?;
            Ok(json!({"started": true}))
        }
        "publish.status" => {
            let Some(run) = &app.extra.publish.run else { return Ok(json!({"running": false})) };
            let g = run.lock().map_err(|_| "publish state unavailable".to_string())?;
            Ok(json!({
                "running": g.done.is_none(),
                "log": g.log,
                "url": g.done.as_ref().and_then(|r| r.as_ref().ok()),
                "error": g.done.as_ref().and_then(|r| r.as_ref().err()),
            }))
        }
        _ => Err(format!("unknown command `{id}`")),
    }
}

fn target_name(t: Target) -> String {
    match t {
        Target::OwnFeed => tr("Own RSS feed (Apple, Spotify, every app)").to_string(),
        Target::Host(h) if h.verified() => h.name().to_string(),
        Target::Host(h) => format!("{} ({})", h.name(), tr("untested format")),
    }
}

fn field(ui: &mut egui::Ui, label: &str, v: &mut String, secret: bool, hint: &str) {
    ui.label(tr(label));
    ui.add(egui::TextEdit::singleline(v).password(secret).hint_text(hint).desired_width(300.0));
    ui.end_row();
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.extra.publish.show {
        return;
    }
    let t = Tokens::current();
    let checks = checklist(app);
    settings(app);
    let mut open = true;
    let mut go = false;
    let mut social = false;
    let mut audiogram = false;
    let (running, log, result) = match &app.extra.publish.run {
        Some(r) => match r.lock() {
            Ok(g) => (g.done.is_none(), g.log.clone(), g.done.clone()),
            Err(_) => (false, Vec::new(), None),
        },
        None => (false, Vec::new(), None),
    };
    if running {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    let before = app.extra.publish.settings.clone();
    let note = app.extra.publish.note.clone();
    egui::Window::new(tr("Publish Episode")).open(&mut open).default_pos([320.0, 130.0]).default_size(vec2(520.0, 620.0)).resizable(true).show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(tr("Checklist"));
            egui::Grid::new("publish_checks").num_columns(3).spacing(vec2(8.0, 3.0)).show(ui, |ui| {
                for c in &checks {
                    // A dot: the interface font has no check mark.
                    let colour = match c.ok {
                        Some(true) => t.meter_green,
                        Some(false) => t.rec,
                        None => t.text_dim,
                    };
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.5, colour);
                    ui.label(tr(c.label));
                    ui.label(RichText::new(&c.detail).small().color(t.text_dim));
                    ui.end_row();
                }
            });
            ui.separator();
            let Some(s) = app.extra.publish.settings.as_mut() else { return };
            ui.heading(tr("Where to"));
            egui::ComboBox::from_id_salt("publish_target").width(320.0).selected_text(target_name(s.target)).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.target, Target::OwnFeed, target_name(Target::OwnFeed));
                for h in Host::ALL {
                    ui.selectable_value(&mut s.target, Target::Host(h), target_name(Target::Host(h)));
                }
            });
            let needs_storage = match s.target {
                Target::OwnFeed => true,
                Target::Host(h) => h.needs_public_url(),
            };
            match s.target {
                Target::OwnFeed => {
                    ui.label(
                        RichText::new(tr(
                            "TalkCraft writes feed.xml with every episode and uploads it with the audio. Submit the feed's address once to Apple Podcasts, Spotify and the other directories.",
                        ))
                        .small()
                        .color(t.text_dim),
                    );
                    egui::Grid::new("publish_show").num_columns(2).show(ui, |ui| {
                        field(ui, "Show description", &mut s.show.description, false, "");
                        field(ui, "Contact e-mail", &mut s.show.email, false, "");
                        field(ui, "Website", &mut s.show.website, false, "https://");
                        field(ui, "Category", &mut s.show.category, false, "Society & Culture");
                        ui.label(tr("Explicit"));
                        ui.checkbox(&mut s.show.explicit, "");
                        ui.end_row();
                    });
                }
                Target::Host(h) => {
                    if !h.verified() {
                        ui.label(
                            RichText::new(tr("This host's upload format comes from secondary sources and could not be tested. Check the result in the host's dashboard."))
                                .small()
                                .color(t.meter_yellow),
                        );
                    }
                    let cfg = s.hosts.entry(h.id().to_string()).or_default();
                    egui::Grid::new("publish_host").num_columns(2).show(ui, |ui| {
                        for (key, label, secret) in h.fields() {
                            let v = match *key {
                                "api_key" => &mut cfg.api_key,
                                "show_id" => &mut cfg.show_id,
                                _ => &mut cfg.user,
                            };
                            field(ui, label, v, *secret, "");
                        }
                    });
                    ui.label(RichText::new(tr("Your own key from the host's account settings. It is stored only on this computer.")).small().color(t.text_dim));
                    ui.checkbox(&mut s.publish_now, tr("Publish right away (off: as a draft to check first)"));
                }
            }
            if needs_storage {
                ui.separator();
                ui.heading(tr("Storage"));
                if let Target::Host(_) = s.target {
                    ui.label(RichText::new(tr("This host takes the audio only as a web address: it goes to your storage first.")).small().color(t.text_dim));
                }
                let st = &mut s.storage;
                egui::ComboBox::from_id_salt("publish_storage").width(320.0).selected_text(tr(st.kind.name())).show_ui(ui, |ui| {
                    for k in StorageKind::ALL {
                        ui.selectable_value(&mut st.kind, k, tr(k.name()));
                    }
                });
                egui::Grid::new("publish_storage_fields").num_columns(2).show(ui, |ui| {
                    match st.kind {
                        StorageKind::Git => {
                            field(ui, "Repository (HTTPS)", &mut st.url, false, "https://codefloe.com/name/podcast.git");
                            field(ui, "Branch", &mut st.branch, false, "pages");
                        }
                        StorageKind::Sftp => {
                            field(ui, "Server", &mut st.url, false, "example.com");
                            field(ui, "User", &mut st.user, false, "");
                            ui.label(tr("Port"));
                            ui.add(egui::DragValue::new(&mut st.port).range(0..=65535));
                            ui.end_row();
                        }
                        StorageKind::Ftp | StorageKind::WebDav => {
                            let hint = if st.kind == StorageKind::Ftp { "ftps://example.com" } else { "https://example.com/dav" };
                            field(ui, "Server", &mut st.url, false, hint);
                            field(ui, "User", &mut st.user, false, "");
                            field(ui, "Password", &mut st.password, true, "");
                        }
                    }
                    field(ui, "Folder", &mut st.dir, false, "podcast");
                    field(ui, "Public address of the folder", &mut st.public_base, false, "https://name.codefloe.page/podcast/");
                });
                let how = match st.kind {
                    StorageKind::Git => "Git uses your system's saved login (as for git push).",
                    StorageKind::Sftp => "SFTP logs in with your SSH key (no password prompt).",
                    StorageKind::Ftp | StorageKind::WebDav => "The password is stored only on this computer.",
                };
                ui.label(RichText::new(tr(how)).small().color(t.text_dim));
            }
            ui.separator();
            ui.horizontal(|ui| {
                let label = if running { tr("Publishing …") } else { tr("Export and Publish") };
                if ui.add_enabled(!running, egui::Button::new(RichText::new(label).strong()).fill(t.accent)).clicked() {
                    go = true;
                }
                if running {
                    ui.spinner();
                }
            });
            if !note.is_empty() {
                ui.label(RichText::new(&note).color(t.rec));
            }
            for line in &log {
                ui.label(RichText::new(line).small().monospace());
            }
            match &result {
                Some(Ok(url)) => {
                    ui.label(RichText::new(tr("Published.")).strong());
                    ui.hyperlink(url);
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(format!("{}: {e}", tr("Failed"))).color(t.rec));
                }
                None => {}
            }
            ui.separator();
            ui.heading(tr("Social media"));
            ui.horizontal(|ui| {
                if ui.button(tr("Make social package")).on_hover_text(tr("A folder with post text, chapters, cover and shownotes")).clicked() {
                    social = true;
                }
                if ui.button(tr("Audiogram (MP4)...")).clicked() {
                    audiogram = true;
                }
            });
            ui.label(
                RichText::new(tr("Quote clips: mark the best sentences in Window › Edit by Text, then Quote Clip."))
                    .small()
                    .color(t.text_dim),
            );
        });
    });
    let now = ctx.input(|i| i.time);
    let w = &mut app.extra.publish;
    if w.settings != before {
        w.changed_at = Some(now);
    }
    if w.changed_at.is_some_and(|at| now - at > 1.0) || !open {
        store(app);
    }
    app.extra.publish.show = open;
    if go {
        app.extra.publish.note.clear();
        if let Err(e) = start(app) {
            app.extra.publish.note = e;
        }
    }
    if social {
        let link = match &result {
            Some(Ok(u)) => u.clone(),
            _ => String::new(),
        };
        app.extra.publish.note = match social_package(app, &link) {
            Ok(dir) => trf("Social package: {}", &[&dir.display()]),
            Err(e) => e,
        };
    }
    if audiogram {
        app.dialogs.open_audiogram(&app.engine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use soundcraft_engine::publish::storage::Storage;
    use std::process::Command;

    #[test]
    fn keys_are_kept_readable_only_by_the_user() {
        let dir = std::env::temp_dir().join(format!("tc-pubset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.hosts.insert("buzzsprout".into(), soundcraft_engine::publish::hosts::HostConfig { api_key: "secret".into(), ..Default::default() });
        save(&dir, &s).unwrap();
        assert_eq!(load(&dir), s);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("publish.json")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checklist_names_what_is_missing() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.extra.publish.dir = Some(std::env::temp_dir().join("tc-pubcheck-unused"));
        app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 1})).unwrap();
        let v = app.run("publish.checklist", json!({})).unwrap();
        let ok = |item: &str| v.as_array().unwrap().iter().find(|c| c["item"] == item).map(|c| c["ok"].clone()).unwrap();
        assert_eq!(ok("Title"), json!(false));
        assert_eq!(ok("Cover"), json!(false));
        assert_eq!(ok("Loudness"), json!(true), "normalized by default");
        app.engine.execute("podcast.set", &json!({"episode": {"title": "Energie"}})).unwrap();
        let v = app.run("publish.checklist", json!({})).unwrap();
        assert_eq!(v[0]["ok"], json!(true));
        // The work mode opens the window.
        app.run("ui.workspace", json!({"mode": "publish"})).unwrap();
        assert!(app.extra.publish.show);
    }

    #[test]
    fn export_and_publish_to_the_own_feed_in_the_background() {
        let root = std::env::temp_dir().join(format!("tc-pubwin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let remote = root.join("remote.git");
        assert!(Command::new("git").args(["init", "--bare", "-q"]).arg(&remote).status().unwrap().success());
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.extra.publish.dir = Some(root.join("publish"));
        app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 1})).unwrap();
        app.engine
            .execute("podcast.set", &json!({"episode": {"title": "Energie", "show": "Europa", "episode": 3}, "export": {"format": "wav"}}))
            .unwrap();
        settings(&mut app).storage = Storage {
            kind: StorageKind::Git,
            url: remote.to_string_lossy().into(),
            branch: "pages".into(),
            public_base: "https://x.example/".into(),
            ..Default::default()
        };
        app.run("publish.start", json!({})).unwrap();
        let mut status = json!({});
        // Up to two minutes: on machines that scan every new process, each git step takes seconds.
        for _ in 0..2400 {
            status = app.run("publish.status", json!({})).unwrap();
            if status["running"] == json!(false) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(status["url"], json!("https://x.example/feed.xml"), "{status}");
        let xml = Command::new("git").arg("--git-dir").arg(&remote).args(["show", "pages:feed.xml"]).output().unwrap();
        assert!(String::from_utf8_lossy(&xml.stdout).contains("003-energie.wav"));
        assert!(load(&root.join("publish")).storage.url.ends_with("remote.git"), "settings saved on publish");
        let dir = social_package(&app, "https://x.example/feed.xml").unwrap();
        assert!(std::fs::read_to_string(dir.join("post.txt")).unwrap().contains("#3 Energie"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
