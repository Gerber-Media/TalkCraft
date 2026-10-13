//! Clip menu commands.

use super::*;
use crate::cmd;
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("clip.edit_lock", "Edit Lock/Unlock", ["Clip"], Some("Cmd+L"), "{clips?}", has_selection, |e, p| flag(
            e,
            p,
            |c| c.edit_locked,
            |c, v| c.edit_locked = v
        )),
        cmd!("clip.time_lock", "Time Lock/Unlock", ["Clip"], Some("Ctrl+Alt+L"), "{clips?}", has_selection, |e, p| flag(
            e,
            p,
            |c| c.time_locked,
            |c, v| c.time_locked = v
        )),
        cmd!("clip.send_to_back", "Send to Back", ["Clip"], Some("Alt+Shift+B"), "{clips?}", has_selection, |e, p| layer(e, p, false)),
        cmd!("clip.bring_to_front", "Bring to Front", ["Clip"], Some("Alt+Shift+F"), "{clips?}", has_selection, |e, p| layer(e, p, true)),
        cmd!("clip.rating", "Rating", ["Clip", "Rating"], Some("Cmd+Alt+0-5"), "{clips?, rating: 0..5}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let r = i64_or(p, "rating", 0).clamp(0, 5) as u8;
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.rating = r
                }
            }
            Ok(json!({"rating": r}))
        }),
        cmd!("clip.group", "Group", ["Clip"], Some("Cmd+Alt+G"), "{clips?}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            if ids.len() < 2 {
                return Err(bad("clip.group", "select two or more clips"));
            }
            let s = e.session_mut();
            let g = s.alloc();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.group = Some(g)
                }
            }
            Ok(json!({"group": g}))
        }),
        cmd!("clip.ungroup", "Ungroup", ["Clip"], Some("Cmd+Alt+U"), "{clips?}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.group = None
                }
            }
            Ok(json!({}))
        }),
        cmd!("clip.ungroup_all", "Ungroup All", ["Clip"], None, "{}", always, |e, _| {
            for t in &mut e.session_mut().tracks {
                for pl in &mut t.playlists {
                    for c in &mut pl.clips {
                        c.group = None
                    }
                }
            }
            Ok(json!({}))
        }),
        cmd!("clip.loop", "Loop...", ["Clip"], Some("Cmd+Alt+L"), "{clips?, count?: n, length?: samples}", has_selection, loop_clips),
        cmd!("clip.rename", "Rename...", ["Clip"], Some("Cmd+Alt+Shift+R"), "{clip?, name}", has_selection, rename),
        cmd!("clip.gain", "Clip Gain", [], None, "{clips?, db | delta_db}", has_selection, gain),
        cmd!(
            "clip.set_fades",
            "Set Clip Fades",
            [],
            None,
            "{clip, fade_in?: samples|{seconds}, fade_out?: samples|{seconds}} — 0 removes a fade; both together never exceed the clip",
            always,
            set_fades
        ),
        cmd!(
            "clip.normalize",
            "Normalize Clip",
            [],
            None,
            "{clips?, peak_db?: -1, lufs?} — sets the clip gain so the clip peaks at `peak_db`, or (with `lufs`) reaches that integrated loudness",
            has_selection,
            normalize
        ),
        cmd!("clip.gain_nudge_up", "Nudge Clip Gain Up", [], Some("Ctrl+Shift+Up"), "{clips?}", has_selection, |e, p| nudge_gain(e, p, 1.0)),
        cmd!("clip.gain_nudge_down", "Nudge Clip Gain Down", [], Some("Ctrl+Shift+Down"), "{clips?}", has_selection, |e, p| nudge_gain(e, p, -1.0)),
        cmd!("clip.gain_render", "Render", ["Clip", "Clip Gain"], None, "{clips?}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let mut n = 0;
            for id in ids {
                if crate::io::render_clip_gain(e, id)? {
                    n += 1
                }
            }
            Ok(json!({"rendered": n}))
        }),
        cmd!("clip.gain_bypass", "Bypass", ["Clip", "Clip Gain"], None, "{clips?}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.gain_db = 0.0;
                    c.gain_env.clear()
                }
            }
            Ok(json!({}))
        }),
        cmd!("clip.identify_sync_point", "Identify Sync Point", ["Clip"], Some("Cmd+,"), "{clip?, at?}", has_selection, |e, p| {
            let at = position_param(e, "clip.identify_sync_point", p, "at")?.unwrap_or(e.session().edit.selection.start);
            let ids = clip_ids_param(e, p);
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.sync_point = (at - c.start).clamp(0, c.length)
                }
            }
            Ok(json!({}))
        }),
        cmd!("clip.quantize_to_grid", "Quantize to Grid", ["Clip"], Some("Cmd+0"), "{clips?}", has_selection, quantize_to_grid),
        cmd!("clip.color", "Clip Color", [], None, "{clips?, color: [r,g,b] | null}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let col = p.get("color").and_then(Value::as_array).map(|a| {
                let g = |i: usize| a.get(i).and_then(Value::as_u64).unwrap_or(0).min(255) as u8;
                [g(0), g(1), g(2)]
            });
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.color = col
                }
            }
            Ok(json!({}))
        }),
        cmd!(
            "clip.elastic_properties",
            "Elastic Properties",
            ["Clip"],
            Some("Alt+5"),
            "{clips?, ratio?: 1.0} — on Elastic Audio tracks (polyphonic/rhythmic/monophonic/x-form) the stretch is rendered pitch-preserving; elsewhere it is varispeed",
            has_selection,
            elastic
        ),
        cmd!("clip.remove_warp", "Remove Warp", ["Clip"], None, "{clips?}", has_selection, remove_warp),
        cmd!(
            "clip.place_source",
            "Place Audio File",
            [],
            None,
            "{source: id, track?: id|name (default: a new track), at?: position}",
            always,
            |e, p| {
                let src = p
                    .get("source")
                    .and_then(Value::as_u64)
                    .map(soundcraft_model::SourceId)
                    .ok_or_else(|| bad("clip.place_source", "`source` id required"))?;
                let at = position_param(e, "clip.place_source", p, "at")?.unwrap_or(e.session().edit.selection.start).max(0);
                let track = track_param(e, "clip.place_source", p, "track")?;
                let (name, frames, chans) = e
                    .session()
                    .source(src)
                    .map(|x| (x.name.clone(), i64::try_from(x.frames).unwrap_or(0), usize::from(x.channels)))
                    .ok_or_else(|| bad("clip.place_source", "no such audio file"))?;
                let s = e.session_mut();
                let t = match track {
                    Some(t) if s.track(t).is_some_and(|x| x.kind == soundcraft_model::TrackKind::Audio) => t,
                    Some(_) => return Err(bad("clip.place_source", "audio files go on audio tracks")),
                    None => s.add_track(soundcraft_model::TrackKind::Audio, soundcraft_model::ChannelFormat::for_channels(chans.min(2)), Some(&name)),
                };
                let id = s.new_clip_id();
                crate::edit::place_clip(s, t, soundcraft_model::Clip::audio(id, name, src, 0, at, frames.max(1)));
                Ok(json!({"clip": id, "track": t}))
            }
        ),
        cmd!("clip.capture", "Capture...", ["Clip"], Some("Cmd+R"), "{name?}", has_range, |e, p| {
            // Captures the selection as a new whole-file-referencing clip in the clip list (we keep it on the track).
            let name = str_param(p, "name").unwrap_or("Captured").to_string();
            let ids = clip_ids_param(e, p);
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.name = name.clone()
                }
            }
            Ok(json!({"clips": ids.len()}))
        }),
    ]
}

