//! Subtitle files (SubRip `.srt` and WebVTT `.vtt`) and keeping cues in step with time edits.

use soundcraft_model::Session;
use soundcraft_time::Range;

/// Cues longer than this are refused on import (a malformed file, not a caption).
const MAX_CUE_MS: u64 = 60 * 60 * 1000;
/// Imports stop after this many cues.
const MAX_CUES: usize = 50_000;

/// A cue in milliseconds, as files store it.
#[derive(Debug, Clone, PartialEq)]
pub struct FileCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: String,
    pub text: String,
}

/// `hh:mm:ss,mmm` (SRT, `sep` = ',') or `hh:mm:ss.mmm` (WebVTT, `sep` = '.').
pub fn timestamp(ms: u64, sep: char) -> String {
    let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    format!("{h:02}:{m:02}:{s:02}{sep}{f:03}")
}

/// `h:mm:ss` or `mm:ss` for shownotes and chapter lists.
pub fn clock(ms: u64) -> String {
    let (h, m, s) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// Parses `[hh:]mm:ss[.,]mmm`; `None` for anything else.
fn parse_timestamp(t: &str) -> Option<u64> {
    let t = t.trim();
    let (hms, frac) = t.rsplit_once([',', '.']).unwrap_or((t, "0"));
    let parts: Vec<&str> = hms.split(':').collect();
    let num = |s: &str| if s.is_empty() || s.len() > 6 || !s.bytes().all(|b| b.is_ascii_digit()) { None } else { s.parse::<u64>().ok() };
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (num(h)?, num(m)?, num(s)?),
        [m, s] => (0, num(m)?, num(s)?),
        _ => return None,
    };
    if m >= 60 || s >= 60 || frac.len() > 3 {
        return None;
    }
    let f = num(frac)? * 10u64.pow(3 - frac.len() as u32);
    h.checked_mul(3_600_000)?.checked_add(m * 60_000 + s * 1000 + f)
}

/// Splits a WebVTT `<v Name>text` voice tag; other markup is kept as text.
fn voice(text: &str) -> (String, String) {
    if let Some(rest) = text.strip_prefix("<v ")
        && let Some((name, body)) = rest.split_once('>')
    {
        return (name.trim().to_string(), body.replace("</v>", "").trim().to_string());
    }
    (String::new(), text.to_string())
}

/// Reads SRT or WebVTT (detected from the `WEBVTT` header). Malformed blocks are skipped.
pub fn parse(text: &str) -> Vec<FileCue> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        if out.len() >= MAX_CUES {
            break;
        }
        let lines: Vec<&str> = block.lines().collect();
        let Some(i) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let Some(timing) = lines.get(i) else { continue };
        let Some((a, b)) = timing.split_once("-->") else { continue };
        // WebVTT puts cue settings after the end time.
        let b = b.split_whitespace().next().unwrap_or("");
        let (Some(start_ms), Some(end_ms)) = (parse_timestamp(a), parse_timestamp(b)) else { continue };
        if end_ms <= start_ms || end_ms - start_ms > MAX_CUE_MS {
            continue;
        }
        let body = lines.get(i + 1..).unwrap_or(&[]).join("\n");
        let (speaker, text) = voice(body.trim());
        out.push(FileCue { start_ms, end_ms, speaker, text });
    }
    out.sort_by_key(|c| c.start_ms);
    out
}

/// Writes SubRip.
pub fn write_srt(cues: &[FileCue]) -> String {
    let mut s = String::new();
    for (i, c) in cues.iter().enumerate() {
        let text = if c.speaker.is_empty() { c.text.clone() } else { format!("{}: {}", c.speaker, c.text) };
        s += &format!("{}\n{} --> {}\n{}\n\n", i + 1, timestamp(c.start_ms, ','), timestamp(c.end_ms, ','), text.trim());
    }
    s
}

/// Writes WebVTT, speakers as `<v Name>` voice tags.
pub fn write_vtt(cues: &[FileCue]) -> String {
    let mut s = String::from("WEBVTT\n\n");
    for c in cues {
        let text = if c.speaker.is_empty() { c.text.trim().to_string() } else { format!("<v {}>{}", c.speaker, c.text.trim()) };
        s += &format!("{} --> {}\n{}\n\n", timestamp(c.start_ms, '.'), timestamp(c.end_ms, '.'), text);
    }
    s
}

