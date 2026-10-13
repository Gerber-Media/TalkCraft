//! MP3 export with ID3v2.4 tags (title, show, author, episode, description, cover art, chapters).
//! Encoding links LAME (LGPL) through the opt-in `mp3` feature; without it [`encode_mp3`] returns
//! [`AudioError::Unsupported`], so the default build stays MIT/Apache and builds for wasm.

use crate::{AudioBuffer, AudioError, Result};

/// The constant bitrates an MP3 export can use (kbit/s).
pub const MP3_BITRATES: [u32; 9] = [32, 48, 64, 96, 128, 160, 192, 256, 320];
/// The output sample rates an MP3 export can use (Hz).
pub const MP3_SAMPLE_RATES: [u32; 3] = [22_050, 44_100, 48_000];

/// One chapter: title and start/end in milliseconds.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_ms: u32,
    pub end_ms: u32,
}

/// Episode metadata written as an ID3v2.4 tag. Empty fields are left out.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct EpisodeTags {
    /// Episode title (TIT2).
    pub title: String,
    /// Host or author (TPE1).
    pub artist: String,
    /// Podcast name (TALB, and TPE2 when no artist is set).
    pub album: String,
    /// Episode number (TRCK), 0 = none.
    pub episode: u32,
    /// Year (TDRC), 0 = none.
    pub year: i32,
    /// Genre (TCON), usually "Podcast".
    pub genre: String,
    /// Episode description (COMM).
    pub description: String,
    /// Language of the description, ISO 639-2 (e.g. "deu", "eng").
    pub language: String,
    /// Front cover: MIME type and image bytes.
    #[serde(skip)]
    pub cover: Option<(String, Vec<u8>)>,
    pub chapters: Vec<Chapter>,
}

/// Settings for [`encode_mp3`].
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mp3Options {
    /// Constant bitrate, one of [`MP3_BITRATES`].
    pub bitrate_kbps: u32,
}

impl Default for Mp3Options {
    fn default() -> Self {
        Self { bitrate_kbps: 128 }
    }
}

/// Whether this build can write MP3 files.
pub const fn mp3_supported() -> bool {
    cfg!(feature = "mp3")
}

/// Encode one or two channels as a constant-bitrate MP3, prefixed with `tags` as ID3v2.4.
/// Samples are clamped to [-1, 1]; more than two channels is an error (fold down first).
pub fn encode_mp3(buf: &AudioBuffer, opts: &Mp3Options, tags: Option<&EpisodeTags>) -> Result<Vec<u8>> {
    if !MP3_BITRATES.contains(&opts.bitrate_kbps) {
        return Err(AudioError::Unsupported(format!("MP3 bitrate {} kbit/s (use one of {MP3_BITRATES:?})", opts.bitrate_kbps)));
    }
    if !MP3_SAMPLE_RATES.contains(&buf.sample_rate) {
        return Err(AudioError::Unsupported(format!("MP3 sample rate {} Hz (use one of {MP3_SAMPLE_RATES:?})", buf.sample_rate)));
    }
    if buf.channels.is_empty() || buf.channels.len() > 2 {
        return Err(AudioError::Unsupported(format!("MP3 needs mono or stereo, not {} channels", buf.channels.len())));
    }
    let mut out = match tags {
        Some(t) => id3_bytes(t)?,
        None => Vec::new(),
    };
    out.extend(lame::encode(buf, opts)?);
    Ok(out)
}

#[cfg(feature = "mp3")]
mod lame {
    use super::Mp3Options;
    use crate::{AudioBuffer, AudioError, Result};
    use mp3lame_encoder::{Bitrate, Builder, DualPcm, FlushGap, Mode, MonoPcm, Quality};

    fn bitrate(kbps: u32) -> Bitrate {
        match kbps {
            32 => Bitrate::Kbps32,
            48 => Bitrate::Kbps48,
            64 => Bitrate::Kbps64,
            96 => Bitrate::Kbps96,
            160 => Bitrate::Kbps160,
            192 => Bitrate::Kbps192,
            256 => Bitrate::Kbps256,
            320 => Bitrate::Kbps320,
            _ => Bitrate::Kbps128,
        }
    }

