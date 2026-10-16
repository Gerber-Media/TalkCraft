//! Offline speech repair for podcasts: crosstalk removal (an adaptive gate across speaker tracks),
//! a leveler that evens out a speaker within a track, breath and mouth-click reduction, and a
//! voice matcher that gives one microphone the tonal balance of another.
//!
//! Most of it works on a coarse level envelope (one value per [`BLOCK_S`]) and returns gains in
//! dB per block, which the engine writes as clip gain envelopes: non-destructive and undoable.
//! Only the click repair and the voice matching change samples. Memory stays at
//! `O(audio / block)` beyond the audio itself, so hour-long episodes are fine.

use rustfft::{FftPlanner, num_complex::Complex};

/// Length of one level block in seconds.
pub const BLOCK_S: f32 = 0.01;
/// Level of digital silence (dB).
pub const SILENCE_DB: f32 = -120.0;

/// Samples per level block at `rate`.
pub fn block_len(rate: f32) -> usize {
    ((if rate.is_finite() { rate } else { 48_000.0 }) * BLOCK_S).round().clamp(8.0, 20_000.0) as usize
}

fn to_db(power: f32) -> f32 {
    if power > 1e-12 { (10.0 * power.log10()).max(SILENCE_DB) } else { SILENCE_DB }
}

/// RMS level (dB) per block of `block` samples, all channels together.
pub fn levels_db(ch: &[Vec<f32>], block: usize) -> Vec<f32> {
    let block = block.max(1);
    let len = ch.iter().map(Vec::len).max().unwrap_or(0);
    let n = ch.len().max(1) as f32;
    (0..len.div_ceil(block))
        .map(|b| {
            let (a, z) = (b * block, ((b + 1) * block).min(len));
            let sum: f32 = ch.iter().map(|c| c.get(a.min(c.len())..z.min(c.len())).map_or(0.0, |s| s.iter().map(|v| v * v).sum::<f32>())).sum();
            to_db(sum / (n * (z - a).max(1) as f32))
        })
        .collect()
}

/// The `p` quantile (0…1) of the blocks that carry any signal; `SILENCE_DB` if none do.
fn quantile(levels: &[f32], p: f32) -> f32 {
    let mut v: Vec<f32> = levels.iter().copied().filter(|l| *l > SILENCE_DB + 1.0).collect();
    if v.is_empty() {
        return SILENCE_DB;
    }
    v.sort_by(f32::total_cmp);
    let i = ((v.len() - 1) as f32 * p.clamp(0.0, 1.0)).round() as usize;
    v.get(i).copied().unwrap_or(SILENCE_DB)
}

/// `levels` with `gain_db` (per block; missing blocks: 0 dB) added.
pub fn with_gain(mut levels: Vec<f32>, gain_db: &[f32]) -> Vec<f32> {
    for (l, g) in levels.iter_mut().zip(gain_db) {
        *l = (*l + g).max(SILENCE_DB);
    }
    levels
}

/// The background level: the quietest tenth of the blocks with any signal.
pub fn noise_floor_db(levels: &[f32]) -> f32 {
    quantile(levels, 0.1)
}

/// The blocks that hold speech: clearly above the floor (`margin_db`).
fn speech_mask(levels: &[f32], margin_db: f32) -> Vec<bool> {
    let floor = noise_floor_db(levels);
    // A clip without pauses has its "floor" in the quiet speech itself: anything above -45 dBFS
    // counts as speech then, whatever the floor says. Another speaker's bleed (after crosstalk
    // removal 25–35 dB below the loudest speech) does not count.
    let threshold = (floor + margin_db).clamp(-70.0, -45.0).max(quantile(levels, 0.95) - 26.0);
    levels.iter().map(|l| *l > threshold).collect()
}

