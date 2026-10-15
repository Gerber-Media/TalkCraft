//! Realtime playback through the system audio device (cpal: CoreAudio, WASAPI, ALSA/Pulse/
//! PipeWire via ALSA, JACK, WebAudio).
//!
//! The UI thread owns a [`Player`]; the audio callback owns the [`MixEngine`] and a snapshot of
//! the session (`Arc<Session>`) that the UI swaps whenever the document changes. Commands go to
//! the callback through a channel; position and meters come back through atomics and a
//! `try_lock`ed snapshot, so the callback never blocks. When no device is available the player
//! falls back to a silent clock thread so the transport, meters and automation still run.
//!
//! Surround: the device is opened with as many output channels as it offers, up to the width of
//! the session's main format. Main channels (SMPTE/WAV order) go to device channels in order; when
//! the device has fewer channels than the main mix, the mix is folded down to stereo (ITU-R
//! BS.775) on the first two device channels.
//!
//! Plugins and threads: hosted third-party plugins (CLAP/VST3) are created, given their stored
//! state and prepared on the thread that calls [`Player::update_session`] (the UI thread), and
//! travel to the audio thread together with the session snapshot that needs them; the mix
//! engine adopts them (see `soundcraft_mix`'s crate docs). Instances the engine retires come back
//! on a return channel and are dropped on the UI thread ([`Player::idle`]). Plugin states for
//! saving are read on request ([`Player::capture_states`]: a command with a reply; the audio
//! thread answers between two blocks, never blocking). Plugin editors are driven on the UI thread
//! through handles taken from each instance before it leaves ([`Player::open_editor`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod record;
mod stretch;

use soundcraft_dsp::{Plugin, PluginEditor};
use soundcraft_mix::{MAX_CHANNELS, MixEngine, PreparedInstance, StripMeter, itu_stereo_fold, main_channels};
use soundcraft_model::{Session, TrackId};
use soundcraft_time::{Range, Samples};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

const BLOCK: usize = 512;

/// The realtime audio thread marker lives in the mix engine, which carries it into the strips it
/// renders on worker threads; the cpal callbacks here set it.
pub use soundcraft_mix::{mark_audio_thread, on_audio_thread};

/// Meter values for the UI.
#[derive(Debug, Clone, Default)]
pub struct MeterSnapshot {
    pub tracks: HashMap<TrackId, StripMeter>,
    pub main: StripMeter,
}

/// Plugin states read from the live instances: `(track, slot or INSTRUMENT_SLOT, state)`.
pub type PluginStates = Vec<(TrackId, usize, Vec<u8>)>;

/// A parameter edit made in a plugin's own editor: `(track, slot, param id, value)`.
pub type EditorEdit = (TrackId, usize, String, f32);

enum Cmd {
    /// A new snapshot plus the third-party instances it needs (created on the UI thread).
    Session(Arc<Session>, Vec<PreparedInstance>),
    /// Read every plugin state and reply.
    CaptureStates(Sender<PluginStates>),
    Input(Arc<record::InputRing>),
    Recording(bool),
    Play {
        from: Samples,
        end: Option<Samples>,
        looped: Option<Range>,
    },
    Stop,
    Locate(Samples),
    /// Play a sound now, beside the session (a jingle pad pressed while stopped).
    Audition(Arc<Audition>),
    StopAudition,
}

/// A sound played once, beside the transport: planar samples at their own rate, and a gain.
pub struct Audition {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
    pub gain: f32,
}

/// The audition being played and where it is.
struct Voice {
    sound: Arc<Audition>,
    read: f64,
}

impl Voice {
    /// The next stereo frame (linear interpolation), or `None` at the end.
    fn next(&mut self, step: f64) -> Option<(f32, f32)> {
        let len = self.sound.channels.first().map_or(0, Vec::len);
        let i = self.read as usize;
        if i + 1 >= len {
            return None;
        }
        let t = (self.read - i as f64) as f32;
        let at = |c: usize| {
            let ch = self.sound.channels.get(c).or_else(|| self.sound.channels.first());
            ch.map_or(0.0, |v| {
                let a = v.get(i).copied().unwrap_or(0.0);
                let b = v.get(i + 1).copied().unwrap_or(a);
                a + (b - a) * t
            })
        };
        let (l, r) = (at(0), at(1));
        self.read += step;
        Some((l * self.sound.gain, r * self.sound.gain))
    }
}

struct Shared {
    position: AtomicI64,
    playing: AtomicBool,
    /// Speed × 1000.
    speed: AtomicU32,
    /// Pitch-preserving playback rate × 1000 (listening faster or slower).
    rate: AtomicU32,
    meters: Mutex<MeterSnapshot>,
    /// The output device went away (unplugged): set by the stream's error callback.
    device_lost: AtomicBool,
}

