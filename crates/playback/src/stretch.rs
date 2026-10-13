//! Pitch-preserving playback rate (WSOLA): listen to an episode faster or slower without the
//! voices turning into chipmunks. The mix is rendered at its normal speed; grains of it are
//! overlapped at a different hop, each one shifted (within a few ms) to where it continues the
//! previous one most smoothly. Buffers are sized once per channel count, so the audio callback
//! does not allocate in steady state.

/// Grain length at 48 kHz (about 40 ms); scaled with the device rate.
const GRAIN_48K: usize = 1920;
/// Search range either side of the nominal grain position (about 10 ms at 48 kHz).
const SEARCH_48K: usize = 480;
/// The correlation looks at every n-th sample: plenty for speech, a quarter of the work.
const DECIMATE: usize = 4;

#[derive(Debug, Default)]
pub struct Stretch {
    n: usize,
    hop: usize,
    search: usize,
    window: Vec<f32>,
    /// Input frames (planar), `len` of them valid, indices relative to the start.
    input: Vec<Vec<f32>>,
    len: usize,
    /// Where the next grain nominally starts, and where the last one would naturally continue.
    pos: f64,
    next: usize,
    started: bool,
    /// Overlap-add accumulator (one grain long) and the finished hop being played out.
    acc: Vec<Vec<f32>>,
    out: Vec<Vec<f32>>,
    out_read: usize,
    out_len: usize,
}

impl Stretch {
    /// Sizes the buffers for `channels` at `rate` Hz (allocates; call outside steady state).
    pub fn prepare(&mut self, channels: usize, rate: f64) {
        let scale = (rate / 48_000.0).clamp(0.25, 4.0);
        let n = ((GRAIN_48K as f64 * scale) as usize).max(64) & !1;
        let ch = channels.max(1);
        if self.n == n && self.input.len() == ch {
            self.reset();
            return;
        }
        self.n = n;
        self.hop = n / 2;
        self.search = ((SEARCH_48K as f64 * scale) as usize).max(8);
        // Periodic Hann: two of them at half overlap add up to exactly one.
        self.window = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
        // Room for the farthest grain at 4× (nominal hop 2n) plus the search on both sides.
        let cap = 4 * n + 4 * self.search;
        self.input = vec![vec![0.0; cap]; ch];
        self.acc = vec![vec![0.0; n]; ch];
        self.out = vec![vec![0.0; self.hop]; ch];
        self.reset();
    }

    pub fn is_prepared_for(&self, channels: usize) -> bool {
        self.n > 0 && self.input.len() == channels.max(1)
    }

    /// Forgets everything buffered (after a locate or a new play).
    pub fn reset(&mut self) {
        self.len = 0;
        self.pos = 0.0;
        self.next = 0;
        self.started = false;
        self.out_read = 0;
        self.out_len = 0;
        for c in &mut self.acc {
            c.iter_mut().for_each(|x| *x = 0.0);
        }
    }

    /// One output frame at `rate` (0.25–4, 1 = normal); `pull` fills one input frame.
    pub fn next_frame(&mut self, rate: f64, frame: &mut [f32], mut pull: impl FnMut(&mut [f32])) {
        if self.n == 0 {
            pull(frame);
            return;
        }
        if self.out_read >= self.out_len {
            self.hop_once(rate.clamp(0.25, 4.0), &mut pull);
        }
        for (c, x) in frame.iter_mut().enumerate() {
            *x = self.out.get(c).and_then(|o| o.get(self.out_read)).copied().unwrap_or(0.0);
        }
        self.out_read += 1;
    }

    fn fill_to(&mut self, need: usize, pull: &mut impl FnMut(&mut [f32])) {
        let cap = self.input.first().map_or(0, Vec::len);
        let mut tmp = [0.0f32; soundcraft_mix::MAX_CHANNELS];
        let ch = self.input.len().min(tmp.len());
        while self.len < need.min(cap) {
            pull(tmp.get_mut(..ch).unwrap_or(&mut []));
            for (c, buf) in self.input.iter_mut().enumerate() {
                if let Some(slot) = buf.get_mut(self.len) {
                    *slot = tmp.get(c).copied().unwrap_or(0.0);
                }
            }
            self.len += 1;
        }
    }

