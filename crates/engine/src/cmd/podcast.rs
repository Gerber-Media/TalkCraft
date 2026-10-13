//! Podcast delivery: export settings and episode metadata saved with the session, and the
//! episode export (MP3 with ID3 tags and chapters, or WAV/FLAC/AIFF) at a loudness target.

use super::*;
use crate::cmd;
use serde_json::json;
use soundcraft_audio_io::mp3::{Chapter, EpisodeTags, MP3_BITRATES, MP3_SAMPLE_RATES, Mp3Options};
use soundcraft_audio_io::{AudioBuffer, BitDepth, EncodeOptions};
use soundcraft_model::{MarkerKind, PodcastSettings};

/// Formats an episode can be exported as.
pub const EXPORT_FORMATS: [&str; 4] = ["mp3", "wav", "flac", "aiff"];
/// Covers larger than this are refused (ID3 readers choke on huge tags).
const MAX_COVER_BYTES: u64 = 10 * 1024 * 1024;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "podcast.settings", "Podcast Settings", [], None, "{} — the session's export settings and episode metadata", always, |e, _| {
            serde_json::to_value(&e.session().podcast).map_err(|err| bad("podcast.settings", err.to_string()))
        }),
        cmd!(
            "podcast.set",
            "Podcast Settings...",
            [],
            None,
            "{export?: {format?: mp3|wav|flac|aiff, bitrate_kbps?, sample_rate?, mono?, bit_depth?: 16|24|32f, loudness_lufs?: LUFS|null, chapters?}, episode?: {title?, author?, show?, episode?, year?, genre?, description?, language?, cover?}}",
            always,
            set
        ),
        cmd!(
            noundo "file.export_podcast",
            "Export Episode...",
            [],
            Some("Cmd+Alt+E"),
            "{path, start?, end?, …any export setting to override for this file only}; the extension follows the format",
            has_tracks,
            export
        ),
        cmd!(
            noundo "podcast.export_shownotes",
            "Export Shownotes...",
            [],
            None,
            "{path, start?, end?} — writes <path>.md (title, description, chapter list) and <path>.chapters.json (Podcasting 2.0)",
            always,
            export_shownotes
        ),
        cmd!(
            "edit.truncate_silence",
            "Shorten Pauses...",
            [],
            None,
            "{threshold_db?: -45, longer_than_ms?: 1200, keep_ms?: 600, start?, end?, dry_run?: false} — shortens every pause in the mix longer than `longer_than_ms` to `keep_ms`, on all tracks at once",
            has_tracks,
            truncate_silence
        ),
    ]
}

/// `base` with the keys of `patch` replaced (one level deep for objects).
fn merge(base: &mut Value, patch: &Value) {
    if let (Some(b), Some(p)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            match b.get_mut(k) {
                Some(slot) if slot.is_object() && v.is_object() => merge(slot, v),
                Some(slot) => *slot = v.clone(),
                None => {}
            }
        }
    }
}

/// Parse and check settings; unknown values are errors, not silently replaced.
fn checked(id: &str, v: Value) -> Result<PodcastSettings> {
    let s: PodcastSettings = serde_json::from_value(v).map_err(|err| bad(id, format!("invalid podcast settings: {err}")))?;
    let x = &s.export;
    if !EXPORT_FORMATS.contains(&x.format.as_str()) {
        return Err(bad(id, format!("unknown format `{}` (use {})", x.format, EXPORT_FORMATS.join(", "))));
    }
    if !MP3_BITRATES.contains(&x.bitrate_kbps) {
        return Err(bad(id, format!("bitrate {} kbit/s is not one of {MP3_BITRATES:?}", x.bitrate_kbps)));
    }
    if !MP3_SAMPLE_RATES.contains(&x.sample_rate) && !(x.format != "mp3" && matches!(x.sample_rate, 88_200 | 96_000)) {
        return Err(bad(id, format!("sample rate {} Hz is not one of {MP3_SAMPLE_RATES:?}", x.sample_rate)));
    }
    if !matches!(x.bit_depth.as_str(), "16" | "24" | "32f") {
        return Err(bad(id, format!("bit depth `{}` (use 16, 24 or 32f)", x.bit_depth)));
    }
    if let Some(l) = x.loudness_lufs
        && !(l.is_finite() && (-40.0..=0.0).contains(&l))
    {
        return Err(bad(id, "`loudness_lufs` is between -40 and 0 LUFS, or null for none"));
    }
    Ok(s)
}

