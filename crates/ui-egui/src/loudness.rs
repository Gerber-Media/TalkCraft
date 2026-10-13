//! The episode's loudness in the toolbar: integrated LUFS and true peak of the whole mix,
//! measured in the background a moment after the audio last changed (not on selection or view
//! changes), so the readout follows editing without slowing it down.

use std::hash::{Hash, Hasher};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// Wait this long after the last change before measuring again.
const SETTLE: Duration = Duration::from_millis(1500);

#[derive(Debug, Default)]
pub struct Loudness {
    /// Engine revision last looked at, and the fingerprint of the audio at that time.
    seen_rev: u64,
    fingerprint: u64,
    /// Fingerprint of the audio the shown value belongs to.
    measured: u64,
    changed_at: Option<Instant>,
    pending: Option<(u64, Receiver<Option<(f32, f32)>>)>,
    /// (integrated LUFS, true peak dBTP); `None` before the first result or for silence.
    pub value: Option<(f32, f32)>,
}

/// What decides the mix: tracks (clips, mixer, automation) and the sources they play.
fn fingerprint(s: &soundcraft_model::Session) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&s.tracks).unwrap_or_default().hash(&mut h);
    s.sources.len().hash(&mut h);
    s.sample_rate.hz().hash(&mut h);
    h.finish()
}

impl Loudness {
    /// Called every frame: notices changes, starts a measurement once they settle, picks up results.
    pub fn update(&mut self, engine: &soundcraft_engine::Engine, playing: bool, ctx: &egui::Context) {
        if let Some((fp, rx)) = &self.pending
            && let Ok(v) = rx.try_recv()
        {
            self.value = v;
            self.measured = *fp;
            self.pending = None;
        }
        if engine.revision != self.seen_rev {
            self.seen_rev = engine.revision;
            let fp = fingerprint(engine.session());
            if fp != self.fingerprint {
                self.fingerprint = fp;
                self.changed_at = Some(Instant::now());
            }
        }
        let settled = self.changed_at.is_none_or(|t| t.elapsed() >= SETTLE);
        if self.pending.is_none() && self.measured != self.fingerprint && settled && !playing {
            self.start(engine.session_arc(), self.fingerprint, ctx);
        }
    }

    /// Whether a newer measurement is on its way (the value shown is for older audio).
    pub fn stale(&self) -> bool {
        self.measured != self.fingerprint
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start(&mut self, s: std::sync::Arc<soundcraft_model::Session>, fp: u64, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("loudness".into()).spawn(move || {
            let _ = tx.send(measure(&s));
            // Show the result now, not at the next input.
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.pending = Some((fp, rx));
        } else {
            self.measured = fp;
        }
    }

    /// The browser has no threads for this: no readout there.
    #[cfg(target_arch = "wasm32")]
    fn start(&mut self, _s: std::sync::Arc<soundcraft_model::Session>, fp: u64, _ctx: &egui::Context) {
        let _ = channel::<()>();
        self.measured = fp;
    }
}

/// Integrated loudness and true peak of the whole mix (folded to stereo); `None` when silent or
/// longer than four hours.
pub fn measure(s: &soundcraft_model::Session) -> Option<(f32, f32)> {
    let end = s.content_end();
    if end <= 0 || end > s.sample_rate.samples(4.0 * 3600.0) {
        return None;
    }
    let ch = soundcraft_mix::render_range(s, soundcraft_time::Range::new(0, end), 4096);
    let ch = if ch.len() > 2 { soundcraft_mix::fold_down_stereo(s.main_format(), &ch) } else { ch };
    let frames = ch.iter().map(Vec::len).min().unwrap_or(0);
    let mut m = soundcraft_dsp::meter::LoudnessMeter::new(s.sample_rate.as_f64() as f32, ch.len());
    m.process(&ch, frames);
    let lufs = m.integrated_lufs();
    lufs.is_finite().then(|| (lufs, m.true_peak_db()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_the_mix_and_only_remeasures_when_the_audio_changes() {
        let e = soundcraft_engine::demo::demo_engine();
        let (lufs, tp) = measure(e.session()).expect("the demo mix has audio");
        assert!((-60.0..0.0).contains(&lufs) && tp < 6.0, "{lufs} LUFS, {tp} dBTP");
        let mut e = e;
        let before = fingerprint(e.session());
        e.execute("edit.select", &serde_json::json!({"start": 0, "end": 48_000})).unwrap();
        assert_eq!(fingerprint(e.session()), before, "a selection is not a change to the audio");
        e.execute("mix.volume", &serde_json::json!({"track": e.session().tracks[0].id.0, "db": -6.0})).unwrap();
        assert_ne!(fingerprint(e.session()), before);
    }
}
