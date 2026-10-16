//! Speech repair (Edit menu, clip menu, Finish Episode): crosstalk removal across the speaker
//! tracks, a leveler within a track, breath reduction, mouth-click repair and voice matching.
//! The first three write clip gain envelopes (non-destructive, added to what is already there);
//! click repair and voice matching render the clip to a new file like `clip.denoise`.

use super::*;
use crate::cmd;
use serde_json::json;
use soundcraft_dsp::voice::{self, BLOCK_S, GateSettings};
use soundcraft_model::{Clip, ClipId, Session, TrackId};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "podcast.remove_crosstalk",
            "Remove Crosstalk",
            [],
            None,
            "{tracks?, reduction_db?: 15, margin_db?: 9} — turns each speaker track down by `reduction_db` where its own speaker is silent (only background or another speaker's bleed); with one track an adaptive noise gate. Writes clip gain envelopes",
            has_tracks,
            remove_crosstalk
        ),
        cmd!(
            "clip.level",
            "Level Speech",
            [],
            None,
            "{clips?, max_db?: 6, window_s?: 3} — evens out a speaker within the clip (quiet passages up, loud ones down, by at most `max_db`); without clips: every clip on the speaker tracks. Writes clip gain envelopes",
            has_tracks,
            level
        ),
        cmd!(
            "clip.reduce_breaths",
            "Reduce Breaths",
            [],
            None,
            "{clips?, reduction_db?: 12} — finds breaths between phrases and lowers them; without clips: every clip on the speaker tracks. Writes clip gain envelopes",
            has_tracks,
            reduce_breaths
        ),
        cmd!(
            "clip.declick",
            "Remove Mouth Clicks",
            [],
            None,
            "{clips?, sensitivity?: 1..10 (5)} — repairs mouth clicks and lip smacks; without clips: every clip on the speaker tracks. Renders the clip to a new file (undoable)",
            has_tracks,
            declick
        ),
        cmd!(
            "podcast.match_voice",
            "Match Voice",
            [],
            None,
            "{reference: track, track? | clips?, max_db?: 9} — gives the clips (or every clip of `track`) the tonal balance of the speech on `reference` (e.g. two microphones of different makes). Renders the clips to new files (undoable)",
            has_tracks,
            match_voice
        ),
    ]
}

/// `clips`/`clip`/the selection, else every audio clip on the speaker tracks.
fn target_clips(e: &Engine, p: &Value) -> Vec<ClipId> {
    let explicit = p.get("clips").is_some() || p.get("clip").is_some();
    let ids = clip_ids_param(e, p);
    if !ids.is_empty() || explicit {
        return ids;
    }
    let s = e.session();
    crate::podcast::record_targets(s).iter().filter_map(|t| s.track(*t)).flat_map(|t| t.clips().iter().map(|c| c.id)).collect()
}

/// The clip's envelope value at `at` (linear between points, held at the ends; 0 when empty).
fn env_at(env: &[(i64, f32)], at: i64) -> f32 {
    let i = env.partition_point(|&(t, _)| t < at);
    match (i.checked_sub(1).and_then(|j| env.get(j)), env.get(i)) {
        (Some(&(pt, pv)), Some(&(t, v))) if t > pt => pv + (v - pv) * ((at - pt) as f32 / (t - pt) as f32),
        (_, Some(&(_, v))) | (Some(&(_, v)), None) => v,
        (None, None) => 0.0,
    }
}

/// Few points that follow `gains` (one per `step` samples from `first`) within `tol` dB:
/// swinging-door compression, so flat stretches and straight ramps cost two points.
fn compress(gains: &[f32], first: i64, step: i64, tol: f32) -> Vec<(i64, f32)> {
    let mut out: Vec<(i64, f32)> = Vec::new();
    let Some(&g0) = gains.first() else { return out };
    let at = |i: usize| first.saturating_add((i as i64).saturating_mul(step));
    let (mut anchor, mut av) = (0usize, g0);
    out.push((at(0), g0));
    let (mut hi, mut lo) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut i = 1;
    while i < gains.len() {
        let d = (i - anchor) as f32;
        let v = gains[i];
        let (up, down) = ((v + tol - av) / d, (v - tol - av) / d);
        let (nhi, nlo) = (hi.min(up), lo.max(down));
        if nlo > nhi {
            // The previous point ends the segment; start a new one there.
            let prev = i - 1;
            let pv = av + (hi + lo) / 2.0 * (prev - anchor) as f32;
            let pv = if hi.is_finite() && lo.is_finite() { pv } else { gains[prev] };
            out.push((at(prev), pv));
            anchor = prev;
            av = pv;
            hi = f32::INFINITY;
            lo = f32::NEG_INFINITY;
            continue;
        }
        hi = nhi;
        lo = nlo;
        i += 1;
    }
    let last = gains.len() - 1;
    if last > anchor {
        let lv = if hi.is_finite() && lo.is_finite() { av + (hi + lo) / 2.0 * (last - anchor) as f32 } else { gains[last] };
        out.push((at(last), lv));
    }
    out
}