/// Warp clips. On Elastic tracks the material is rendered with a pitch-preserving time stretch
/// (the original source is remembered in `edit.values` so Remove Warp restores it).
fn elastic(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let ratio = f64_or(p, "ratio", 1.0).clamp(0.05, 20.0);
    let s = e.session_mut();
    let mut rendered = 0;
    for id in &ids {
        let Some((t, c)) = s.find_clip(*id).map(|(t, c)| (t, c.clone())) else { continue };
        let elastic = s.track(t).and_then(|x| x.elastic.clone()).filter(|a| a != "varispeed");
        // Start from the unwarped original if this clip was warped before.
        let key = |k: &str| format!("elastic.{}.{k}", id.0);
        let orig = warp_original(s, *id);
        let mut base = c.clone();
        if let Some((src, off, len)) = orig {
            base.content = soundcraft_model::ClipContent::Audio { source: src, offset: off };
            base.length = len.max(1);
            base.stretch = 1.0;
        }
        if elastic.is_none() || !base.is_audio() {
            if let Some(cm) = s.find_clip_mut(*id) {
                cm.stretch = ratio;
            }
            continue;
        }
        let out_len = soundcraft_time::to_samples(base.length as f64 * ratio).max(1);
        let mut warp = base.clone();
        warp.stretch = ratio;
        let Some(src) = super::more_util::render_stretched(s, "clip.elastic_properties", &warp, out_len)? else { continue };
        if orig.is_none()
            && let soundcraft_model::ClipContent::Audio { source, offset } = base.content
        {
            s.edit.values.insert(key("src"), source.0 as f64);
            s.edit.values.insert(key("off"), offset as f64);
            s.edit.values.insert(key("len"), base.length as f64);
        }
        s.edit.values.insert(key("ratio"), ratio);
        let mut nc = base;
        nc.content = soundcraft_model::ClipContent::Audio { source: src, offset: 0 };
        nc.length = out_len;
        nc.stretch = 1.0;
        super::more_util::replace_clip(s, t, *id, nc);
        rendered += 1;
    }
    Ok(json!({"ratio": ratio, "rendered": rendered}))
}

