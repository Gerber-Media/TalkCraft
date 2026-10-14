//! Podcast delivery: export settings and episode metadata saved with the session, and the
//! episode export (MP3 with ID3 tags and chapters, or WAV/FLAC/AIFF) at a loudness target.

use super::*;
use crate::cmd;
use serde_json::json;
use soundcraft_audio_io::mp3::{Chapter, EpisodeTags, MP3_BITRATES, MP3_SAMPLE_RATES, Mp3Options};
use soundcraft_audio_io::{AudioBuffer, BitDepth, EncodeOptions};
use soundcraft_model::{MarkerKind, PodcastSettings};

/// Formats an episode can be exported as.
/// M4A (AAC) first: better sound than MP3 at the same size and played by every podcast app;
/// M4B is the audiobook variant.
pub const EXPORT_FORMATS: [&str; 6] = ["m4a", "mp3", "m4b", "wav", "flac", "aiff"];
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
            "{export?: {format?: m4a|mp3|m4b|wav|flac|aiff, bitrate_kbps?, sample_rate?, mono?, bit_depth?: 16|24|32f, loudness_lufs?: LUFS|null, chapters?}, episode?: {title?, author?, show?, episode?, year?, genre?, description?, language?, cover?}}",
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
        cmd!(
            "podcast.match_speakers",
            "Match Speaker Levels",
            [],
            None,
            "{target_lufs?: -20, tracks?} — sets the clip gains of each speaker track (default: every audio track not locked against recording) so all speakers are equally loud",
            has_tracks,
            match_speakers
        ),
        cmd!(
            "podcast.place_jingle",
            "Place Jingle",
            [],
            None,
            "{pad: 1..8, at?: position (default: the insertion point)} — places the pad's audio file on the jingle track",
            has_tracks,
            place_jingle
        ),
        cmd!(
            "podcast.sync_clip",
            "Sync Clip to Other Tracks",
            [],
            None,
            "{clip, max_shift_s?: 20, reference?: track} — moves the clip so its audio lines up with the other tracks, or only with `reference` (cross-correlation; e.g. a guest's own recording against the call)",
            has_tracks,
            sync_clip
        ),
        cmd!(
            "podcast.add_speaker",
            "Add Speaker",
            [],
            None,
            "{name?} — a mono speaker track with the voice chain on the next free microphone input, named \"Gast N\" unless `name` is given, placed after the other speakers",
            always,
            add_speaker
        ),
        cmd!(
            "edit.extend_clip",
            "Extend Clip Edge",
            [],
            None,
            "{clip, edge: start|end, to: position, ripple?: false, all_tracks?: false} — pulls a clip edge back out over audio that was cut away (never past the recording). With `ripple` the rest of the track moves along (with `all_tracks` every track, and clips that were cut at the same point there open up too); without, the edge stops at the neighbouring clip",
            has_tracks,
            extend_clip
        ),
        cmd!(
            "podcast.import_remote",
            "Import Remote Guest",
            [],
            None,
            "{guest: name, path: the guest's own recording, at?: position, live?: path of the call recording, live_at?: position (default `at`)} — puts the recording on the guest's track (made when missing); with `live`, the call recording goes muted onto \"<guest> (Live)\" and the recording is synced to it",
            always,
            import_remote
        ),
        cmd!(
            "podcast.auto_duck",
            "Auto-Duck Music",
            [],
            None,
            "{music?: track (default: the locked jingle track), amount_db?: -12, attack_ms?: 300, release_ms?: 800, threshold_db?: -40} — writes volume automation that lowers the music while anyone speaks",
            has_tracks,
            auto_duck
        ),
    ]
}

/// Most jingle pads.
pub const JINGLE_PADS: usize = 8;

/// The music/jingle track: the first audio track locked against recording.
pub fn jingle_track(s: &soundcraft_model::Session) -> Option<soundcraft_model::TrackId> {
    s.tracks.iter().find(|t| t.kind == soundcraft_model::TrackKind::Audio && t.mixer.record_lock).map(|t| t.id)
}

fn place_jingle(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "podcast.place_jingle";
    let pad = p.get("pad").and_then(Value::as_u64).filter(|n| (1..=JINGLE_PADS as u64).contains(n)).ok_or_else(|| bad(ID, "`pad` is 1 to 8"))?;
    let path = e.session().podcast.jingles.get(pad as usize - 1).cloned().unwrap_or_default();
    if path.trim().is_empty() {
        return Err(bad(ID, format!("pad {pad} is empty: give it an audio file first")));
    }
    let track = jingle_track(e.session()).ok_or_else(|| bad(ID, "no jingle track: lock a track against recording to use it for jingles"))?;
    let at = position_param(e, ID, p, "at")?.unwrap_or(e.session().edit.selection.start).max(0);
    let bytes = std::fs::read(&path).map_err(|err| bad(ID, format!("{path}: {err}")))?;
    let name = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or("Jingle").to_string();
    let r = crate::io::import_audio_bytes(e, &name, &bytes, Some(path.as_str()), Some(track), at)?;
    Ok(json!({"pad": pad, "placed": r}))
}