/// Adds `points` (offset from the clip start, dB) to the clip's gain envelope.
fn add_env(c: &mut Clip, points: &[(i64, f32)]) {
    if points.iter().all(|(_, v)| v.abs() < 0.01) {
        return;
    }
    let len = c.length.max(1);
    let mut times: Vec<i64> = c.gain_env.iter().map(|p| p.0).chain(points.iter().map(|p| p.0)).map(|t| t.clamp(0, len)).collect();
    times.sort_unstable();
    times.dedup();
    let env: Vec<(i64, f32)> = times.iter().map(|&t| (t, (env_at(&c.gain_env, t) + env_at(points, t)).clamp(-144.0, 36.0))).collect();
    c.gain_env = env;
}

/// Block gains for the clip's own audio (block `i` covers source samples `i·block…`) as
/// envelope points on the clip.
fn clip_points(c: &Clip, gains: &[f32], block: usize, src_len: usize) -> Vec<(i64, f32)> {
    // Source samples per clip sample (stretch); the envelope is in clip samples.
    let ratio = if c.length > 0 && src_len > 0 { src_len as f64 / c.length as f64 } else { 1.0 };
    let step = ((block as f64 / ratio).round() as i64).max(1);
    // A point in the middle of each block.
    compress(gains, step / 2, step, 0.3)
}

/// The track's level envelope on the timeline (one value per block of `block` session samples,
/// `n` blocks), from its clips at their clip gain.
fn track_levels(s: &Session, t: TrackId, block: usize, n: usize) -> Vec<f32> {
    let mut out = vec![voice::SILENCE_DB; n];
    let Some(track) = s.track(t) else { return out };
    for c in track.clips() {
        let Some((ch, rate)) = super::clip::clip_audio(s, c) else { continue };
        let src_len = ch.iter().map(Vec::len).max().unwrap_or(0);
        if src_len == 0 {
            continue;
        }
        let src_block = voice::block_len(rate);
        let lv = voice::levels_db(&ch, src_block);
        // Timeline samples per source block.
        let per = c.length as f64 / src_len as f64 * src_block as f64;
        for (i, l) in lv.iter().enumerate() {
            let at = c.start as f64 + i as f64 * per;
            let Ok(b) = usize::try_from((at / block as f64) as i64) else { continue };
            if let Some(o) = out.get_mut(b) {
                // Clips can overlap: the louder wins.
                *o = o.max(l + c.gain_db);
            }
        }
    }
    out
}

fn remove_crosstalk(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "podcast.remove_crosstalk";
    let tracks = super::podcast::speaker_tracks(e, p);
    if tracks.is_empty() {
        return Err(bad(ID, "no speaker tracks"));
    }
    let set = GateSettings {
        reduction_db: f64_or(p, "reduction_db", 15.0).clamp(3.0, 60.0) as f32,
        margin_db: f64_or(p, "margin_db", 9.0).clamp(1.0, 30.0) as f32,
        ..GateSettings::default()
    };
    let s = e.session();
    let block = voice::block_len(s.sample_rate.as_f64() as f32);
    let end = usize::try_from(s.content_end()).unwrap_or(0);
    if end == 0 {
        return Err(bad(ID, "the session is empty"));
    }
    let n = end.div_ceil(block);
    let levels: Vec<Vec<f32>> = tracks.iter().map(|t| track_levels(s, *t, block, n)).collect();
    let gains = voice::crosstalk_gains(&levels, set);
    let mut out = Vec::new();
    let s = e.session_mut();
    for (t, g) in tracks.iter().zip(&gains) {
        let Some(track) = s.track_mut(*t) else { continue };
        let name = track.name.clone();
        let closed = g.iter().filter(|v| **v < -0.5).count() as f64 * f64::from(BLOCK_S);
        if let Some(pl) = track.playlist_mut() {
            for c in pl.clips.iter_mut().filter(|c| matches!(c.content, soundcraft_model::ClipContent::Audio { .. })) {
                // The timeline gains under this clip, as points relative to its start.
                let first = usize::try_from(c.start.max(0)).unwrap_or(0) / block;
                let last = usize::try_from(c.end().max(0)).unwrap_or(0).div_ceil(block);
                let Some(slice) = g.get(first.min(g.len())..last.min(g.len())) else { continue };
                let start_rel = (first * block) as i64 - c.start + block as i64 / 2;
                add_env(c, &compress(slice, start_rel, block as i64, 0.3));
            }
        }
        out.push(json!({"track": name, "lowered_s": (closed * 10.0).round() / 10.0}));
    }
    Ok(json!({"tracks": out, "reduction_db": set.reduction_db}))
}

