//! Audiogram videos for social media: the episode cover, the title, a moving waveform and the
//! subtitle being spoken, over a stretch of the episode (the edit selection, else the first
//! minute). The frames are drawn here; ffmpeg only encodes them (H.264 + AAC in MP4), so no
//! ffmpeg text or subtitle filters are needed.

use soundcraft_engine::subtitles::FileCue;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, channel};

const FPS: u32 = 25;
/// Longest audiogram (seconds).
const MAX_SECONDS: f64 = 600.0;
const PAPER: [u8; 3] = [0xef, 0xe9, 0xdc];
const INK: [u8; 3] = [0x0b, 0x0b, 0x0c];
const GREEN: [u8; 3] = [0x2f, 0x9e, 0x63];

/// What the background thread needs, gathered on the UI thread.
pub struct Request {
    pub out: String,
    pub portrait: bool,
    pub title: String,
    pub cover: Option<String>,
    /// Mono mix at `rate`.
    pub audio: Vec<f32>,
    pub rate: u32,
    /// Cues relative to the start of `audio`.
    pub cues: Vec<FileCue>,
}

/// A render in progress.
pub struct Job {
    /// Done fraction × 1000.
    pub progress: Arc<AtomicU32>,
    pub done: Receiver<Result<String, String>>,
}

/// The stretch an audiogram covers: the edit selection, else the first minute (both capped).
pub fn range(s: &soundcraft_model::Session) -> soundcraft_time::Range {
    let sel = s.edit.selection;
    let max = s.sample_rate.samples(MAX_SECONDS);
    if sel.is_empty() {
        soundcraft_time::Range::new(0, s.content_end().min(s.sample_rate.samples(60.0)).max(1))
    } else {
        soundcraft_time::Range::new(sel.start, sel.end.min(sel.start + max))
    }
}

/// ffmpeg and the best H.264 encoder it has (libx264, then VideoToolbox, then MPEG-4 part 2).
fn ffmpeg() -> Option<(PathBuf, &'static str)> {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    let ff = dirs.iter().map(|d| d.join(name)).find(|p| p.is_file())?;
    let out = Command::new(&ff).args(["-hide_banner", "-encoders"]).stdin(Stdio::null()).output().ok()?;
    let list = String::from_utf8_lossy(&out.stdout);
    let codec = ["libx264", "h264_videotoolbox"].into_iter().find(|c| list.contains(c)).unwrap_or("mpeg4");
    Some((ff, codec))
}

/// Whether audiograms can be made here.
pub fn available() -> bool {
    ffmpeg().is_some()
}

/// A bold sans-serif from the system, else egui's built-in font.
fn font() -> Option<fontdue::Font> {
    let (bytes, index) = crate::fonts::bold_font_bytes()?;
    fontdue::Font::from_bytes(bytes, fontdue::FontSettings { collection_index: index, ..Default::default() }).ok()
}

pub fn start(req: Request) -> Result<Job, String> {
    let (ff, codec) = ffmpeg().ok_or("Audiograms need ffmpeg (macOS: brew install ffmpeg)")?;
    let font = font().ok_or("no font to draw the text with")?;
    let progress = Arc::new(AtomicU32::new(0));
    let (tx, rx) = channel();
    let p = Arc::clone(&progress);
    std::thread::Builder::new()
        .name("audiogram".into())
        .spawn(move || {
            let _ = tx.send(render(&ff, codec, &font, &req, &p));
        })
        .map_err(|e| e.to_string())?;
    Ok(Job { progress, done: rx })
}