    /// Mono (first two channels) sample at `i`, for matching.
    fn mono(&self, i: usize) -> f32 {
        self.input.iter().take(2).map(|c| c.get(i).copied().unwrap_or(0.0)).sum()
    }

    /// Where near `nominal` a grain continues the last one best (normalised cross-correlation).
    fn best_start(&self, nominal: usize) -> usize {
        let lo = nominal.saturating_sub(self.search);
        let hi = nominal + self.search;
        let mut best = (nominal, f32::MIN);
        for k in (lo..=hi).step_by(2) {
            let (mut dot, mut energy) = (0.0f32, 1e-9f32);
            for j in (0..self.hop).step_by(DECIMATE) {
                let a = self.mono(k + j);
                dot += a * self.mono(self.next + j);
                energy += a * a;
            }
            let score = dot / energy.sqrt();
            if score > best.1 {
                best = (k, score);
            }
        }
        best.0
    }

    fn hop_once(&mut self, rate: f64, pull: &mut impl FnMut(&mut [f32])) {
        let nominal = self.pos.max(0.0) as usize;
        let start = if self.started {
            self.fill_to(nominal + self.search + self.n, pull);
            self.best_start(nominal)
        } else {
            self.fill_to(self.n, pull);
            self.started = true;
            0
        };
        let (n, hop) = (self.n, self.hop);
        for ((acc, inp), out) in self.acc.iter_mut().zip(&self.input).zip(&mut self.out) {
            for (i, (a, w)) in acc.iter_mut().zip(&self.window).enumerate() {
                *a += inp.get(start + i).copied().unwrap_or(0.0) * w;
            }
            out.copy_from_slice(acc.get(..hop).unwrap_or(&[]));
            acc.copy_within(hop..n, 0);
            acc.get_mut(n - hop..).unwrap_or(&mut []).iter_mut().for_each(|x| *x = 0.0);
        }
        self.out_read = 0;
        self.out_len = hop;
        self.next = start + hop;
        self.pos += hop as f64 * rate;
        // Drop input no later grain can reach.
        let keep_from = (self.pos as usize).saturating_sub(self.search).min(self.next);
        if keep_from > 0 {
            for c in &mut self.input {
                c.copy_within(keep_from..self.len, 0);
            }
            self.len -= keep_from;
            self.next -= keep_from;
            self.pos -= keep_from as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(rate: f64, input: &[f32], frames: usize) -> (Vec<f32>, usize) {
        let mut s = Stretch::default();
        s.prepare(1, 48_000.0);
        let mut i = 0usize;
        let mut out = Vec::new();
        let mut f = [0.0f32; 1];
        for _ in 0..frames {
            s.next_frame(rate, &mut f, |x| {
                x[0] = input.get(i).copied().unwrap_or(0.0);
                i += 1;
            });
            out.push(f[0]);
        }
        (out, i)
    }

    /// Zero crossings per second: the pitch of a sine.
    fn pitch(x: &[f32]) -> f64 {
        let z = x.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        z as f64 / 2.0 / (x.len() as f64 / 48_000.0)
    }

    #[test]
    fn faster_and_slower_keep_the_pitch() {
        let tone: Vec<f32> = (0..48_000 * 8).map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / 48_000.0).sin() * 0.5).collect();
        for rate in [0.75, 1.5, 2.0] {
            let (out, used) = run(rate, &tone, 48_000 * 2);
            let consumed = used as f64 / (48_000.0 * 2.0);
            assert!((consumed - rate).abs() < 0.08, "rate {rate}: consumed {consumed}");
            let p = pitch(&out[4_800..]);
            assert!((p - 220.0).abs() < 6.0, "rate {rate}: pitch {p}");
            let peak = out[4_800..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!((peak - 0.5).abs() < 0.08, "rate {rate}: level {peak}");
        }
    }
}