/// Gains (dB, 0 or `-reduction_db`, ramped) for a mask of open blocks: opens `pre` blocks early,
/// stays open `hold` blocks after, and ramps over `ramp` blocks so nothing clicks.
fn gate_from_mask(open: &[bool], reduction_db: f32, pre: usize, hold: usize, ramp: usize) -> Vec<f32> {
    let n = open.len();
    let mut keep = vec![false; n];
    for (i, o) in open.iter().enumerate() {
        if *o {
            for k in keep.iter_mut().take((i + hold + 1).min(n)).skip(i.saturating_sub(pre)) {
                *k = true;
            }
        }
    }
    // Closed stretches shorter than a ramp in and out stay open (no pumping between words).
    let min_closed = 2 * ramp + 1;
    let mut i = 0;
    while i < n {
        if keep.get(i) == Some(&false) {
            let start = i;
            while keep.get(i) == Some(&false) {
                i += 1;
            }
            if start > 0 && i < n && i - start < min_closed {
                keep.iter_mut().take(i).skip(start).for_each(|k| *k = true);
            }
        } else {
            i += 1;
        }
    }
    // Distance to the nearest open block, turned into a ramp.
    let mut dist = vec![usize::MAX; n];
    let mut last = None;
    for (i, d) in dist.iter_mut().enumerate() {
        if keep.get(i) == Some(&true) {
            last = Some(i);
        }
        if let Some(l) = last {
            *d = i - l;
        }
    }
    last = None;
    for i in (0..n).rev() {
        if keep.get(i) == Some(&true) {
            last = Some(i);
        }
        if let (Some(l), Some(d)) = (last, dist.get_mut(i)) {
            *d = (*d).min(l - i);
        }
    }
    let ramp = ramp.max(1) as f32;
    dist.iter().map(|d| if *d == usize::MAX { -reduction_db } else { -reduction_db * (*d as f32 / ramp).min(1.0) }).collect()
}

/// Settings for [`crosstalk_gains`].
#[derive(Debug, Clone, Copy)]
pub struct GateSettings {
    /// How far a track is turned down where it holds only background or another speaker (dB).
    pub reduction_db: f32,
    /// A track stays open while it is at most this much quieter than the loudest track (dB):
    /// bleed of another speaker is usually 10–20 dB below that speaker's own microphone.
    pub margin_db: f32,
    /// How far above its background a track must rise to count as speaking (dB).
    pub open_db: f32,
}

impl Default for GateSettings {
    fn default() -> Self {
        Self { reduction_db: 15.0, margin_db: 9.0, open_db: 8.0 }
    }
}

/// Crosstalk removal: for each track's level envelope (all on the same timeline, one value per
/// block), the gains in dB per block. A track is open where it rises above its own background and
/// is not clearly quieter than the loudest track, i.e. where its own speaker talks. With one
/// track it is an adaptive noise gate.
pub fn crosstalk_gains(levels: &[Vec<f32>], set: GateSettings) -> Vec<Vec<f32>> {
    let n = levels.iter().map(Vec::len).max().unwrap_or(0);
    let red = set.reduction_db.clamp(0.0, 60.0);
    let floors: Vec<f32> = levels.iter().map(|l| noise_floor_db(l)).collect();
    // The loudest track per block, over ±30 ms: at the edges of words the levels move fast, and
    // a single block where the bleed briefly beats the fading speaker must not open the gate.
    let reach = 3;
    levels
        .iter()
        .enumerate()
        .map(|(t, l)| {
            let floor = floors.get(t).copied().unwrap_or(SILENCE_DB);
            let loudest_other = |i: usize| {
                levels
                    .iter()
                    .enumerate()
                    .filter(|(o, _)| *o != t)
                    .flat_map(|(_, x)| x.get(i.saturating_sub(reach)..(i + reach + 1).min(x.len())).unwrap_or(&[]).iter().copied())
                    .fold(SILENCE_DB, f32::max)
            };
            let mut open: Vec<bool> = (0..n)
                .map(|i| {
                    let own = l.get(i).copied().unwrap_or(SILENCE_DB);
                    own > floor + set.open_db && own > -70.0 && own >= loudest_other(i) - set.margin_db
                })
                .collect();
            // Openings shorter than 30 ms are noise, not speech.
            let mut i = 0;
            while i < n {
                if open.get(i) == Some(&true) {
                    let start = i;
                    while open.get(i) == Some(&true) {
                        i += 1;
                    }
                    if i - start < 3 {
                        open.iter_mut().take(i).skip(start).for_each(|o| *o = false);
                    }
                } else {
                    i += 1;
                }
            }
            // 30 ms early, 250 ms hold, 40 ms ramps.
            gate_from_mask(&open, red, 3, 25, 4)
        })
        .collect()
}