fn render(ff: &std::path::Path, codec: &str, font: &fontdue::Font, req: &Request, progress: &AtomicU32) -> Result<String, String> {
    let (w, h) = if req.portrait { (1080usize, 1920usize) } else { (1080, 1080) };
    let dir = std::env::temp_dir().join(format!("talkcraft-audiogram-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let wav = dir.join("audio.wav");
    let buf = soundcraft_audio_io::AudioBuffer { sample_rate: req.rate, channels: vec![req.audio.clone()] };
    std::fs::write(&wav, soundcraft_audio_io::encode(&buf, &Default::default()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let cover_px = if req.portrait { 820 } else { 440 };
    let cover_y = if req.portrait { 220 } else { 60 };
    let mut c = Command::new(ff);
    c.args([
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-s",
        &format!("{w}x{h}"),
        "-r",
        &FPS.to_string(),
        "-i",
        "-",
    ])
    .arg("-i")
    .arg(&wav);
    let has_cover = req.cover.as_ref().is_some_and(|p| std::path::Path::new(p).is_file());
    if let (true, Some(cover)) = (has_cover, &req.cover) {
        c.args(["-loop", "1", "-i", cover]);
        c.args(["-filter_complex", &format!("[2:v]scale={cover_px}:{cover_px}[c];[0:v][c]overlay=(W-w)/2:{cover_y}:shortest=1[v]"), "-map", "[v]"]);
    } else {
        c.args(["-map", "0:v"]);
    }
    c.args(["-map", "1:a", "-c:v", codec, "-pix_fmt", "yuv420p"]);
    if codec == "libx264" {
        c.args(["-preset", "veryfast", "-crf", "20"]);
    } else {
        c.args(["-b:v", "6M"]);
    }
    c.args(["-c:a", "aac", "-b:a", "160k", "-shortest", "-movflags", "+faststart"]).arg(&req.out);
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("ffmpeg has no input")?;

    // 10 ms loudness steps for the waveform.
    let step = (req.rate / 100).max(1) as usize;
    let env: Vec<f32> = req.audio.chunks(step).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len().max(1) as f32).sqrt()).collect();
    let peak = env.iter().copied().fold(1e-6f32, f32::max);
    let secs = req.audio.len() as f64 / f64::from(req.rate.max(1));
    let frames = (secs * f64::from(FPS)).ceil() as usize;
    let mut img = vec![0u8; w * h * 3];
    let text_y = if has_cover { cover_y + cover_px + 40 } else { h / 4 };
    let wave_y = text_y + if req.portrait { 220 } else { 120 };
    let sub_y = wave_y + if req.portrait { 260 } else { 110 };
    let sub_px = 38.0;
    let sub_lines = if req.portrait { 4 } else { 3 };
    let mut write_err = None;
    for f in 0..frames {
        let t = f as f64 / f64::from(FPS);
        fill(&mut img, GREEN);
        let title = wrap(font, &req.title, 46.0, w - 160, 2);
        text(&mut img, w, font, &title, 46.0, 80, text_y, PAPER);
        // 48 bars: the last 2.4 s, the newest on the right.
        let bars = 48usize;
        let bar_w = (w - 160) / bars;
        let now = (t * 100.0) as i64;
        for b in 0..bars {
            let i = now - (bars - 1 - b) as i64 * 5;
            let v = usize::try_from(i).ok().and_then(|i| env.get(i)).copied().unwrap_or(0.0) / peak;
            let bh = ((v.sqrt() * 110.0) as usize).max(4);
            let x = 80 + b * bar_w;
            rect(&mut img, w, x + 2, wave_y.saturating_sub(bh / 2), bar_w.saturating_sub(4).max(2), bh, PAPER);
        }
        let ms = (t * 1000.0) as u64;
        if let Some(cue) = req.cues.iter().find(|c| c.start_ms <= ms && ms < c.end_ms) {
            let line = if cue.speaker.trim().is_empty() { cue.text.clone() } else { format!("{}: {}", cue.speaker.trim(), cue.text) };
            let lines = wrap(font, &line, sub_px, w - 180, sub_lines);
            let box_h = lines.len() * (sub_px * 1.25) as usize + 40;
            rect(&mut img, w, 60, sub_y, w - 120, box_h.min(h.saturating_sub(sub_y)), INK);
            text(&mut img, w, font, &lines, sub_px, 90, sub_y + 14, PAPER);
        }
        if let Err(e) = stdin.write_all(&img) {
            write_err = Some(e.to_string());
            break;
        }
        progress.store(((f + 1) * 1000 / frames.max(1)) as u32, Ordering::Relaxed);
    }
    drop(stdin);
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(crate::i18n::trf(
            "ffmpeg failed: {}",
            &[&(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(write_err.as_deref().unwrap_or("?")).trim())],
        ));
    }
    Ok(req.out.clone())
}

fn fill(img: &mut [u8], c: [u8; 3]) {
    for px in img.chunks_exact_mut(3) {
        px.copy_from_slice(&c);
    }
}

fn rect(img: &mut [u8], w: usize, x: usize, y: usize, rw: usize, rh: usize, c: [u8; 3]) {
    for yy in y..y + rh {
        for xx in x..(x + rw).min(w) {
            if let Some(px) = img.get_mut((yy * w + xx) * 3..(yy * w + xx) * 3 + 3) {
                px.copy_from_slice(&c);
            }
        }
    }
}

/// `s` word-wrapped to `max_w` pixels, at most `max_lines` lines (the last one ends in "…" when
/// cut).
fn wrap(font: &fontdue::Font, s: &str, px: f32, max_w: usize, max_lines: usize) -> Vec<String> {
    let width = |t: &str| t.chars().map(|ch| font.metrics(ch, px).advance_width).sum::<f32>();
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if width(&next) > max_w as f32 && !cur.is_empty() {
            lines.push(std::mem::replace(&mut cur, word.to_string()));
        } else {
            cur = next;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            last.push('…');
        }
    }
    lines
}