fn remove_warp(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    let mut n = 0;
    for id in &ids {
        let key = |k: &str| format!("elastic.{}.{k}", id.0);
        let orig = warp_original(s, *id);
        if let Some(c) = s.find_clip_mut(*id) {
            c.stretch = 1.0;
            if let Some((src, off, len)) = orig {
                c.content = soundcraft_model::ClipContent::Audio { source: src, offset: off };
                c.length = len.max(1);
                c.fade_out.len = c.fade_out.len.min(c.length);
                n += 1;
            }
        }
        for k in ["src", "off", "len", "ratio"] {
            s.edit.values.remove(&key(k));
        }
    }
    Ok(json!({"restored": n}))
}

/// The audio a warped clip was rendered from, while that file is still in the session.
fn warp_original(s: &soundcraft_model::Session, id: ClipId) -> Option<(soundcraft_model::SourceId, Samples, Samples)> {
    let v = |k: &str| s.edit.values.get(&format!("elastic.{}.{k}", id.0)).copied();
    let src = soundcraft_model::SourceId(v("src")? as u64);
    s.source(src)?;
    Some((src, v("off")? as i64, v("len")? as i64))
}

fn flag(e: &mut Engine, p: &Value, get: fn(&soundcraft_model::Clip) -> bool, set: fn(&mut soundcraft_model::Clip, bool)) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    let v = p.get("value").and_then(Value::as_bool).unwrap_or_else(|| !ids.iter().all(|id| s.find_clip(*id).is_some_and(|(_, c)| get(c))));
    for id in &ids {
        if let Some(c) = s.find_clip_mut(*id) {
            set(c, v);
        }
    }
    Ok(json!({"value": v, "clips": ids.len()}))
}

fn layer(e: &mut Engine, p: &Value, front: bool) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    for t in &mut s.tracks {
        if let Some(pl) = t.playlist_mut() {
            let (mut sel, mut rest): (Vec<_>, Vec<_>) = pl.clips.drain(..).partition(|c| ids.contains(&c.id));
            if front {
                rest.append(&mut sel);
                pl.clips = rest;
            } else {
                sel.append(&mut rest);
                pl.clips = sel;
            }
        }
    }
    Ok(json!({}))
}

fn loop_clips(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let count = i64_or(p, "count", 4).clamp(1, 512);
    let total = position_param(e, "clip.loop", p, "length")?;
    let s = e.session_mut();
    let mut made = 0;
    for id in ids {
        let Some((tid, c)) = s.find_clip(id).map(|(t, c)| (t, c.clone())) else { continue };
        let reps = total.map_or(count - 1, |l| (l / c.length.max(1)).max(1) - 1).clamp(0, 1000);
        let mut copies = Vec::new();
        for k in 1..=reps {
            let mut nc = c.clone();
            nc.start = 0;
            copies.push((c.start + c.length * k, nc));
        }
        for (at, nc) in copies {
            crate::edit::paste(s, tid, at, &[nc], c.length, false);
            made += 1;
        }
    }
    Ok(json!({"loops": made}))
}

