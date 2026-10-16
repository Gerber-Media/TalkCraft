//! The pictures of the tutorial page (talkcraft.gerber.media/anleitung): drives the real interface
//! through each scene and writes keyframes plus a timeline that `site/tutorial/make_gifs.py` turns
//! into GIFs. Not a test, so it is ignored; the whole run:
//!
//!   site/tutorial/make_media.sh MEDIA OUT     (audio, then this, then the GIFs)
//!
//! TALKCRAFT_SCENE=<name> runs one scene. Nothing here opens a microphone or plays sound: the audio
//! engine is there for the readouts only, and nothing presses Play or fires a jingle.
use egui::{Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use soundcraft_ui_egui::{Services, SoundApp, i18n, workspace::Workspace};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SIZE: [f32; 2] = [1280.0, 720.0];
const RATE: f64 = 48_000.0;

/// One scene: the harness, its keyframes so far and where the drawn pointer is.
struct Shoot {
    h: Harness<'static, SoundApp>,
    dir: PathBuf,
    name: String,
    lang: String,
    frames: Vec<Value>,
    cursor: Pos2,
}

impl Shoot {
    fn new(media: &Path, lang: &str, name: &str, app: SoundApp) -> Self {
        // The app picks its work mode on the first frame (All, unless it asks): set it again after.
        let workspace = app.ui.workspace;
        let dir = media.join("frames").join(lang).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut h = Harness::builder().with_size(vec2(SIZE[0], SIZE[1])).wgpu().build_ui_state(
            |ui, app: &mut SoundApp| {
                app.logic(&ui.ctx().clone());
                app.ui(ui);
            },
            app,
        );
        h.input_mut().max_texture_side = Some(8192);
        h.run_steps(4);
        h.state_mut().ui.workspace = workspace;
        h.run_steps(6);
        Shoot { h, dir, name: name.into(), lang: lang.into(), frames: Vec::new(), cursor: pos2(SIZE[0] * 0.62, SIZE[1] * 0.72) }
    }

    /// The caption in this scene's language.
    fn cap<'a>(&self, de: &'a str, en: &'a str) -> &'a str {
        if self.lang == "de" { de } else { en }
    }

    fn settle(&mut self) {
        self.h.run_steps(8);
    }

    fn app(&mut self) -> &mut SoundApp {
        self.h.state_mut()
    }

    fn run(&mut self, id: &str, p: Value) -> Value {
        let r = self.app().run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
        self.settle();
        r
    }

    /// A keyframe: the screen as it is now, held `ms`, with a caption under it.
    fn snap(&mut self, caption: &str, ms: u32) {
        self.snap_with(caption, ms, false);
    }

    fn snap_with(&mut self, caption: &str, ms: u32, click: bool) {
        // Without the harness's own pointer: make_gifs.py draws one, also on the way between frames.
        self.h.input_mut().events.push(egui::Event::PointerGone);
        self.h.run_steps(2);
        let file = format!("{:03}.png", self.frames.len());
        self.h.render().unwrap().save(self.dir.join(&file)).unwrap();
        self.frames.push(json!({"file": file, "cursor": [self.cursor.x, self.cursor.y], "click": click, "caption": caption, "ms": ms}));
    }

    /// The topmost widget whose label is `english` shown translated; a label with `{}` (numbers
    /// filled in) is found by its longest fixed part. With `click`, it is also clicked.
    fn node(&mut self, english: &str, click: bool) -> Rect {
        let label = i18n::tr(english).to_string();
        let part = label.split("{}").map(str::trim).max_by_key(|p| p.len()).unwrap_or_default().to_string();
        let rect = {
            let node = if label.contains("{}") { self.h.query_all_by_label_contains(&part).last() } else { self.h.query_all_by_label(&label).last() };
            let node = node.unwrap_or_else(|| panic!("{}: no widget labelled {label:?}", self.name));
            if click {
                node.click();
            }
            node.rect()
        };
        if click {
            self.settle();
        }
        rect
    }

    fn node_rect(&mut self, english: &str) -> Rect {
        self.node(english, false)
    }

    fn press(&mut self, english: &str) {
        self.node(english, true);
    }

    /// Moves the pointer to the widget, clicks it and shows the result.
    fn click(&mut self, english: &str, caption: &str, ms: u32) {
        let at = self.node_rect(english).center();
        self.point_at(at, caption, 650);
        self.press(english);
        self.snap_with(caption, ms, true);
    }

    /// Points at the widget without clicking.
    fn show(&mut self, english: &str, caption: &str, ms: u32) {
        let at = self.node_rect(english).center();
        self.point_at(at, caption, ms);
    }

    /// Moves the drawn pointer (the GIF animates the way there) and holds.
    fn point_at(&mut self, at: Pos2, caption: &str, ms: u32) {
        self.cursor = at;
        self.snap(caption, ms);
    }

    /// The timeline x of `seconds` and the middle of the first track row.
    fn timeline_at(&self, seconds: f64) -> Pos2 {
        let app = self.h.state();
        let [x0, y0, x1, y1] = app.edit_layout.timeline;
        let tl = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        let x = soundcraft_ui_egui::edit_window::x_of(app.engine.session(), tl, (seconds * RATE) as _);
        let y = app.edit_layout.rows.first().map_or(y0 + 40.0, |(_, r)| (r[1] + r[3]) / 2.0);
        pos2(x, y)
    }

    fn finish(self) {
        let timeline = json!({"scene": self.name, "lang": self.lang, "size": SIZE, "frames": self.frames});
        std::fs::write(self.dir.join("timeline.json"), serde_json::to_string_pretty(&timeline).unwrap()).unwrap();
        println!("{}/{}: {} keyframes", self.lang, self.name, self.frames.len());
    }
}

/// The sample episode from `make_audio.py`, as a TalkCraft session in `lang`.
struct Episode {
    dir: PathBuf,
    script: Value,
    lang: String,
}

impl Episode {
    fn load(media: &Path, lang: &str) -> Self {
        let dir = media.join(lang);
        let script: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("script.json")).unwrap()).unwrap();
        Episode { dir, script, lang: lang.into() }
    }

    fn name(&self, who: &str) -> String {
        self.script["names"][who].as_str().unwrap().to_string()
    }

    fn lines(&self) -> Vec<Value> {
        self.script["lines"].as_array().unwrap().clone()
    }

    fn length(&self) -> f64 {
        self.script["length"].as_f64().unwrap()
    }

    fn line_start(&self, i: usize) -> f64 {
        self.lines()[i]["start"].as_f64().unwrap()
    }

    /// The slip (the line said again) as (start, end) seconds, up to the retake.
    fn slip(&self) -> (f64, f64) {
        let i = self.lines().iter().position(|l| l["slip"] == true).unwrap();
        (self.line_start(i) - 0.2, self.line_start(i + 1) - 0.2)
    }

    /// Where each topic of the run sheet begins (seconds).
    fn topic_starts(&self) -> [f64; 4] {
        [0.0, self.line_start(2) - 0.2, self.line_start(4) - 0.2, self.line_start(7) - 0.2]
    }

    fn jingles_track(&self) -> &'static str {
        if self.lang == "de" { "Jingles & Musik" } else { "Jingles & Music" }
    }

    fn topics(&self) -> [&'static str; 4] {
        if self.lang == "de" {
            ["Begrüßung", "Wie alles anfing", "Tipps für Einsteiger", "Verabschiedung"]
        } else {
            ["Welcome", "How it started", "Tips for beginners", "Goodbye"]
        }
    }

    fn speaker_tracks(&self) -> Vec<String> {
        vec![self.name("host"), self.name("guest")]
    }

    /// The episode as a session: the template, the run sheet, the jingle pad and (`audio`) the
    /// recorded voices, (`marked`) with its chapter and mistake markers.
    fn engine(&self, audio: bool, marked: bool) -> soundcraft_engine::Engine {
        self.engine_with(audio, marked, marked, false)
    }

    fn engine_with(&self, audio: bool, chapters: bool, cut: bool, subtitles: bool) -> soundcraft_engine::Engine {
        let de = self.lang == "de";
        let mut e = soundcraft_engine::Engine::default();
        e.execute("session.new", &json!({"name": if de { "Folge 12" } else { "Episode 12" }, "template": "podcast", "speakers": 2})).unwrap();
        if !de {
            for (from, to) in [("Moderation", self.name("host")), ("Gast 1", self.name("guest")), ("Jingles & Musik", self.jingles_track().into())] {
                e.execute("track.rename", &json!({"track": from, "name": to})).unwrap();
            }
        }
        let jingle = self.dir.join("Intro.wav").display().to_string();
        let rundown: Vec<Value> = self.topics().iter().zip([1, 5, 8, 1]).map(|(t, m)| json!({"title": t, "minutes": m})).collect();
        e.execute(
            "podcast.set",
            &json!({
                "rundown": rundown,
                "jingles": [jingle],
                "jingle_names": ["Intro"],
                "jingle_pads": 4,
                "episode": {
                    "title": if de { "Podcasten zu Hause" } else { "Podcasting from home" },
                    "show": if de { "Werkstattgespräch" } else { "Workshop Talk" },
                    "author": "GERBER.MEDIA",
                    "episode": 12,
                    "year": 2026,
                    "cover": self.dir.join("cover.png").display().to_string(),
                },
            }),
        )
        .unwrap();
        if audio {
            for track in [self.name("host"), self.name("guest")] {
                let path = self.dir.join(format!("{track}.wav")).display().to_string();
                e.execute("file.import_audio", &json!({"path": path, "track": track, "at": 0})).unwrap();
            }
            e.execute("file.import_audio", &json!({"path": jingle, "track": self.jingles_track(), "at": 0})).unwrap();
            e.execute("view.zoom_fit", &json!({"width_px": 760})).unwrap();
        }
        if chapters {
            for (title, at) in self.topics().iter().zip(self.topic_starts()).skip(1) {
                e.execute("markers.add", &json!({"name": title, "at": {"seconds": at}})).unwrap();
            }
        }
        if cut {
            let (slip, _) = self.slip();
            e.execute("markers.add_cut", &json!({"at": {"seconds": slip + 1.0}})).unwrap();
        }
        if subtitles {
            for l in self.lines() {
                let p = json!({"start": {"seconds": l["start"]}, "end": {"seconds": l["end"]}, "text": l["text"], "speaker": l["name"]});
                e.execute("subtitle.add", &p).unwrap();
            }
        }
        e.execute("transport.locate", &json!({"at": 0})).unwrap();
        // A saved session: the pictures do not show "modified".
        e.mark_clean();
        e
    }

    fn app(&self, audio: bool, marked: bool, workspace: Workspace) -> SoundApp {
        dress(self.engine(audio, marked), &self.lang, workspace)
    }
}