    fn err(e: impl std::fmt::Display) -> AudioError {
        AudioError::Unsupported(format!("MP3 encoder: {e}"))
    }

    pub(super) fn encode(buf: &AudioBuffer, opts: &Mp3Options) -> Result<Vec<u8>> {
        let stereo = buf.channels.len() == 2;
        let mut b = Builder::new().ok_or_else(|| err("could not start LAME"))?;
        b.set_num_channels(if stereo { 2 } else { 1 }).map_err(err)?;
        b.set_sample_rate(buf.sample_rate).map_err(err)?;
        b.set_brate(bitrate(opts.bitrate_kbps)).map_err(err)?;
        b.set_mode(if stereo { Mode::JointStereo } else { Mode::Mono }).map_err(err)?;
        b.set_quality(Quality::NearBest).map_err(err)?;
        // The Xing/LAME info frame would need patching after the fact; constant bitrate does without.
        b.set_to_write_vbr_tag(false).map_err(err)?;
        let mut enc = b.build().map_err(err)?;
        let clamp = |c: &[f32]| c.iter().map(|s| if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 }).collect::<Vec<f32>>();
        let left = clamp(buf.channels.first().map_or(&[][..], Vec::as_slice));
        let right = if stereo { clamp(buf.channels.get(1).map_or(&[][..], Vec::as_slice)) } else { Vec::new() };
        let frames = if stereo { left.len().min(right.len()) } else { left.len() };
        const CHUNK: usize = 1152 * 64;
        let mut out = Vec::with_capacity(frames / 8 + 8192);
        let mut at = 0;
        while at < frames {
            let end = (at + CHUNK).min(frames);
            out.reserve(CHUNK * 5 / 4 + 7200);
            let (l, r) = (left.get(at..end).unwrap_or(&[]), right.get(at..end).unwrap_or(&[]));
            if stereo {
                enc.encode_to_vec(DualPcm { left: l, right: r }, &mut out).map_err(err)?;
            } else {
                enc.encode_to_vec(MonoPcm(l), &mut out).map_err(err)?;
            }
            at = end;
        }
        out.reserve(7200);
        enc.flush_to_vec::<FlushGap>(&mut out).map_err(err)?;
        Ok(out)
    }
}

#[cfg(not(feature = "mp3"))]
mod lame {
    use super::Mp3Options;
    use crate::{AudioBuffer, AudioError, Result};

    pub(super) fn encode(_: &AudioBuffer, _: &Mp3Options) -> Result<Vec<u8>> {
        Err(AudioError::Unsupported("MP3 export is not part of this build (enable the `mp3` feature)".into()))
    }
}

#[cfg(feature = "mp3")]
fn id3_bytes(t: &EpisodeTags) -> Result<Vec<u8>> {
    use id3::frame::{Chapter as Chap, Comment, Picture, PictureType, TableOfContents};
    use id3::{Frame, Tag, TagLike, Version};
    let mut tag = Tag::new();
    let set = |s: &str| !s.trim().is_empty();
    if set(&t.title) {
        tag.set_title(t.title.trim());
    }
    if set(&t.artist) {
        tag.set_artist(t.artist.trim());
    }
    if set(&t.album) {
        tag.set_album(t.album.trim());
        tag.set_album_artist(if set(&t.artist) { t.artist.trim() } else { t.album.trim() });
    }
    if t.episode > 0 {
        tag.set_track(t.episode);
    }
    if t.year > 0 {
        tag.set_year(t.year);
    }
    if set(&t.genre) {
        tag.set_genre(t.genre.trim());
    }
    let lang = if t.language.len() == 3 && t.language.is_ascii() { t.language.to_ascii_lowercase() } else { "eng".into() };
    if set(&t.description) {
        tag.add_frame(Comment { lang, description: String::new(), text: t.description.trim().to_string() });
    }
    if let Some((mime, data)) = &t.cover
        && !data.is_empty()
    {
        tag.add_frame(Picture { mime_type: mime.clone(), picture_type: PictureType::CoverFront, description: "Cover".into(), data: data.clone() });
    }
    let chapters: Vec<&Chapter> = t.chapters.iter().filter(|c| c.end_ms > c.start_ms).collect();
    if !chapters.is_empty() {
        let ids: Vec<String> = (0..chapters.len()).map(|i| format!("chp{i}")).collect();
        for (c, id) in chapters.iter().zip(&ids) {
            tag.add_frame(Chap {
                element_id: id.clone(),
                start_time: c.start_ms,
                end_time: c.end_ms,
                start_offset: u32::MAX,
                end_offset: u32::MAX,
                frames: vec![Frame::text("TIT2", c.title.clone())],
            });
        }
        tag.add_frame(TableOfContents { element_id: "toc".into(), top_level: true, ordered: true, elements: ids, frames: Vec::new() });
    }
    let mut out = Vec::new();
    tag.write_to(&mut out, Version::Id3v24).map_err(|e| AudioError::Unsupported(format!("ID3 tag: {e}")))?;
    Ok(out)
}

