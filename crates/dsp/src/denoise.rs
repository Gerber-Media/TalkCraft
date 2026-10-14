//! Offline noise reduction by spectral gating: the noise floor is learned per frequency from the
//! quietest moments of the audio (the room between sentences), then every frequency that does
//! not rise clearly above it is turned down by up to `reduction_db`. Gains are smoothed over time
//! and frequency so the result does not warble ("musical noise").

use rustfft::{FftPlanner, num_complex::Complex};

const FFT: usize = 2048;
const HOP: usize = FFT / 4;

/// How far above the learned floor a bin must rise to pass untouched (dB).
const MARGIN_DB: f32 = 6.0;

/// Periodic Hann window (sums to a constant at 75 % overlap when used on both sides, scaled).
fn hann() -> Vec<f32> {
    (0..FFT).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT as f32).cos()).collect()
}

/// `ch` with its steady background noise lowered by up to `reduction_db` (0 = unchanged).
pub fn denoise(ch: &[Vec<f32>], reduction_db: f32) -> Vec<Vec<f32>> {
    let reduction_db = reduction_db.clamp(0.0, 40.0);
    if reduction_db <= 0.0 {
        return ch.to_vec();
    }
    ch.iter().map(|c| denoise_channel(c, reduction_db)).collect()
}

/// One windowed frame of `x` as if it were padded with `FFT` zeros in front (and zeros after),
/// so the first and last samples are covered by full windows.
fn frame(x: &[f32], win: &[f32], f: usize, buf: &mut [Complex<f32>]) {
    let start = f * HOP;
    for (i, c) in buf.iter_mut().enumerate() {
        let j = start + i;
        let v = if j < FFT { 0.0 } else { x.get(j - FFT).copied().unwrap_or(0.0) };
        *c = Complex::new(v * win.get(i).copied().unwrap_or(0.0), 0.0);
    }
}