/// State owned by the audio callback.
struct AudioState {
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    session: Arc<Session>,
    mix: MixEngine,
    pos: Samples,
    playing: bool,
    end: Option<Samples>,
    looped: Option<Range>,
    /// Rendered frames waiting to be consumed (planar, main channels) and the fractional read index.
    pending: Vec<Vec<f32>>,
    /// Stereo fold-down gains per main channel (used when the device has fewer channels).
    fold: [(f32, f32); MAX_CHANNELS],
    pending_len: usize,
    read: f64,
    device_rate: f64,
    input: Option<Arc<record::InputRing>>,
    recording: bool,
    /// Retired plugin instances go back to the UI thread to be dropped there.
    garbage: Sender<Vec<Box<dyn Plugin>>>,
    /// Pitch-preserving rate change between the mix and the device.
    stretch: stretch::Stretch,
    /// A sound playing beside the transport. The player keeps its own reference to every sound,
    /// so letting go of one here never frees memory on the audio thread.
    voice: Option<Voice>,
}

fn new_engine(sr: f32) -> MixEngine {
    let mut m = MixEngine::new(sr, BLOCK);
    m.set_external_instances(true);
    m.metronome = true;
    m
}

impl AudioState {
    fn new(rx: Receiver<Cmd>, shared: Arc<Shared>, session: Arc<Session>, device_rate: f64, garbage: Sender<Vec<Box<dyn Plugin>>>) -> Self {
        let sr = session.sample_rate.as_f64() as f32;
        let nch = main_channels(&session);
        AudioState {
            garbage,
            fold: fold_gains(&session),
            rx,
            shared,
            mix: new_engine(sr),
            session,
            pos: 0,
            playing: false,
            end: None,
            looped: None,
            pending: vec![vec![0.0; BLOCK]; nch],
            pending_len: 0,
            read: 0.0,
            device_rate,
            input: None,
            recording: false,
            stretch: stretch::Stretch::default(),
            voice: None,
        }
    }

    fn drain(&mut self) {
        let mut changed = false;
        while let Ok(c) = self.rx.try_recv() {
            match c {
                Cmd::Input(r) => self.input = Some(r),
                Cmd::Recording(r) => self.recording = r,
                Cmd::CaptureStates(reply) => {
                    self.mix.ensure_synced(&self.session);
                    let _ = reply.send(self.mix.capture_states());
                }
                Cmd::Session(s, instances) => {
                    changed = true;
                    if (s.sample_rate.as_f64() - self.session.sample_rate.as_f64()).abs() > 0.5 {
                        let mut old = std::mem::replace(&mut self.mix, new_engine(s.sample_rate.as_f64() as f32));
                        let _ = self.garbage.send(old.drain_plugins());
                    }
                    self.mix.adopt(instances);
                    // The main format changed (a document edit, not steady state): resize.
                    let nch = main_channels(&s);
                    if self.pending.len() != nch {
                        self.pending = vec![vec![0.0; BLOCK]; nch];
                        self.pending_len = 0;
                        self.read = 0.0;
                    }
                    self.fold = fold_gains(&s);
                    self.session = s;
                }
                Cmd::Play { from, end, looped } => {
                    self.pos = from;
                    self.end = end;
                    self.looped = looped.filter(|r| r.len() > 64);
                    self.playing = true;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.stretch.reset();
                    self.mix.reset();
                }
                Cmd::Stop => {
                    self.playing = false;
                    self.end = None;
                    self.looped = None;
                    self.mix.reset();
                }
                Cmd::Locate(at) => {
                    self.pos = at;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.stretch.reset();
                }
                Cmd::Audition(sound) => self.voice = Some(Voice { sound, read: 0.0 }),
                Cmd::StopAudition => self.voice = None,
            }
        }
        if changed {
            // Place adopted instances now (not only once playback starts), so state captures and
            // editors see them; `sync` creates no third-party plugins here.
            self.mix.ensure_synced(&self.session);
        }
        self.send_garbage();
        self.shared.playing.store(self.playing, Ordering::Relaxed);
    }

    fn send_garbage(&mut self) {
        let g = self.mix.take_retired();
        if !g.is_empty() {
            let _ = self.garbage.send(g);
        }
    }

