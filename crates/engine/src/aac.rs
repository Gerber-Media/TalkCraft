//! AAC in MP4 (`.m4a` for podcasts, `.m4b` for audiobooks) through an encoder already on the
//! machine: `ffmpeg` when installed (it also writes QuickTime chapters, which Apple's apps read),
//! else macOS's built-in `afconvert`. Nothing of either is linked. After `afconvert`, the tags, the
//! cover and Nero chapters (`chpl`) are written into the file here.

use soundcraft_audio_io::mp3::{Chapter, EpisodeTags};
use soundcraft_audio_io::{AudioBuffer, BitDepth, EncodeOptions, FileFormat};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// An AAC encoder found on this machine.
#[derive(Debug, Clone, PartialEq)]
pub enum Encoder {
    Ffmpeg(PathBuf),
    Afconvert(PathBuf),
}

fn on_path(names: &[&str], extra: &[&str]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // An app started from the Finder or the Start menu does not get the shell's PATH.
    dirs.extend(extra.iter().map(PathBuf::from));
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// ffmpeg first (chapters Apple reads), then afconvert (always there on macOS).
pub fn find_encoder() -> Option<Encoder> {
    let ffmpeg = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Some(p) = on_path(&[ffmpeg], &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]) {
        return Some(Encoder::Ffmpeg(p));
    }
    if cfg!(target_os = "macos") {
        return on_path(&["afconvert"], &["/usr/bin"]).map(Encoder::Afconvert);
    }
    None
}

/// What to install when there is no encoder.
pub fn install_hint() -> &'static str {
    if cfg!(windows) {
        "M4A needs ffmpeg: install it (e.g. `winget install ffmpeg`) or export MP3"
    } else {
        "M4A needs ffmpeg: install it with your package manager (e.g. `sudo apt install ffmpeg`) or export MP3"
    }
}

fn temp_dir() -> Result<PathBuf, String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("talkcraft-aac-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("{what}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{what} failed: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no message").trim()))
    }
}

/// Encodes `buf` as AAC at `bitrate_kbps` into an `.m4a` (or, with `audiobook`, `.m4b`) with
/// `tags`, cover and chapters.
pub fn encode(buf: &AudioBuffer, bitrate_kbps: u32, tags: &EpisodeTags, audiobook: bool) -> Result<Vec<u8>, String> {
    let enc = find_encoder().ok_or_else(|| install_hint().to_string())?;
    let dir = temp_dir()?;
    let result = encode_in(&dir, &enc, buf, bitrate_kbps, tags, audiobook);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn encode_in(dir: &Path, enc: &Encoder, buf: &AudioBuffer, kbps: u32, tags: &EpisodeTags, audiobook: bool) -> Result<Vec<u8>, String> {
    let wav = dir.join("in.wav");
    let opts = EncodeOptions { format: FileFormat::Wav, bit_depth: BitDepth::Int24, dither: true, bwf: None };
    std::fs::write(&wav, soundcraft_audio_io::encode(buf, &opts).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let out = dir.join(if audiobook { "out.m4b" } else { "out.m4a" });
    match enc {
        Encoder::Ffmpeg(ff) => {
            let meta = dir.join("meta.txt");
            std::fs::write(&meta, ffmetadata(tags)).map_err(|e| e.to_string())?;
            let mut c = Command::new(ff);
            c.args(["-y", "-hide_banner", "-loglevel", "error", "-i"]).arg(&wav).arg("-i").arg(&meta);
            let mut cover_input = false;
            if let Some((mime, data)) = &tags.cover {
                let ext = if mime.contains("png") { "png" } else { "jpg" };
                let path = dir.join(format!("cover.{ext}"));
                std::fs::write(&path, data).map_err(|e| e.to_string())?;
                c.arg("-i").arg(&path);
                cover_input = true;
            }
            c.args(["-map", "0:a", "-map_metadata", "1", "-map_chapters", "1"]);
            if cover_input {
                c.args(["-map", "2:v", "-c:v", "copy", "-disposition:v:0", "attached_pic"]);
            }
            c.args(["-c:a", "aac", "-b:a", &format!("{kbps}k"), "-movflags", "+faststart", "-f", "ipod"]).arg(&out);
            run(&mut c, "ffmpeg")?;
            std::fs::read(&out).map_err(|e| e.to_string())
        }
        Encoder::Afconvert(af) => {
            let mut c = Command::new(af);
            c.args(["-f", if audiobook { "m4bf" } else { "m4af" }, "-d", "aac", "-b", &(kbps * 1000).to_string(), "-q", "127"]).arg(&wav).arg(&out);
            run(&mut c, "afconvert")?;
            let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
            add_metadata(&bytes, tags, audiobook)
        }
    }
}

/// `;FFMETADATA1` with the tags and chapters (`=`, `;`, `#`, `\` and newlines escaped).
fn ffmetadata(t: &EpisodeTags) -> String {
    let esc = |s: &str| {
        let mut o = String::new();
        for ch in s.chars() {
            if matches!(ch, '=' | ';' | '#' | '\\' | '\n') {
                o.push('\\');
            }
            o.push(ch);
        }
        o
    };
    let mut s = String::from(";FFMETADATA1\n");
    for (k, v) in [
        ("title", &t.title),
        ("artist", &t.artist),
        ("album", &t.album),
        ("genre", &t.genre),
        ("comment", &t.description),
        ("description", &t.description),
    ] {
        if !v.trim().is_empty() {
            s += &format!("{k}={}\n", esc(v));
        }
    }
    if t.year > 0 {
        s += &format!("date={}\n", t.year);
    }
    if t.episode > 0 {
        s += &format!("track={}\n", t.episode);
    }
    for c in &t.chapters {
        s += &format!("[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={}\n", c.start_ms, c.end_ms.max(c.start_ms + 1), esc(&c.title));
    }
    s
}

// ---- MP4 atoms -------------------------------------------------------------------------------

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 8);
    v.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    v.extend_from_slice(kind);
    v.extend_from_slice(body);
    v
}