/// The app around `e` as the pictures show it: an audio engine (output only), `lang`, `workspace`.
fn dress(e: soundcraft_engine::Engine, lang: &str, workspace: Workspace) -> SoundApp {
    let player = soundcraft_playback::Player::new(Arc::new(e.session().clone()));
    let mut app = SoundApp::new(e, Some(player), Services::default());
    // Never open a microphone here.
    app.recorder_failed = true;
    app.ui.language = lang.into();
    i18n::set(lang);
    app.ui.workspace = workspace;
    app.ui.ask_workspace = false;
    // Pictures show the timeline: the side lists stay closed.
    app.ui.show_clip_list = false;
    app.ui.show_undo_history = false;
    app.ui.show_memory_locations = false;
    // A podcaster's own preferences (rulers, grid, …): the podcast template's.
    let mut template = soundcraft_engine::Engine::default();
    template.execute("session.new", &json!({"template": "podcast"})).unwrap();
    app.ui.edit_prefs = Some(soundcraft_ui_egui::prefs::capture(&template.session().edit));
    app
}

#[test]
#[ignore = "writes the tutorial pictures; run with TALKCRAFT_MEDIA, see the file header"]
fn tutorial_pictures() {
    let media = PathBuf::from(std::env::var("TALKCRAFT_MEDIA").expect("TALKCRAFT_MEDIA: the folder make_audio.py wrote"));
    let only = std::env::var("TALKCRAFT_SCENE").ok();
    // site/tutorial/make_media.sh gives the run settings of its own (nothing of the user's is read
    // or written) and a stand-in speech model, so the windows show their normal buttons.
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    assert!(home.starts_with(&media), "run through site/tutorial/make_media.sh (HOME must be inside TALKCRAFT_MEDIA)");
    type Scene = fn(&Path, &Episode);
    let scenes: &[(&str, Scene)] = &[
        ("new-episode", new_episode),
        ("remote-guests", remote_guests),
        ("recording", recording),
        ("mistakes", mistakes),
        ("pauses", pauses),
        ("text-edit", text_edit),
        ("finish", finish),
        ("subtitles", subtitles),
        ("export", export),
        ("publish", publish),
    ];
    for lang in ["de", "en"] {
        let ep = Episode::load(&media, lang);
        for (name, scene) in scenes {
            if only.as_deref().is_none_or(|o| o == *name) {
                scene(&media, &ep);
            }
        }
    }
}

