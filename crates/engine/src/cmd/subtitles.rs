//! Subtitle cues: add, edit, delete, import/export SRT and WebVTT, and empty cues laid out along
//! the speech in the mix so a transcript only needs typing in.

use super::*;
use crate::cmd;
use crate::subtitles::{self as subs, FileCue};
use serde_json::json;
use soundcraft_model::SubtitleCue;

/// Default length of a cue added at a point (seconds).
const DEFAULT_CUE_S: f64 = 3.0;
/// Subtitle files larger than this are refused.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "subtitle.list", "Subtitles", [], None, "{} — every cue with id, start/end (samples and seconds), speaker, text", always, list),
        cmd!(
            "subtitle.add",
            "Add Subtitle",
            [],
            Some("Cmd+Ctrl+U"),
            "{start?, end?, text?, speaker?} — default: the edit selection, or 3 s from the playhead",
            always,
            add
        ),
        cmd!("subtitle.set", "Edit Subtitle", [], None, "{id, text?, speaker?, start?, end?}", always, set),
        cmd!("subtitle.delete", "Delete Subtitle", [], None, "{id | ids}", always, delete),
        cmd!(
            "subtitle.clear",
            "Clear Subtitles",
            [],
            None,
            "{}",
            |e| if e.session().subtitles.is_empty() { Err("there are no subtitles".into()) } else { Ok(()) },
            |e, _| {
                let n = e.session().subtitles.len();
                e.session_mut().subtitles.clear();
                Ok(json!({"removed": n}))
            }
        ),
        cmd!("subtitle.import", "Import Subtitles...", [], None, "{path, at?: 0, replace?: false} — .srt or .vtt", always, import),
        cmd!(noundo "subtitle.export", "Export Subtitles...", [], None, "{path, format?: srt|vtt (from the extension), start?, end?}", always, export),
        cmd!(
            "subtitle.segment",
            "Create Cues from Speech",
            [],
            None,
            "{threshold_db?: -40, min_gap_ms?: 350, max_len_s?: 6, start?, end?, replace?: false} — empty cues along the speech in the mix",
            has_tracks,
            segment
        ),
        cmd!(
            "subtitle.transcribe",
            "Transcribe Speech",
            [],
            None,
            "{language?: from the episode (deu → de, auto), model?: path, start?, end?, replace?: true, speakers?: true, srt?: whisper output to place instead of running whisper} — subtitles with text from whisper.cpp",
            has_tracks,
            transcribe
        ),
        cmd!(
            "subtitle.assign_speakers",
            "Name Speakers from Tracks",
            [],
            None,
            "{start?, end?, overwrite?: false} — each cue's speaker = the audio track loudest under it",
            has_tracks,
            |e, p| {
                let r = full_range(e, range_param(e, "subtitle.assign_speakers", p)?);
                let n = crate::transcribe::assign_speakers(e, r, bool_or(p, "overwrite", false), "subtitle.assign_speakers")?;
                Ok(json!({"named": n}))
            }
        ),
    ]
}

fn cue_json(e: &Engine, c: &SubtitleCue) -> Value {
    let sr = e.session().sample_rate;
    json!({"id": c.id, "start": c.start, "end": c.end, "start_s": sr.seconds(c.start), "end_s": sr.seconds(c.end), "speaker": c.speaker, "text": c.text})
}

fn list(e: &mut Engine, _: &Value) -> Result<Value> {
    Ok(json!({"cues": e.session().subtitles.iter().map(|c| cue_json(e, c)).collect::<Vec<_>>()}))
}

fn find(e: &Engine, id: &str, p: &Value) -> Result<usize> {
    let want = p.get("id").and_then(Value::as_u64).ok_or_else(|| bad(id, "`id` required"))?;
    e.session().subtitles.iter().position(|c| c.id == want).ok_or_else(|| bad(id, format!("no subtitle with id {want}")))
}