    /// Render the next block at the session rate into `pending`.
    fn render_next(&mut self, quantum: usize) {
        let mut n = quantum.clamp(1, BLOCK);
        if let Some(l) = self.looped
            && self.pos >= l.end
        {
            self.pos = l.start;
        }
        if let Some(l) = self.looped {
            n = n.min(usize::try_from((l.end - self.pos).max(1)).unwrap_or(BLOCK));
        } else if let Some(e) = self.end {
            if self.pos >= e {
                self.playing = false;
                self.end = None;
                self.shared.playing.store(false, Ordering::Relaxed);
                n = 0;
            } else {
                n = n.min(usize::try_from(e - self.pos).unwrap_or(BLOCK));
            }
        }
        if n == 0 {
            for c in &mut self.pending {
                c.iter_mut().for_each(|x| *x = 0.0);
            }
            self.pending_len = BLOCK;
            return;
        }
        if let Some(ring) = &self.input {
            let ch = ring.channels.load(Ordering::Relaxed).max(1);
            if self.mix.input.len() != ch {
                self.mix.input = vec![vec![0.0; BLOCK]; ch];
            }
            ring.pop_into(&mut self.mix.input, n, self.session.sample_rate.hz());
        }
        self.mix.monitor_only = !self.playing;
        self.mix.recording = self.recording;
        self.mix.render(&self.session, self.pos, n, &mut self.pending);
        self.send_garbage();
        self.pending_len = n;
        if let Ok(mut m) = self.shared.meters.try_lock() {
            m.main.copy_from(&self.mix.main_meter);
            for (k, v) in &self.mix.meters {
                match m.tracks.get_mut(k) {
                    Some(d) => d.copy_from(v),
                    None => {
                        m.tracks.insert(*k, v.clone());
                    }
                }
            }
        }
        if self.playing {
            self.pos += n as i64;
            self.shared.position.store(self.pos, Ordering::Relaxed);
        }
    }

    /// Fill an interleaved device buffer.
    fn fill(&mut self, out: &mut [f32], channels: usize) {
        self.drain();
        let channels = channels.max(1);
        let monitoring = self.input.is_some() && self.session.tracks.iter().any(|t| t.mixer.input_monitor || t.mixer.record_arm);
        if !self.playing && !monitoring {
            out.iter_mut().for_each(|x| *x = 0.0);
            if let Ok(mut m) = self.shared.meters.try_lock() {
                m.main = StripMeter::default();
                m.tracks.clear();
            }
            self.add_voice(out, channels);
            return;
        }
        let speed = f64::from(self.shared.speed.load(Ordering::Relaxed)) / 1000.0;
        let step = self.session.sample_rate.as_f64() / self.device_rate.max(1.0) * speed.clamp(0.1, 4.0);
        let rate = f64::from(self.shared.rate.load(Ordering::Relaxed)) / 1000.0;
        // Only for listening back: while recording or monitoring an input, time runs at the wall
        // clock (a take must land where it was played, and the input ring must not run dry).
        let stretching = self.playing && !self.recording && !monitoring && (rate - 1.0).abs() > 0.005;
        let callback_frames = out.len() / channels;
        let quantum = ((callback_frames as f64 * step * if stretching { rate.clamp(0.25, 4.0) } else { 1.0 }).ceil() as usize).clamp(1, BLOCK);
        let nch = self.pending.len().min(MAX_CHANNELS);
        if stretching && !self.stretch.is_prepared_for(nch) {
            // A new channel count (a document edit, not steady state).
            self.stretch.prepare(nch, self.device_rate);
        }
        let mut stretch = std::mem::take(&mut self.stretch);
        let mut cur = [0.0f32; MAX_CHANNELS];
        for frame in out.chunks_mut(channels) {
            let cur = cur.get_mut(..nch).unwrap_or(&mut []);
            if stretching {
                stretch.next_frame(rate, cur, |f| self.input_frame(f, step, quantum, monitoring));
            } else {
                self.input_frame(cur, step, quantum, monitoring);
            }
            let sounding = self.playing || monitoring;
            let sample = |c: usize| cur.get(c).copied().unwrap_or(0.0);
            if nch > 2 && channels < nch {
                // Fewer device channels than the main mix: ITU fold-down to stereo.
                let (mut l, mut r) = (0.0f32, 0.0f32);
                if sounding {
                    for (c, (gl, gr)) in self.fold.iter().enumerate().take(nch) {
                        let v = sample(c);
                        l += v * gl;
                        r += v * gr;
                    }
                }
                for (c, o) in frame.iter_mut().enumerate() {
                    *o = match c {
                        0 => l.clamp(-1.0, 1.0),
                        1 => r.clamp(-1.0, 1.0),
                        _ => 0.0,
                    };
                }
            } else {
                for (c, o) in frame.iter_mut().enumerate() {
                    // Stereo mixes on a mono device play the left channel (as before).
                    *o = if c < nch.max(1) && sounding { sample(c).clamp(-1.0, 1.0) } else { 0.0 };
                }
            }
        }
        if !stretching {
            stretch.reset();
        }
        self.stretch = stretch;
        self.add_voice(out, channels);
    }

    /// Mixes the audition into the device buffer (stereo; mono devices get the left channel).
    fn add_voice(&mut self, out: &mut [f32], channels: usize) {
        let Some(v) = &mut self.voice else { return };
        let step = f64::from(v.sound.sample_rate.max(1)) / self.device_rate.max(1.0);
        let mut ended = false;
        for frame in out.chunks_mut(channels) {
            let Some((l, r)) = v.next(step) else {
                ended = true;
                break;
            };
            for (c, o) in frame.iter_mut().enumerate() {
                let add = match c {
                    0 => l,
                    1 => r,
                    _ => 0.0,
                };
                *o = (*o + add).clamp(-1.0, 1.0);
            }
        }
        if ended {
            self.voice = None;
        }
    }