/// The session's cues inside `r`, in milliseconds from `r.start`, clipped to `r`.
pub fn file_cues(s: &Session, r: Range) -> Vec<FileCue> {
    let sr = s.sample_rate.as_f64().max(1.0);
    let ms = |at: i64| ((at.clamp(r.start, r.end) - r.start).max(0) as f64 * 1000.0 / sr).round() as u64;
    s.subtitles
        .iter()
        .filter(|c| c.end > r.start && c.start < r.end && !c.text.trim().is_empty())
        .map(|c| FileCue { start_ms: ms(c.start), end_ms: ms(c.end), speaker: c.speaker.clone(), text: c.text.clone() })
        .filter(|c| c.end_ms > c.start_ms)
        .collect()
}

/// A fresh cue id.
pub fn next_id(s: &mut Session) -> u64 {
    let id = s.next_id.max(s.subtitles.iter().map(|c| c.id + 1).max().unwrap_or(1));
    s.next_id = id + 1;
    id
}

/// Keeps cues sorted by start (then id) so lists and files read in order.
pub fn sort(s: &mut Session) {
    s.subtitles.sort_by_key(|c| (c.start, c.id));
}

/// Removes `r` from the timeline for the cues: cues inside it go, cues across it shrink, later
/// cues move earlier by its length.
pub fn cut_time(s: &mut Session, r: Range) {
    let len = r.len();
    s.subtitles.retain(|c| !(c.start >= r.start && c.end <= r.end));
    for c in &mut s.subtitles {
        let cut = |at: i64| {
            if at >= r.end {
                at - len
            } else if at > r.start {
                r.start
            } else {
                at
            }
        };
        c.start = cut(c.start);
        c.end = cut(c.end);
    }
    s.subtitles.retain(|c| c.end > c.start);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_model::SubtitleCue;

    #[test]
    fn srt_and_vtt_round_trip_with_speakers() {
        let cues = vec![
            FileCue { start_ms: 0, end_ms: 2500, speaker: String::new(), text: "Hallo und willkommen".into() },
            FileCue { start_ms: 3_723_004, end_ms: 3_725_000, speaker: "Sven".into(), text: "Zweite Zeile\nmit Umbruch".into() },
        ];
        let vtt = write_vtt(&cues);
        assert!(vtt.starts_with("WEBVTT\n\n00:00:00.000 --> 00:00:02.500\n"));
        assert!(vtt.contains("01:02:03.004 --> 01:02:05.000\n<v Sven>Zweite Zeile"));
        assert_eq!(parse(&vtt), cues);
        let srt = write_srt(&cues);
        assert!(srt.starts_with("1\n00:00:00,000 --> 00:00:02,500\nHallo"));
        let back = parse(&srt);
        assert_eq!(back.len(), 2);
        assert_eq!(back[1].text, "Sven: Zweite Zeile\nmit Umbruch");
    }

    #[test]
    fn hostile_files_are_skipped_not_panics() {
        let text = "WEBVTT\r\n\r\n00:01.5 --> 00:02.000 align:start\r\nkurz\r\n\r\n99:99:99,999 --> 1\nkaputt\n\n\
                    00:00:05,000 --> 00:00:04,000\nrückwärts\n\n--> \n\n00:00:01,0000 --> 00:00:02,000\nzu genau";
        let cues = parse(text);
        assert_eq!(cues, vec![FileCue { start_ms: 1500, end_ms: 2000, speaker: String::new(), text: "kurz".into() }]);
        assert!(parse("").is_empty());
        assert!(parse("\u{feff}1\n00:00:00,000 --> 00:00:01,000\n").first().is_some_and(|c| c.text.is_empty()));
    }

    #[test]
    fn cutting_time_moves_shrinks_and_drops_cues() {
        let mut s = Session::default();
        for (id, (a, b)) in [(0, 100), (150, 260), (210, 240), (300, 400)].into_iter().enumerate() {
            s.subtitles.push(SubtitleCue { id: id as u64, start: a, end: b, speaker: String::new(), text: "x".into() });
        }
        cut_time(&mut s, Range::new(200, 250));
        let got: Vec<_> = s.subtitles.iter().map(|c| (c.start, c.end)).collect();
        assert_eq!(got, [(0, 100), (150, 210), (250, 350)]);
    }

    #[test]
    fn clock_formats() {
        assert_eq!(clock(65_000), "01:05");
        assert_eq!(clock(3_725_000), "1:02:05");
    }
}