/// A new episode from the podcast template.
fn new_episode(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "new-episode", dress(soundcraft_engine::Engine::default(), &ep.lang, Workspace::All));
    let c = s.cap("Ablage › Neue Session …", "File › New Session…");
    s.point_at(pos2(70.0, 18.0), c, 1100);
    {
        let app = s.app();
        app.dialogs.open_for_command(&app.engine, "session.new");
    }
    s.settle();
    let c = s.cap("Vorlage „Podcast-Folge“ und die Zahl der Sprecher wählen", "Pick the Podcast episode template and the number of speakers");
    s.show("Podcast episode", c, 2200);
    let c = s.cap(
        "Anlegen: jede Stimme bekommt ihre eigene Spur, dazu Jingles und Master",
        "Create: every voice gets its own track, plus jingles and master",
    );
    s.click("Create", c, 3200);
    s.finish();
}

/// Remote guests: the palette and the invitation link.
fn remote_guests(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "remote-guests", ep.app(false, false, Workspace::Record));
    let c = s.cap("Fenster › Ferngäste", "Window › Remote Guests");
    s.point_at(pos2(560.0, 18.0), c, 1000);
    s.run("window.remote", json!({"value": true}));
    let c = s.cap("Gäste kommen über ihren Browser dazu – ohne Installation", "Guests join from their browser – nothing to install");
    s.snap(c, 2200);
    let c = s.cap("„Studio öffnen“ startet die Studio-Seite mit dem Einladungslink", "Open Studio starts the studio page with the invitation link");
    s.show("Open Studio", c, 2600);
    s.finish();
}