/// The speaker tracks: `tracks`, else every audio track that records (not locked).
fn speaker_tracks(e: &Engine, p: &Value) -> Vec<soundcraft_model::TrackId> {
    match p.get("tracks").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_u64).map(soundcraft_model::TrackId).collect(),
        None => crate::podcast::record_targets(e.session()),
    }
}

fn match_speakers(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "podcast.match_speakers";
    let target = f64_or(p, "target_lufs", -20.0).clamp(-40.0, -5.0) as f32;
    let mut changes: Vec<(soundcraft_model::TrackId, f32)> = Vec::new();
    for t in speaker_tracks(e, p) {
        let s = e.session();
        let Some(track) = s.track(t) else { continue };
        // The track's clips back to back, each at its clip gain: what the speaker sounds like.
        let mut joined: Vec<Vec<f32>> = Vec::new();
        let mut rate = s.sample_rate.as_f64() as f32;
        for c in track.clips() {
            let Some((ch, r)) = super::clip::clip_audio(s, c) else { continue };
            rate = r;
            let g = 10f32.powf(c.gain_db / 20.0);
            let mono: Vec<f32> =
                (0..ch.iter().map(Vec::len).min().unwrap_or(0)).map(|i| ch.iter().map(|x| x[i]).sum::<f32>() / ch.len() as f32 * g).collect();
            if joined.is_empty() {
                joined.push(Vec::new());
            }
            if let Some(j) = joined.first_mut() {
                j.extend(mono);
            }
        }
        if joined.first().is_none_or(Vec::is_empty) {
            continue;
        }
        let lufs = soundcraft_dsp::offline::integrated_lufs(&joined, rate);
        if lufs.is_finite() && lufs > -70.0 {
            changes.push((t, (target - lufs).clamp(-30.0, 30.0)));
        }
    }
    if changes.is_empty() {
        return Err(bad(ID, "no speaker track holds audio"));
    }
    let s = e.session_mut();
    let mut out = Vec::new();
    for (t, delta) in changes {
        if let Some(track) = s.track_mut(t) {
            if let Some(pl) = track.playlist_mut() {
                for c in &mut pl.clips {
                    c.gain_db = (c.gain_db + delta).clamp(-144.0, 36.0);
                }
            }
            out.push(json!({"track": track.name, "change_db": delta}));
        }
    }
    Ok(json!({"tracks": out}))
}

/// Mean absolute level per block of `block` samples (a coarse envelope for correlation).
fn envelope(x: &[f32], block: usize) -> Vec<f32> {
    x.chunks(block.max(1)).map(|c| c.iter().map(|v| v.abs()).sum::<f32>() / c.len().max(1) as f32).collect()
}

/// The lag in `lags` where `b` matches `a` best (`a[i + lag] ≈ b[i]`), by normalised
/// cross-correlation of the mean-removed signals, with its correlation (-1…1).
fn best_lag(a: &[f32], b: &[f32], lags: std::ops::RangeInclusive<i64>) -> Option<(i64, f32)> {
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let (ma, mb) = (mean(a), mean(b));
    let mut best: Option<(i64, f32)> = None;
    for lag in lags {
        let (mut num, mut ea, mut eb) = (0.0f64, 0.0f64, 0.0f64);
        for (i, y) in b.iter().enumerate() {
            let Some(x) = usize::try_from(i as i64 + lag).ok().and_then(|j| a.get(j)) else { continue };
            let (x, y) = (f64::from(x - ma), f64::from(y - mb));
            num += x * y;
            ea += x * x;
            eb += y * y;
        }
        if ea > 0.0 && eb > 0.0 {
            let r = (num / (ea * eb).sqrt()) as f32;
            if best.is_none_or(|(_, br)| r > br) {
                best = Some((lag, r));
            }
        }
    }
    best
}