fn add(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "subtitle.add";
    let s = e.session();
    let sel = s.edit.selection;
    let (d_start, d_end) =
        if sel.is_empty() { (s.edit.playhead, s.edit.playhead + s.sample_rate.samples(DEFAULT_CUE_S)) } else { (sel.start, sel.end) };
    let start = position_param(e, ID, p, "start")?.unwrap_or(d_start).max(0);
    let end = position_param(e, ID, p, "end")?.unwrap_or(d_end.max(start + 1));
    if end <= start {
        return Err(bad(ID, "`end` must be after `start`"));
    }
    let text = str_param(p, "text").unwrap_or("").to_string();
    let speaker = str_param(p, "speaker").unwrap_or("").to_string();
    let s = e.session_mut();
    let id = subs::next_id(s);
    s.subtitles.push(SubtitleCue { id, start, end, speaker, text });
    subs::sort(s);
    let c = e.session().subtitles.iter().find(|c| c.id == id).cloned().unwrap_or_default();
    Ok(cue_json(e, &c))
}

fn set(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "subtitle.set";
    let i = find(e, ID, p)?;
    let start = position_param(e, ID, p, "start")?;
    let end = position_param(e, ID, p, "end")?;
    let Some(c) = e.session().subtitles.get(i).cloned() else { return Err(bad(ID, "no such subtitle")) };
    let mut c = c;
    if let Some(t) = str_param(p, "text") {
        c.text = t.to_string();
    }
    if let Some(t) = str_param(p, "speaker") {
        c.speaker = t.to_string();
    }
    c.start = start.unwrap_or(c.start).max(0);
    c.end = end.unwrap_or(c.end);
    if c.end <= c.start {
        return Err(bad(ID, "`end` must be after `start`"));
    }
    let s = e.session_mut();
    if let Some(slot) = s.subtitles.get_mut(i) {
        *slot = c.clone();
    }
    subs::sort(s);
    Ok(cue_json(e, &c))
}

fn delete(e: &mut Engine, p: &Value) -> Result<Value> {
    let mut ids: Vec<u64> = p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    ids.extend(p.get("id").and_then(Value::as_u64));
    if ids.is_empty() {
        return Err(bad("subtitle.delete", "`id` or `ids` required"));
    }
    let s = e.session_mut();
    let before = s.subtitles.len();
    s.subtitles.retain(|c| !ids.contains(&c.id));
    let removed = before - s.subtitles.len();
    if removed == 0 {
        return Err(bad("subtitle.delete", "no such subtitle"));
    }
    Ok(json!({"removed": removed}))
}

fn import(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "subtitle.import";
    let path = str_param(p, "path").ok_or_else(|| bad(ID, "`path` required"))?.to_string();
    let len = std::fs::metadata(&path).map_err(|err| bad(ID, format!("{path}: {err}")))?.len();
    if len > MAX_FILE_BYTES {
        return Err(bad(ID, format!("{path} is larger than 20 MB")));
    }
    let bytes = std::fs::read(&path).map_err(|err| bad(ID, format!("{path}: {err}")))?;
    let text = String::from_utf8_lossy(&bytes);
    let cues = subs::parse(&text);
    if cues.is_empty() {
        return Err(bad(ID, format!("{path} has no subtitle cues")));
    }
    let at = position_param(e, ID, p, "at")?.unwrap_or(0).max(0);
    let sr = e.session().sample_rate;
    let s = e.session_mut();
    if bool_or(p, "replace", false) {
        s.subtitles.clear();
    }
    let n = cues.len();
    for c in cues {
        let id = subs::next_id(s);
        let start = at.saturating_add(sr.samples(c.start_ms as f64 / 1000.0));
        let end = at.saturating_add(sr.samples(c.end_ms as f64 / 1000.0));
        s.subtitles.push(SubtitleCue { id, start, end, speaker: c.speaker, text: c.text });
    }
    subs::sort(s);
    Ok(json!({"imported": n}))
}

/// Writes `cues` as `.srt` or `.vtt` (chosen by `format`, else the extension). Returns the path written.
pub fn write_cues(id: &str, path: &str, format: Option<&str>, cues: &[FileCue]) -> Result<String> {
    let ext = std::path::Path::new(path).extension().and_then(|x| x.to_str()).unwrap_or("srt").to_ascii_lowercase();
    let format = format.map(str::to_ascii_lowercase).unwrap_or(ext);
    let (text, ext) = match format.as_str() {
        "vtt" | "webvtt" => (subs::write_vtt(cues), "vtt"),
        "srt" => (subs::write_srt(cues), "srt"),
        other => return Err(bad(id, format!("unknown subtitle format `{other}` (use srt or vtt)"))),
    };
    let path = std::path::Path::new(path).with_extension(ext).to_string_lossy().to_string();
    crate::io::write_file(&path, text.as_bytes())?;
    Ok(path)
}