fn set(e: &mut Engine, p: &Value) -> Result<Value> {
    let mut v = serde_json::to_value(&e.session().podcast).map_err(|err| bad("podcast.set", err.to_string()))?;
    merge(&mut v, p);
    let s = checked("podcast.set", v)?;
    e.session_mut().podcast = s;
    serde_json::to_value(&e.session().podcast).map_err(|err| bad("podcast.set", err.to_string()))
}

/// Markers inside `r` as chapters, each running to the next marker (or the end of the export).
fn chapters(e: &Engine, r: soundcraft_time::Range) -> Vec<Chapter> {
    let s = e.session();
    let sr = s.sample_rate.as_f64().max(1.0);
    let ms = |at: i64| ((at.saturating_sub(r.start)).max(0) as f64 * 1000.0 / sr).min(f64::from(u32::MAX)) as u32;
    let mut marks: Vec<_> = s.markers.iter().filter(|m| m.kind == MarkerKind::Marker && m.start >= r.start && m.start < r.end).collect();
    marks.sort_by_key(|m| m.start);
    let starts: Vec<i64> = marks.iter().map(|m| m.start).chain(std::iter::once(r.end)).collect();
    marks
        .iter()
        .zip(starts.windows(2))
        .map(|(m, w)| Chapter {
            title: m.name.clone(),
            start_ms: ms(w.first().copied().unwrap_or(r.start)),
            end_ms: ms(w.get(1).copied().unwrap_or(r.end)),
        })
        .filter(|c| c.end_ms > c.start_ms)
        .collect()
}

fn cover(id: &str, path: &str) -> Result<Option<(String, Vec<u8>)>> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    let ext = std::path::Path::new(path).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        _ => return Err(bad(id, format!("cover `{path}` must be a JPEG or PNG"))),
    };
    let len = std::fs::metadata(path).map_err(|err| bad(id, format!("cover `{path}`: {err}")))?.len();
    if len > MAX_COVER_BYTES {
        return Err(bad(id, format!("cover `{path}` is larger than 10 MB")));
    }
    let data = std::fs::read(path).map_err(|err| bad(id, format!("cover `{path}`: {err}")))?;
    Ok(Some((mime.to_string(), data)))
}