    /// The next frame of the mix (all main channels) at the device rate, rendering as needed.
    fn input_frame(&mut self, frame: &mut [f32], step: f64, quantum: usize, monitoring: bool) {
        while self.read >= self.pending_len as f64 {
            self.read -= self.pending_len as f64;
            if !self.playing && !monitoring {
                break;
            }
            self.render_next(quantum);
            if self.pending_len == 0 {
                break;
            }
        }
        let i0 = self.read.floor() as usize;
        let fr = (self.read - i0 as f64) as f32;
        for (c, x) in frame.iter_mut().enumerate() {
            let ch = self.pending.get(c);
            let a = ch.and_then(|v| v.get(i0)).copied().unwrap_or(0.0);
            let b = ch.and_then(|v| v.get(i0 + 1)).copied().unwrap_or(a);
            *x = a + (b - a) * fr;
        }
        self.read += step;
    }
}

/// UI-thread bookkeeping of the third-party instances the audio thread holds.
struct Side {
    /// What was last created for each slot: (plugin id, channels, created ok).
    shadow: HashMap<(TrackId, usize), (String, usize, bool)>,
    /// Session sample rate the instances were prepared for.
    rate: f32,
    /// Editor handles of the live instances.
    editors: HashMap<(TrackId, usize), Box<dyn PluginEditor>>,
    garbage: Receiver<Vec<Box<dyn Plugin>>>,
}

impl Side {
    /// Drops retired instances (closing their editors first, inside the plugin's drop).
    fn collect_garbage(&mut self) {
        while let Ok(g) = self.garbage.try_recv() {
            drop(g);
        }
    }

    fn forget(&mut self, key: (TrackId, usize)) {
        self.shadow.remove(&key);
        if let Some(mut e) = self.editors.remove(&key) {
            e.close();
        }
    }
}

/// The transport and audio device.
pub struct Player {
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    side: Mutex<Side>,
    /// Every audition sent to the audio thread, until it lets go (see `audition`).
    auditions: Mutex<Vec<Arc<Audition>>>,
    #[cfg(not(target_arch = "wasm32"))]
    _stream: Option<cpal::Stream>,
    #[cfg(target_arch = "wasm32")]
    _stream: Option<cpal::Stream>,
    pub device_name: String,
    pub device_rate: u32,
    /// Output channels the device was opened with (0 for the silent clock).
    pub device_channels: u16,
    /// True when no audio device could be opened (silent clock).
    pub silent: bool,
}

impl Player {
    /// True once the output device has gone away (unplugged); open a new Player to recover.
    pub fn device_lost(&self) -> bool {
        self.shared.device_lost.load(Ordering::Relaxed)
    }

    /// Open the default output device. Never fails: without a device it runs a silent clock.
    pub fn new(session: Arc<Session>) -> Player {
        let p = Player::open(Arc::clone(&session));
        // Hand the initial session's third-party plugins over.
        p.update_session(session);
        p
    }

