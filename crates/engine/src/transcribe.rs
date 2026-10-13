//! Automatic transcription with whisper.cpp. The mix is written as a 16 kHz mono WAV, the
//! external `whisper-cli` program turns it into an `.srt`, and the cues are placed on the
//! timeline. Nothing of whisper.cpp is linked; it is found on the PATH or in Homebrew's prefix,
//! the speech model (`ggml-*.bin`) in the models folder of the app's settings.

use crate::subtitles::{self as subs, FileCue};
use crate::{Engine, EngineError, Result};
use soundcraft_audio_io::{AudioBuffer, BitDepth, EncodeOptions, FileFormat};
use soundcraft_model::{SubtitleCue, TrackKind};
use soundcraft_time::Range;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// The sample rate whisper expects.
const WHISPER_HZ: u32 = 16_000;
/// Longest cue whisper writes (characters); split on words. Two subtitle lines of ~42.
const MAX_CUE_CHARS: u32 = 84;
/// Tracks quieter than this in a cue never become its speaker.
const SPEAKER_FLOOR_DB: f32 = -55.0;
/// Speech models, best first (the first one found is used).
pub const MODEL_PREFERENCE: [&str; 8] = [
    "ggml-large-v3-turbo.bin",
    "ggml-large-v3-turbo-q8_0.bin",
    "ggml-large-v3-turbo-q5_0.bin",
    "ggml-large-v3.bin",
    "ggml-medium.bin",
    "ggml-small.bin",
    "ggml-base.bin",
    "ggml-tiny.bin",
];
/// The model the app offers to download: good German, fast on Apple Silicon (~550 MB).
pub const DEFAULT_MODEL: &str = "ggml-large-v3-turbo-q5_0.bin";
pub const DEFAULT_MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin";

/// whisper's language code for an ISO 639-2 code (`deu` → `de`); `auto` when unknown.
pub fn whisper_language(code: &str) -> &'static str {
    match code.trim().to_ascii_lowercase().as_str() {
        "deu" | "ger" | "de" => "de",
        "eng" | "en" => "en",
        "dan" | "da" => "da",
        "fra" | "fre" | "fr" => "fr",
        "spa" | "es" => "es",
        "ita" | "it" => "it",
        "nld" | "dut" | "nl" => "nl",
        "swe" | "sv" => "sv",
        "nor" | "nob" | "no" => "no",
        "pol" | "pl" => "pl",
        "por" | "pt" => "pt",
        _ => "auto",
    }
}

/// `whisper-cli` on the PATH, else in Homebrew's usual prefixes.
pub fn find_cli() -> Option<PathBuf> {
    let names = ["whisper-cli", "whisper-cpp"];
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // An app started from the Finder does not get the shell's PATH.
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/home/linuxbrew/.linuxbrew/bin"].map(PathBuf::from));
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// Where speech models are looked for: `$PODCAST_STUDIO_MODELS`, then `models` in the app's
/// settings folder.
pub fn model_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let mut out: Vec<PathBuf> = env("PODCAST_STUDIO_MODELS").into_iter().collect();
    let settings = if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support/Podcast Studio"))
    } else if cfg!(windows) {
        env("APPDATA").map(|a| a.join("Podcast Studio"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config"))).map(|c| c.join("podcast-studio"))
    };
    out.extend(settings.map(|s| s.join("models")));
    out
}

/// The folder a downloaded model goes to.
pub fn default_model_dir() -> Option<PathBuf> {
    model_dirs().into_iter().last()
}

/// `explicit` when it is a file, else the best model found in [`model_dirs`].
pub fn find_model(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit.filter(|p| !p.trim().is_empty()) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let dirs = model_dirs();
    let preferred = MODEL_PREFERENCE.iter().flat_map(|m| dirs.iter().map(move |d| d.join(m))).find(|p| p.is_file());
    preferred.or_else(|| {
        // Any other ggml model someone put there.
        dirs.iter()
            .filter_map(|d| std::fs::read_dir(d).ok())
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("ggml-") && n.ends_with(".bin")))
    })
}

/// What is missing for transcription, as a message for the user; `None` when ready.
pub fn missing(model: Option<&str>) -> Option<String> {
    if find_cli().is_none() {
        return Some("whisper.cpp is not installed (macOS: `brew install whisper-cpp`)".into());
    }
    if find_model(model).is_none() {
        let dir = default_model_dir().map(|d| d.display().to_string()).unwrap_or_else(|| "the models folder".into());
        return Some(format!("no speech model found — download one (e.g. {DEFAULT_MODEL}) into {dir}"));
    }
    None
}

/// A transcription in progress: the WAV whisper reads, the files it writes, its log.
#[derive(Debug, Clone)]
pub struct Job {
    pub dir: PathBuf,
    pub cli: PathBuf,
    pub model: PathBuf,
    pub language: String,
    /// The timeline range the WAV covers; cue times are relative to its start.
    pub range: Range,
    /// Length of the audio (seconds), for progress.
    pub seconds: f64,
}