/// The clip's envelope per block of its audio (block `i` covers source samples `i·block…`).
fn env_blocks(c: &Clip, block: usize, src_len: usize) -> Vec<f32> {
    if c.gain_env.is_empty() {
        return Vec::new();
    }
    let ratio = if c.length > 0 && src_len > 0 { src_len as f64 / c.length as f64 } else { 1.0 };
    (0..src_len.div_ceil(block.max(1))).map(|i| env_at(&c.gain_env, (((i * block + block / 2) as f64) / ratio) as i64)).collect()
}

/// Runs `f` on each target clip's audio (with the gain its envelope already applies, per block)
/// and adds the returned block gains to its envelope.
fn per_clip_env(e: &mut Engine, id: &str, p: &Value, mut f: impl FnMut(&[Vec<f32>], f32, &[f32]) -> (Vec<f32>, usize)) -> Result<Value> {
    let ids = target_clips(e, p);
    if ids.is_empty() {
        return Err(bad(id, "no clips (select some, or record on a speaker track)"));
    }
    let (mut clips, mut found) = (0, 0);
    for cid in ids {
        let s = e.session();
        let Some((_, c)) = s.find_clip(cid) else { continue };
        let Some((ch, rate)) = super::clip::clip_audio(s, c) else { continue };
        let src_len = ch.iter().map(Vec::len).max().unwrap_or(0);
        if src_len == 0 {
            continue;
        }
        let before = env_blocks(c, voice::block_len(rate), src_len);
        let (gains, n) = f(&ch, rate, &before);
        let points = clip_points(c, &gains, voice::block_len(rate), src_len);
        if let Some(c) = e.session_mut().find_clip_mut(cid) {
            add_env(c, &points);
            clips += 1;
            found += n;
        }
    }
    if clips == 0 {
        return Err(bad(id, "the clips hold no audio"));
    }
    Ok(json!({"clips": clips, "found": found}))
}

fn level(e: &mut Engine, p: &Value) -> Result<Value> {
    let max_db = f64_or(p, "max_db", 6.0).clamp(1.0, 20.0) as f32;
    let window = f64_or(p, "window_s", 3.0).clamp(0.5, 30.0) as f32;
    per_clip_env(e, "clip.level", p, |ch, rate, before| {
        let g = voice::leveler_gains(&voice::with_gain(voice::levels_db(ch, voice::block_len(rate)), before), window, max_db);
        let changed = usize::from(g.iter().any(|v| v.abs() > 0.5));
        (g, changed)
    })
}

fn reduce_breaths(e: &mut Engine, p: &Value) -> Result<Value> {
    let db = f64_or(p, "reduction_db", 12.0).clamp(3.0, 60.0) as f32;
    per_clip_env(e, "clip.reduce_breaths", p, |ch, rate, before| {
        let r = voice::breath_ranges(ch, rate, before);
        let n = voice::levels_db(ch, voice::block_len(rate)).len();
        (voice::dip_gains(n, &r, db), r.len())
    })
    .map(|mut v| {
        v["breaths"] = v["found"].clone();
        v
    })
}

/// Replaces the clip's audio with `ch` (a new file in the session, like `clip.denoise`).
fn replace_audio(e: &mut Engine, cid: ClipId, suffix: &str, ch: Vec<Vec<f32>>, rate: f32) {
    let s = e.session_mut();
    let name = s.find_clip(cid).map(|(_, c)| c.name.clone()).unwrap_or_default();
    let buf = soundcraft_audio_io::AudioBuffer { sample_rate: rate as u32, channels: ch };
    let src = crate::io::add_source(s, &format!("{name}_{suffix}"), buf, None, soundcraft_audio_io::FileFormat::Wav);
    if let Some(c) = s.find_clip_mut(cid) {
        c.content = soundcraft_model::ClipContent::Audio { source: src, offset: 0 };
    }
}