/// Leveler: per block, the gain (dB) that brings the speech around it (a window of `window_s`)
/// to the clip's typical speech level, within ±`max_db`. Pauses keep the gain of the speech next
/// to them, so the background does not swell between sentences.
pub fn leveler_gains(levels: &[f32], window_s: f32, max_db: f32) -> Vec<f32> {
    let n = levels.len();
    let speech = speech_mask(levels, 10.0);
    let spoken: Vec<f32> = levels.iter().zip(&speech).filter(|(_, s)| **s).map(|(l, _)| *l).collect();
    if spoken.len() < 10 {
        return vec![0.0; n];
    }
    let target = quantile(&spoken, 0.5);
    // Prefix sums of speech power and speech block count.
    let mut pw = vec![0.0f64; n + 1];
    let mut cnt = vec![0usize; n + 1];
    for i in 0..n {
        let (l, s) = (levels.get(i).copied().unwrap_or(SILENCE_DB), speech.get(i) == Some(&true));
        let p = if s { 10f64.powf(f64::from(l) / 10.0) } else { 0.0 };
        pw[i + 1] = pw[i] + p;
        cnt[i + 1] = cnt[i] + usize::from(s);
    }
    let half = ((window_s / BLOCK_S) / 2.0).clamp(10.0, 6000.0) as usize;
    let max_db = max_db.clamp(0.0, 20.0);
    let mut raw: Vec<Option<f32>> = (0..n)
        .map(|i| {
            let (a, z) = (i.saturating_sub(half), (i + half + 1).min(n));
            let c = cnt[z] - cnt[a];
            // At least a fifth of a second of speech in the window.
            (c >= 20).then(|| (target - to_db(((pw[z] - pw[a]) / c as f64) as f32)).clamp(-max_db, max_db))
        })
        .collect();
    // Windows without speech take the nearest gain.
    let mut last = None;
    for g in raw.iter_mut() {
        match g {
            Some(v) => last = Some(*v),
            None => *g = last,
        }
    }
    let first = raw.iter().flatten().next().copied().unwrap_or(0.0);
    let mut g: Vec<f32> = raw.into_iter().map(|v| v.unwrap_or(first)).collect();
    // Smooth both ways (about 0.3 s), so the gain glides.
    let a = 1.0 - (-1.0 / 30.0f32).exp();
    for i in 1..n {
        g[i] = g[i - 1] + a * (g[i] - g[i - 1]);
    }
    for i in (0..n.saturating_sub(1)).rev() {
        g[i] = g[i + 1] + a * (g[i] - g[i + 1]);
    }
    g
}

/// The "average" frequency of `x` in Hz (RMS frequency, from how fast it changes from sample to
/// sample): voiced speech sits low, an inhale around 1.5–4 kHz, an "s" higher.
fn rms_frequency(x: &[f32], rate: f32) -> f32 {
    let e: f32 = x.iter().map(|v| v * v).sum();
    if x.len() < 2 || e <= 0.0 {
        return 0.0;
    }
    let d: f32 = x.windows(2).map(|w| if let [a, b] = w { (b - a) * (b - a) } else { 0.0 }).sum();
    // For a sine of angular frequency w the ratio is 2 (1 − cos w).
    let ratio = (d / e).clamp(0.0, 4.0);
    (1.0 - ratio / 2.0).acos() * rate / std::f32::consts::TAU
}