impl Job {
    fn wav(&self) -> PathBuf {
        self.dir.join("mix.wav")
    }

    /// The `.srt` whisper writes.
    pub fn srt(&self) -> PathBuf {
        self.dir.join("mix.srt")
    }

    /// whisper's progress messages and errors.
    pub fn log(&self) -> PathBuf {
        self.dir.join("whisper.log")
    }

    /// Starts `whisper-cli` in the background; stdout and stderr go to [`Job::log`].
    pub fn spawn(&self) -> Result<Child> {
        let log = std::fs::File::create(self.log()).map_err(|err| EngineError::Io(format!("{}: {err}", self.log().display())))?;
        let err_log = log.try_clone().map_err(|err| EngineError::Io(err.to_string()))?;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().clamp(1, 8));
        Command::new(&self.cli)
            .arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(self.wav())
            .args(["-l", &self.language, "-t", &threads.to_string(), "-ml", &MAX_CUE_CHARS.to_string(), "-sow", "-osrt", "-pp", "-of"])
            .arg(self.dir.join("mix"))
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(err_log)
            .spawn()
            .map_err(|err| EngineError::Io(format!("{}: {err}", self.cli.display())))
    }

    /// Progress 0…1 from whisper's log (`progress = 42%`).
    pub fn progress(&self) -> f32 {
        let text = std::fs::read_to_string(self.log()).unwrap_or_default();
        text.rmatch_indices("progress =")
            .next()
            .and_then(|(i, _)| text.get(i + 10..))
            .and_then(|rest| rest.trim_start().split('%').next())
            .and_then(|n| n.trim().parse::<f32>().ok())
            .map_or(0.0, |p| (p / 100.0).clamp(0.0, 1.0))
    }

    /// The cues whisper wrote, after it exited with `ok`.
    pub fn result(&self, ok: bool) -> Result<Vec<FileCue>> {
        let srt = std::fs::read(self.srt());
        match srt {
            Ok(bytes) if ok => Ok(subs::parse(&String::from_utf8_lossy(&bytes))),
            _ => {
                let log = std::fs::read_to_string(self.log()).unwrap_or_default();
                let last = log.lines().rev().find(|l| l.contains("error") || l.contains("failed")).unwrap_or("whisper-cli failed");
                Err(EngineError::Io(format!("transcription failed: {}", last.trim())))
            }
        }
    }

    /// Deletes the temporary folder.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Runs whisper and waits (for scripts and the CLI; the app polls a spawned child instead).
    pub fn run_blocking(&self) -> Result<Vec<FileCue>> {
        let status = self.spawn()?.wait().map_err(|err| EngineError::Io(err.to_string()))?;
        self.result(status.success())
    }
}