fn sync_clip(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "podcast.sync_clip";
    let id = soundcraft_model::ClipId(p.get("clip").and_then(Value::as_u64).ok_or_else(|| bad(ID, "`clip` required"))?);
    let max_s = f64_or(p, "max_shift_s", 20.0).clamp(0.1, 120.0);
    let s = e.session();
    let (track, clip) = s.find_clip(id).map(|(t, c)| (t, c.clone())).ok_or_else(|| bad(ID, "no such clip"))?;
    if !matches!(clip.content, soundcraft_model::ClipContent::Audio { .. }) {
        return Err(bad(ID, "the clip holds no audio"));
    }
    let sr = s.sample_rate;
    // Up to 60 s of the clip are plenty to find the offset. Both sides are compared as they sound
    // (through their tracks' plugins), so filter delays do not bias the match.
    let n = clip.length.min(sr.samples(60.0)).max(1);
    let ch = crate::io::render_track(e, soundcraft_time::Range::new(clip.start, clip.start + n), track, ID)?;
    let n = ch.iter().map(Vec::len).min().unwrap_or(0);
    let mono: Vec<f32> = (0..n).map(|i| ch.iter().filter_map(|c| c.get(i)).sum::<f32>() / ch.len().max(1) as f32).collect();
    let s = e.session();
    // Everything else, over the clip's span plus the search range.
    let shift = sr.samples(max_s);
    let r = soundcraft_time::Range::new((clip.start - shift).max(0), clip.start + n as i64 + shift);
    let reference_track = track_param(e, ID, p, "reference")?;
    let mut others = s.clone();
    for t in &mut others.tracks {
        if t.id == track || reference_track.is_some_and(|r| r != t.id) {
            t.mixer.mute = true;
        } else if reference_track.is_some() {
            t.mixer.mute = false;
        }
    }
    let refch = soundcraft_mix::render_range(&others, r, 1024);
    let rn = refch.iter().map(Vec::len).min().unwrap_or(0);
    let reference: Vec<f32> = (0..rn).map(|i| refch.iter().filter_map(|c| c.get(i)).sum::<f32>() / refch.len().max(1) as f32).collect();
    // Coarse: 5 ms envelopes over the whole search range.
    let block = (sr.samples(0.005).max(1)) as usize;
    let (ea, eb) = (envelope(&reference, block), envelope(&mono, block));
    let span = (ea.len() as i64 - eb.len() as i64).max(0);
    let (coarse, score) = best_lag(&ea, &eb, 0..=span).ok_or_else(|| bad(ID, "the other tracks are silent there"))?;
    if score < 0.3 {
        return Err(bad(ID, format!("no clear match with the other tracks (correlation {score:.2})")));
    }
    // Fine: sample-accurate within two blocks of the coarse match, on 5 s of audio.
    let fine_len = mono.len().min(sr.samples(5.0).max(0) as usize);
    let around = coarse * block as i64;
    let lo = (around - 2 * block as i64).max(0) as usize;
    let hi = lo + fine_len + 4 * block;
    let window = reference.get(lo.min(reference.len())..hi.min(reference.len())).unwrap_or(&[]);
    let lag = best_lag(window, mono.get(..fine_len).unwrap_or(&[]), 0..=(4 * block) as i64).map_or(around - lo as i64, |(l, _)| l);
    let new_start = r.start + lo as i64 + lag;
    let delta = new_start - clip.start;
    if delta != 0 {
        let s = e.session_mut();
        crate::edit::move_clips(s, &[id], delta, None);
    }
    Ok(json!({"moved": delta, "moved_s": sr.seconds(delta), "correlation": score}))
}

/// Clips on `tracks` (other than `except`) starting at or after `from`.
fn clips_from(
    s: &soundcraft_model::Session,
    tracks: &[soundcraft_model::TrackId],
    from: Samples,
    except: soundcraft_model::ClipId,
) -> Vec<soundcraft_model::ClipId> {
    s.tracks
        .iter()
        .filter(|t| tracks.contains(&t.id))
        .flat_map(|t| t.clips().iter())
        .filter(|c| c.start >= from && c.id != except)
        .map(|c| c.id)
        .collect()
}

/// Where a clip's recording begins and ends on the timeline (its trim limits).
fn file_span(s: &soundcraft_model::Session, c: &soundcraft_model::Clip) -> (Samples, Samples) {
    let first = c.start - c.source_offset();
    let frames = c.source().and_then(|src| s.source(src)).map_or(c.length, |src| i64::try_from(src.frames).unwrap_or(i64::MAX));
    (first, first.saturating_add(frames))
}

/// How far a clip's edge can be pulled out on its own track: never past its recording, never
/// further than the cut to the next piece of the same recording (that audio already plays there),
/// and, unless the rest of the track moves along (`ripple`), never into the neighbouring clip.
fn edge_room(s: &soundcraft_model::Session, track: soundcraft_model::TrackId, c: &soundcraft_model::Clip, end: bool, ripple: bool) -> Samples {
    let (first, last) = file_span(s, c);
    let others: Vec<&soundcraft_model::Clip> = s.track(track).map(|t| t.clips().iter().filter(|o| o.id != c.id).collect()).unwrap_or_default();
    let same = |o: &soundcraft_model::Clip| o.source().is_some() && o.source() == c.source();
    let mut room = if end { last - c.end() } else { c.start - first };
    if end {
        let our_end = c.source_offset() + c.length;
        if let Some(n) =
            others.iter().filter(|o| same(o) && o.start == c.end() && o.source_offset() >= our_end).map(|o| o.source_offset() - our_end).min()
        {
            room = room.min(n);
        }
        if !ripple && let Some(next) = others.iter().filter(|o| o.start >= c.end()).map(|o| o.start).min() {
            room = room.min(next - c.end());
        }
    } else {
        if let Some(n) = others
            .iter()
            .filter(|o| same(o) && o.end() == c.start && o.source_offset() + o.length <= c.source_offset())
            .map(|o| c.source_offset() - (o.source_offset() + o.length))
            .min()
        {
            room = room.min(n);
        }
        if !ripple && let Some(prev) = others.iter().filter(|o| o.end() <= c.start).map(|o| o.end()).max() {
            room = room.min(c.start - prev);
        }
    }
    room.max(0)
}