fn rename(e: &mut Engine, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("clip.rename", "`name` required"))?.to_string();
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    for id in &ids {
        if let Some(c) = s.find_clip_mut(*id) {
            c.name = name.clone();
        }
    }
    Ok(json!({"clips": ids.len()}))
}

/// The clip's audio as planar channels (its stretch applied to the source span), `None` for
/// MIDI/video clips or audio not loaded.
fn clip_audio(s: &soundcraft_model::Session, c: &soundcraft_model::Clip) -> Option<(Vec<Vec<f32>>, f32)> {
    let soundcraft_model::ClipContent::Audio { source, offset } = &c.content else { return None };
    let audio = s.pool.get(*source)?;
    let rate = s.source(*source).map_or(s.sample_rate.hz(), |x| x.sample_rate).max(1);
    let a = (*offset).max(0) as usize;
    let b = a.saturating_add(((c.length as f64) * c.stretch.max(0.01)).round() as usize);
    let ch = audio.buffer.channels.iter().map(|x| x.get(a.min(x.len())..b.min(x.len())).unwrap_or(&[]).to_vec()).collect();
    Some((ch, rate as f32))
}

fn set_fades(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "clip.set_fades";
    let id = ClipId(p.get("clip").and_then(Value::as_u64).ok_or_else(|| bad(ID, "`clip` required"))?);
    let fin = position_param(e, ID, p, "fade_in")?;
    let fout = position_param(e, ID, p, "fade_out")?;
    if fin.is_none() && fout.is_none() {
        return Err(bad(ID, "`fade_in` or `fade_out` required"));
    }
    let c = e.session().find_clip(id).map(|(_, c)| c.clone()).ok_or_else(|| bad(ID, format!("no clip {}", id.0)))?;
    let len = c.length.max(1);
    let fin = fin.unwrap_or(c.fade_in.len).clamp(0, len);
    let fout = fout.unwrap_or(c.fade_out.len).clamp(0, len - fin);
    let shape = |f: &soundcraft_model::Fade| if f.len == 0 { soundcraft_model::FadeShape::SCurve } else { f.shape };
    let s = e.session_mut();
    if let Some(c) = s.find_clip_mut(id) {
        c.fade_in = soundcraft_model::Fade { len: fin, shape: shape(&c.fade_in) };
        c.fade_out = soundcraft_model::Fade { len: fout, shape: shape(&c.fade_out) };
    }
    Ok(json!({"fade_in": fin, "fade_out": fout}))
}

fn normalize(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "clip.normalize";
    let ids = clip_ids_param(e, p);
    if ids.is_empty() {
        return Err(bad(ID, "select a clip"));
    }
    let lufs = p.get("lufs").and_then(Value::as_f64).map(|v| v.clamp(-40.0, -5.0) as f32);
    let peak_db = f64_or(p, "peak_db", -1.0).clamp(-30.0, 0.0) as f32;
    let mut gains = Vec::new();
    for id in &ids {
        let s = e.session();
        let Some((_, c)) = s.find_clip(*id) else { continue };
        let Some((ch, rate)) = clip_audio(s, c) else { continue };
        let measured = match lufs {
            Some(_) => soundcraft_dsp::offline::integrated_lufs(&ch, rate),
            None => soundcraft_dsp::offline::peak_db(&ch),
        };
        if !measured.is_finite() || measured < -90.0 {
            continue;
        }
        let gain = (lufs.unwrap_or(peak_db) - measured).clamp(-40.0, 36.0);
        if let Some(c) = e.session_mut().find_clip_mut(*id) {
            c.gain_db = gain;
            gains.push(json!({"clip": id.0, "gain_db": gain}));
        }
    }
    if gains.is_empty() {
        return Err(bad(ID, "the selected clips hold no audio to normalize"));
    }
    Ok(json!({"clips": gains}))
}

fn gain(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let abs = p.get("db").and_then(Value::as_f64);
    let delta = p.get("delta_db").and_then(Value::as_f64);
    if abs.is_none() && delta.is_none() {
        return Err(bad("clip.gain", "`db` or `delta_db` required"));
    }
    let s = e.session_mut();
    for id in &ids {
        if let Some(c) = s.find_clip_mut(*id) {
            let v = abs.unwrap_or(f64::from(c.gain_db) + delta.unwrap_or(0.0));
            c.gain_db = if v.is_finite() { (v as f32).clamp(-144.0, 36.0) } else { c.gain_db };
        }
    }
    Ok(json!({"clips": ids.len()}))
}