/// Renders `r` of the mix to a 16 kHz mono WAV in a fresh temporary folder.
pub fn prepare(e: &Engine, r: Range, language: &str, model: Option<&str>, id: &str) -> Result<Job> {
    if let Some(why) = missing(model) {
        return Err(EngineError::Disabled(id.into(), why));
    }
    let (Some(cli), Some(model)) = (find_cli(), find_model(model)) else {
        return Err(EngineError::Disabled(id.into(), "whisper.cpp is not available".into()));
    };
    let ch = crate::io::render_mix(e, r, id)?;
    let frames = ch.iter().map(Vec::len).min().unwrap_or(0);
    if frames == 0 {
        return Err(EngineError::BadParams(id.into(), "there is no audio to transcribe".into()));
    }
    let n = ch.len().max(1) as f32;
    let mono: Vec<f32> = (0..frames).map(|i| ch.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / n).collect();
    let from = e.session().sample_rate.hz();
    let mono = if from == WHISPER_HZ { vec![mono] } else { soundcraft_dsp::offline::resample(&[mono], from, WHISPER_HZ) };
    let seconds = mono.first().map_or(0, Vec::len) as f64 / f64::from(WHISPER_HZ);
    let buf = AudioBuffer { sample_rate: WHISPER_HZ, channels: mono };
    let opts = EncodeOptions { format: FileFormat::Wav, bit_depth: BitDepth::Int16, dither: false, bwf: None };
    let bytes = soundcraft_audio_io::encode(&buf, &opts).map_err(|err| EngineError::Io(err.to_string()))?;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("podcast-studio-whisper-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|err| EngineError::Io(format!("{}: {err}", dir.display())))?;
    let job = Job { dir, cli, model, language: language.to_string(), range: r, seconds };
    std::fs::write(job.wav(), bytes).map_err(|err| EngineError::Io(format!("{}: {err}", job.wav().display())))?;
    Ok(job)
}

/// Mean level (dBFS) of planar `ch` over frames `a..b`.
fn level_db(ch: &[Vec<f32>], a: usize, b: usize) -> f32 {
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for c in ch {
        for x in c.get(a..b.min(c.len())).unwrap_or(&[]) {
            sum += f64::from(*x) * f64::from(*x);
            n += 1;
        }
    }
    if n == 0 || sum <= 0.0 { -120.0 } else { (10.0 * (sum / n as f64).log10()) as f32 }
}

/// Names the speaker of each cue overlapping `r` after the audio track loudest under it (each
/// track soloed). Cues where no track is above the floor, or with a speaker already set unless
/// `overwrite`, are left alone. Returns how many cues got a speaker.
pub fn assign_speakers(e: &mut Engine, r: Range, overwrite: bool, id: &str) -> Result<usize> {
    let tracks: Vec<(soundcraft_model::TrackId, String)> =
        e.session().tracks.iter().filter(|t| t.kind == TrackKind::Audio && !t.inactive && !t.mixer.mute).map(|t| (t.id, t.name.clone())).collect();
    if tracks.len() < 2 {
        return Ok(0);
    }
    let rendered: Vec<(String, Vec<Vec<f32>>)> =
        tracks.into_iter().map(|(t, name)| crate::io::render_track(e, r, t, id).map(|ch| (name, ch))).collect::<Result<_>>()?;
    let mut names: Vec<(u64, String)> = Vec::new();
    for c in &e.session().subtitles {
        if c.end <= r.start || c.start >= r.end || (!overwrite && !c.speaker.trim().is_empty()) {
            continue;
        }
        let a = (c.start.max(r.start) - r.start).max(0) as usize;
        let b = (c.end.min(r.end) - r.start).max(0) as usize;
        let best =
            rendered.iter().map(|(name, ch)| (name, level_db(ch, a, b))).filter(|(_, db)| *db > SPEAKER_FLOOR_DB).max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((name, _)) = best {
            names.push((c.id, name.clone()));
        }
    }
    let n = names.len();
    if n > 0 {
        let s = e.session_mut();
        for (id, name) in names {
            if let Some(c) = s.subtitles.iter_mut().find(|c| c.id == id) {
                c.speaker = name;
            }
        }
    }
    Ok(n)
}

/// Places whisper's cues (relative to `r.start`) on the timeline, replacing the cues inside `r`
/// when `replace`. Empty cues and whisper's `[Musik]`-style annotations are dropped.
pub fn place(e: &mut Engine, r: Range, cues: Vec<FileCue>, replace: bool) -> usize {
    let sr = e.session().sample_rate;
    let s = e.session_mut();
    if replace {
        s.subtitles.retain(|c| c.end <= r.start || c.start >= r.end);
    }
    let mut n = 0;
    for c in cues {
        let text = c.text.trim();
        if text.is_empty() || (text.starts_with('[') && text.ends_with(']')) || (text.starts_with('(') && text.ends_with(')')) {
            continue;
        }
        let start = r.start.saturating_add(sr.samples(c.start_ms as f64 / 1000.0));
        let end = r.start.saturating_add(sr.samples(c.end_ms as f64 / 1000.0)).min(r.end);
        if end <= start {
            continue;
        }
        let id = subs::next_id(s);
        s.subtitles.push(SubtitleCue { id, start, end, speaker: c.speaker, text: text.to_string() });
        n += 1;
    }
    subs::sort(s);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_map_to_whisper() {
        assert_eq!(whisper_language("deu"), "de");
        assert_eq!(whisper_language(" ENG "), "en");
        assert_eq!(whisper_language("dan"), "da");
        assert_eq!(whisper_language("xyz"), "auto");
    }

    #[test]
    fn progress_is_read_from_the_log() {
        let dir = std::env::temp_dir().join(format!("podcast-studio-whisper-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let job = Job { dir: dir.clone(), cli: PathBuf::new(), model: PathBuf::new(), language: "de".into(), range: Range::new(0, 1), seconds: 1.0 };
        assert_eq!(job.progress(), 0.0);
        std::fs::write(job.log(), "whisper_print_progress_callback: progress =  10%\nwhisper_print_progress_callback: progress =  55%\n").unwrap();
        assert!((job.progress() - 0.55).abs() < 1e-6);
        assert!(job.result(false).is_err());
        std::fs::write(job.srt(), "1\n00:00:00,000 --> 00:00:01,500\n Hallo zusammen.\n\n").unwrap();
        let cues = job.result(true).unwrap();
        assert_eq!(cues.len(), 1);
        job.cleanup();
        assert!(!dir.exists());
    }

    #[test]
    fn placing_drops_annotations_and_replaces_the_range() {
        let mut e = Engine::default();
        e.execute("subtitle.add", &serde_json::json!({"start": 0, "end": 100, "text": "alt"})).unwrap();
        e.execute("subtitle.add", &serde_json::json!({"start": 960_000, "end": 970_000, "text": "bleibt"})).unwrap();
        let cue = |a, b, t: &str| FileCue { start_ms: a, end_ms: b, speaker: String::new(), text: t.into() };
        let r = Range::new(0, 480_000);
        let n = place(&mut e, r, vec![cue(0, 1000, " Hallo."), cue(1000, 2000, "[Musik]"), cue(2000, 2000, "leer"), cue(3000, 4000, "  ")], true);
        assert_eq!(n, 1);
        let texts: Vec<&str> = e.session().subtitles.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["Hallo.", "bleibt"]);
    }
}
