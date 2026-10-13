<p align="center">
  <img alt="TalkCraft icon: a talking parrot" src="assets/app-icon/talkcraft-1024.png" width="160">
</p>

<h1 align="center">TalkCraft</h1>

<p align="center">
  <b>A podcast studio: record every speaker on their own track, cut, subtitle and publish.</b><br>
  Native on macOS, Windows and Linux, written in Rust. A fork of
  <a href="https://github.com/storytold/soundcraft">SoundCraft</a> by GERBER.MEDIA.
</p>

## What it does

- **Podcast template.** New sessions start with a track per speaker (each on its own input, with
  a voice chain: high-pass EQ, gate, compressor, de-esser), a jingle track and a master limited to
  -1 dBTP.
- **Audio interface.** Setup > Audio Interface picks the input and output device, finds each
  speaker's microphone ("please speak now") and runs a soundcheck that says per microphone whether
  to turn the gain up or down. Known podcast interfaces are recognised; unplugging one mid-session
  is caught instead of crashing.
- **Recording.** Record (Cmd+Space) records every speaker track except locked ones; a track's own
  record button records just that speaker. The take is drawn live while it records.
- **Remote guests.** Window > Remote Guests: guests join from their browser with a link (nothing
  to install). Each guest appears as a track "Ferngast – Name" with a live meter; M on the track
  mutes them in the call. Record in TalkCraft starts and stops a lossless recording in every
  guest's browser; after Stop their file arrives on their track, synced to a backup of the call.
  The connection runs browser to browser (WebRTC); the guest page lives in
  [Talkcraft-Gast](https://codefloe.com/GerberMedia/Talkcraft-Gast).
- **Editing.** The blade splits a clip where you click (Alt: every track). Delete/Backspace removes
  the selected section after asking, closing the gap on all tracks so speakers stay in sync.
  Magnetic snapping, Close Gap, Z/Shift+Z zoom. Edit > Shorten Pauses trims long silences on all
  tracks at once. Markers and subtitles move along.
- **Clip tools.** Normalize or set a clip's loudness, clip volume, fades with handles, noise
  reduction.
- **Speakers and music.** Edit > Match Speaker Levels makes every speaker equally loud;
  Edit > Auto-Duck Music lowers the music while anyone speaks; Sync Clip lines up a separately
  recorded track by cross-correlation.
- **Jingles.** Window > Jingles: eight pads for intro, outro and stingers, placed with one click.
- **Subtitles and transcript.** Window > Subtitles: a cue list with speakers, a subtitle ruler, SRT
  and WebVTT import/export, and automatic transcription with
  [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (speakers named after the loudest track).
  File > Export > Transcript writes the text.
- **Filler words.** Window > Filler Words shows the transcript as running text with "ähm", "uh"
  and friends marked; select words and cut them out of every track at once.
- **Loudness.** The toolbar shows the episode's loudness (LUFS) live.
- **Delivery.** File > Export Episode: M4A (AAC, the default), M4B or MP3 (LAME) with tags, cover
  and chapters from markers, or WAV/FLAC/AIFF; loudness normalised to -16 LUFS (or another target).
  Shownotes (Markdown with a chapter list) and Podcasting 2.0 `chapters.json` are written alongside.
- **Audiogram.** File > Export > Audiogram (MP4): a square or portrait video with cover, title,
  waveform and subtitles for social media (needs ffmpeg).
- **Languages.** English and German interface (Automatic follows the system).
- **Settings.** Setup > Preferences holds the episode details and export defaults, saved with the
  session.

The full SoundCraft engine is still underneath (mixer, plugins, automation, CLI, MCP server); the
menus leave out what a podcast does not need.

## Build and run

Needs a recent Rust toolchain (`rustup`). On Linux also the ALSA and GTK development packages.

```sh
cargo talkcraft                       # build (release) and start the app
cargo run --release -p talkcraft-cli -- --help
cargo xtask ci                        # fmt, clippy, tests, asset and layer checks, wasm
```

The first build compiles LAME for MP3 export (autotools on macOS/Linux, `cc` on Windows), which
can take several minutes once.

### Automatic transcription

TalkCraft calls the external `whisper-cli` program; nothing of whisper.cpp is linked.

- macOS: `brew install whisper-cpp`
- Windows: unpack `whisper-bin-x64.zip` from the whisper.cpp releases into
  `%APPDATA%\TalkCraft\whisper`
- Linux: `whisper-cli` on the `PATH`

The speech model (`ggml-*.bin`, e.g. `ggml-large-v3-turbo-q5_0.bin`, ~550 MB) goes into the
`models` folder of the settings (macOS: `~/Library/Application Support/TalkCraft/models`); the
Subtitles window offers to download it.

## Installers

`packaging/` builds a DMG (macOS), MSI and portable zip (Windows), AppImage, deb, rpm, Flatpak
and tarball (Linux) and a static web build. See `docs/releasing.md`. Locally on a Mac:

```sh
packaging/macos/package.sh --arch aarch64   # dist/release/talkcraft-<version>-macos-aarch64.dmg
```

Unsigned builds are for testing: Gatekeeper and SmartScreen warn about them.

## Licence and credits

MIT OR Apache-2.0, like SoundCraft. TalkCraft is based on SoundCraft by the SoundCraft authors
(Learning Machines LLC, [storytold/soundcraft](https://github.com/storytold/soundcraft)); see
`NOTICE` and `ATTRIBUTION.md`. MP3 export links LAME (LGPL) as an optional Cargo feature; `NOTICE`
lists what that means for distributing binaries. TalkCraft is not affiliated with or endorsed by
the SoundCraft or ArtCraft projects.

Source: [codefloe.com/GerberMedia/TalkCraft](https://codefloe.com/GerberMedia/TalkCraft)