/// An iTunes `ilst` item holding one `data` atom of `kind` (1 = UTF-8, 0 = binary, 13/14 = JPEG/PNG,
/// 21 = integer).
fn item(name: &[u8; 4], kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(payload.len() + 8);
    data.extend_from_slice(&kind.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(payload);
    atom(name, &atom(b"data", &data))
}

/// `udta` with iTunes-style tags and Nero chapters.
fn udta(t: &EpisodeTags, audiobook: bool) -> Vec<u8> {
    let mut ilst = Vec::new();
    for (name, v) in [
        (b"\xa9nam", &t.title),
        (b"\xa9ART", &t.artist),
        (b"\xa9alb", &t.album),
        (b"\xa9gen", &t.genre),
        (b"\xa9cmt", &t.description),
        (b"desc", &t.description),
    ] {
        if !v.trim().is_empty() {
            ilst.extend(item(name, 1, v.as_bytes()));
        }
    }
    if t.year > 0 {
        ilst.extend(item(b"\xa9day", 1, t.year.to_string().as_bytes()));
    }
    if t.episode > 0 {
        let n = u16::try_from(t.episode).unwrap_or(u16::MAX);
        let mut trkn = vec![0, 0];
        trkn.extend_from_slice(&n.to_be_bytes());
        trkn.extend_from_slice(&[0, 0, 0, 0]);
        ilst.extend(item(b"trkn", 0, &trkn));
    }
    // Media kind: 2 = audiobook, 21 = podcast.
    ilst.extend(item(b"stik", 21, &[if audiobook { 2 } else { 21 }]));
    if let Some((mime, data)) = &t.cover {
        ilst.extend(item(b"covr", if mime.contains("png") { 14 } else { 13 }, data));
    }
    let mut hdlr = vec![0u8; 8];
    hdlr.extend_from_slice(b"mdirappl");
    hdlr.extend_from_slice(&[0u8; 9]);
    let mut meta = vec![0u8; 4];
    meta.extend(atom(b"hdlr", &hdlr));
    meta.extend(atom(b"ilst", &ilst));
    let mut body = atom(b"meta", &meta);
    if !t.chapters.is_empty() {
        body.extend(chpl(&t.chapters));
    }
    atom(b"udta", &body)
}

/// Nero chapters: version 1, then start times in 100 ns units and titles of up to 255 bytes.
fn chpl(chapters: &[Chapter]) -> Vec<u8> {
    let mut b = vec![1, 0, 0, 0, 0, 0, 0, 0];
    let list: Vec<&Chapter> = chapters.iter().take(255).collect();
    b.push(list.len() as u8);
    for c in list {
        b.extend_from_slice(&(u64::from(c.start_ms) * 10_000).to_be_bytes());
        let mut title = c.title.as_bytes();
        if title.len() > 255 {
            // Cut on a character boundary.
            let mut n = 255;
            while n > 0 && !c.title.is_char_boundary(n) {
                n -= 1;
            }
            title = title.get(..n).unwrap_or_default();
        }
        b.push(title.len() as u8);
        b.extend_from_slice(title);
    }
    atom(b"chpl", &b)
}

/// The atoms in `data[start..end]` as (offset, header length, total length, kind).
fn children(data: &[u8], start: usize, end: usize) -> Result<Vec<(usize, usize, usize, [u8; 4])>, String> {
    let mut out = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let size32 = u32::from_be_bytes(data.get(at..at + 4).and_then(|b| b.try_into().ok()).ok_or("truncated atom")?) as usize;
        let kind: [u8; 4] = data.get(at + 4..at + 8).and_then(|b| b.try_into().ok()).ok_or("truncated atom")?;
        let (header, size) = match size32 {
            1 => (16, u64::from_be_bytes(data.get(at + 8..at + 16).and_then(|b| b.try_into().ok()).ok_or("truncated atom")?) as usize),
            0 => (8, end - at),
            n => (8, n),
        };
        if size < header || at + size > end {
            return Err(format!("bad {} atom", String::from_utf8_lossy(&kind)));
        }
        out.push((at, header, size, kind));
        at += size;
    }
    Ok(out)
}