fn declick(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "clip.declick";
    let sens = f64_or(p, "sensitivity", 5.0).clamp(1.0, 10.0) as f32;
    let ids = target_clips(e, p);
    if ids.is_empty() {
        return Err(bad(ID, "no clips (select some, or record on a speaker track)"));
    }
    let (mut clips, mut clicks) = (0, 0);
    for cid in ids {
        let s = e.session();
        let Some((_, c)) = s.find_clip(cid) else { continue };
        let Some((ch, rate)) = super::clip::clip_audio(s, c) else { continue };
        if ch.iter().all(Vec::is_empty) {
            continue;
        }
        let (fixed, n) = voice::declick(&ch, rate, sens);
        clips += 1;
        if n > 0 {
            replace_audio(e, cid, "declicked", fixed, rate);
            clicks += n;
        }
    }
    if clips == 0 {
        return Err(bad(ID, "the clips hold no audio"));
    }
    Ok(json!({"clips": clips, "clicks": clicks}))
}

/// The speech spectrum of a set of clips (each at its clip gain, averaged by speech time).
fn clips_spectrum(s: &Session, clips: &[ClipId]) -> Option<(Vec<f32>, f32)> {
    let mut acc: Option<Vec<f32>> = None;
    let mut rate = s.sample_rate.as_f64() as f32;
    let mut n = 0.0f32;
    for cid in clips {
        let Some((_, c)) = s.find_clip(*cid) else { continue };
        let Some((ch, r)) = super::clip::clip_audio(s, c) else { continue };
        let spec = voice::speech_spectrum(&ch);
        if spec.iter().all(|v| *v <= 0.0) {
            continue;
        }
        rate = r;
        let a = acc.get_or_insert_with(|| vec![0.0; spec.len()]);
        for (x, y) in a.iter_mut().zip(&spec) {
            *x += y;
        }
        n += 1.0;
    }
    acc.map(|a| (a.into_iter().map(|v| v / n.max(1.0)).collect(), rate))
}