/// `lines` drawn at (`x`, `y`) in `c`.
#[allow(clippy::too_many_arguments)]
fn text(img: &mut [u8], w: usize, font: &fontdue::Font, lines: &[String], px: f32, x: usize, y: usize, c: [u8; 3]) {
    let line_h = (px * 1.25) as usize;
    for (n, line) in lines.iter().enumerate() {
        let mut pen = x as f32;
        let base = y + n * line_h + px as usize;
        for ch in line.chars() {
            let (m, bitmap) = font.rasterize(ch, px);
            let gx = pen as i64 + i64::from(m.xmin);
            let gy = base as i64 - m.height as i64 - i64::from(m.ymin);
            for row in 0..m.height {
                for col in 0..m.width {
                    let a = f32::from(bitmap.get(row * m.width + col).copied().unwrap_or(0)) / 255.0;
                    let (xx, yy) = (gx + col as i64, gy + row as i64);
                    if a <= 0.0 || xx < 0 || yy < 0 || xx as usize >= w {
                        continue;
                    }
                    let i = (yy as usize * w + xx as usize) * 3;
                    if let Some(p) = img.get_mut(i..i + 3) {
                        for k in 0..3 {
                            p[k] = (f32::from(p[k]) * (1.0 - a) + f32::from(c[k]) * a) as u8;
                        }
                    }
                }
            }
            pen += m.advance_width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_an_mp4_with_audio_and_video() {
        if !available() || font().is_none() {
            eprintln!("skipping: no ffmpeg or font");
            return;
        }
        let dir = std::env::temp_dir().join(format!("talkcraft-audiogram-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("a.mp4").to_string_lossy().to_string();
        let rate = 48_000u32;
        let audio: Vec<f32> = (0..rate as usize * 2).map(|i| (i as f32 * 0.03).sin() * 0.3).collect();
        let cues = vec![FileCue { start_ms: 200, end_ms: 1800, speaker: "Dirk".into(), text: "Hallo und willkommen zu Folge zwölf".into() }];
        let req = Request { out: out.clone(), portrait: false, title: "Europa 2.0 · Folge 12".into(), cover: None, audio, rate, cues };
        let job = start(req).unwrap();
        let res = job.done.recv().unwrap();
        assert_eq!(res.as_deref(), Ok(out.as_str()));
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp");
        assert_eq!(job.progress.load(Ordering::Relaxed), 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