/// Recording: run sheet and jingles at hand, chapters and mistakes marked on the fly.
fn recording(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "recording", ep.app(true, false, Workspace::Record));
    // The two palettes on the right, one above the other, the timeline free on the left.
    for (id, rect) in [("rundown", [880.0, 118.0, 370.0, 250.0]), ("jingles", [770.0, 440.0, 480.0, 120.0])] {
        let p = soundcraft_ui_egui::palettes::PaletteState { open: true, detached: false, rect: Some(rect), outer: None };
        s.app().ui.palettes.insert(id.into(), p);
    }
    s.settle();
    let tracks: Vec<String> =
        s.app().engine.session().tracks.iter().filter(|t| t.kind == soundcraft_model::TrackKind::Audio).map(|t| t.name.clone()).collect();
    let end = ep.length() + 1.0;
    // What has been recorded by `t`: the rest of the take is cleared for the picture, then restored.
    let take = |s: &mut Shoot, t: f64, caption: &str, ms: u32, click: bool| {
        s.run("transport.locate", json!({"at": {"seconds": t}}));
        s.run("edit.clear", json!({"tracks": tracks, "start": {"seconds": t}, "end": {"seconds": end}}));
        s.snap_with(caption, ms, click);
        s.run("edit.undo", json!({}));
    };
    let c = s.cap("Ablaufplan und Jingle-Pads liegen griffbereit", "Run sheet and jingle pads at hand");
    take(&mut s, 0.0, c, 1800, false);
    let c = s.cap("Das Intro-Jingle läuft vom Pad", "The intro jingle plays from its pad");
    // The first pad sits under the palette's title (pads are drawn, not labelled widgets).
    let title = s.node_rect("Jingles");
    let pad = pos2(title.min.x + 40.0, title.max.y + 60.0);
    s.cursor = pad;
    take(&mut s, 2.5, c, 1400, true);
    let topics = ep.topic_starts();
    let names = ep.topics();
    // The first topic starts with the episode (silently: the pad click is the picture there).
    s.run("transport.locate", json!({"at": {"seconds": 0.2}}));
    s.press("Next topic ▶ {}");
    for (k, (at, name)) in topics.iter().zip(names).enumerate().skip(1).take(2) {
        let c = s.cap("Neues Thema: ein Klick setzt das Kapitel", "Next topic: one click sets the chapter");
        s.run("transport.locate", json!({"at": {"seconds": at}}));
        let next = s.node_rect("Next topic ▶ {}").center();
        s.point_at_quiet(next);
        s.press("Next topic ▶ {}");
        let _ = (k, name);
        take(&mut s, *at + 0.05, c, 1500, true);
        if k == 1 {
            let c = s.cap("Aufnahme läuft …", "Recording …");
            take(&mut s, at + 5.0, c, 900, false);
        }
    }
    let (slip, _) = ep.slip();
    let c = s.cap("Versprecher? „Fehler“ merkt die Stelle zum späteren Schneiden", "A slip? Mistake marks the spot to cut later");
    s.run("transport.locate", json!({"at": {"seconds": slip + 1.5}}));
    let mistake = s.node_rect("✂ Mistake").center();
    s.point_at_quiet(mistake);
    s.press("✂ Mistake");
    take(&mut s, slip + 1.55, c, 2000, true);
    let c = s.cap("Weiter geht’s – geschnitten wird später", "Carry on – cutting comes later");
    take(&mut s, topics[3] + 1.0, c, 2200, false);
    s.finish();
}