fn export(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "file.export_podcast";
    let path = str_param(p, "path").ok_or_else(|| bad(ID, "`path` required"))?.to_string();
    let mut v = serde_json::to_value(&e.session().podcast).map_err(|err| bad(ID, err.to_string()))?;
    if let Some(over) = p.as_object() {
        let export: serde_json::Map<String, Value> =
            over.iter().filter(|(k, _)| !matches!(k.as_str(), "path" | "start" | "end")).map(|(k, v)| (k.clone(), v.clone())).collect();
        merge(&mut v, &json!({"export": export}));
    }
    let settings = checked(ID, v)?;
    let x = &settings.export;
    let r = range_param(e, ID, p)?;
    let r = if r.is_empty() { soundcraft_time::Range::new(0, e.session().content_end().max(1)) } else { r };

    let mut ch = crate::io::render_mix(e, r, ID)?;
    let frames = ch.iter().map(Vec::len).min().unwrap_or(0);
    if x.mono && ch.len() > 1 {
        let n = ch.len() as f32;
        let m: Vec<f32> = (0..frames).map(|i| ch.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / n).collect();
        ch = vec![m];
    } else if !x.mono && ch.len() == 1 {
        let c = ch.first().cloned().unwrap_or_default();
        ch = vec![c.clone(), c];
    }
    let from = e.session().sample_rate.hz();
    if from != x.sample_rate {
        ch = soundcraft_dsp::offline::resample(&ch, from, x.sample_rate);
    }
    let sr = x.sample_rate as f32;
    if let Some(target) = x.loudness_lufs {
        soundcraft_dsp::offline::normalize_loudness(&mut ch, sr, target, crate::podcast::PODCAST_TRUE_PEAK - 0.5);
    }
    let mut meter = soundcraft_dsp::meter::LoudnessMeter::new(sr, ch.len());
    meter.process(&ch, ch.iter().map(Vec::len).min().unwrap_or(0));
    let (lufs, true_peak) = (meter.integrated_lufs(), meter.true_peak_db());

    let buf = AudioBuffer { sample_rate: x.sample_rate, channels: ch };
    let mut chapter_count = 0;
    let bytes = if x.format == "mp3" {
        let ep = &settings.episode;
        let chaps = if x.chapters { chapters(e, r) } else { Vec::new() };
        chapter_count = chaps.len();
        let tags = EpisodeTags {
            title: if ep.title.trim().is_empty() { e.session().name.clone() } else { ep.title.clone() },
            artist: ep.author.clone(),
            album: ep.show.clone(),
            episode: ep.episode,
            year: ep.year,
            genre: ep.genre.clone(),
            description: ep.description.clone(),
            language: ep.language.clone(),
            cover: cover(ID, &ep.cover)?,
            chapters: chaps,
        };
        soundcraft_audio_io::mp3::encode_mp3(&buf, &Mp3Options { bitrate_kbps: x.bitrate_kbps }, Some(&tags))
            .map_err(|err| bad(ID, err.to_string()))?
    } else {
        let format = soundcraft_audio_io::encode_format_for(&x.format).ok_or_else(|| bad(ID, format!("cannot write `{}`", x.format)))?;
        let bit_depth = match x.bit_depth.as_str() {
            "16" => BitDepth::Int16,
            "32f" => BitDepth::Float32,
            _ => BitDepth::Int24,
        };
        soundcraft_audio_io::encode(&buf, &EncodeOptions { format, bit_depth, dither: bit_depth != BitDepth::Float32, bwf: None })
            .map_err(|err| bad(ID, err.to_string()))?
    };
    let ext = if x.format == "aiff" { "aif" } else { x.format.as_str() };
    let path = std::path::Path::new(&path).with_extension(ext).to_string_lossy().to_string();
    crate::io::write_file(&path, &bytes)?;
    let mut extra = Vec::new();
    if x.shownotes {
        extra.extend(write_shownotes(e, &settings, r, &path)?);
    }
    if x.subtitles {
        let cues = crate::subtitles::file_cues(e.session(), r);
        if !cues.is_empty() {
            for f in ["srt", "vtt"] {
                extra.push(super::subtitles::write_cues(ID, &path, Some(f), &cues)?);
            }
        }
    }
    Ok(json!({
        "path": path, "format": x.format, "bytes": bytes.len(), "seconds": buf.frames() as f64 / f64::from(x.sample_rate),
        "sample_rate": x.sample_rate, "channels": buf.channels.len(), "lufs": lufs, "true_peak_db": true_peak, "chapters": chapter_count,
        "also_written": extra
    }))
}

/// Shownotes as Markdown in the episode's language (German or English).
pub fn shownotes_markdown(e: &Engine, settings: &PodcastSettings, r: soundcraft_time::Range) -> String {
    let ep = &settings.episode;
    let de = ep.language == "deu";
    let title = if ep.title.trim().is_empty() { e.session().name.clone() } else { ep.title.trim().to_string() };
    let ms = (e.session().sample_rate.seconds(r.len()) * 1000.0).max(0.0) as u64;
    let mut head = Vec::new();
    if !ep.show.trim().is_empty() {
        head.push(ep.show.trim().to_string());
    }
    if ep.episode > 0 {
        head.push(format!("{} {}", if de { "Folge" } else { "Episode" }, ep.episode));
    }
    head.push(format!("{} {}", if de { "Dauer" } else { "Length" }, crate::subtitles::clock(ms)));
    let mut md = format!("# {title}\n\n{}\n", head.join(" · "));
    if !ep.author.trim().is_empty() {
        md += &format!("\n{}: {}\n", if de { "Mit" } else { "With" }, ep.author.trim());
    }
    if !ep.description.trim().is_empty() {
        md += &format!("\n{}\n", ep.description.trim());
    }
    let chaps = chapters(e, r);
    if !chaps.is_empty() {
        md += &format!("\n## {}\n\n", if de { "Kapitel" } else { "Chapters" });
        for c in &chaps {
            md += &format!("- {} {}\n", crate::subtitles::clock(u64::from(c.start_ms)), c.title);
        }
    }
    md
}

