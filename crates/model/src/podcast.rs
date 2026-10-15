//! Podcast delivery settings and episode metadata, saved with the session so every export of an
//! episode uses the same format, loudness and tags.

/// How an episode is exported.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PodcastExport {
    /// `m4a` (AAC, the default), `mp3`, `m4b` (audiobook), `wav`, `flac` or `aiff`.
    pub format: String,
    /// MP3/AAC bitrate (kbit/s).
    pub bitrate_kbps: u32,
    /// Output sample rate (Hz); the mix is resampled when the session differs.
    pub sample_rate: u32,
    /// Fold the mix to one channel (spoken-word shows often are mono).
    pub mono: bool,
    /// WAV/FLAC/AIFF bit depth: `16`, `24` or `32f`.
    pub bit_depth: String,
    /// Integrated loudness target (LUFS), `None` = no loudness normalization.
    pub loudness_lufs: Option<f32>,
    /// Write the session's markers as chapters (MP3, M4A, M4B).
    pub chapters: bool,
    /// Also write shownotes (`.md`) and Podcasting 2.0 chapters (`.chapters.json`) next to the file.
    pub shownotes: bool,
    /// Also write the subtitles as `.srt` and `.vtt` next to the file (when there are any).
    pub subtitles: bool,
}

impl Default for PodcastExport {
    fn default() -> Self {
        Self {
            format: "m4a".into(),
            bitrate_kbps: 128,
            sample_rate: 44_100,
            mono: false,
            bit_depth: "16".into(),
            loudness_lufs: Some(-16.0),
            chapters: true,
            shownotes: true,
            subtitles: true,
        }
    }
}

/// Episode metadata, written as ID3 tags into MP3 exports.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EpisodeInfo {
    pub title: String,
    /// Host or author.
    pub author: String,
    /// The podcast's name.
    pub show: String,
    /// Episode number, 0 = none.
    pub episode: u32,
    /// Year, 0 = none.
    pub year: i32,
    pub genre: String,
    pub description: String,
    /// ISO 639-2 code (`deu`, `eng`, `dan` …).
    pub language: String,
    /// Path of the cover image (JPEG or PNG, square, ideally 3000 × 3000 px).
    pub cover: String,
}

impl Default for EpisodeInfo {
    fn default() -> Self {
        Self {
            title: String::new(),
            author: String::new(),
            show: String::new(),
            episode: 0,
            year: 0,
            genre: "Podcast".into(),
            description: String::new(),
            language: "deu".into(),
            cover: String::new(),
        }
    }
}

/// Everything podcast-specific a session carries.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PodcastSettings {
    pub export: PodcastExport,
    pub episode: EpisodeInfo,
    /// Audio files on the jingle pads (intro, outro, stingers); "" = empty pad.
    pub jingles: Vec<String>,
    /// A picture per pad (JPEG/PNG path, "" = none), shown on the pad.
    pub jingle_images: Vec<String>,
    /// A name per pad ("" = the file's name).
    pub jingle_names: Vec<String>,
    /// How many pads the Jingles window shows (0 = the standard eight).
    pub jingle_pads: u32,
    /// The run sheet: the episode's topics in order, each with its planned length.
    pub rundown: Vec<RundownItem>,
}

/// One topic of the run sheet.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RundownItem {
    pub title: String,
    /// Planned length in minutes (0 = open).
    pub minutes: f32,
}

/// One subtitle (caption) cue on the session timeline.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SubtitleCue {
    /// Stable within the session (not reused after a delete).
    pub id: u64,
    pub start: soundcraft_time::Samples,
    pub end: soundcraft_time::Samples,
    /// Who speaks (written as a WebVTT voice tag), may be empty.
    #[serde(default)]
    pub speaker: String,
    pub text: String,
}