/// Two passes over the audio that recompute each frame's spectrum instead of keeping them all:
/// memory stays at the input and output (an hour of mono would otherwise hold about 5.5 GB of
/// spectra). The result is the same, sample for sample.
fn denoise_channel(x: &[f32], reduction_db: f32) -> Vec<f32> {
    if x.len() < FFT {
        return x.to_vec();
    }
    let win = hann();
    let mut planner = FftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(FFT);
    let inv = planner.plan_fft_inverse(FFT);
    let bins = FFT / 2 + 1;
    // The padded length (FFT zeros, the audio, FFT + HOP zeros) and its frames.
    let padded_len = FFT + x.len() + FFT + HOP;
    let frames = (padded_len - FFT) / HOP + 1;
    let mut buf = vec![Complex::new(0.0f32, 0.0); FFT];

    // Pass 1: each frame's energy.
    let mut energy: Vec<(f32, usize)> = Vec::with_capacity(frames);
    for f in 0..frames {
        frame(x, &win, f, &mut buf);
        fwd.process(&mut buf);
        let e: f32 = buf.iter().take(bins).map(|c| c.norm_sqr()).sum();
        energy.push((e, f));
    }
    // Noise floor: the mean magnitude per bin over the quietest 10 % of the frames that carry
    // any signal (digital silence from padding would make the floor zero).
    let mut quiet: Vec<(f32, usize)> = energy.iter().copied().filter(|(e, _)| *e > 1e-12).collect();
    drop(energy);
    quiet.sort_by(|a, b| a.0.total_cmp(&b.0));
    let take = (quiet.len() / 10).max(1).min(quiet.len());
    let mut floor = vec![0.0f32; bins];
    for (_, f) in quiet.iter().take(take) {
        frame(x, &win, *f, &mut buf);
        fwd.process(&mut buf);
        for (k, v) in floor.iter_mut().enumerate() {
            *v += buf.get(k).map_or(0.0, |c| c.norm()) / take as f32;
        }
    }
    let threshold: Vec<f32> = floor.iter().map(|v| v * 10f32.powf(MARGIN_DB / 20.0)).collect();
    let min_gain = 10f32.powf(-reduction_db / 20.0);
    // Every output sample lies under four windows; their squared sum, added in frame order.
    let norm: Vec<f32> = (0..HOP)
        .map(|r| {
            let mut n = 0.0f32;
            for k in (0..FFT / HOP).rev() {
                let w = win.get(r + k * HOP).copied().unwrap_or(0.0);
                n += w * w;
            }
            n
        })
        .collect();

    // Pass 2: gains (smoothed across frequency, then with fast attack / slow release over time),
    // inverse FFT, overlap-add straight into the output.
    let mut out = vec![0.0f32; x.len()];
    let mut prev = vec![1.0f32; bins];
    let mut raw = vec![0.0f32; bins];
    for f in 0..frames {
        frame(x, &win, f, &mut buf);
        fwd.process(&mut buf);
        for (k, g) in raw.iter_mut().enumerate() {
            let m = buf.get(k).map_or(0.0, |c| c.norm());
            let t = threshold.get(k).copied().unwrap_or(0.0);
            *g = if m <= 0.0 || t <= 0.0 {
                1.0
            } else {
                // Soft knee: full gain at twice the threshold, min_gain at or below it.
                ((m / t - 1.0).clamp(0.0, 1.0) * (1.0 - min_gain) + min_gain).clamp(min_gain, 1.0)
            };
        }
        for k in 0..bins {
            let lo = k.saturating_sub(2);
            let hi = (k + 2).min(bins - 1);
            let avg = raw.get(lo..=hi).map_or(1.0, |w| w.iter().sum::<f32>() / w.len() as f32);
            let p = prev.get(k).copied().unwrap_or(1.0);
            // Open at once for speech onsets, close over a few frames.
            let g = if avg > p { avg } else { p + (avg - p) * 0.35 };
            if let Some(slot) = prev.get_mut(k) {
                *slot = g;
            }
            if let Some(c) = buf.get_mut(k) {
                *c *= g;
            }
            if k > 0
                && k < FFT / 2
                && let Some(mirror) = buf.get_mut(FFT - k)
            {
                *mirror *= g;
            }
        }
        inv.process(&mut buf);
        let start = f * HOP;
        for i in 0..FFT {
            let j = start + i;
            if j < FFT {
                continue;
            }
            let w = win.get(i).copied().unwrap_or(0.0);
            if let (Some(o), Some(c)) = (out.get_mut(j - FFT), buf.get(i)) {
                *o += c.re / FFT as f32 * w;
            }
        }
    }
    for (i, o) in out.iter_mut().enumerate() {
        let n = norm.get((i + FFT) % HOP).copied().unwrap_or(0.0);
        *o = if n > 1e-6 { *o / n } else { 0.0 };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_db(x: &[f32]) -> f32 {
        let e = x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32;
        10.0 * e.max(1e-20).log10()
    }

    /// The earlier implementation, which kept every spectrum: the new one must match it exactly.
    fn reference(x: &[f32], reduction_db: f32) -> Vec<f32> {
        if x.len() < FFT {
            return x.to_vec();
        }
        let win = hann();
        let mut planner = FftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(FFT);
        let inv = planner.plan_fft_inverse(FFT);
        let bins = FFT / 2 + 1;
        // Pad so the first and last samples are covered by full windows.
        let mut padded = vec![0.0f32; FFT];
        padded.extend_from_slice(x);
        padded.extend(std::iter::repeat_n(0.0, FFT + HOP));
        let frames = (padded.len() - FFT) / HOP + 1;

        // Pass 1: magnitudes per frame, and each frame's energy.
        let mut spectra: Vec<Vec<Complex<f32>>> = Vec::with_capacity(frames);
        let mut energy: Vec<(f32, usize)> = Vec::with_capacity(frames);
        for f in 0..frames {
            let start = f * HOP;
            let mut buf: Vec<Complex<f32>> =
                (0..FFT).map(|i| Complex::new(padded.get(start + i).copied().unwrap_or(0.0) * win.get(i).copied().unwrap_or(0.0), 0.0)).collect();
            fwd.process(&mut buf);
            let e: f32 = buf.iter().take(bins).map(|c| c.norm_sqr()).sum();
            energy.push((e, f));
            spectra.push(buf);
        }
        // Noise floor: the mean magnitude per bin over the quietest 10 % of the frames that carry
        // any signal (digital silence from padding would make the floor zero).
        let mut quiet: Vec<(f32, usize)> = energy.iter().copied().filter(|(e, _)| *e > 1e-12).collect();
        quiet.sort_by(|a, b| a.0.total_cmp(&b.0));
        let take = (quiet.len() / 10).max(1).min(quiet.len());
        let mut floor = vec![0.0f32; bins];
        for (_, f) in quiet.iter().take(take) {
            if let Some(sp) = spectra.get(*f) {
                for (k, v) in floor.iter_mut().enumerate() {
                    *v += sp.get(k).map_or(0.0, |c| c.norm()) / take as f32;
                }
            }
        }
        let threshold: Vec<f32> = floor.iter().map(|v| v * 10f32.powf(MARGIN_DB / 20.0)).collect();
        let min_gain = 10f32.powf(-reduction_db / 20.0);

        // Pass 2: gains (smoothed across frequency, then with fast attack / slow release over time),
        // inverse FFT, overlap-add.
        let mut out = vec![0.0f32; padded.len()];
        let mut norm = vec![0.0f32; padded.len()];
        let mut prev = vec![1.0f32; bins];
        for (f, sp) in spectra.iter_mut().enumerate() {
            let raw: Vec<f32> = (0..bins)
                .map(|k| {
                    let m = sp.get(k).map_or(0.0, |c| c.norm());
                    let t = threshold.get(k).copied().unwrap_or(0.0);
                    if m <= 0.0 || t <= 0.0 {
                        1.0
                    } else {
                        // Soft knee: full gain at twice the threshold, min_gain at or below it.
                        ((m / t - 1.0).clamp(0.0, 1.0) * (1.0 - min_gain) + min_gain).clamp(min_gain, 1.0)
                    }
                })
                .collect();
            for k in 0..bins {
                let lo = k.saturating_sub(2);
                let hi = (k + 2).min(bins - 1);
                let avg = raw.get(lo..=hi).map_or(1.0, |w| w.iter().sum::<f32>() / w.len() as f32);
                let p = prev.get(k).copied().unwrap_or(1.0);
                // Open at once for speech onsets, close over a few frames.
                let g = if avg > p { avg } else { p + (avg - p) * 0.35 };
                if let Some(slot) = prev.get_mut(k) {
                    *slot = g;
                }
                if let Some(c) = sp.get_mut(k) {
                    *c *= g;
                }
                if k > 0
                    && k < FFT / 2
                    && let Some(mirror) = sp.get_mut(FFT - k)
                {
                    *mirror *= g;
                }
            }
            inv.process(sp);
            let start = f * HOP;
            for i in 0..FFT {
                let w = win.get(i).copied().unwrap_or(0.0);
                if let (Some(o), Some(c)) = (out.get_mut(start + i), sp.get(i)) {
                    *o += c.re / FFT as f32 * w;
                }
                if let Some(n) = norm.get_mut(start + i) {
                    *n += w * w;
                }
            }
        }
        (0..x.len())
            .map(|i| {
                let j = i + FFT;
                let n = norm.get(j).copied().unwrap_or(0.0);
                if n > 1e-6 { out.get(j).copied().unwrap_or(0.0) / n } else { 0.0 }
            })
            .collect()
    }

    #[test]
    fn two_passes_give_the_same_result_as_keeping_every_spectrum() {
        let mut seed = 9u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for len in [2_048usize, 2_049, 30_001, 96_000] {
            let x: Vec<f32> = (0..len).map(|i| rnd() * 0.02 + if (i / 9_000) % 2 == 1 { 0.3 * (i as f32 * 0.03).sin() } else { 0.0 }).collect();
            let a = denoise_channel(&x, 18.0);
            let b = reference(&x, 18.0);
            assert_eq!(a.len(), b.len());
            assert!(a.iter().zip(&b).all(|(p, q)| p.to_bits() == q.to_bits()), "len {len}: results differ");
        }
    }

    #[test]
    fn hiss_between_words_drops_and_the_words_stay() {
        let sr = 48_000usize;
        let mut seed = 3u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.02
        };
        // 1 s hiss, 1 s "voice" (two tones) over hiss, 1 s hiss.
        let x: Vec<f32> = (0..sr * 3)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let voice = if (sr..2 * sr).contains(&i) {
                    0.3 * (t * 220.0 * std::f32::consts::TAU).sin() + 0.15 * (t * 660.0 * std::f32::consts::TAU).sin()
                } else {
                    0.0
                };
                voice + noise()
            })
            .collect();
        let y = denoise(std::slice::from_ref(&x), 18.0).remove(0);
        assert_eq!(y.len(), x.len());
        let quiet = |v: &[f32]| rms_db(&v[sr / 4..3 * sr / 4]);
        let words = |v: &[f32]| rms_db(&v[sr + sr / 4..2 * sr - sr / 4]);
        assert!(quiet(&x) - quiet(&y) > 12.0, "hiss lowered: {} -> {}", quiet(&x), quiet(&y));
        assert!((words(&x) - words(&y)).abs() < 1.0, "voice kept: {} -> {}", words(&x), words(&y));
        assert_eq!(denoise(std::slice::from_ref(&x), 0.0)[0], x, "0 dB leaves the audio alone");
    }
}