fn extend_clip(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "edit.extend_clip";
    let id = soundcraft_model::ClipId(p.get("clip").and_then(Value::as_u64).ok_or_else(|| bad(ID, "`clip` required"))?);
    let end = match str_param(p, "edge") {
        Some("end") => true,
        Some("start") => false,
        _ => return Err(bad(ID, "`edge` must be start or end")),
    };
    let to = position_param(e, ID, p, "to")?.ok_or_else(|| bad(ID, "`to` required"))?;
    let ripple = bool_or(p, "ripple", false);
    let all = bool_or(p, "all_tracks", false);
    let s = e.session_mut();
    let (track, c) = s.find_clip(id).map(|(t, c)| (t, c.clone())).ok_or_else(|| bad(ID, "no such clip"))?;
    let tracks: Vec<soundcraft_model::TrackId> = if all { s.tracks.iter().map(|t| t.id).collect() } else { vec![track] };
    let wanted = if end { to - c.end() } else { c.start - to };
    let delta = wanted.min(edge_room(s, track, &c, end, ripple));
    if delta <= 0 {
        return Ok(json!({"extended": 0}));
    }
    // Clips on the other tracks cut at the same point open up with this one, each within its own
    // limits (measured before anything moves).
    let partners: Vec<(soundcraft_model::Clip, Samples)> = if all {
        s.tracks
            .iter()
            .filter(|t| t.id != track)
            .flat_map(|t| t.clips().iter().map(move |o| (t.id, o)))
            .filter(|(_, o)| if end { o.end() == c.end() } else { o.start == c.start })
            .map(|(t, o)| (o.clone(), delta.min(edge_room(s, t, o, end, ripple))))
            .collect()
    } else {
        Vec::new()
    };
    if end {
        if ripple {
            let later = clips_from(s, &tracks, c.end(), id);
            crate::edit::move_clips(s, &later, delta, None);
        }
        if let Some(x) = s.find_clip_mut(id) {
            x.trim_end_to(c.end() + delta);
        }
        for (o, d) in &partners {
            if let Some(x) = s.find_clip_mut(o.id) {
                x.trim_end_to(o.end() + d);
            }
        }
    } else if ripple {
        // The clip keeps its place; what is revealed pushes it and everything after it later.
        let mut later = clips_from(s, &tracks, c.start, id);
        later.push(id);
        crate::edit::move_clips(s, &later, delta, None);
        if let Some(x) = s.find_clip_mut(id) {
            x.trim_start_to(c.start);
        }
        for (o, d) in &partners {
            // Moved by `delta` with the rest; reveal what its own limits allow.
            if let Some(x) = s.find_clip_mut(o.id) {
                x.trim_start_to(o.start + delta - d);
            }
        }
    } else {
        if let Some(x) = s.find_clip_mut(id) {
            x.trim_start_to(c.start - delta);
        }
        for (o, d) in &partners {
            if let Some(x) = s.find_clip_mut(o.id) {
                x.trim_start_to(o.start - d);
            }
        }
    }
    Ok(json!({"extended": delta, "extended_s": s.sample_rate.seconds(delta)}))
}

fn add_speaker(e: &mut Engine, p: &Value) -> Result<Value> {
    let s = e.session_mut();
    let speakers: Vec<usize> = s.tracks.iter().enumerate().filter(|(_, t)| crate::podcast::is_speaker(t)).map(|(i, _)| i).collect();
    // The next input after the highest one a speaker uses.
    let used = |t: &soundcraft_model::Track| match &t.mixer.input {
        soundcraft_model::Route::Hardware(h) => h.trim_start_matches("In ").split(['-', ' ']).next().and_then(|n| n.parse::<usize>().ok()),
        _ => None,
    };
    let input = s.tracks.iter().filter(|t| crate::podcast::is_speaker(t)).filter_map(used).max().unwrap_or(0) + 1;
    let name = match str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => (1..).map(|n| format!("Gast {n}")).find(|n| s.track_by_name(n).is_none()).unwrap_or_else(|| "Gast".into()),
    };
    let id = s.add_track(soundcraft_model::TrackKind::Audio, soundcraft_model::ChannelFormat::Mono, Some(&name));
    crate::podcast::make_speaker(s, id, input);
    // After the last speaker (before the jingle track and the master).
    if let (Some(last), Some(from)) = (speakers.last(), s.tracks.iter().position(|t| t.id == id)) {
        let t = s.tracks.remove(from);
        s.tracks.insert((last + 1).min(s.tracks.len()), t);
    }
    Ok(json!({"track": id, "name": name, "input": input}))
}

/// Track named `name`, made as a mono audio track when there is none.
fn track_named(e: &mut Engine, name: &str) -> soundcraft_model::TrackId {
    if let Some(t) = e.session().track_by_name(name) {
        return t.id;
    }
    e.session_mut().add_track(soundcraft_model::TrackKind::Audio, soundcraft_model::ChannelFormat::Mono, Some(name))
}

fn import_file(e: &mut Engine, id: &str, path: &str, track: soundcraft_model::TrackId, at: Samples) -> Result<u64> {
    let bytes = std::fs::read(path).map_err(|err| EngineError::Io(format!("{path}: {err}")))?;
    let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("Audio").to_string();
    let v = crate::io::import_audio_bytes(e, &name, &bytes, Some(path), Some(track), at.max(0))?;
    v["clip"].as_u64().ok_or_else(|| bad(id, "import made no clip"))
}

