//! Podcast delivery settings and episode metadata, saved with the session so every export of an
//! episode uses the same format, loudness and tags.

/// How an episode is exported.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PodcastExport {
    /// `mp3`, `wav`, `flac` or `aiff`.
    pub format: String,
    /// MP3 constant bitrate (kbit/s).
    pub bitrate_kbps: u32,
    /// Output sample rate (Hz); the mix is resampled when the session differs.
    pub sample_rate: u32,
    /// Fold the mix to one channel (spoken-word shows often are mono).
    pub mono: bool,
    /// WAV/FLAC/AIFF bit depth: `16`, `24` or `32f`.
    pub bit_depth: String,
    /// Integrated loudness target (LUFS), `None` = no loudness normalization.
    pub loudness_lufs: Option<f32>,
    /// Write the session's markers as MP3 chapters.
    pub chapters: bool,
}

impl Default for PodcastExport {
    fn default() -> Self {
        Self {
            format: "mp3".into(),
            bitrate_kbps: 128,
            sample_rate: 44_100,
            mono: false,
            bit_depth: "16".into(),
            loudness_lufs: Some(-16.0),
            chapters: true,
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
}