/// Podcasting 2.0 chapters (`application/json+chapters`, version 1.2.0).
pub fn chapters_json(e: &Engine, r: soundcraft_time::Range) -> Value {
    let list: Vec<Value> = chapters(e, r).iter().map(|c| json!({"startTime": f64::from(c.start_ms) / 1000.0, "title": c.title})).collect();
    json!({"version": "1.2.0", "chapters": list})
}

/// Writes `<base>.md` and `<base>.chapters.json`; returns the paths.
fn write_shownotes(e: &Engine, settings: &PodcastSettings, r: soundcraft_time::Range, base: &str) -> Result<Vec<String>> {
    let stem = std::path::Path::new(base).with_extension("");
    let md = stem.with_extension("md").to_string_lossy().to_string();
    crate::io::write_file(&md, shownotes_markdown(e, settings, r).as_bytes())?;
    let mut paths = vec![md];
    if !chapters(e, r).is_empty() {
        let json_path = format!("{}.chapters.json", stem.to_string_lossy());
        let text = serde_json::to_string_pretty(&chapters_json(e, r)).map_err(|err| bad("podcast.export_shownotes", err.to_string()))?;
        crate::io::write_file(&json_path, text.as_bytes())?;
        paths.push(json_path);
    }
    Ok(paths)
}

fn export_shownotes(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "podcast.export_shownotes";
    let path = str_param(p, "path").ok_or_else(|| bad(ID, "`path` required"))?.to_string();
    let r = range_param(e, ID, p)?;
    let r = if r.is_empty() { soundcraft_time::Range::new(0, e.session().content_end().max(1)) } else { r };
    let settings = e.session().podcast.clone();
    Ok(json!({"written": write_shownotes(e, &settings, r, &path)?}))
}

/// Pauses in the mix over `r`: `(start, end)` in session samples of every silent gap between
/// speech that is longer than `longer_than` samples. Leading and trailing silence is left alone.
fn pauses(e: &Engine, r: soundcraft_time::Range, threshold_db: f32, longer_than: i64) -> Result<Vec<(i64, i64)>> {
    let ch = crate::io::render_mix(e, r, "edit.truncate_silence")?;
    let sr = e.session().sample_rate;
    // Gaps shorter than 150 ms (between words) never split speech.
    let min_gap = sr.samples(0.15).max(1) as usize;
    let speech = soundcraft_dsp::offline::non_silent_ranges(&ch, threshold_db, min_gap, 0, 0);
    Ok(speech
        .windows(2)
        .filter_map(|w| match w {
            [(_, a_end), (b_start, _)] => Some((r.start + *a_end as i64, r.start + *b_start as i64)),
            _ => None,
        })
        .filter(|(a, b)| b - a > longer_than)
        .collect())
}