fn export(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "subtitle.export";
    let path = str_param(p, "path").ok_or_else(|| bad(ID, "`path` required"))?.to_string();
    let r = range_param(e, ID, p)?;
    let r = if r.is_empty() { soundcraft_time::Range::new(0, e.session().content_end().max(1)) } else { r };
    let cues = subs::file_cues(e.session(), r);
    if cues.is_empty() {
        return Err(bad(ID, "no subtitles with text to export"));
    }
    let written = write_cues(ID, &path, str_param(p, "format"), &cues)?;
    Ok(json!({"path": written, "cues": cues.len()}))
}

fn segment(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "subtitle.segment";
    let threshold = f64_or(p, "threshold_db", -40.0).clamp(-90.0, -6.0) as f32;
    let min_gap_ms = f64_or(p, "min_gap_ms", 350.0).clamp(50.0, 5000.0);
    let max_len_s = f64_or(p, "max_len_s", 6.0).clamp(1.0, 30.0);
    let r = range_param(e, ID, p)?;
    let r = if r.is_empty() { soundcraft_time::Range::new(0, e.session().content_end().max(1)) } else { r };
    let ch = crate::io::render_mix(e, r, ID)?;
    let sr = e.session().sample_rate;
    let min_gap = sr.samples(min_gap_ms / 1000.0).max(1) as usize;
    let max_len = sr.samples(max_len_s).max(1);
    let pad = sr.samples(0.05).max(0) as usize;
    let speech = soundcraft_dsp::offline::non_silent_ranges(&ch, threshold, min_gap, pad, pad);
    let mut spans = Vec::new();
    for (a, b) in speech {
        let (a, b) = (r.start + a as i64, (r.start + b as i64).min(r.end));
        // Long stretches without a pause are split evenly so no cue runs past `max_len_s`.
        let parts = ((b - a) as f64 / max_len as f64).ceil().max(1.0) as i64;
        let step = (b - a) / parts;
        for k in 0..parts {
            let s0 = a + k * step;
            let s1 = if k + 1 == parts { b } else { s0 + step };
            if s1 > s0 {
                spans.push((s0, s1));
            }
        }
    }
    let s = e.session_mut();
    if bool_or(p, "replace", false) {
        s.subtitles.retain(|c| c.end <= r.start || c.start >= r.end);
    }
    let n = spans.len();
    for (start, end) in spans {
        let id = subs::next_id(s);
        s.subtitles.push(SubtitleCue { id, start, end, speaker: String::new(), text: String::new() });
    }
    subs::sort(s);
    Ok(json!({"created": n}))
}

/// `r`, or the whole session when it is empty.
fn full_range(e: &Engine, r: soundcraft_time::Range) -> soundcraft_time::Range {
    if r.is_empty() { soundcraft_time::Range::new(0, e.session().content_end().max(1)) } else { r }
}

/// The language whisper should listen for: `language`, else the episode's.
pub fn transcribe_language(e: &Engine, p: &Value) -> String {
    let code = str_param(p, "language").map(str::to_string).unwrap_or_else(|| e.session().podcast.episode.language.clone());
    let w = crate::transcribe::whisper_language(&code);
    // Already a whisper code like `de`.
    if w == "auto" && code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase()) { code } else { w.to_string() }
}

