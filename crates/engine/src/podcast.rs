//! The podcast template: speaker tracks with a voice chain, a jingle track and a master that
//! keeps the mix below -1 dBTP. Loudness targets for delivery live here too.

use soundcraft_model::{ChannelFormat, Insert, MarkerKind, Session, TrackKind};
use soundcraft_time::SampleRate;

/// Integrated loudness most podcast platforms normalise stereo episodes to (LUFS).
pub const PODCAST_LUFS: f32 = -16.0;
/// True-peak ceiling for delivery (dBTP).
pub const PODCAST_TRUE_PEAK: f32 = -1.0;

/// An insert with every parameter at its default, then `overrides` applied.
fn insert(id: &str, overrides: &[(&str, f32)]) -> Insert {
    let mut i = Insert::new(id);
    if let Some(info) = soundcraft_dsp::plugin_info(id) {
        for p in info.params {
            i.params.insert(p.id.to_string(), p.default);
        }
    }
    for (k, v) in overrides {
        i.params.insert((*k).to_string(), *v);
    }
    i
}

/// High-pass and a little presence, a gentle gate against room noise, levelling, de-essing.
fn voice_chain() -> [Insert; 4] {
    [
        insert(
            "eq_7band",
            &[
                ("hpf_on", 1.0),
                ("hpf_freq", 80.0),
                ("low_mid_freq", 300.0),
                ("low_mid_gain", -2.0),
                ("high_mid_freq", 4000.0),
                ("high_mid_gain", 2.0),
            ],
        ),
        insert("expander_gate", &[("threshold", -50.0), ("ratio", 2.0), ("range", 12.0), ("attack", 1.0), ("hold", 120.0), ("release", 250.0)]),
        insert("compressor", &[("threshold", -22.0), ("ratio", 3.0), ("attack", 10.0), ("release", 120.0), ("makeup", 4.0)]),
        insert("de_esser", &[("freq", 6500.0), ("threshold", -30.0), ("range", 8.0)]),
    ]
}

/// A new podcast session: `speakers` mono voice tracks (1..=8), a stereo jingle track and a master.
pub fn podcast_session(name: &str, sample_rate: SampleRate, speakers: usize) -> Session {
    let mut s = Session::new(name, sample_rate);
    let speakers = speakers.clamp(1, 8);
    for n in 0..speakers {
        let label = if n == 0 { "Moderation".to_string() } else { format!("Gast {n}") };
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some(&label));
        if let Some(tr) = s.track_mut(t) {
            for (slot, ins) in tr.mixer.inserts.iter_mut().zip(voice_chain()) {
                *slot = Some(ins);
            }
            // Each speaker on their own microphone input.
            tr.mixer.input = soundcraft_model::Route::Hardware(format!("In {}", n + 1));
        }
    }
    let j = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Jingles & Musik"));
    if let Some(tr) = s.track_mut(j) {
        tr.mixer.volume_db = -6.0;
        tr.mixer.record_lock = true;
    }
    let m = s.add_track(TrackKind::Master, ChannelFormat::Stereo, Some("Master"));
    if let Some(tr) = s.track_mut(m) {
        tr.name = "Master".into();
        if let Some(slot) = tr.mixer.inserts.first_mut() {
            *slot = Some(insert("maximizer", &[("threshold", -3.0), ("ceiling", PODCAST_TRUE_PEAK - 0.5)]));
        }
    }
    s.add_marker("Intro", MarkerKind::Marker, 0, 0);
    if let Some(first) = s.tracks.first().map(|t| t.id) {
        s.edit.selected_tracks = vec![first];
    }
    s.edit.main_counter = soundcraft_time::TimeFormat::MinSecs;
    s.edit.rulers = vec!["min_secs".into(), "markers".into(), "subtitles".into()];
    s.edit.grid = soundcraft_time::GridValue::Seconds(1.0);
    s
}

/// The tracks the Record button records: every active audio track that is not locked.
pub fn record_targets(s: &Session) -> Vec<soundcraft_model::TrackId> {
    s.tracks.iter().filter(|t| t.kind == TrackKind::Audio && !t.inactive && !t.mixer.record_lock).map(|t| t.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_has_voice_tracks_with_chain_and_a_limited_master() {
        let s = podcast_session("Folge 1", SampleRate::new(48_000).unwrap(), 2);
        let names: Vec<_> = s.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Moderation", "Gast 1", "Jingles & Musik", "Master"]);
        let chain: Vec<_> = s.tracks[0].mixer.inserts.iter().flatten().map(|i| i.plugin.as_str()).collect();
        assert_eq!(chain, ["eq_7band", "expander_gate", "compressor", "de_esser"]);
        for p in &chain {
            assert!(soundcraft_dsp::plugin_info(p).is_some(), "{p} is a built-in plugin");
        }
        assert_eq!(s.tracks[3].mixer.inserts[0].as_ref().map(|i| i.plugin.as_str()), Some("maximizer"));
        let inputs: Vec<_> = s.tracks.iter().take(2).map(|t| t.mixer.input.clone()).collect();
        assert_eq!(inputs, [soundcraft_model::Route::Hardware("In 1".into()), soundcraft_model::Route::Hardware("In 2".into())]);
        assert_eq!(record_targets(&s), [s.tracks[0].id, s.tracks[1].id], "the jingle track is locked, the master never records");
    }

    #[test]
    fn speaker_count_is_clamped() {
        let sr = SampleRate::new(48_000).unwrap();
        assert_eq!(podcast_session("x", sr, 0).tracks.len(), 3);
        assert_eq!(podcast_session("x", sr, 99).tracks.len(), 10);
    }
}