impl Shoot {
    /// Moves the drawn pointer without a keyframe of its own (the next keyframe shows it there).
    fn point_at_quiet(&mut self, at: Pos2) {
        self.cursor = at;
    }
}

/// Editing: jump from mistake to mistake and cut each one on every track at once.
fn mistakes(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "mistakes", ep.app(true, true, Workspace::Edit));
    let c = s.cap("Im Modus „Schneiden“: die roten Fehler-Marker zeigen die Versprecher", "In Edit mode the red mistake markers show the slips");
    s.snap(c, 1800);
    let c = s.cap("Der Pfeil springt zum nächsten Fehler", "The arrow jumps to the next mistake");
    s.click("▶", c, 1500);
    let (a, b) = ep.slip();
    let tracks = ep.speaker_tracks();
    let c = s.cap("Den Versprecher auswählen …", "Select the slip …");
    let from = s.timeline_at(a);
    s.point_at(from, c, 600);
    s.run("edit.select", json!({"tracks": tracks, "start": {"seconds": a}, "end": {"seconds": b}}));
    let to = s.timeline_at(b);
    s.point_at(to, c, 1400);
    let c = s.cap("… und löschen: die Lücke schließt sich auf allen Spuren", "… and delete it: the gap closes on every track");
    s.run("event.cut_time", json!({"start": {"seconds": a}, "end": {"seconds": b}}));
    s.snap(c, 2000);
    // The marker goes with the cut; the tick is for a mistake left in.
    let c = s.cap("Der Fehler-Marker ist mit dem Schnitt verschwunden", "The mistake marker went with the cut");
    s.snap(c, 2200);
    s.finish();
}

/// Shorten long pauses on every track at once.
fn pauses(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "pauses", ep.app(true, true, Workspace::Edit));
    let c = s.cap("Bearbeiten › Pausen kürzen …", "Edit › Shorten Pauses…");
    s.point_at(pos2(110.0, 18.0), c, 1100);
    {
        let app = s.app();
        app.dialogs.open_for_command(&app.engine, "edit.truncate_silence");
    }
    s.settle();
    let c = s.cap("Ab welcher Länge, und auf wie lang gekürzt wird", "From which length, and how short they become");
    s.snap(c, 2200);
    let c = s.cap("Kürzen: alle Spuren, Marker und Untertitel rücken mit", "Shorten: every track, marker and subtitle moves along");
    s.click("Shorten", c, 2800);
    s.finish();
}

/// Edit by text: the filler words, found and cut in one go.
fn text_edit(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "text-edit", ep.app(true, true, Workspace::Edit));
    let words: Vec<soundcraft_ui_egui::words_window::Word> = ep
        .lines()
        .iter()
        .flat_map(|l| l["words"].as_array().cloned().unwrap_or_default())
        .map(|w| soundcraft_ui_egui::words_window::Word {
            start: (w["start"].as_f64().unwrap() * RATE) as _,
            end: (w["end"].as_f64().unwrap() * RATE) as _,
            text: w["text"].as_str().unwrap().to_string(),
            filler: w["filler"] == true,
            selected: false,
        })
        .collect();
    s.run("window.words", json!({"value": true}));
    s.app().extra.words.words = words;
    s.settle();
    // Low on the screen, so the cut shows in the timeline above it.
    let c = s.cap("Fenster › Schnitt am Text: jedes Wort mit seiner Zeit", "Window › Edit by Text: every word with its time");
    s.snap(c, 2200);
    let c = s.cap("Füllwörter auf einen Klick auswählen", "Select all filler words in one click");
    s.click("Select all fillers ({})", c, 1600);
    let c = s.cap("Schneiden: aus allen Spuren, ein Rückgängig-Schritt", "Cut: from every track, one undo step");
    s.click("Cut {} word(s) · {} s", c, 2000);
    s.run("window.words", json!({"value": false}));
    let c = s.cap("Die „Ähms“ sind weg – an jeder Schnittstelle beginnt ein neuer Clip", "The ums are gone – each cut starts a new clip");
    let at = s.timeline_at(13.0);
    s.point_at(at, c, 2600);
    s.finish();
}