/// Breaths: block ranges `(start, end)` of quiet noise in the pattern pause → breath → speech,
/// 0.12–1 s long, clearly above the room and well below the voice. `gain_db` (per block, may be
/// empty) is the gain the clip's envelope already applies, e.g. from crosstalk removal: it keeps
/// another speaker's bleed from counting as this speaker's voice.
///
/// Breaths are told from consonants by their sound (noise centred around 1–4 kHz, where an "s"
/// or "sch" sits higher) and by their place: after a pause, right before speech.
pub fn breath_ranges(ch: &[Vec<f32>], rate: f32, gain_db: &[f32]) -> Vec<(usize, usize)> {
    let block = block_len(rate);
    let raw = levels_db(ch, block);
    let heard = with_gain(raw.clone(), gain_db);
    let floor = noise_floor_db(&raw);
    let speech: Vec<f32> = heard.iter().zip(speech_mask(&heard, 10.0)).filter(|(_, s)| *s).map(|(l, _)| *l).collect();
    if speech.len() < 50 {
        return Vec::new();
    }
    let loud = quantile(&speech, 0.5);
    if loud - floor < 25.0 {
        // Too little distance between the room and the voice to tell a breath apart.
        return Vec::new();
    }
    let len = ch.iter().map(Vec::len).max().unwrap_or(0);
    let mono: Vec<f32> = (0..len).map(|i| ch.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>()).collect();
    let rate = if rate.is_finite() && rate > 1000.0 { rate } else { 48_000.0 };
    let candidate: Vec<bool> = raw
        .iter()
        .zip(&heard)
        .enumerate()
        .map(|(b, (r, h))| {
            let f = mono.get(b * block..((b + 1) * block).min(len)).map_or(0.0, |x| rms_frequency(x, rate));
            *r > floor + 6.0 && *h < loud - 15.0 && (1000.0..=6000.0).contains(&f)
        })
        .collect();
    let blocks = |s: f32| (s / BLOCK_S) as usize;
    let (min_b, max_b) = (blocks(0.12), blocks(1.0));
    let quiet = |k: usize| raw.get(k).is_some_and(|l| *l < floor + 4.0);
    let spoken = |k: usize| heard.get(k).is_some_and(|l| *l > loud - 10.0);
    let mut out = Vec::new();
    let mut i = 0;
    while i < candidate.len() {
        if candidate.get(i) == Some(&true) {
            let start = i;
            // Allow single-block dropouts inside a breath.
            while i < candidate.len() && (candidate.get(i) == Some(&true) || candidate.get(i + 1) == Some(&true)) {
                i += 1;
            }
            let pause_before = (start.saturating_sub(blocks(0.4))..start).filter(|k| quiet(*k)).count() >= blocks(0.08);
            let speech_after = (i..i + blocks(0.5)).any(spoken);
            if (min_b..=max_b).contains(&(i - start)) && pause_before && speech_after {
                out.push((start, i));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Gains (dB per block, `n` blocks) that lower `ranges` by `reduction_db`, with 20 ms ramps.
pub fn dip_gains(n: usize, ranges: &[(usize, usize)], reduction_db: f32) -> Vec<f32> {
    let mut open = vec![true; n];
    for (a, z) in ranges {
        open.iter_mut().take((*z).min(n)).skip(*a).for_each(|o| *o = false);
    }
    gate_from_mask(&open, reduction_db.clamp(0.0, 60.0), 0, 0, 2)
}

/// Mouth clicks and lip smacks: short spikes that stand out from the sound around them. Each is
/// replaced by a smoothed copy of itself (its low frequencies stay, the click goes).
/// `sensitivity` 1 (only obvious clicks) … 10 (many). Returns the audio and how many it repaired.
pub fn declick(ch: &[Vec<f32>], rate: f32, sensitivity: f32) -> (Vec<Vec<f32>>, usize) {
    let s = if sensitivity.is_finite() { sensitivity.clamp(1.0, 10.0) } else { 5.0 };
    // How many times the local average change a click must exceed: 33 (1) … 10 (10). Voiced
    // speech has sharp glottal pulses too (about 10× the local average, every pitch period);
    // real clicks stand out 40–100×.
    let k = 36.0 - 2.6 * s;
    let rate = if rate.is_finite() && rate > 1000.0 { rate } else { 48_000.0 };
    let ms = |m: f32| ((rate * m / 1000.0).round() as usize).max(1);
    let (win, max_len, pad, smooth) = (ms(5.0), ms(4.0), ms(0.3), ms(0.4));
    let mut total = 0;
    let out = ch
        .iter()
        .map(|x| {
            let n = x.len();
            if n < 4 * win {
                return x.clone();
            }
            // First difference (the click is in the high frequencies) and its running mean.
            let d: Vec<f32> = (0..n).map(|i| (x[i] - if i > 0 { x[i - 1] } else { 0.0 }).abs()).collect();
            let mut pre = vec![0.0f64; n + 1];
            for i in 0..n {
                pre[i + 1] = pre[i] + f64::from(d[i]);
            }
            let local = |i: usize| {
                let (a, z) = (i.saturating_sub(win), (i + win).min(n));
                ((pre[z] - pre[a]) / (z - a).max(1) as f64) as f32
            };
            let hits: Vec<usize> = (0..n).filter(|&i| d[i] > 0.01 && d[i] > k * local(i)).collect();
            let mut y = x.clone();
            let mut j = 0;
            while j < hits.len() {
                let start = hits[j];
                let mut end = start;
                while j + 1 < hits.len() && hits[j + 1] - end <= ms(1.0) {
                    j += 1;
                    end = hits[j];
                }
                j += 1;
                if end - start > max_len {
                    // Too long for a click (a plosive or an onset): leave it.
                    continue;
                }
                let (a, z) = (start.saturating_sub(pad), (end + pad + 1).min(n));
                for i in a..z {
                    let (sa, sz) = (i.saturating_sub(smooth), (i + smooth + 1).min(n));
                    let avg = x.get(sa..sz).map_or(0.0, |w| w.iter().sum::<f32>() / w.len().max(1) as f32);
                    // Blend in and out over the padding, fully smoothed in the middle.
                    let edge = ((i - a).min(z - 1 - i) as f32 / pad.max(1) as f32).min(1.0);
                    y[i] = x[i] + (avg - x[i]) * edge;
                }
                total += 1;
            }
            y
        })
        .collect();
    (out, total)
}

const FFT: usize = 4096;
const HOP: usize = FFT / 4;

fn hann() -> Vec<f32> {
    (0..FFT).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT as f32).cos()).collect()
}

/// Average power per FFT bin over the frames that hold speech (the voice's tonal balance).
pub fn speech_spectrum(ch: &[Vec<f32>]) -> Vec<f32> {
    let bins = FFT / 2 + 1;
    let len = ch.iter().map(Vec::len).max().unwrap_or(0);
    let mut acc = vec![0.0f64; bins];
    if len < FFT {
        return vec![0.0; bins];
    }
    let mono: Vec<f32> = (0..len).map(|i| ch.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>()).collect();
    let levels: Vec<f32> = (0..(len - FFT) / HOP + 1)
        .map(|f| {
            let w = mono.get(f * HOP..f * HOP + FFT).unwrap_or(&[]);
            to_db(w.iter().map(|v| v * v).sum::<f32>() / FFT as f32)
        })
        .collect();
    let speech = speech_mask(&levels, 12.0);
    let win = hann();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT);
    let mut buf = vec![Complex::new(0.0f32, 0.0); FFT];
    let mut used = 0usize;
    for (f, s) in speech.iter().enumerate() {
        if !s {
            continue;
        }
        for (i, c) in buf.iter_mut().enumerate() {
            *c = Complex::new(mono.get(f * HOP + i).copied().unwrap_or(0.0) * win[i], 0.0);
        }
        fft.process(&mut buf);
        for (a, c) in acc.iter_mut().zip(&buf) {
            *a += f64::from(c.norm_sqr());
        }
        used += 1;
    }
    acc.iter().map(|a| (a / used.max(1) as f64) as f32).collect()
}

/// The gain per bin (linear) that turns the balance `from` into `to`: smoothed over a third of an
/// octave, only between 60 Hz and 12 kHz, loudness-neutral around the voice (200 Hz–4 kHz) and
/// limited to ±`max_db`.
pub fn match_curve(from: &[f32], to: &[f32], rate: f32, max_db: f32) -> Vec<f32> {
    let bins = FFT / 2 + 1;
    let hz = |k: usize| k as f32 * rate / FFT as f32;
    let max_db = max_db.clamp(0.0, 24.0);
    let diff: Vec<f32> = (0..bins)
        .map(|k| {
            let (a, b) = (from.get(k).copied().unwrap_or(0.0), to.get(k).copied().unwrap_or(0.0));
            if a > 1e-12 && b > 1e-12 { 10.0 * (b / a).log10() } else { 0.0 }
        })
        .collect();
    // Third-octave smoothing: the mean over [f / 2^(1/6), f · 2^(1/6)].
    let mut pre = vec![0.0f64; bins + 1];
    for k in 0..bins {
        pre[k + 1] = pre[k] + f64::from(diff[k]);
    }
    let band = 2f32.powf(1.0 / 6.0);
    let smooth = |k: usize| {
        let a = ((k as f32 / band).floor() as usize).min(bins - 1);
        let z = ((k as f32 * band).ceil() as usize + 1).clamp(a + 1, bins);
        ((pre[z] - pre[a]) / (z - a) as f64) as f32
    };
    let (lo, hi) = ((60.0 * FFT as f32 / rate) as usize, ((12_000.0 * FFT as f32 / rate) as usize).min(bins - 1));
    let mut db: Vec<f32> = (0..bins).map(|k| smooth(k.clamp(lo.max(1), hi.max(lo.max(1))))).collect();
    let voice: Vec<f32> = (0..bins).filter(|k| (200.0..=4000.0).contains(&hz(*k))).filter_map(|k| db.get(k).copied()).collect();
    let mean = if voice.is_empty() { 0.0 } else { voice.iter().sum::<f32>() / voice.len() as f32 };
    for d in &mut db {
        *d = (*d - mean).clamp(-max_db, max_db);
    }
    db.iter().map(|d| 10f32.powf(d / 20.0)).collect()
}

/// `ch` through a fixed EQ given as linear gain per bin (overlap-add, 75 % Hann).
pub fn apply_curve(ch: &[Vec<f32>], gains: &[f32]) -> Vec<Vec<f32>> {
    let win = hann();
    let mut planner = FftPlanner::<f32>::new();
    let (fwd, inv) = (planner.plan_fft_forward(FFT), planner.plan_fft_inverse(FFT));
    // Hann at 75 % overlap sums to 2.
    let scale = 1.0 / (FFT as f32 * 2.0);
    ch.iter()
        .map(|x| {
            let n = x.len();
            let mut out = vec![0.0f32; n];
            let mut buf = vec![Complex::new(0.0f32, 0.0); FFT];
            // Frames start FFT before the audio so its first samples get full overlap.
            let mut start: i64 = -(FFT as i64);
            while start < n as i64 {
                for (i, c) in buf.iter_mut().enumerate() {
                    let j = start + i as i64;
                    let v = usize::try_from(j).ok().and_then(|j| x.get(j)).copied().unwrap_or(0.0);
                    *c = Complex::new(v * win[i], 0.0);
                }
                fwd.process(&mut buf);
                for k in 0..=FFT / 2 {
                    let g = gains.get(k).copied().unwrap_or(1.0);
                    buf[k] *= g;
                    if k > 0 && k < FFT / 2 {
                        buf[FFT - k] *= g;
                    }
                }
                inv.process(&mut buf);
                for (i, c) in buf.iter().enumerate() {
                    if let Some(o) = usize::try_from(start + i as i64).ok().and_then(|j| out.get_mut(j)) {
                        *o += c.re * scale;
                    }
                }
                start += HOP as i64;
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// A "voice": a 150 Hz tone with harmonics, `amp`.
    fn voice(secs: f32, amp: f32) -> Vec<f32> {
        (0..(secs * SR) as usize)
            .map(|i| {
                let t = i as f32 / SR;
                amp * ((std::f32::consts::TAU * 150.0 * t).sin() + 0.5 * (std::f32::consts::TAU * 450.0 * t).sin())
            })
            .collect()
    }

    fn noise(secs: f32, amp: f32, seed: u32) -> Vec<f32> {
        let mut s = seed.max(1);
        (0..(secs * SR) as usize)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                amp * (s as f32 / u32::MAX as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    fn mix(a: &[f32], b: &[f32]) -> Vec<f32> {
        (0..a.len().max(b.len())).map(|i| a.get(i).unwrap_or(&0.0) + b.get(i).unwrap_or(&0.0)).collect()
    }

    #[test]
    fn crosstalk_closes_the_track_whose_speaker_is_silent() {
        // A speaks for 2 s, then B for 2 s; each mic hears the other 18 dB down.
        let bleed = 10f32.powf(-18.0 / 20.0);
        let a_voice = [voice(2.0, 0.3), vec![0.0; (2.0 * SR) as usize]].concat();
        let b_voice = [vec![0.0; (2.0 * SR) as usize], voice(2.0, 0.3)].concat();
        let room = noise(4.0, 0.001, 7);
        let mic_a: Vec<f32> = mix(&mix(&a_voice, &b_voice.iter().map(|v| v * bleed).collect::<Vec<_>>()), &room);
        let mic_b: Vec<f32> = mix(&mix(&b_voice, &a_voice.iter().map(|v| v * bleed).collect::<Vec<_>>()), &noise(4.0, 0.001, 9));
        let block = block_len(SR);
        let g = crosstalk_gains(&[levels_db(&[mic_a], block), levels_db(&[mic_b], block)], GateSettings::default());
        // Middle of A's part: A open, B closed; middle of B's part: the other way round.
        assert_eq!(g[0][100], 0.0);
        assert_eq!(g[1][100], -15.0);
        assert_eq!(g[0][300], -15.0);
        assert_eq!(g[1][300], 0.0);
        // Ramps, never jumps: neighbouring blocks differ by at most a quarter of the reduction.
        for t in &g {
            assert!(t.windows(2).all(|w| (w[0] - w[1]).abs() <= 15.0 / 4.0 + 1e-3));
        }
    }

    #[test]
    fn single_track_is_a_noise_gate() {
        let x = [voice(1.0, 0.2), noise(1.0, 0.002, 3), voice(1.0, 0.2)].concat();
        let g = crosstalk_gains(&[levels_db(&[x], block_len(SR))], GateSettings::default());
        assert_eq!(g[0][50], 0.0);
        assert_eq!(g[0][150], -15.0);
        assert_eq!(g[0][250], 0.0);
    }

    #[test]
    fn leveler_lifts_the_quiet_part_and_lowers_the_loud_one() {
        // Mostly normal speech, a quiet stretch and a loud one.
        let x = [voice(6.0, 0.05), voice(10.0, 0.15), voice(6.0, 0.6)].concat();
        let g = leveler_gains(&levels_db(&[x], block_len(SR)), 3.0, 9.0);
        assert!(g[300] > 4.0, "quiet part raised: {}", g[300]);
        assert!(g[1100].abs() < 1.0, "normal part kept: {}", g[1100]);
        assert!(g[1900] < -4.0, "loud part lowered: {}", g[1900]);
        assert!(g.iter().all(|v| v.abs() <= 9.0));
        // Gliding, no steps.
        assert!(g.windows(2).all(|w| (w[0] - w[1]).abs() < 0.5));
    }

    #[test]
    fn leveler_leaves_even_speech_alone() {
        let x = voice(10.0, 0.2);
        let g = leveler_gains(&levels_db(&[x], block_len(SR)), 3.0, 9.0);
        assert!(g.iter().all(|v| v.abs() < 0.5));
    }

    /// Noise around 1–3 kHz, like an inhale (three one-pole low-passes and a high-pass).
    fn breath(secs: f32, amp: f32) -> Vec<f32> {
        let a = 1.0 - (-std::f32::consts::TAU * 2000.0 / SR).exp();
        let r = (-std::f32::consts::TAU * 500.0 / SR).exp();
        let (mut l1, mut l2, mut l3, mut prev, mut hp) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let raw: Vec<f32> = noise(secs, 1.0, 21)
            .into_iter()
            .map(|v| {
                l1 += a * (v - l1);
                l2 += a * (l1 - l2);
                l3 += a * (l2 - l3);
                hp = r * (hp + l3 - prev);
                prev = l3;
                hp
            })
            .collect();
        let rms = (raw.iter().map(|v| v * v).sum::<f32>() / raw.len().max(1) as f32).sqrt().max(1e-9);
        raw.into_iter().map(|v| v / rms * amp).collect()
    }

    #[test]
    fn finds_the_breath_between_phrases() {
        // Speech, a pause, a breath, speech again.
        let room = || noise(0.3, 0.0003, 11);
        let x = [voice(2.0, 0.3), room(), breath(0.4, 0.01), voice(2.0, 0.3), room()].concat();
        let r = breath_ranges(&[x], SR, &[]);
        assert_eq!(r.len(), 1, "{r:?}");
        let (a, z) = r[0];
        assert!((225..=240).contains(&a) && (260..=275).contains(&z), "{r:?}");
        let g = dip_gains(500, &r, 12.0);
        assert_eq!(g[250], -12.0);
        assert_eq!(g[100], 0.0);
    }

    #[test]
    fn a_hiss_inside_speech_is_no_breath() {
        // An "s": high, noisy, quiet, but right between two words without a pause.
        let x = [voice(2.0, 0.3), noise(0.2, 0.02, 4), voice(2.0, 0.3), noise(0.5, 0.0003, 11)].concat();
        assert!(breath_ranges(&[x], SR, &[]).is_empty());
    }

    #[test]
    fn no_breaths_in_plain_speech() {
        assert!(breath_ranges(&[voice(5.0, 0.3)], SR, &[]).is_empty());
        assert!(breath_ranges(&[], SR, &[]).is_empty());
    }

    #[test]
    fn declick_removes_spikes_and_keeps_the_voice() {
        let mut x = voice(1.0, 0.2);
        let clean = x.clone();
        for at in [10_000usize, 20_000, 30_000] {
            x[at] += 0.6;
            x[at + 1] -= 0.5;
        }
        let (y, n) = declick(&[x], SR, 5.0);
        assert_eq!(n, 3);
        let err = |at: usize| (y[0][at] - clean[at]).abs();
        assert!(err(10_000) < 0.15 && err(20_001) < 0.15, "{} {}", err(10_000), err(20_001));
        // Far from the clicks nothing changed.
        assert_eq!(y[0][5_000], clean[5_000]);
        // Clean speech: nothing to repair.
        assert_eq!(declick(&[clean], SR, 5.0).1, 0);
    }

    #[test]
    fn voice_match_moves_the_balance_and_keeps_the_loudness() {
        // "Dull" mic: the 450 Hz partial is quieter than on the reference.
        let tone = |a1: f32, a3: f32| -> Vec<f32> {
            (0..(SR * 4.0) as usize)
                .map(|i| {
                    let t = i as f32 / SR;
                    a1 * (std::f32::consts::TAU * 150.0 * t).sin() + a3 * (std::f32::consts::TAU * 3000.0 * t).sin()
                })
                .collect()
        };
        let dull = tone(0.2, 0.02);
        let reference = tone(0.2, 0.08);
        let curve = match_curve(&speech_spectrum(std::slice::from_ref(&dull)), &speech_spectrum(&[reference]), SR, 12.0);
        let k3000 = (3000.0 * FFT as f32 / SR).round() as usize;
        let k150 = (150.0 * FFT as f32 / SR).round() as usize;
        assert!(curve[k3000] > curve[k150] * 2.0, "{} vs {}", curve[k3000], curve[k150]);
        let out = apply_curve(std::slice::from_ref(&dull), &curve);
        assert_eq!(out[0].len(), dull.len());
        // A flat curve passes the audio through unchanged.
        let flat = apply_curve(std::slice::from_ref(&dull), &vec![1.0; FFT / 2 + 1]);
        assert!(flat[0].iter().zip(&dull).skip(100).take(100_000).all(|(a, b)| (a - b).abs() < 1e-3));
    }

    #[test]
    fn hostile_input_does_not_panic() {
        assert!(levels_db(&[], 480).is_empty());
        assert!(crosstalk_gains(&[], GateSettings::default()).is_empty());
        assert!(leveler_gains(&[], 3.0, 6.0).is_empty());
        let _ = declick(&[vec![f32::NAN; 10_000]], f32::NAN, f32::NAN);
        let _ = apply_curve(&[vec![0.0; 10]], &[]);
        let _ = match_curve(&[], &[], 48_000.0, 6.0);
        let _ = speech_spectrum(&[vec![1.0; 5000]]);
    }
}