/// A remote guest's take: their own lossless recording, and optionally the call as heard in the
/// studio. The call recording sits where the studio heard it, so the guest's file is synced onto it.
fn import_remote(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "podcast.import_remote";
    let guest = str_param(p, "guest").map(str::trim).filter(|g| !g.is_empty()).ok_or_else(|| bad(ID, "`guest` required"))?.to_string();
    let path = str_param(p, "path").ok_or_else(|| bad(ID, "`path` required"))?.to_string();
    let at = position_param(e, ID, p, "at")?.unwrap_or(0);
    let track = track_named(e, &guest);
    let clip = import_file(e, ID, &path, track, at)?;
    let Some(live) = str_param(p, "live").map(str::to_string) else {
        return Ok(json!({"track": track, "clip": clip}));
    };
    let live_at = position_param(e, ID, p, "live_at")?.unwrap_or(at);
    let live_track = track_named(e, &format!("{guest} (Live)"));
    let live_clip = import_file(e, ID, &live, live_track, live_at)?;
    if let Some(t) = e.session_mut().track_mut(live_track) {
        t.mixer.mute = true;
        t.mixer.record_lock = true;
    }
    // A failed sync leaves the recording where the studio started it, which is close: say so.
    let sync =
        sync_clip(e, &json!({"clip": clip, "reference": live_track.0, "max_shift_s": 5.0})).unwrap_or_else(|err| json!({"error": err.to_string()}));
    Ok(json!({"track": track, "clip": clip, "live_track": live_track, "live_clip": live_clip, "sync": sync}))
}