#[cfg(not(feature = "mp3"))]
fn id3_bytes(_: &EpisodeTags) -> Result<Vec<u8>> {
    Ok(Vec::new())
}

#[cfg(all(test, feature = "mp3"))]
mod tests {
    use super::*;

    fn tone(sr: u32, ch: usize, secs: f32) -> AudioBuffer {
        let n = (sr as f32 * secs) as usize;
        let c: Vec<f32> = (0..n).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / sr as f32).sin() * 0.3).collect();
        AudioBuffer { sample_rate: sr, channels: vec![c; ch] }
    }

    #[test]
    fn mono_and_stereo_encode_and_decode_back() {
        for ch in [1, 2] {
            let bytes = encode_mp3(&tone(44_100, ch, 2.0), &Mp3Options { bitrate_kbps: 128 }, None).unwrap();
            // 2 s at 128 kbit/s is about 32 kB.
            assert!((28_000..40_000).contains(&bytes.len()), "{ch} ch: {} bytes", bytes.len());
            let (info, back) = crate::decode(&bytes, Some("mp3")).unwrap();
            assert_eq!(info.sample_rate, 44_100);
            assert_eq!(back.channels.len(), ch);
            assert!(back.frames() >= 88_200, "{} frames", back.frames());
        }
    }

    #[test]
    fn tags_carry_metadata_cover_and_chapters() {
        let tags = EpisodeTags {
            title: "Folge 1".into(),
            artist: "Dirk".into(),
            album: "Mein Podcast".into(),
            episode: 1,
            year: 2026,
            genre: "Podcast".into(),
            description: "Worum es geht".into(),
            language: "deu".into(),
            cover: Some(("image/png".into(), vec![0x89, b'P', b'N', b'G'])),
            chapters: vec![
                Chapter { title: "Intro".into(), start_ms: 0, end_ms: 1000 },
                Chapter { title: "Thema".into(), start_ms: 1000, end_ms: 2000 },
            ],
        };
        let bytes = encode_mp3(&tone(48_000, 2, 2.0), &Mp3Options::default(), Some(&tags)).unwrap();
        let tag = id3::Tag::read_from2(std::io::Cursor::new(&bytes)).unwrap();
        use id3::TagLike;
        assert_eq!(tag.title(), Some("Folge 1"));
        assert_eq!(tag.album(), Some("Mein Podcast"));
        assert_eq!(tag.track(), Some(1));
        assert_eq!(tag.chapters().count(), 2);
        assert_eq!(tag.frames().filter(|f| f.id() == "APIC").count(), 1);
        assert!(crate::decode(&bytes, Some("mp3")).is_ok());
    }

    #[test]
    fn bad_settings_are_errors_not_panics() {
        assert!(encode_mp3(&tone(44_100, 1, 0.1), &Mp3Options { bitrate_kbps: 100 }, None).is_err());
        assert!(encode_mp3(&tone(96_000, 1, 0.1), &Mp3Options::default(), None).is_err());
        assert!(encode_mp3(&tone(44_100, 6, 0.1), &Mp3Options::default(), None).is_err());
        assert!(encode_mp3(&AudioBuffer { sample_rate: 44_100, channels: vec![Vec::new()] }, &Mp3Options::default(), None).is_ok());
    }
}