fn match_voice(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "podcast.match_voice";
    let reference = TrackId(p.get("reference").and_then(Value::as_u64).ok_or_else(|| bad(ID, "`reference` (a track) required"))?);
    let max_db = f64_or(p, "max_db", 9.0).clamp(1.0, 24.0) as f32;
    let s = e.session();
    let ref_track = s.track(reference).ok_or_else(|| bad(ID, "no such reference track"))?;
    let ref_clips: Vec<ClipId> = ref_track.clips().iter().map(|c| c.id).collect();
    let ids: Vec<ClipId> = match p.get("track").and_then(Value::as_u64) {
        Some(t) => s.track(TrackId(t)).ok_or_else(|| bad(ID, "no such track"))?.clips().iter().map(|c| c.id).collect(),
        None => clip_ids_param(e, p),
    };
    let ids: Vec<ClipId> = ids.into_iter().filter(|c| !ref_clips.contains(c)).collect();
    if ids.is_empty() {
        return Err(bad(ID, "choose clips (or `track`) other than the reference"));
    }
    let (to, rate) = clips_spectrum(s, &ref_clips).ok_or_else(|| bad(ID, "the reference track holds no speech"))?;
    let (from, _) = clips_spectrum(s, &ids).ok_or_else(|| bad(ID, "the clips hold no speech"))?;
    let curve = voice::match_curve(&from, &to, rate, max_db);
    let mut done = 0;
    for cid in ids {
        let s = e.session();
        let Some((_, c)) = s.find_clip(cid) else { continue };
        let Some((ch, r)) = super::clip::clip_audio(s, c) else { continue };
        if ch.iter().all(Vec::is_empty) {
            continue;
        }
        let out = voice::apply_curve(&ch, &curve);
        replace_audio(e, cid, "matched", out, r);
        done += 1;
    }
    let reference = e.session().track(reference).map(|t| t.name.clone()).unwrap_or_default();
    Ok(json!({"clips": done, "reference": reference}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_audio_io::{AudioBuffer, FileFormat};

    const SR: f32 = 48_000.0;

    fn tone(secs: f32, amp: f32) -> Vec<f32> {
        (0..(secs * SR) as usize).map(|i| amp * (std::f32::consts::TAU * 180.0 * i as f32 / SR).sin()).collect()
    }

    /// A session with two speaker tracks holding `a` and `b` from 0.
    fn session(a: Vec<f32>, b: Vec<f32>) -> (Engine, Vec<TrackId>) {
        let mut e = Engine::default();
        e.execute("track.new", &json!({"count": 2, "format": "mono"})).unwrap();
        let tracks: Vec<TrackId> = e.session().tracks.iter().map(|t| t.id).collect();
        for (t, x) in tracks.iter().zip([a, b]) {
            let len = x.len() as i64;
            let s = e.session_mut();
            let src = crate::io::add_source(s, "take", AudioBuffer { sample_rate: SR as u32, channels: vec![x] }, None, FileFormat::Wav);
            let id = s.alloc();
            let clip = soundcraft_model::Clip::audio(soundcraft_model::ClipId(id), "take", src, 0, 0, len);
            s.track_mut(*t).unwrap().playlist_mut().unwrap().clips.push(clip);
        }
        (e, tracks)
    }

    fn clip_of(e: &Engine, t: TrackId) -> Clip {
        e.session().track(t).unwrap().clips()[0].clone()
    }

    #[test]
    fn compress_keeps_flat_runs_and_ramps_short() {
        let mut g = vec![0.0; 100];
        g.extend((0..4).map(|i| -15.0 * (i + 1) as f32 / 4.0));
        g.extend(vec![-15.0; 100]);
        let pts = compress(&g, 0, 10, 0.3);
        assert!(pts.len() <= 5, "{pts:?}");
        for (i, v) in g.iter().enumerate() {
            assert!((env_at(&pts, i as i64 * 10) - v).abs() <= 0.31, "block {i}");
        }
    }

    #[test]
    fn crosstalk_writes_envelopes_where_the_speaker_is_silent() {
        let half = (2.0 * SR) as usize;
        let bleed = 0.3 * 10f32.powf(-18.0 / 20.0);
        let a = [tone(2.0, 0.3), tone(2.0, bleed)].concat();
        let b = [tone(2.0, bleed), tone(2.0, 0.3)].concat();
        let (mut e, t) = session(a, b);
        let v = e.execute("podcast.remove_crosstalk", &json!({"tracks": [t[0].0, t[1].0]})).unwrap();
        assert_eq!(v["tracks"].as_array().unwrap().len(), 2, "{v}");
        let (ca, cb) = (clip_of(&e, t[0]), clip_of(&e, t[1]));
        let at = |c: &Clip, s: usize| env_at(&c.gain_env, s as i64);
        assert!(at(&ca, half / 2).abs() < 0.5 && at(&ca, half * 3 / 2) < -14.0, "{:?}", ca.gain_env);
        assert!(at(&cb, half / 2) < -14.0 && at(&cb, half * 3 / 2).abs() < 0.5, "{:?}", cb.gain_env);
        assert!(ca.gain_env.len() < 20, "compact: {}", ca.gain_env.len());
        // Undo takes it back.
        assert!(e.undo());
        assert!(clip_of(&e, t[0]).gain_env.is_empty());
    }

    #[test]
    fn leveler_and_breaths_add_to_an_existing_envelope() {
        let x = [tone(6.0, 0.03), tone(10.0, 0.1), tone(6.0, 0.4)].concat();
        let (mut e, t) = session(x, Vec::new());
        let cid = clip_of(&e, t[0]).id;
        e.session_mut().find_clip_mut(cid).unwrap().gain_env = vec![(0, -3.0), (10, -3.0)];
        e.execute("clip.level", &json!({"clips": [cid.0]})).unwrap();
        let c = clip_of(&e, t[0]);
        let at = |s: f32| env_at(&c.gain_env, (s * SR) as i64);
        assert!(at(3.0) > 1.0, "quiet raised (on top of −3): {}", at(3.0));
        assert!(at(19.0) < -6.0, "loud lowered: {}", at(19.0));
        // Without clips: every clip on the speaker tracks; nothing to find is fine.
        let v = e.execute("clip.reduce_breaths", &json!({})).unwrap();
        assert_eq!(v["breaths"], 0);
    }

    #[test]
    fn declick_renders_only_when_it_found_clicks() {
        let mut x = tone(1.0, 0.2);
        x[10_000] += 0.7;
        let (mut e, t) = session(x, tone(1.0, 0.2));
        let before = clip_of(&e, t[1]).content.clone();
        let v = e.execute("clip.declick", &json!({})).unwrap();
        assert_eq!(v["clicks"], 1, "{v}");
        assert_ne!(clip_of(&e, t[0]).content, clip_of(&e, t[1]).content);
        assert_eq!(clip_of(&e, t[1]).content, before, "the clean clip keeps its file");
    }

    #[test]
    fn match_voice_needs_a_reference_and_other_clips() {
        let (mut e, t) = session(tone(3.0, 0.2), tone(3.0, 0.2));
        assert!(e.execute("podcast.match_voice", &json!({"track": t[0].0})).is_err());
        assert!(e.execute("podcast.match_voice", &json!({"track": t[0].0, "reference": t[0].0})).is_err());
        let v = e.execute("podcast.match_voice", &json!({"track": t[1].0, "reference": t[0].0})).unwrap();
        assert_eq!(v["clips"], 1);
    }
}