/// Adds `delta` to every chunk offset (`stco`/`co64`) inside the `moov` body `m`.
fn shift_chunk_offsets(m: &mut [u8], start: usize, end: usize, delta: i64) -> Result<(), String> {
    for (at, header, size, kind) in children(m, start, end)? {
        match &kind {
            b"trak" | b"mdia" | b"minf" | b"stbl" => shift_chunk_offsets(m, at + header, at + size, delta)?,
            b"stco" | b"co64" => {
                let wide = &kind == b"co64";
                let body = at + header + 4;
                let n = u32::from_be_bytes(m.get(body..body + 4).and_then(|b| b.try_into().ok()).ok_or("bad stco")?) as usize;
                let w = if wide { 8 } else { 4 };
                for i in 0..n {
                    let p = body + 4 + i * w;
                    let cell = m.get_mut(p..p + w).ok_or("bad stco")?;
                    if wide {
                        let v = u64::from_be_bytes(cell.try_into().map_err(|_| "bad co64")?) as i64 + delta;
                        cell.copy_from_slice(&(v.max(0) as u64).to_be_bytes());
                    } else {
                        let v = i64::from(u32::from_be_bytes(cell.try_into().map_err(|_| "bad stco")?)) + delta;
                        cell.copy_from_slice(&(u32::try_from(v.max(0)).map_err(|_| "file too large for stco")?).to_be_bytes());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Rewrites `moov` with our `udta` (any existing one replaced). When `moov` comes before `mdat`,
/// the chunk offsets move by the size change.
pub fn add_metadata(file: &[u8], tags: &EpisodeTags, audiobook: bool) -> Result<Vec<u8>, String> {
    let top = children(file, 0, file.len())?;
    let &(moov_at, moov_header, moov_size, _) = top.iter().find(|a| &a.3 == b"moov").ok_or("no moov atom")?;
    let mdat_after = top.iter().any(|a| &a.3 == b"mdat" && a.0 > moov_at);
    let body_start = moov_at + moov_header;
    let mut body = Vec::new();
    for (at, _, size, kind) in children(file, body_start, moov_at + moov_size)? {
        if &kind != b"udta" {
            body.extend_from_slice(file.get(at..at + size).ok_or("bad moov")?);
        }
    }
    body.extend(udta(tags, audiobook));
    let mut moov = atom(b"moov", &body);
    if mdat_after {
        let delta = moov.len() as i64 - moov_size as i64;
        let end = moov.len();
        shift_chunk_offsets(&mut moov, 8, end, delta)?;
    }
    let mut out = Vec::with_capacity(file.len() + moov.len());
    out.extend_from_slice(file.get(..moov_at).ok_or("bad file")?);
    out.extend(moov);
    out.extend_from_slice(file.get(moov_at + moov_size..).ok_or("bad file")?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> AudioBuffer {
        let sr = 44_100;
        let ch: Vec<f32> = (0..sr * 3).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / sr as f32).sin() * 0.3).collect();
        AudioBuffer { sample_rate: sr as u32, channels: vec![ch.clone(), ch] }
    }

    fn tags() -> EpisodeTags {
        EpisodeTags {
            title: "Folge 12: Energie".into(),
            artist: "Dirk".into(),
            album: "Europa 2.0".into(),
            episode: 12,
            year: 2026,
            genre: "Podcast".into(),
            description: "Worum es geht; mit = und #.".into(),
            language: "deu".into(),
            cover: None,
            chapters: vec![
                Chapter { title: "Intro".into(), start_ms: 0, end_ms: 1500 },
                Chapter { title: "Thema".into(), start_ms: 1500, end_ms: 3000 },
            ],
        }
    }

    /// `ffprobe`'s view of a file: (format tags JSON, chapter titles), or None without ffprobe.
    fn probe(bytes: &[u8], ext: &str) -> Option<(serde_json::Value, Vec<String>)> {
        let ffprobe = on_path(&["ffprobe"], &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"])?;
        let dir = temp_dir().ok()?;
        let f = dir.join(format!("t.{ext}"));
        std::fs::write(&f, bytes).ok()?;
        let out =
            Command::new(ffprobe).args(["-v", "error", "-show_format", "-show_chapters", "-show_streams", "-of", "json"]).arg(&f).output().ok()?;
        let _ = std::fs::remove_dir_all(&dir);
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let titles =
            v["chapters"].as_array().map(|a| a.iter().filter_map(|c| c["tags"]["title"].as_str().map(str::to_string)).collect()).unwrap_or_default();
        Some((v, titles))
    }

    #[test]
    fn ffmetadata_escapes_and_lists_chapters() {
        let m = ffmetadata(&tags());
        assert!(m.starts_with(";FFMETADATA1\ntitle=Folge 12: Energie\n"));
        assert!(m.contains("description=Worum es geht\\; mit \\= und \\#.\n"));
        assert!(m.contains("[CHAPTER]\nTIMEBASE=1/1000\nSTART=1500\nEND=3000\ntitle=Thema\n"));
    }

    #[test]
    fn each_encoder_writes_aac_with_tags_and_chapters() {
        let mut encoders = Vec::new();
        if let Some(Encoder::Ffmpeg(p)) = find_encoder() {
            encoders.push(Encoder::Ffmpeg(p));
        }
        if cfg!(target_os = "macos")
            && let Some(p) = on_path(&["afconvert"], &["/usr/bin"])
        {
            encoders.push(Encoder::Afconvert(p));
        }
        if encoders.is_empty() {
            eprintln!("skipping: no AAC encoder on this machine");
            return;
        }
        for enc in encoders {
            for audiobook in [false, true] {
                let dir = temp_dir().unwrap();
                let bytes = encode_in(&dir, &enc, &tone(), 128, &tags(), audiobook).unwrap();
                let _ = std::fs::remove_dir_all(&dir);
                assert_eq!(bytes.get(4..8), Some(&b"ftyp"[..]), "{enc:?}: an MP4 file");
                let Some((v, chapters)) = probe(&bytes, if audiobook { "m4b" } else { "m4a" }) else { continue };
                assert_eq!(v["streams"][0]["codec_name"], "aac", "{enc:?}");
                assert_eq!(v["format"]["tags"]["title"], "Folge 12: Energie", "{enc:?}: {v}");
                assert_eq!(v["format"]["tags"]["album"], "Europa 2.0", "{enc:?}");
                assert_eq!(chapters, ["Intro", "Thema"], "{enc:?}");
                let secs: f64 = v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
                assert!((secs - 3.0).abs() < 0.2, "{enc:?}: {secs} s");
            }
        }
    }
}