fn auto_duck(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "podcast.auto_duck";
    let s = e.session();
    let music = match p.get("music").and_then(Value::as_u64) {
        Some(m) => soundcraft_model::TrackId(m),
        None => jingle_track(s).ok_or_else(|| bad(ID, "name the music track (`music`): no locked jingle track found"))?,
    };
    let amount = f64_or(p, "amount_db", -12.0).clamp(-40.0, -1.0) as f32;
    let attack = s.sample_rate.samples(f64_or(p, "attack_ms", 300.0).clamp(10.0, 5000.0) / 1000.0);
    let release = s.sample_rate.samples(f64_or(p, "release_ms", 800.0).clamp(10.0, 10_000.0) / 1000.0);
    let threshold = f64_or(p, "threshold_db", -40.0).clamp(-80.0, -10.0) as f32;
    let end = s.content_end();
    if end <= 0 {
        return Err(bad(ID, "the session is empty"));
    }
    let r = soundcraft_time::Range::new(0, end);
    let speakers: Vec<_> = speaker_tracks(e, p).into_iter().filter(|t| *t != music).collect();
    // Everyone's speech, summed to one channel.
    let mut speech: Vec<f32> = Vec::new();
    for t in speakers {
        let ch = crate::io::render_track(e, r, t, ID)?;
        let n = ch.iter().map(Vec::len).min().unwrap_or(0);
        if speech.len() < n {
            speech.resize(n, 0.0);
        }
        for (i, x) in speech.iter_mut().enumerate().take(n) {
            *x += ch.iter().map(|c| c[i].abs()).fold(0.0, f32::max);
        }
    }
    // Pauses shorter than attack + release stay ducked (no pumping between sentences).
    let gap = usize::try_from(attack + release).unwrap_or(0).max(1);
    let ranges = soundcraft_dsp::offline::non_silent_ranges(&[speech], threshold, gap, 0, 0);
    let base = e.session().track(music).map(|t| t.mixer.volume_db).ok_or_else(|| bad(ID, "no such music track"))?;
    let ducked = base + amount;
    let mut points: Vec<soundcraft_model::AutomationPoint> = vec![soundcraft_model::AutomationPoint { at: 0, value: base }];
    for (a, b) in &ranges {
        let (a, b) = (*a as i64, *b as i64);
        let down = (a - attack).max(points.last().map_or(0, |p| p.at + 1));
        points.push(soundcraft_model::AutomationPoint { at: down, value: base });
        points.push(soundcraft_model::AutomationPoint { at: a.max(down + 1), value: ducked });
        points.push(soundcraft_model::AutomationPoint { at: b.max(a.max(down + 1) + 1), value: ducked });
        points.push(soundcraft_model::AutomationPoint { at: b.max(a.max(down + 1) + 1) + release, value: base });
    }
    let n = ranges.len();
    let s = e.session_mut();
    let track = s.track_mut(music).ok_or_else(|| bad(ID, "no such music track"))?;
    let lane = track.lane_mut(&soundcraft_model::AutoParam::Volume);
    lane.points = points;
    lane.enabled = true;
    track.mixer.automation_mode = soundcraft_model::AutomationMode::Read;
    Ok(json!({"speech_ranges": n, "ducked_db": amount}))
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
    if !MP3_SAMPLE_RATES.contains(&x.sample_rate)
        && !(matches!(x.format.as_str(), "wav" | "flac" | "aiff") && matches!(x.sample_rate, 88_200 | 96_000))
    {
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
    if s.jingles.len() > JINGLE_PADS {
        return Err(bad(id, format!("at most {JINGLE_PADS} jingle pads")));
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
    // "Cut here" markers are notes for the edit, not chapters.
    let mut marks: Vec<_> = s
        .markers
        .iter()
        .filter(|m| m.kind == MarkerKind::Marker && !crate::podcast::is_cut_marker(m) && m.start >= r.start && m.start < r.end)
        .collect();
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
    let bytes = if matches!(x.format.as_str(), "mp3" | "m4a" | "m4b") {
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
        if x.format == "mp3" {
            soundcraft_audio_io::mp3::encode_mp3(&buf, &Mp3Options { bitrate_kbps: x.bitrate_kbps }, Some(&tags))
                .map_err(|err| bad(ID, err.to_string()))?
        } else {
            crate::aac::encode(&buf, x.bitrate_kbps, &tags, x.format == "m4b").map_err(|err| bad(ID, err))?
        }
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
            let txt = std::path::Path::new(&path).with_extension("transcript.txt").to_string_lossy().to_string();
            crate::io::write_file(&txt, crate::subtitles::write_transcript(&cues, false).as_bytes())?;
            extra.push(txt);
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

    /// A podcast session with a tone of `amp` on each named track at `at` seconds for `secs`.
    fn toned(dir: &std::path::Path, parts: &[(&str, f32, f64, f64)]) -> Engine {
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "T", "template": "podcast", "speakers": 2})).unwrap();
        for (i, (track, amp, at, secs)) in parts.iter().enumerate() {
            let n = (48_000.0 * secs) as usize;
            let ch: Vec<f32> = (0..n).map(|k| (k as f32 * 0.05).sin() * amp).collect();
            let buf = AudioBuffer { sample_rate: 48_000, channels: vec![ch] };
            let wav = dir.join(format!("t{i}.wav"));
            std::fs::write(&wav, soundcraft_audio_io::encode(&buf, &EncodeOptions::default()).unwrap()).unwrap();
            e.execute("file.import_audio", &json!({"path": wav.to_string_lossy(), "track": track, "new_tracks": false, "at": {"seconds": at}}))
                .unwrap();
        }
        e
    }

    #[test]
    fn speakers_are_matched_to_the_same_loudness() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-match-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = toned(&dir, &[("Moderation", 0.5, 0.0, 3.0), ("Gast 1", 0.05, 3.0, 3.0)]);
        let v = e.execute("podcast.match_speakers", &json!({"target_lufs": -20.0})).unwrap();
        assert_eq!(v["tracks"].as_array().unwrap().len(), 2, "{v}");
        let s = e.session();
        let gain = |name: &str| s.track_by_name(name).unwrap().clips()[0].gain_db;
        assert!((gain("Gast 1") - gain("Moderation") - 20.0).abs() < 0.5, "a 20 dB quieter guest gets 20 dB more: {v}");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn a_late_clip_is_synced_onto_the_other_tracks() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Speech-like: noise in irregular bursts (deterministic).
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let talk: Vec<f32> =
            (0..48_000 * 6).map(|i| if (i / 7_000) % 3 == 1 || (i / 11_000) % 4 == 2 { rnd() * 0.6 } else { rnd() * 0.01 }).collect();
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 2})).unwrap();
        for (i, (track, at)) in [("Moderation", 2.0), ("Gast 1", 2.7)].iter().enumerate() {
            let buf = AudioBuffer { sample_rate: 48_000, channels: vec![talk.clone()] };
            let wav = dir.join(format!("s{i}.wav"));
            std::fs::write(&wav, soundcraft_audio_io::encode(&buf, &EncodeOptions::default()).unwrap()).unwrap();
            e.execute("file.import_audio", &json!({"path": wav.to_string_lossy(), "track": track, "new_tracks": false, "at": {"seconds": at}}))
                .unwrap();
        }
        let guest = e.session().track_by_name("Gast 1").unwrap().clips()[0].id;
        let v = e.execute("podcast.sync_clip", &json!({"clip": guest.0, "max_shift_s": 2.0})).unwrap();
        let start = e.session().find_clip(guest).unwrap().1.start;
        assert_eq!(start, 96_000, "moved onto the moderation track exactly: {v}");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn a_remote_guest_is_synced_onto_the_call_recording() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut seed = 11u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let talk: Vec<f32> =
            (0..48_000 * 6).map(|i| if (i / 6_000) % 3 == 1 || (i / 13_000) % 4 == 2 { rnd() * 0.6 } else { rnd() * 0.01 }).collect();
        // The call drops a little and arrives 0.25 s late; the guest's own file is clean.
        let live: Vec<f32> = std::iter::repeat_n(0.0, 12_000).chain(talk.iter().map(|x| x * 0.5)).collect();
        let write = |name: &str, ch: Vec<f32>| {
            let path = dir.join(name);
            let buf = AudioBuffer { sample_rate: 48_000, channels: vec![ch] };
            std::fs::write(&path, soundcraft_audio_io::encode(&buf, &EncodeOptions::default()).unwrap()).unwrap();
            path.to_string_lossy().to_string()
        };
        let (hq, call) = (write("hq.wav", talk.clone()), write("live.wav", live));
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "R", "template": "podcast", "speakers": 1})).unwrap();
        let v = e.execute("podcast.import_remote", &json!({"guest": "Anna", "path": hq, "live": call, "at": {"seconds": 1.0}})).unwrap();
        let s = e.session();
        let anna = s.track_by_name("Anna").unwrap();
        assert_eq!(anna.clips()[0].start, 48_000 + 12_000, "lined up with the call: {v}");
        let call_track = s.track_by_name("Anna (Live)").unwrap();
        assert!(call_track.mixer.mute && call_track.mixer.record_lock, "the call recording is only a backup");
        // A second take goes onto the same tracks.
        e.execute("podcast.import_remote", &json!({"guest": "Anna", "path": hq, "at": {"seconds": 20.0}})).unwrap();
        assert_eq!(e.session().track_by_name("Anna").unwrap().clips().len(), 2);
        assert!(e.execute("podcast.import_remote", &json!({"guest": " ", "path": hq})).is_err());
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn a_cut_edge_pulls_back_out() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-extend-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("talk.wav");
        let buf = AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.1; 48_000 * 5]] };
        std::fs::write(&wav, soundcraft_audio_io::encode(&buf, &EncodeOptions::default()).unwrap()).unwrap();
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "X", "template": "podcast", "speakers": 2})).unwrap();
        for t in ["Moderation", "Gast 1"] {
            e.execute("file.import_audio", &json!({"path": wav.to_string_lossy(), "track": t, "new_tracks": false})).unwrap();
        }
        // Cut 2–3 s out of both tracks and close the gap: two clips per track meeting at 2 s.
        let sec = 48_000i64;
        for t in ["Moderation", "Gast 1"] {
            let s = e.session_mut();
            let tid = s.track_by_name(t).unwrap().id;
            let whole = s.track(tid).unwrap().clips()[0].clone();
            let mut b = whole.clone();
            b.id = s.new_clip_id();
            b.trim_start_to(3 * sec);
            b.start -= sec;
            s.find_clip_mut(whole.id).unwrap().trim_end_to(2 * sec);
            crate::edit::place_clip(s, tid, b);
        }
        let clips = |e: &Engine, t: &str| e.session().track_by_name(t).unwrap().clips().iter().map(|c| (c.start, c.end())).collect::<Vec<_>>();
        assert_eq!(clips(&e, "Gast 1"), vec![(0, 2 * sec), (2 * sec, 4 * sec)]);
        let left = e.session().track_by_name("Gast 1").unwrap().clips()[0].id.0;
        // Slip: the neighbour stops the edge.
        e.execute("edit.extend_clip", &json!({"clip": left, "edge": "end", "to": {"seconds": 2.5}})).unwrap();
        assert_eq!(clips(&e, "Gast 1")[0], (0, 2 * sec));
        // Slip with Alt: every track's edge stops at its own neighbour too, nothing overlaps.
        e.execute("edit.extend_clip", &json!({"clip": left, "edge": "end", "to": {"seconds": 2.5}, "all_tracks": true})).unwrap();
        for t in ["Moderation", "Gast 1"] {
            assert_eq!(clips(&e, t), vec![(0, 2 * sec), (2 * sec, 4 * sec)], "{t}");
        }
        // Ripple on all tracks: the cut second comes back everywhere, in sync.
        let v =
            e.execute("edit.extend_clip", &json!({"clip": left, "edge": "end", "to": {"seconds": 9.0}, "ripple": true, "all_tracks": true})).unwrap();
        assert_eq!(v["extended"], json!(sec), "only as far as the recording reaches past the cut: {v}");
        for t in ["Moderation", "Gast 1"] {
            assert_eq!(clips(&e, t), vec![(0, 3 * sec), (3 * sec, 5 * sec)], "{t}");
        }
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(clips(&e, "Moderation"), vec![(0, 2 * sec), (2 * sec, 4 * sec)], "one undo step");
        // The start edge of the right clip, this track only.
        let right = e.session().track_by_name("Gast 1").unwrap().clips()[1].id.0;
        e.execute("edit.extend_clip", &json!({"clip": right, "edge": "start", "to": {"seconds": 1.5}, "ripple": true})).unwrap();
        assert_eq!(clips(&e, "Gast 1"), vec![(0, 2 * sec), (2 * sec, 4 * sec + sec / 2)]);
        let r = e.session().track_by_name("Gast 1").unwrap().clips()[1].clone();
        assert_eq!(r.source_offset(), 3 * sec - sec / 2, "half a second of the cut came back");
        // Slip with Alt where only one track has room: the other stops at its neighbour.
        e.execute("edit.undo", &json!({})).unwrap();
        let s = e.session_mut();
        let gast = s.track_by_name("Gast 1").unwrap().id;
        let right_id = s.track(gast).unwrap().clips()[1].id;
        crate::edit::move_clips(s, &[right_id], sec, None);
        e.execute("edit.extend_clip", &json!({"clip": left, "edge": "end", "to": {"seconds": 2.5}, "all_tracks": true})).unwrap();
        assert_eq!(clips(&e, "Gast 1")[0], (0, 2 * sec + sec / 2));
        assert_eq!(clips(&e, "Moderation"), vec![(0, 2 * sec), (2 * sec, 4 * sec)], "no overlap on the other track");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn a_new_speaker_gets_the_voice_chain_and_the_next_input_after_the_others() {
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 2})).unwrap();
        let v = e.execute("podcast.add_speaker", &json!({})).unwrap();
        assert_eq!(v["name"], "Gast 2");
        assert_eq!(v["input"], 3);
        let names: Vec<String> = e.session().tracks.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, ["Moderation", "Gast 1", "Gast 2", "Jingles & Musik", "Master"]);
        let t = e.session().track_by_name("Gast 2").unwrap();
        assert!(crate::podcast::is_speaker(t), "voice chain on the new track");
        e.execute("podcast.add_speaker", &json!({"name": "Anna"})).unwrap();
        assert_eq!(e.session().tracks[3].name, "Anna");
        e.execute("edit.undo", &json!({})).unwrap();
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(e.session().tracks.len(), 4, "each addition is one undo step");
    }

    #[test]
    fn mistakes_are_marked_found_and_ticked_off_and_never_become_chapters() {
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 1})).unwrap();
        for at in [5.0, 2.0] {
            e.execute("markers.add_cut", &json!({"at": {"seconds": at}})).unwrap();
        }
        e.execute("markers.add", &json!({"at": {"seconds": 3.0}, "name": "Thema"})).unwrap();
        let titles: Vec<String> = chapters(&e, soundcraft_time::Range::new(0, 48_000 * 10)).iter().map(|c| c.title.clone()).collect();
        assert!(!titles.iter().any(|t| t.starts_with('✂')), "no cut marker among the chapters: {titles:?}");
        e.execute("markers.next_cut", &json!({})).unwrap();
        assert_eq!(e.session().edit.selection.start, 2 * 48_000);
        e.execute("markers.next_cut", &json!({})).unwrap();
        assert_eq!(e.session().edit.selection.start, 5 * 48_000, "skips the chapter marker");
        let v = e.execute("markers.done_cut", &json!({})).unwrap();
        assert_eq!(v["cut_markers"], 1);
        e.execute("markers.previous_cut", &json!({})).unwrap();
        assert_eq!(e.session().edit.selection.start, 2 * 48_000);
        assert!(e.execute("markers.done_cut", &json!({"at": {"seconds": 8.0}})).is_err(), "nothing there");
    }

    #[test]
    fn a_jingle_pad_places_its_file_on_the_jingle_track() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-jingle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("intro.wav");
        let buf = AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.1; 48_000]] };
        std::fs::write(&wav, soundcraft_audio_io::encode(&buf, &EncodeOptions::default()).unwrap()).unwrap();
        let mut e = Engine::default();
        e.execute("session.new", &json!({"name": "J", "template": "podcast", "speakers": 1})).unwrap();
        assert!(e.execute("podcast.place_jingle", &json!({"pad": 1})).is_err(), "empty pad");
        e.execute("podcast.set", &json!({"jingles": ["", wav.to_string_lossy()]})).unwrap();
        e.execute("podcast.place_jingle", &json!({"pad": 2, "at": {"seconds": 2.0}})).unwrap();
        let music = e.session().track_by_name("Jingles & Musik").unwrap();
        assert_eq!(music.clips().len(), 1);
        assert_eq!(music.clips()[0].start, 96_000);
        assert!(e.execute("podcast.set", &json!({"jingles": vec![""; 9]})).is_err(), "eight pads at most");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn music_ducks_while_someone_speaks() {
        let dir = std::env::temp_dir().join(format!("sc-podcast-duck-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = toned(&dir, &[("Jingles & Musik", 0.2, 0.0, 10.0), ("Moderation", 0.3, 3.0, 3.0)]);
        let v = e.execute("podcast.auto_duck", &json!({})).unwrap();
        assert_eq!(v["speech_ranges"], 1, "{v}");
        let s = e.session();
        let music = s.track_by_name("Jingles & Musik").unwrap();
        let lane = music.lane(&soundcraft_model::AutoParam::Volume).unwrap();
        let at = |secs: f64| lane.value_at(s.sample_rate.samples(secs), 0.0);
        assert_eq!(at(1.0), -6.0, "full level before the speech");
        assert_eq!(at(4.5), -18.0, "ducked by 12 dB while speaking");
        assert_eq!(at(8.0), -6.0, "back after the release");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn m4a_is_the_default_and_exports_aac_in_mp4() {
        if crate::aac::find_encoder().is_none() {
            eprintln!("skipping: no AAC encoder");
            return;
        }
        let dir = std::env::temp_dir().join(format!("sc-podcast-m4a-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = episode_engine(&dir);
        assert_eq!(e.session().podcast.export.format, "m4a");
        let v = e.execute("file.export_podcast", &json!({"path": dir.join("folge.mp3").to_string_lossy()})).unwrap();
        let path = v["path"].as_str().unwrap().to_string();
        assert!(path.ends_with("folge.m4a"), "the extension follows the format: {path}");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp");
        assert_eq!(v["chapters"], 2, "{v}");
        assert!(std::fs::remove_dir_all(&dir).is_ok());
    }

    #[test]
    fn settings_merge_validate_and_persist_in_the_session() {
        let mut e = Engine::default();
        let v = e.execute("podcast.set", &json!({"export": {"bitrate_kbps": 96, "mono": true}, "episode": {"show": "Europa 2.0"}})).unwrap();
        assert_eq!(v["export"]["bitrate_kbps"], 96);
        assert_eq!(v["export"]["format"], "m4a", "untouched keys keep their value");
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
        assert_eq!(e.session().podcast.export.format, "m4a", "overrides are for this file only");
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