/// Finish Episode: the usual clean-up steps in one go.
fn finish(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "finish", ep.app(true, true, Workspace::Edit));
    s.run("window.finish", json!({"value": true}));
    let c = s.cap("Folge fertig machen: die üblichen Schlussschritte auf einmal", "Finish Episode: the usual final steps at once");
    s.snap(c, 2400);
    let c = s.cap("Ein Klick – danach mit Vorher/Nachher vergleichen", "One click – then compare before and after");
    s.click("Finish Episode", c, 3000);
    s.finish();
}

/// Subtitles from the speech and chapters from the pauses.
fn subtitles(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "subtitles", dress(ep.engine_with(true, true, false, true), &ep.lang, Workspace::Edit));
    s.run("window.subtitles", json!({"value": true}));
    let c = s.cap("Transkribieren: auf dem eigenen Rechner, nichts geht ins Netz", "Transcribe: on your own computer, nothing goes online");
    s.show("Transcribe", c, 2400);
    let c = s.cap("Jeder Untertitel mit Zeit und Sprecher, direkt bearbeitbar", "Every subtitle with its time and speaker, ready to edit");
    let first = s.node_rect("Text").center();
    s.point_at(pos2(first.x + 120.0, first.y + 30.0), c, 2600);
    let c = s.cap("Als SRT oder WebVTT exportieren – oder als Transkript", "Export as SRT or WebVTT – or as a transcript");
    s.show("Export…", c, 2600);
    s.finish();
}

/// Export: format, loudness and the episode's details.
fn export(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "export", dress(ep.engine_with(true, true, false, true), &ep.lang, Workspace::Publish));
    {
        let app = s.app();
        app.dialogs.open_for_command(&app.engine, "file.export_podcast");
        // Paths as on a podcaster's computer, not this run's work folder.
        let folder = if ep.lang == "de" { "/Users/anna/Podcasts/Werkstattgespräch" } else { "/Users/anna/Podcasts/Workshop Talk" };
        if let Some(soundcraft_ui_egui::dialogs::Dialog::ExportEpisode { path, settings }) = &mut app.dialogs.open {
            let file = std::path::Path::new(path.as_str()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            *path = format!("{folder}/{file}");
            settings.episode.cover = format!("{folder}/cover.png");
        }
    }
    s.settle();
    let c = s.cap("Ablage › Folge exportieren …", "File › Export Episode…");
    s.snap(c, 1600);
    let c = s.cap("M4A, MP3 oder M4B – mit Kapiteln, Cover und −16 LUFS", "M4A, MP3 or M4B – with chapters, cover and −16 LUFS");
    s.show("Write markers as chapters", c, 2400);
    let c = s.cap("Shownotes und Untertitel entstehen gleich mit", "Show notes and subtitles come along");
    s.show("Export", c, 2400);
    s.finish();
}

/// Publish: the checklist and where the episode goes.
fn publish(media: &Path, ep: &Episode) {
    let mut s = Shoot::new(media, &ep.lang, "publish", dress(ep.engine_with(true, true, false, true), &ep.lang, Workspace::Publish));
    s.run("window.publish", json!({"value": true}));
    let c = s.cap("Folge veröffentlichen: die Checkliste sagt, was noch fehlt", "Publish Episode: the checklist says what is missing");
    s.snap(c, 2600);
    let c = s.cap("Ziel wählen: eigener RSS-Feed oder ein Podcast-Hoster", "Pick the target: your own RSS feed or a podcast host");
    s.show("Where to", c, 2600);
    s.finish();
}