    fn side(&self) -> std::sync::MutexGuard<'_, Side> {
        self.side.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(session: Arc<Session>) -> Player {
        let (gtx, grx) = channel();
        let side = Mutex::new(Side { shadow: HashMap::new(), rate: session.sample_rate.as_f64() as f32, editors: HashMap::new(), garbage: grx });
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let want = session.sample_rate.hz();
        let want_ch = u16::try_from(main_channels(&session)).unwrap_or(2);
        match open_device(want, want_ch) {
            Ok((device, config, name)) => {
                let rate = config.sample_rate.0;
                let channels = usize::from(config.channels);
                let mut state = AudioState::new(rx, Arc::clone(&shared), session, f64::from(rate), gtx);
                // cpal may call this on the audio thread (on ALSA in a loop while the device
                // stays broken): marked, so the logger hands the record off instead of writing it.
                let lost = Arc::clone(&shared);
                let err_fn = move |e: cpal::StreamError| {
                    mark_audio_thread();
                    if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                        lost.device_lost.store(true, Ordering::Relaxed);
                    }
                    log::warn!("audio stream error: {e}");
                };
                use cpal::traits::{DeviceTrait, StreamTrait};
                match device.build_output_stream(
                    &config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        mark_audio_thread();
                        state.fill(data, channels);
                    },
                    err_fn,
                    None,
                ) {
                    Ok(stream) => {
                        if let Err(e) = stream.play() {
                            log::warn!("audio stream failed to start: {e}");
                        }
                        log::info!("audio device: {name} @ {rate} Hz, {channels} ch");
                        Player {
                            auditions: Mutex::new(Vec::new()),
                            shared,
                            tx,
                            side,
                            _stream: Some(stream),
                            device_name: name,
                            device_rate: rate,
                            device_channels: config.channels,
                            silent: false,
                        }
                    }
                    Err(e) => {
                        log::warn!("cannot open audio stream: {e}; using a silent clock");
                        let (tx2, rx2) = channel();
                        let (gtx2, grx2) = channel();
                        // The state (and its receivers) moved into the failed closure; rebuild.
                        let mut side = side;
                        side.get_mut().unwrap_or_else(PoisonError::into_inner).garbage = grx2;
                        Player::silent_clock(shared, tx2, rx2, want, side, gtx2)
                    }
                }
            }
            Err(e) => {
                log::warn!("no audio output: {e}; using a silent clock");
                let s2 = Arc::clone(&shared);
                Player::silent_clock_with(s2, tx, rx, session, side, gtx)
            }
        }
    }

    fn silent_clock(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        rate: u32,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        let s = Arc::new(Session::new("Untitled", soundcraft_time::SampleRate::new(rate).unwrap_or_default()));
        Player::silent_clock_with(shared, tx, rx, s, side, gtx)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        let rate = session.sample_rate.hz();
        let mut state = AudioState::new(rx, Arc::clone(&shared), session, f64::from(rate), gtx);
        let spawn = std::thread::Builder::new().name("soundcraft-clock".into()).spawn(move || {
            let mut buf = vec![0.0f32; 4096];
            let mut last = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
                let now = std::time::Instant::now();
                let frames = ((now - last).as_secs_f64() * state.device_rate) as usize;
                last = now;
                let n = (frames * 2).min(buf.len());
                if let Some(b) = buf.get_mut(..n) {
                    state.fill(b, 2);
                }
                if Arc::strong_count(&state.shared) <= 1 {
                    break;
                }
            }
        });
        if let Err(e) = spawn {
            log::warn!("clock thread failed: {e}");
        }
        Player {
            auditions: Mutex::new(Vec::new()),
            shared,
            tx,
            side,
            _stream: None,
            device_name: "No audio device (silent)".into(),
            device_rate: rate,
            device_channels: 0,
            silent: true,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        _rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        _gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        Player {
            auditions: Mutex::new(Vec::new()),
            shared,
            tx,
            side,
            _stream: None,
            device_name: "No audio device".into(),
            device_rate: session.sample_rate.hz(),
            device_channels: 0,
            silent: true,
        }
    }

    /// Sends a new session snapshot to the audio thread, with freshly created (state-restored,
    /// prepared) instances of any third-party plugin it newly needs. Call on the UI thread.
    pub fn update_session(&self, s: Arc<Session>) {
        let instances = self.side().prepare(&s);
        let _ = self.tx.send(Cmd::Session(s, instances));
    }

    /// Reads the state of every live third-party plugin (for saving the session). Waits briefly
    /// for the audio thread to answer between two blocks; empty when it does not answer.
    pub fn capture_states(&self) -> PluginStates {
        self.side().collect_garbage();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = channel();
            if self.tx.send(Cmd::CaptureStates(tx)).is_err() {
                return Vec::new();
            }
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or_default()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Vec::new()
        }
    }

    /// Per-UI-frame housekeeping: drops retired instances and services plugin editors. Returns
    /// the parameter edits made in open editors since the last call.
    pub fn idle(&self) -> Vec<EditorEdit> {
        let mut side = self.side();
        side.collect_garbage();
        let mut out = Vec::new();
        for (&(t, slot), e) in side.editors.iter_mut() {
            for (id, v) in e.idle() {
                out.push((t, slot, id, v));
            }
        }
        out
    }

    /// True when the plugin in this slot has its own editor.
    pub fn has_editor(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.contains_key(&(track, slot))
    }

    /// Opens the plugin's own editor window. Main thread only.
    pub fn open_editor(&self, track: TrackId, slot: usize) -> Result<(), String> {
        match self.side().editors.get_mut(&(track, slot)) {
            Some(e) => e.open(),
            None => Err("this plugin has no editor (or it is not loaded)".into()),
        }
    }

    pub fn close_editor(&self, track: TrackId, slot: usize) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.close();
        }
    }

    pub fn editor_open(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.get(&(track, slot)).is_some_and(|e| e.is_open())
    }

    pub fn any_editor_open(&self) -> bool {
        self.side().editors.values().any(|e| e.is_open())
    }

    /// Shows a parameter change made elsewhere in the plugin's open editor.
    pub fn editor_set_param(&self, track: TrackId, slot: usize, id: &str, value: f32) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.set_param(id, value);
        }
    }

    /// Start playback at `from`, stopping at `end`, or looping `looped`.
    pub fn play(&self, from: Samples, end: Option<Samples>, looped: Option<Range>) {
        self.shared.position.store(from, Ordering::Relaxed);
        self.shared.playing.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Play { from, end, looped });
    }

    /// Plays `sound` now, beside the transport (it replaces one still playing).
    pub fn audition(&self, sound: Arc<Audition>) {
        if let Ok(mut held) = self.auditions.lock() {
            // Sounds the audio thread no longer has are freed here, on this thread.
            held.retain(|a| Arc::strong_count(a) > 1);
            held.push(sound.clone());
        }
        let _ = self.tx.send(Cmd::Audition(sound));
    }

    pub fn stop_audition(&self) {
        let _ = self.tx.send(Cmd::StopAudition);
    }

    /// Feed live input (from a `record::Recorder`) into the mix for input monitoring.
    pub fn set_input(&self, ring: Arc<record::InputRing>) {
        let _ = self.tx.send(Cmd::Input(ring));
    }

    pub fn set_recording(&self, on: bool) {
        let _ = self.tx.send(Cmd::Recording(on));
    }

    pub fn stop(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Stop);
    }

    pub fn locate(&self, at: Samples) {
        self.shared.position.store(at, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Locate(at));
    }

    /// Listening rate that keeps the pitch (0.5–2 is sensible; 1 = normal).
    pub fn set_rate(&self, rate: f32) {
        let v = if rate.is_finite() { (rate.clamp(0.25, 4.0) * 1000.0) as u32 } else { 1000 };
        self.shared.rate.store(v, Ordering::Relaxed);
    }

    pub fn rate(&self) -> f32 {
        self.shared.rate.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn set_speed(&self, speed: f32) {
        let v = if speed.is_finite() { (speed.clamp(0.1, 4.0) * 1000.0) as u32 } else { 1000 };
        self.shared.speed.store(v, Ordering::Relaxed);
    }

    /// Playback rate last requested, 1.0 at full speed.
    pub fn speed(&self) -> f32 {
        self.shared.speed.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn position(&self) -> Samples {
        self.shared.position.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn meters(&self) -> MeterSnapshot {
        self.shared.meters.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Copy meters into reusable UI storage. Once track/channel counts are stable this performs
    /// no allocation, unlike cloning a new snapshot every video frame.
    pub fn copy_meters(&self, out: &mut MeterSnapshot) {
        let source = self.shared.meters.lock().unwrap_or_else(PoisonError::into_inner);
        copy_meter_snapshot(&source, out);
    }
}

fn copy_meter_snapshot(source: &MeterSnapshot, out: &mut MeterSnapshot) {
    out.tracks.retain(|id, _| source.tracks.contains_key(id));
    for (id, meter) in &source.tracks {
        out.tracks.entry(*id).or_default().copy_from(meter);
    }
    out.main.copy_from(&source.main);
}

impl Side {
    /// Creates the third-party instances `s` needs that the audio thread does not have yet.
    fn prepare(&mut self, s: &Session) -> Vec<PreparedInstance> {
        self.collect_garbage();
        let sr = s.sample_rate.as_f64() as f32;
        if (sr - self.rate).abs() > 0.5 {
            // The audio thread rebuilds its engine: everything is created anew.
            let keys: Vec<(TrackId, usize)> = self.shadow.keys().copied().collect();
            for k in keys {
                self.forget(k);
            }
            self.rate = sr;
        }
        let specs = soundcraft_mix::instance_specs(s);
        let stale: Vec<(TrackId, usize)> = self.shadow.keys().filter(|k| !specs.iter().any(|x| (x.track, x.slot) == **k)).copied().collect();
        for k in stale {
            self.forget(k);
        }
        let mut out = Vec::new();
        for spec in specs {
            let key = (spec.track, spec.slot);
            if self.shadow.get(&key).is_some_and(|(id, ch, _)| *id == spec.id && *ch == spec.ch) {
                continue;
            }
            self.forget(key);
            let made = soundcraft_mix::create_instance(&spec.id, spec.state.as_deref(), sr, BLOCK, spec.ch);
            self.shadow.insert(key, (spec.id.clone(), spec.ch, made.is_some()));
            if let Some(mut plugin) = made {
                if let Some(e) = plugin.editor() {
                    self.editors.insert(key, e);
                }
                out.push(PreparedInstance { track: spec.track, slot: spec.slot, id: spec.id, ch: spec.ch, plugin });
            }
        }
        out
    }
}

/// ITU stereo fold-down gains for each main channel of a session.
fn fold_gains(s: &Session) -> [(f32, f32); MAX_CHANNELS] {
    let mut g = [(0.0, 0.0); MAX_CHANNELS];
    let f = s.main_format();
    let sp = f.speakers();
    if sp.len() == f.channels() {
        for (o, x) in g.iter_mut().zip(sp.iter()) {
            *o = itu_stereo_fold(*x);
        }
    } else {
        // Ambisonics: W only, at -3 dB per side.
        g[0] = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
    }
    g
}

/// Open the default output device: f32 at the session rate if possible, with as many channels as
/// it offers up to `want_ch` (at least stereo when available).
fn open_device(want_rate: u32, want_ch: u16) -> Result<(cpal::Device, cpal::StreamConfig, String), String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    // The device picked in the audio setup, if it is connected; else the system default.
    #[allow(deprecated)]
    let wanted = record::preferred_output().and_then(|want| host.output_devices().ok()?.find(|d| d.name().is_ok_and(|n| n == want)));
    let device = wanted.or_else(|| host.default_output_device()).ok_or_else(|| "no default output device".to_string())?;
    #[allow(deprecated)]
    let name = device.name().unwrap_or_else(|_| "Audio device".into());
    // Prefer an f32 config at the session rate; else the device default (we resample).
    let want_ch = want_ch.max(2);
    let mut chosen: Option<cpal::StreamConfig> = None;
    if let Ok(configs) = device.supported_output_configs() {
        for c in configs {
            if c.sample_format() == cpal::SampleFormat::F32
                && c.min_sample_rate().0 <= want_rate
                && c.max_sample_rate().0 >= want_rate
                && c.channels() >= 2
                && c.channels() <= want_ch
                && chosen.as_ref().is_none_or(|x| c.channels() > x.channels)
            {
                chosen = Some(c.with_sample_rate(cpal::SampleRate(want_rate)).config());
            }
        }
    }
    let config = match chosen {
        Some(c) => c,
        None => {
            let d = device.default_output_config().map_err(|e| e.to_string())?;
            if d.sample_format() != cpal::SampleFormat::F32 {
                return Err(format!("unsupported device sample format {:?}", d.sample_format()));
            }
            d.config()
        }
    };
    Ok((device, config, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusable_meter_snapshot_updates_and_drops_stale_tracks() {
        let keep = TrackId(1);
        let stale = TrackId(2);
        let mut source = MeterSnapshot::default();
        source.tracks.insert(keep, StripMeter { peak: [0.5, 0.25], peaks: vec![0.5, 0.25], gain_reduction_db: 3.0 });
        source.main.peaks = vec![0.75, 0.5];
        let mut out = MeterSnapshot::default();
        out.tracks.insert(stale, StripMeter::default());
        copy_meter_snapshot(&source, &mut out);
        assert_eq!(out.tracks.len(), 1);
        assert_eq!(out.tracks[&keep], source.tracks[&keep]);
        assert_eq!(out.main, source.main);
    }

    #[test]
    fn audio_state_plays_and_stops_at_end() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(shared.position.load(Ordering::Relaxed) >= 1000);
        assert!(!shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn an_audition_sounds_while_stopped_and_then_falls_silent() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        // 0.05 s of a 24 kHz sound at half gain on a 48 kHz device: about 4800 frames.
        let sound = Arc::new(Audition { channels: vec![vec![0.8; 1200]], sample_rate: 24_000, gain: 0.5 });
        tx.send(Cmd::Audition(Arc::clone(&sound))).unwrap();
        let mut out = vec![0.0f32; 2 * 1024];
        st.fill(&mut out, 2);
        assert!((out[0] - 0.4).abs() < 1e-3 && (out[1] - 0.4).abs() < 1e-3, "both channels, at the gain: {:?}", &out[..2]);
        assert!(!shared.playing.load(Ordering::Relaxed), "the transport did not start");
        for _ in 0..4 {
            st.fill(&mut out, 2);
        }
        assert!(out.iter().all(|x| *x == 0.0), "silent after the end");
        assert!(st.voice.is_none());
        assert_eq!(Arc::strong_count(&sound), 1, "the audio thread let go (the test holds the last reference)");
    }

    #[test]
    fn the_listening_rate_never_applies_while_recording() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1500),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        let mut out = vec![0.0f32; 2 * 4_800];
        for _ in 0..10 {
            st.fill(&mut out, 2);
        }
        let listening = shared.position.load(Ordering::Relaxed);
        assert!(listening > 60_000, "1.5× while listening: {listening}");
        tx.send(Cmd::Recording(true)).unwrap();
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        for _ in 0..10 {
            st.fill(&mut out, 2);
        }
        let recording = shared.position.load(Ordering::Relaxed);
        // The mix renders up to one block ahead of what was played.
        assert!((48_000..=48_000 + BLOCK as i64).contains(&recording), "recording runs at the wall clock: {recording}");
    }

    #[test]
    fn short_device_callbacks_render_short_mix_blocks() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        let mut out = vec![0.0f32; 2 * 64];
        st.fill(&mut out, 2);
        assert_eq!(shared.position.load(Ordering::Relaxed), 64);
    }

    #[test]
    fn surround_main_maps_to_device_channels_or_folds_down() {
        use soundcraft_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};
        let mut s = soundcraft_model::Session::default();
        s.outputs[0].format = ChannelFormat::Surround51;
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Surround51, None);
        let chans: Vec<Vec<f32>> = (0..6).map(|c| vec![0.1 * (c + 1) as f32; 48_000]).collect();
        s.pool.insert(SourceId(5), Arc::new(SourceAudio::new(soundcraft_audio_io::AudioBuffer { sample_rate: 48_000, channels: chans })));
        let id = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "x", SourceId(5), 0, 0, 48_000));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&s), 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        // An 8-channel device: channels in order, the rest silent.
        let mut out = vec![0.0f32; 8 * 256];
        st.fill(&mut out, 8);
        let f = &out[8 * 100..8 * 101];
        for c in 0..6 {
            assert!((f[c] - 0.1 * (c + 1) as f32).abs() < 1e-5, "{f:?}");
        }
        assert!(f[6] == 0.0 && f[7] == 0.0);
        // A stereo device: ITU fold-down.
        let mut out = vec![0.0f32; 2 * 256];
        st.fill(&mut out, 2);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let l = (0.1 + h * 0.3 + h * 0.5f32).min(1.0);
        assert!((out[200] - l).abs() < 1e-4, "{} vs {l}", out[200]);
        assert_eq!(shared.meters.lock().unwrap().main.peaks.len(), 6);
    }

    struct Stateful(Box<dyn Plugin>);

    impl Plugin for Stateful {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
            self.0.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.0.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.0.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.0.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.0.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.0.process(io, frames);
        }
        fn save_state(&mut self) -> Option<Vec<u8>> {
            Some(b"blob".to_vec())
        }
    }

    #[test]
    fn instances_arrive_with_the_session_and_leave_through_the_return_channel() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("clap:test.fake"));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(soundcraft_model::Session::default()), 48_000.0, gtx);
        let plugin: Box<dyn Plugin> = Box::new(Stateful(soundcraft_dsp::create("gain").unwrap()));
        tx.send(Cmd::Session(Arc::clone(&s), vec![PreparedInstance { track: t, slot: 1, id: "clap:test.fake".into(), ch: 2, plugin }])).unwrap();
        let (rtx, rrx) = channel();
        tx.send(Cmd::CaptureStates(rtx)).unwrap();
        // Stopped: the callback only drains commands (adopting the instance) and answers.
        let mut out = vec![0.0f32; 256];
        st.fill(&mut out, 2);
        assert_eq!(rrx.try_recv().unwrap(), vec![(t, 1, b"blob".to_vec())]);
        assert!(grx.try_recv().is_err(), "nothing retired yet");
        // A session without the insert retires the instance to the return channel.
        tx.send(Cmd::Session(Arc::new(soundcraft_model::Session::default()), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
        // A sample-rate change rebuilds the engine without dropping plugins on this thread.
        tx.send(Cmd::Session(
            Arc::clone(&s),
            vec![PreparedInstance {
                track: t,
                slot: 1,
                id: "clap:test.fake".into(),
                ch: 2,
                plugin: Box::new(Stateful(soundcraft_dsp::create("gain").unwrap())),
            }],
        ))
        .unwrap();
        st.fill(&mut out, 2);
        let mut s44 = (*s).clone();
        s44.sample_rate = soundcraft_time::SampleRate::new(44_100).unwrap();
        tx.send(Cmd::Session(Arc::new(s44), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
    }

    #[test]
    fn side_creates_each_missing_instance_once() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let (_gtx, grx) = channel();
        let mut side = Side { shadow: HashMap::new(), rate: 48_000.0, editors: HashMap::new(), garbage: grx };
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("clap:does.not.exist"));
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("gain"));
        assert!(side.prepare(&s).is_empty(), "the plugin is not installed; built-ins stay with the engine");
        assert_eq!(side.shadow.get(&(t, 0)), Some(&("clap:does.not.exist".to_string(), 1, false)));
        assert!(side.prepare(&s).is_empty());
        assert_eq!(side.shadow.len(), 1, "failures are remembered, not retried every update");
        s.track_mut(t).unwrap().mixer.inserts[0] = None;
        side.prepare(&s);
        assert!(side.shadow.is_empty());
    }

    #[test]
    fn looping_wraps() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: Some(Range::new(0, 1000)) }).unwrap();
        let mut out = vec![0.0f32; 20_000];
        st.fill(&mut out, 2);
        let p = shared.position.load(Ordering::Relaxed);
        assert!(p <= 1000, "position {p} should stay inside the loop");
        assert!(shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn armed_tracks_are_heard_and_metered_after_playback_stops() {
        use soundcraft_model::{ChannelFormat, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.record_arm = true;
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            rate: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            device_lost: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);
        let ring = record::InputRing::new(1, 48_000);
        tx.send(Cmd::Input(Arc::clone(&ring))).unwrap();
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(!shared.playing.load(Ordering::Relaxed));
        ring.push(&[0.5; 4096]);
        st.fill(&mut out, 2);
        assert!(out.iter().any(|x| x.abs() > 0.1), "the input is heard");
        assert!(shared.meters.lock().unwrap().tracks.get(&t).is_some_and(|m| m.peak[0] > 0.1), "and metered");
    }
}