fn transcribe(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "subtitle.transcribe";
    let r = full_range(e, range_param(e, ID, p)?);
    let cues = if let Some(srt) = str_param(p, "srt") {
        let bytes = std::fs::read(srt).map_err(|err| bad(ID, format!("{srt}: {err}")))?;
        subs::parse(&String::from_utf8_lossy(&bytes))
    } else {
        let job = crate::transcribe::prepare(e, r, &transcribe_language(e, p), str_param(p, "model"), ID)?;
        let out = job.run_blocking();
        job.cleanup();
        out?
    };
    let placed = crate::transcribe::place(e, r, cues, bool_or(p, "replace", true));
    let named = if bool_or(p, "speakers", true) && placed > 0 { crate::transcribe::assign_speakers(e, r, false, ID)? } else { 0 };
    Ok(json!({"created": placed, "named": named}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    #[test]
    fn add_edit_delete_and_undo() {
        let mut e = Engine::default();
        let a = e.execute("subtitle.add", &json!({"start": {"seconds": 2.0}, "end": {"seconds": 4.0}, "text": "Zweite"})).unwrap();
        e.execute("subtitle.add", &json!({"start": 0, "end": 48_000, "text": "Erste", "speaker": "Dirk"})).unwrap();
        let cues = e.execute("subtitle.list", &json!({})).unwrap();
        assert_eq!(cues["cues"][0]["text"], "Erste", "sorted by start");
        let id = a["id"].as_u64().unwrap();
        e.execute("subtitle.set", &json!({"id": id, "text": "Geändert", "end": {"seconds": 5.0}})).unwrap();
        assert_eq!(e.session().subtitles[1].text, "Geändert");
        assert!(e.execute("subtitle.set", &json!({"id": id, "end": 0})).is_err());
        e.execute("subtitle.delete", &json!({"id": id})).unwrap();
        assert_eq!(e.session().subtitles.len(), 1);
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(e.session().subtitles.len(), 2, "delete is undoable");
    }

    #[test]
    fn export_and_import_round_trip() {
        let dir = std::env::temp_dir().join(format!("sc-subs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = Engine::default();
        e.execute("subtitle.add", &json!({"start": 0, "end": 72_000, "text": "Hallo", "speaker": "Sven"})).unwrap();
        e.execute("subtitle.add", &json!({"start": 96_000, "end": 144_000, "text": ""})).unwrap();
        // The export range is the content; give the session some content end via a long selection.
        let out = dir.join("folge.vtt");
        let v = e.execute("subtitle.export", &json!({"path": out.to_string_lossy(), "start": 0, "end": 480_000})).unwrap();
        assert_eq!(v["cues"], 1, "empty cues are left out");
        let mut f = Engine::default();
        f.execute("subtitle.import", &json!({"path": out.to_string_lossy(), "at": {"seconds": 10.0}})).unwrap();
        let c = &f.session().subtitles[0];
        assert_eq!((c.start, c.end, c.speaker.as_str(), c.text.as_str()), (480_000, 552_000, "Sven", "Hallo"));
        assert!(f.execute("subtitle.import", &json!({"path": dir.join("missing.srt").to_string_lossy()})).is_err());
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn whisper_output_is_placed_with_speakers_from_the_tracks() {
        use soundcraft_audio_io::{AudioBuffer, EncodeOptions};
        let dir = std::env::temp_dir().join(format!("sc-subs-whisper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tone =
            |secs: usize| AudioBuffer { sample_rate: 48_000, channels: vec![(0..48_000 * secs).map(|i| (i as f32 * 0.02).sin() * 0.2).collect()] };
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "Folge", "template": "podcast", "speakers": 2})).unwrap();
        for (name, at) in [("Moderation", 0.0), ("Gast 1", 3.0)] {
            let wav = dir.join(format!("{name}.wav"));
            std::fs::write(&wav, soundcraft_audio_io::encode(&tone(2), &EncodeOptions::default()).unwrap()).unwrap();
            e.execute("file.import_audio", &json!({"path": wav.to_string_lossy(), "track": name, "new_tracks": false, "at": {"seconds": at}}))
                .unwrap();
        }
        let srt = dir.join("mix.srt");
        std::fs::write(&srt, "1\n00:00:00,200 --> 00:00:01,800\n Hallo, willkommen.\n\n2\n00:00:03,100 --> 00:00:04,900\n Danke!\n\n3\n00:00:05,000 --> 00:00:05,400\n[Musik]\n\n").unwrap();
        let v = e.execute("subtitle.transcribe", &json!({"srt": srt.to_string_lossy()})).unwrap();
        assert_eq!((v["created"].as_u64(), v["named"].as_u64()), (Some(2), Some(2)), "{v}");
        let got: Vec<(&str, &str)> = e.session().subtitles.iter().map(|c| (c.speaker.as_str(), c.text.as_str())).collect();
        assert_eq!(got, [("Moderation", "Hallo, willkommen."), ("Gast 1", "Danke!")]);
        e.execute("edit.undo", &json!({})).unwrap();
        assert!(e.session().subtitles.is_empty(), "one undo step");
        assert_eq!(transcribe_language(&e, &json!({})), "de");
        assert_eq!(transcribe_language(&e, &json!({"language": "en"})), "en");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }
}