fn nudge_gain(e: &mut Engine, p: &Value, db: f64) -> Result<Value> {
    let clips = clip_ids_param(e, p);
    gain(e, &json!({"clips": clips, "delta_db": db}))
}

fn quantize_to_grid(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session();
    let moves: Vec<(soundcraft_model::ClipId, i64)> = ids
        .iter()
        .filter_map(|id| s.find_clip(*id).map(|(_, c)| (*id, c.start + c.sync_point)))
        .map(|(id, anchor)| (id, s.edit.grid.snap(anchor, s.sample_rate, &s.tempo, s.frame_rate) - anchor))
        .collect();
    let s = e.session_mut();
    for (id, d) in &moves {
        crate::edit::move_clips(s, &[*id], *d, None);
    }
    Ok(json!({"clips": moves.len()}))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use soundcraft_model::{ClipId, TrackId};

    #[test]
    fn normalize_sets_the_clip_gain_to_the_peak_or_loudness_target() {
        let mut e = crate::demo::demo_engine();
        let pad = e.session().track_by_name("Pad").map(|t| t.id).unwrap();
        let cid = e.session().track(pad).unwrap().clips()[0].id.0;
        e.execute("clip.normalize", &json!({"clips": [cid], "peak_db": -3.0})).unwrap();
        let s = e.session();
        let c = s.find_clip(ClipId(cid)).unwrap().1.clone();
        let (ch, _) = super::clip_audio(s, &c).unwrap();
        let peak = soundcraft_dsp::offline::peak_db(&ch) + c.gain_db;
        assert!((peak + 3.0).abs() < 0.01, "peak after normalize {peak}");
        let peak_gain = c.gain_db;
        e.execute("clip.normalize", &json!({"clips": [cid], "lufs": -20.0})).unwrap();
        let s = e.session();
        let c = s.find_clip(ClipId(cid)).unwrap().1.clone();
        let (ch, rate) = super::clip_audio(s, &c).unwrap();
        let lufs = soundcraft_dsp::offline::integrated_lufs(&ch, rate) + c.gain_db;
        assert!((lufs + 20.0).abs() < 0.05, "loudness after normalize {lufs}");
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(e.session().find_clip(ClipId(cid)).unwrap().1.gain_db, peak_gain, "one undo step per normalize");
    }

    #[test]
    fn set_fades_lengthens_shortens_and_removes_within_the_clip() {
        let mut e = crate::demo::demo_engine();
        let pad = e.session().track_by_name("Pad").map(|t| t.id).unwrap();
        let c = e.session().track(pad).unwrap().clips()[0].clone();
        e.execute("clip.set_fades", &json!({"clip": c.id.0, "fade_in": {"seconds": 0.5}})).unwrap();
        let sr = e.session().sample_rate;
        assert_eq!(e.session().find_clip(c.id).unwrap().1.fade_in.len, sr.samples(0.5));
        e.execute("clip.set_fades", &json!({"clip": c.id.0, "fade_out": c.length * 2})).unwrap();
        let k = e.session().find_clip(c.id).unwrap().1.clone();
        assert_eq!(k.fade_in.len + k.fade_out.len, c.length, "the fades never overlap past the clip");
        e.execute("clip.set_fades", &json!({"clip": c.id.0, "fade_in": 0, "fade_out": 0})).unwrap();
        let k = e.session().find_clip(c.id).unwrap().1.clone();
        assert_eq!((k.fade_in.len, k.fade_out.len), (0, 0));
        assert!(e.execute("clip.set_fades", &json!({"clip": c.id.0})).is_err());
    }

    #[test]
    fn close_gap_moves_the_later_clips_back_on_one_track_or_cuts_time_everywhere() {
        let mut e = crate::demo::demo_engine();
        let pad = e.session().track_by_name("Pad").map(|t| t.id).unwrap();
        let first = e.session().track(pad).unwrap().clips()[0].clone();
        let sr = e.session().sample_rate;
        // Cut a hole into the first clip: separate twice and remove the middle.
        let (a, b) = (first.start + sr.samples(0.5), first.start + sr.samples(1.0));
        e.execute("edit.separate", &json!({"tracks": [pad.0], "at": a})).unwrap();
        e.execute("edit.separate", &json!({"tracks": [pad.0], "at": b})).unwrap();
        e.execute("edit.clear", &json!({"tracks": [pad.0], "start": a, "end": b})).unwrap();
        let after: Vec<i64> = e.session().track(pad).unwrap().clips().iter().filter(|c| c.start >= b).map(|c| c.start).collect();
        let mid = (a + b) / 2;
        assert_eq!(crate::edit::gap_at(e.session(), pad, mid), Some(soundcraft_time::Range::new(a, b)));
        assert!(e.execute("edit.close_gap", &json!({"track": pad.0, "at": mid, "all_tracks": true})).is_err(), "other tracks play there");
        e.execute("edit.close_gap", &json!({"track": pad.0, "at": mid})).unwrap();
        let now: Vec<i64> = e.session().track(pad).unwrap().clips().iter().filter(|c| c.start >= a).map(|c| c.start).collect();
        assert_eq!(now, after.iter().map(|x| x - (b - a)).collect::<Vec<_>>());
        assert!(e.execute("edit.close_gap", &json!({"track": pad.0, "at": mid})).is_err(), "no gap left");
    }

    #[test]
    fn elastic_warp_renders_and_remove_warp_restores() {
        let mut e = crate::demo::demo_engine();
        let pad = e.session().track_by_name("Pad").map(|t| t.id.0).unwrap();
        e.execute("track.elastic", &json!({"track": pad, "algorithm": "polyphonic"})).unwrap();
        let cid = e.session().track(TrackId(pad)).unwrap().clips()[0].id.0;
        let len0 = e.session().find_clip(ClipId(cid)).unwrap().1.length;
        let r = e.execute("clip.elastic_properties", &json!({"clips": [cid], "ratio": 1.5})).unwrap();
        assert_eq!(r["rendered"], 1);
        let c = e.session().find_clip(ClipId(cid)).unwrap().1.clone();
        assert!((c.length as f64 - len0 as f64 * 1.5).abs() < 2.0, "{} vs {}", c.length, len0);
        assert_eq!(c.stretch, 1.0);
        // Re-warping starts from the original, not the rendered audio.
        e.execute("clip.elastic_properties", &json!({"clips": [cid], "ratio": 0.5})).unwrap();
        let c = e.session().find_clip(ClipId(cid)).unwrap().1.clone();
        assert!((c.length as f64 - len0 as f64 * 0.5).abs() < 2.0);
        e.execute("clip.remove_warp", &json!({"clips": [cid]})).unwrap();
        assert_eq!(e.session().find_clip(ClipId(cid)).unwrap().1.length, len0);
    }

    #[test]
    fn remove_warp_keeps_the_audio_when_the_original_file_is_gone() {
        let mut e = crate::demo::demo_engine();
        let pad = e.session().track_by_name("Pad").map(|t| t.id).unwrap();
        e.execute("track.elastic", &json!({"track": pad, "algorithm": "polyphonic"})).unwrap();
        let clip = e.session().track(pad).unwrap().clips()[0].clone();
        e.execute("clip.elastic_properties", &json!({"clips": [clip.id], "ratio": 1.5})).unwrap();
        e.execute("clip.clear", &json!({"sources": [clip.source()]})).unwrap();
        let r = e.execute("clip.remove_warp", &json!({"clips": [clip.id]})).unwrap();
        assert_eq!(r["restored"], 0);
        let src = e.session().find_clip(clip.id).and_then(|(_, c)| c.source()).unwrap();
        assert!(e.session().source(src).is_some());
    }

    #[test]
    fn gain_nudges_one_db_at_a_time_up_to_the_ceiling() {
        let mut e = crate::demo::demo_engine();
        let kick = e.session().track_by_name("Kick").unwrap().clips()[0].id;
        let gain = |e: &crate::Engine| e.session().find_clip(kick).unwrap().1.gain_db;
        e.execute("edit.select", &json!({"clips": [kick.0]})).unwrap();
        e.execute("clip.gain_nudge_up", &json!({})).unwrap();
        e.execute("clip.gain_nudge_up", &json!({})).unwrap();
        e.execute("clip.gain_nudge_down", &json!({})).unwrap();
        assert_eq!(gain(&e), 1.0);
        e.execute("clip.gain", &json!({"db": 35.5})).unwrap();
        e.execute("clip.gain_nudge_up", &json!({})).unwrap();
        assert_eq!(gain(&e), 36.0);
    }
}