fn truncate_silence(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "edit.truncate_silence";
    let threshold = f64_or(p, "threshold_db", -45.0).clamp(-90.0, -10.0) as f32;
    let sr = e.session().sample_rate;
    let longer = sr.samples(f64_or(p, "longer_than_ms", 1200.0).clamp(100.0, 60_000.0) / 1000.0);
    let keep = sr.samples(f64_or(p, "keep_ms", 600.0).clamp(0.0, 60_000.0) / 1000.0).min(longer);
    let r = range_param(e, ID, p)?;
    let r = if r.is_empty() { soundcraft_time::Range::new(0, e.session().content_end().max(1)) } else { r };
    let found = pauses(e, r, threshold, longer)?;
    // Keep half of `keep` on each side of the pause so breaths and word endings stay natural.
    let cuts: Vec<soundcraft_time::Range> =
        found.iter().map(|(a, b)| soundcraft_time::Range::new(a + keep / 2, b - (keep - keep / 2))).filter(|c| !c.is_empty()).collect();
    let removed: i64 = cuts.iter().map(soundcraft_time::Range::len).sum();
    let list: Vec<Value> = found.iter().map(|(a, b)| json!({"start_s": sr.seconds(*a), "length_s": sr.seconds(b - a)})).collect();
    if !bool_or(p, "dry_run", false) {
        // Last pause first, so earlier positions stay valid.
        for c in cuts.iter().rev() {
            crate::edit::cut_time(e.session_mut(), *c, true);
        }
    }
    Ok(json!({"pauses": list, "count": cuts.len(), "removed_s": sr.seconds(removed)}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    fn episode_engine(dir: &std::path::Path) -> Engine {
        let sr = 48_000.0f32;
        let voice: Vec<f32> = (0..(48_000 * 6))
            .map(|i| {
                let t = i as f32 / sr;
                let f = 140.0 * std::f32::consts::TAU * t;
                (f.sin() + 0.5 * (2.0 * f).sin()) * (t * 3.0 * std::f32::consts::TAU).sin().abs() * 0.1
            })
            .collect();
        let buf = AudioBuffer { sample_rate: 48_000, channels: vec![voice] };
        let wav = dir.join("voice.wav");
        std::fs::write(&wav, soundcraft_audio_io::encode(&buf, &EncodeOptions::default()).unwrap()).unwrap();
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "Folge 7", "template": "podcast", "speakers": 1})).unwrap();
        e.execute("file.import_audio", &json!({"path": wav.to_string_lossy(), "track": "Moderation", "new_tracks": false})).unwrap();
        e.session_mut().add_marker("Thema", MarkerKind::Marker, 48_000 * 3, 48_000 * 3);
        e
    }

    #[test]
    fn settings_merge_validate_and_persist_in_the_session() {
        let mut e = Engine::default();
        let v = e.execute("podcast.set", &json!({"export": {"bitrate_kbps": 96, "mono": true}, "episode": {"show": "Europa 2.0"}})).unwrap();
        assert_eq!(v["export"]["bitrate_kbps"], 96);
        assert_eq!(v["export"]["format"], "mp3", "untouched keys keep their value");
        assert_eq!(e.session().podcast.episode.show, "Europa 2.0");
        for bad in [json!({"export": {"bitrate_kbps": 100}}), json!({"export": {"format": "ogg"}}), json!({"export": {"loudness_lufs": 3}})] {
            assert!(e.execute("podcast.set", &bad).is_err(), "{bad}");
        }
        assert_eq!(e.session().podcast.export.bitrate_kbps, 96, "a refused change leaves the settings alone");
        e.execute("undo", &json!({})).ok();
    }

    #[test]
    fn wav_export_resamples_folds_to_mono_and_hits_loudness() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = episode_engine(&dir);
        let out = dir.join("folge.mp3");
        let v = e
            .execute(
                "file.export_podcast",
                &json!({"path": out.to_string_lossy(), "format": "wav", "mono": true, "sample_rate": 44_100, "loudness_lufs": -19}),
            )
            .unwrap();
        assert!(v["path"].as_str().unwrap().ends_with("folge.wav"), "{v}");
        assert_eq!(v["channels"], 1);
        assert!((v["lufs"].as_f64().unwrap() + 19.0).abs() < 1.0, "{v}");
        assert!(v["true_peak_db"].as_f64().unwrap() <= -0.9, "{v}");
        let (info, _) = soundcraft_audio_io::decode(&std::fs::read(dir.join("folge.wav")).unwrap(), Some("wav")).unwrap();
        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(e.session().podcast.export.format, "mp3", "overrides are for this file only");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn chapters_run_from_marker_to_marker() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-chapters-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let e = episode_engine(&dir);
        let c = chapters(&e, soundcraft_time::Range::new(0, 48_000 * 6));
        let got: Vec<_> = c.iter().map(|c| (c.title.as_str(), c.start_ms, c.end_ms)).collect();
        assert_eq!(got, [("Intro", 0, 3000), ("Thema", 3000, 6000)]);
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    /// 2 s speech, 3 s silence, 2 s speech, as a mono 48 kHz WAV on the Moderation track.
    fn engine_with_pause(dir: &std::path::Path) -> Engine {
        let sr = 48_000usize;
        let talk = |n: usize| (0..n).map(|i| (i as f32 * 140.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.3).collect::<Vec<f32>>();
        let mut v = talk(2 * sr);
        v.extend(std::iter::repeat_n(0.0, 3 * sr));
        v.extend(talk(2 * sr));
        let wav = dir.join("pause.wav");
        let buf = AudioBuffer { sample_rate: 48_000, channels: vec![v] };
        std::fs::write(&wav, soundcraft_audio_io::encode(&buf, &EncodeOptions::default()).unwrap()).unwrap();
        let mut e = Engine::default();
        e.execute("session.new", &json!({"template": "podcast", "speakers": 1})).unwrap();
        e.execute("file.import_audio", &json!({"path": wav.to_string_lossy(), "track": "Moderation", "new_tracks": false})).unwrap();
        e
    }

    #[test]
    fn long_pauses_shrink_on_all_tracks_and_markers_and_subtitles_follow() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-pause-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = engine_with_pause(&dir);
        e.session_mut().add_marker("Teil 2", MarkerKind::Marker, 48_000 * 4, 48_000 * 4);
        e.execute("subtitle.add", &json!({"start": 48_000 * 5, "end": 48_000 * 6, "text": "nach der Pause"})).unwrap();
        let dry = e.execute("edit.truncate_silence", &json!({"dry_run": true})).unwrap();
        assert_eq!(dry["count"], 1, "{dry}");
        assert_eq!(e.session().content_end(), 48_000 * 7, "a dry run changes nothing");
        let v = e.execute("edit.truncate_silence", &json!({"longer_than_ms": 1000, "keep_ms": 600})).unwrap();
        let removed = v["removed_s"].as_f64().unwrap();
        assert!((removed - 2.4).abs() < 0.1, "{v}");
        let end = e.session().content_end() as f64 / 48_000.0;
        assert!((end - (7.0 - removed)).abs() < 0.01, "content now ends at {end}");
        let m = e.session().markers.iter().find(|m| m.name == "Teil 2").map(|m| m.start).unwrap();
        assert!((m as f64 / 48_000.0 - 2.3).abs() < 0.1, "the marker in the pause moves to the cut, at {m}");
        let c = &e.session().subtitles[0];
        assert!(((c.start as f64 / 48_000.0) - (5.0 - removed)).abs() < 0.01);
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(e.session().content_end(), 48_000 * 7, "one undo restores everything");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn shownotes_list_chapters_in_the_episode_language() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = episode_engine(&dir);
        e.execute("podcast.set", &json!({"episode": {"title": "Energie", "show": "Europa 2.0", "episode": 12, "description": "Worum es geht."}}))
            .unwrap();
        let v = e.execute("podcast.export_shownotes", &json!({"path": dir.join("folge12.mp3").to_string_lossy()})).unwrap();
        assert_eq!(v["written"].as_array().unwrap().len(), 2, "{v}");
        let md = std::fs::read_to_string(dir.join("folge12.md")).unwrap();
        assert!(md.starts_with("# Energie\n\nEuropa 2.0 · Folge 12 · Dauer 00:06\n"), "{md}");
        assert!(md.contains("## Kapitel\n\n- 00:00 Intro\n- 00:03 Thema\n"), "{md}");
        let json: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("folge12.chapters.json")).unwrap()).unwrap();
        assert_eq!(json["chapters"][1], json!({"startTime": 3.0, "title": "Thema"}));
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[cfg(feature = "mp3")]
    #[test]
    fn mp3_export_carries_tags_and_chapters() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-mp3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = episode_engine(&dir);
        e.execute("podcast.set", &json!({"episode": {"show": "Mein Podcast", "author": "Dirk", "episode": 7}})).unwrap();
        let v = e.execute("file.export_podcast", &json!({"path": dir.join("folge").to_string_lossy()})).unwrap();
        assert_eq!(v["chapters"], 2, "{v}");
        let bytes = std::fs::read(v["path"].as_str().unwrap()).unwrap();
        let (info, _) = soundcraft_audio_io::decode(&bytes, Some("mp3")).unwrap();
        assert_eq!(info.sample_rate, 44_100);
        assert!((v["lufs"].as_f64().unwrap() + 16.0).abs() < 1.0, "{v}");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }
}
