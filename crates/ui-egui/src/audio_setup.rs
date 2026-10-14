//! Setup > Audio Interface: pick the input and output device, see a live meter per input, let
//! TalkCraft find which input each speaker's microphone is on ("please speak now"), and run a
//! soundcheck that says per microphone whether to turn the gain up or down.

use crate::SoundApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use egui::{Color32, RichText, vec2};
use serde_json::json;
use soundcraft_model::{Route, TrackId};
use std::time::{Duration, Instant};

/// How long a speaker is listened to when detecting their input.
const DETECT: Duration = Duration::from_secs(3);
/// How long the soundcheck listens.
const SOUNDCHECK: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default)]
pub struct AudioSetup {
    /// Live peak per input (linear), decaying.
    pub meters: Vec<f32>,
    /// Listening for one track's microphone: (track, since, energy per input).
    detect: Option<(TrackId, Instant, Vec<f32>)>,
    /// Soundcheck running: (since, highest peak per input).
    check: Option<(Instant, Vec<f32>)>,
    /// Last soundcheck: per speaker track, (name, input, peak dBFS).
    pub results: Vec<(String, Option<usize>, f32)>,
    /// The device lists and when they were read: asking Core Audio every frame is expensive.
    devices: Option<(Instant, DeviceLists)>,
}

type DeviceLists = (Vec<soundcraft_playback::record::DeviceInfo>, Vec<soundcraft_playback::record::DeviceInfo>);

/// The 0-based input of `In N` routes.
pub fn input_index(route: &Route) -> Option<usize> {
    match route {
        Route::Hardware(h) => h.trim_start_matches("In ").split(['-', ' ']).next()?.parse::<usize>().ok()?.checked_sub(1),
        _ => None,
    }
}

/// The input that carried the speech: clearly the loudest (twice the energy of the next) and
/// above a floor; `None` when unclear or silent.
pub fn detect_channel(energy: &[f32]) -> Option<usize> {
    let mut idx: Vec<usize> = (0..energy.len()).collect();
    idx.sort_by(|a, b| energy.get(*b).unwrap_or(&0.0).total_cmp(energy.get(*a).unwrap_or(&0.0)));
    let best = *idx.first()?;
    let e1 = energy.get(best).copied().unwrap_or(0.0);
    let e2 = idx.get(1).and_then(|i| energy.get(*i)).copied().unwrap_or(0.0);
    (e1 > 1e-4 && e1 >= 2.0 * e2).then_some(best)
}

/// What to do about a microphone whose speech peaked at `peak_db` dBFS.
#[derive(Debug, Clone, PartialEq)]
pub enum Advice {
    NoSignal,
    TooQuiet { raise_db: f32 },
    Good,
    TooLoud { lower_db: f32 },
    Clipping,
}

/// Speech should peak around −12 dBFS: room above for laughter, well above the noise floor.
pub fn advice(peak_db: f32) -> Advice {
    if peak_db < -50.0 {
        Advice::NoSignal
    } else if peak_db >= -0.5 {
        Advice::Clipping
    } else if peak_db < -20.0 {
        Advice::TooQuiet { raise_db: (-12.0 - peak_db).round() }
    } else if peak_db > -6.0 {
        Advice::TooLoud { lower_db: (peak_db + 12.0).round() }
    } else {
        Advice::Good
    }
}

fn advice_text(a: &Advice) -> (String, Color32) {
    let de = crate::i18n::german();
    match a {
        Advice::NoSignal => (
            if de {
                "Kein Signal: Kabel, Eingang und 48-V-Phantomspeisung prüfen"
            } else {
                "No signal: check the cable, the input and 48 V phantom power"
            }
            .into(),
            Color32::from_rgb(240, 110, 110),
        ),
        Advice::TooQuiet { raise_db } => (
            if de {
                format!("Zu leise: Gain am Interface um etwa {raise_db:.0} dB aufdrehen")
            } else {
                format!("Too quiet: turn the interface gain up by about {raise_db:.0} dB")
            },
            Color32::from_rgb(240, 180, 80),
        ),
        Advice::Good => (if de { "Gut" } else { "Good" }.into(), Color32::from_rgb(120, 230, 140)),
        Advice::TooLoud { lower_db } => (
            if de {
                format!("Etwas zu laut: Gain um etwa {lower_db:.0} dB zurückdrehen")
            } else {
                format!("A bit loud: turn the gain down by about {lower_db:.0} dB")
            },
            Color32::from_rgb(240, 180, 80),
        ),
        Advice::Clipping => (
            if de { "Übersteuert: Gain deutlich zurückdrehen" } else { "Clipping: turn the gain down clearly" }.into(),
            Color32::from_rgb(240, 110, 110),
        ),
    }
}

/// What kind of podcast hardware a device is, by its name.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// An audio interface whose microphone inputs come in order: mic 1 on In 1, mic 2 on In 2 …
    Interface { mics: usize },
    /// A podcast mixer/recorder: its USB channel layout depends on mode and firmware.
    Mixer,
    /// A USB microphone: one speaker per device.
    UsbMic,
}

/// A known device: display name and kind.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: &'static str,
    pub kind: Kind,
}

/// Known podcast hardware, matched by (lower-case) parts of the device name.
const PROFILES: &[(&[&str], &str, Kind)] = &[
    (&["rodecaster", "rødecaster"], "RØDECaster", Kind::Mixer),
    (&["podtrak"], "Zoom PodTrak", Kind::Mixer),
    (&["mixcast"], "Tascam Mixcast", Kind::Mixer),
    (&["go xlr", "goxlr"], "GoXLR", Kind::Mixer),
    (&["scarlett 2i2", "scarlett solo"], "Focusrite Scarlett", Kind::Interface { mics: 2 }),
    (&["scarlett 4i4"], "Focusrite Scarlett 4i4", Kind::Interface { mics: 2 }),
    (&["scarlett 8i6", "scarlett 18i8", "scarlett 18i20"], "Focusrite Scarlett", Kind::Interface { mics: 4 }),
    (&["motu m2"], "MOTU M2", Kind::Interface { mics: 2 }),
    (&["motu m4"], "MOTU M4", Kind::Interface { mics: 2 }),
    (&["ssl 2+", "ssl 2"], "SSL 2", Kind::Interface { mics: 2 }),
    (&["evo4", "evo 4"], "Audient EVO 4", Kind::Interface { mics: 2 }),
    (&["evo8", "evo 8"], "Audient EVO 8", Kind::Interface { mics: 4 }),
    (&["id4"], "Audient iD4", Kind::Interface { mics: 1 }),
    (&["id14", "id24"], "Audient iD", Kind::Interface { mics: 2 }),
    (&["umc202", "umc204", "umc404"], "Behringer U-Phoria", Kind::Interface { mics: 2 }),
    (&["volt 2", "volt 276"], "UA Volt", Kind::Interface { mics: 2 }),
    (&["volt 4", "volt 476"], "UA Volt", Kind::Interface { mics: 4 }),
    (&["babyface"], "RME Babyface", Kind::Interface { mics: 2 }),
    (&["shure mv7", "mv7"], "Shure MV7", Kind::UsbMic),
    (&["nt-usb", "podmic usb", "rode podmic"], "RØDE USB mic", Kind::UsbMic),
    (&["yeti"], "Blue Yeti", Kind::UsbMic),
    (&["at2020usb", "atr2100"], "Audio-Technica USB mic", Kind::UsbMic),
    (&["samson q2u", "q9u"], "Samson USB mic", Kind::UsbMic),
];

/// The profile of a device called `name`, if it is known podcast hardware.
pub fn profile(name: &str) -> Option<Profile> {
    let n = name.to_lowercase();
    PROFILES.iter().find(|(keys, _, _)| keys.iter().any(|k| n.contains(k))).map(|(_, name, kind)| Profile { name, kind: kind.clone() })
}

/// The tip shown for a known device.
fn profile_tip(p: &Profile, speakers: usize) -> String {
    let de = crate::i18n::german();
    match &p.kind {
        Kind::Interface { mics } if speakers > *mics => {
            if de {
                format!("{}: {mics} Mikrofoneingänge – für {speakers} Sprecher reicht das nicht ganz.", p.name)
            } else {
                format!("{}: {mics} microphone inputs, fewer than the {speakers} speakers.", p.name)
            }
        }
        Kind::Interface { .. } => {
            if de {
                format!("{} erkannt: Mikrofon 1 auf Eingang 1, Mikrofon 2 auf Eingang 2 …", p.name)
            } else {
                format!("{} recognised: mic 1 on input 1, mic 2 on input 2 …", p.name)
            }
        }
        Kind::Mixer => {
            if de {
                format!(
                    "{} erkannt: Für getrennte Spuren das Gerät in den Mehrspur-Modus (Multitrack) schalten. Die Kanalbelegung hängt vom Modus ab – deshalb pro Sprecher „Erkennen“ nutzen.",
                    p.name
                )
            } else {
                format!(
                    "{} recognised: switch it to multitrack mode for separate tracks. Its channel layout depends on the mode, so use “Detect” for each speaker.",
                    p.name
                )
            }
        }
        Kind::UsbMic => {
            if de {
                format!(
                    "{} erkannt: ein USB-Mikrofon nimmt nur eine Person auf. Für mehrere USB-Mikrofone in „Audio-MIDI-Setup“ ein Aggregatgerät anlegen und dieses hier wählen.",
                    p.name
                )
            } else {
                format!(
                    "{} recognised: a USB microphone records one person. For several USB mics, create an aggregate device in Audio MIDI Setup and pick it here.",
                    p.name
                )
            }
        }
    }
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-6).log10()
}

/// Reads the input peaks and moves detection and soundcheck along. Called every frame while the
/// window is open.
fn update(app: &mut SoundApp) {
    let Some(rec) = &app.recorder else { return };
    let peaks = rec.levels.take();
    let st = &mut app.extra.audio_setup;
    st.meters.resize(peaks.len(), 0.0);
    for (m, p) in st.meters.iter_mut().zip(&peaks) {
        *m = (*m * 0.85).max(*p);
    }
    if let Some((_, _, energy)) = &mut st.detect {
        energy.resize(peaks.len(), 0.0);
        for (e, p) in energy.iter_mut().zip(&peaks) {
            *e += p * p;
        }
    }
    if let Some((_, max)) = &mut st.check {
        max.resize(peaks.len(), 0.0);
        for (m, p) in max.iter_mut().zip(&peaks) {
            *m = m.max(*p);
        }
    }
    // Finished listening for a speaker: assign the input that carried the voice.
    if let Some((track, since, energy)) = st.detect.clone()
        && since.elapsed() >= DETECT
    {
        st.detect = None;
        let name = app.engine.session().track(track).map(|t| t.name.clone()).unwrap_or_default();
        app.ui.status = match detect_channel(&energy) {
            Some(i) => match app.run("track.input", json!({"tracks": [track.0], "input": format!("In {}", i + 1)})) {
                Ok(_) => format!("{name}: {} {}", tr("microphone on input"), i + 1),
                Err(e) => e,
            },
            None => format!("{name}: {}", tr("no clear voice on any input — speak louder or check the cable")),
        };
    }
    if let Some((since, max)) = app.extra.audio_setup.check.clone()
        && since.elapsed() >= SOUNDCHECK
    {
        app.extra.audio_setup.check = None;
        let s = app.engine.session();
        app.extra.audio_setup.results = soundcraft_engine::podcast::record_targets(s)
            .into_iter()
            .filter_map(|t| s.track(t))
            .map(|t| {
                let i = input_index(&t.mixer.input).or(Some(0));
                (t.name.clone(), i, db(i.and_then(|i| max.get(i)).copied().unwrap_or(0.0)))
            })
            .collect();
    }
}

fn meter(ui: &mut egui::Ui, level: f32, width: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(width, 10.0), egui::Sense::hover());
    let d = db(level);
    let k = ((d + 60.0) / 60.0).clamp(0.0, 1.0);
    let col = if d >= -0.5 {
        Color32::from_rgb(240, 80, 80)
    } else if d > -6.0 {
        Color32::from_rgb(240, 190, 70)
    } else {
        Color32::from_rgb(110, 220, 130)
    };
    ui.painter().rect_filled(r, 2.0, Color32::from_gray(40));
    ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(r.width() * k, r.height())), 2.0, col);
    // The −12 dB target mark.
    let x = r.min.x + r.width() * 0.8;
    ui.painter().line_segment([egui::pos2(x, r.min.y), egui::pos2(x, r.max.y)], egui::Stroke::new(1.0, Color32::WHITE));
}

fn device_combo(ui: &mut egui::Ui, id: &str, current: &mut Option<String>, devices: &[soundcraft_playback::record::DeviceInfo]) -> bool {
    let label = |d: &soundcraft_playback::record::DeviceInfo| {
        format!("{} ({} ch, {} Hz){}", d.name, d.channels, d.sample_rate, if d.is_default { " ★" } else { "" })
    };
    let shown = current.as_ref().map_or_else(|| tr("System default").to_string(), Clone::clone);
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).width(330.0).selected_text(shown).show_ui(ui, |ui| {
        if ui.selectable_label(current.is_none(), tr("System default")).clicked() {
            *current = None;
            changed = true;
        }
        for d in devices {
            if ui.selectable_label(current.as_deref() == Some(d.name.as_str()), label(d)).clicked() {
                *current = Some(d.name.clone());
                changed = true;
            }
        }
    });
    changed
}

/// Setup > Audio Interface.
pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_playback_engine {
        return;
    }
    app.ensure_input();
    update(app);
    let t = Tokens::current();
    let fresh = app.extra.audio_setup.devices.as_ref().is_some_and(|(t, _)| t.elapsed() < Duration::from_secs(2));
    if !fresh {
        app.extra.audio_setup.devices = Some((Instant::now(), soundcraft_playback::record::list_devices()));
    }
    let (outs, ins) = app.extra.audio_setup.devices.as_ref().map(|(_, l)| l.clone()).unwrap_or_default();
    let mut open = true;
    let mut out_dev = app.ui.output_device.clone();
    let mut in_dev = app.ui.input_device.clone();
    let (mut out_changed, mut in_changed) = (false, false);
    let mut detect: Option<TrackId> = None;
    let mut assign: Option<(TrackId, usize)> = None;
    let mut soundcheck = false;
    let mut in_order: Option<usize> = None;
    let channels = app.recorder.as_ref().map_or(0, |r| r.channels);
    let speakers: Vec<(TrackId, String, Option<usize>)> = {
        let s = app.engine.session();
        soundcraft_engine::podcast::record_targets(s)
            .into_iter()
            .filter_map(|t| s.track(t))
            .map(|t| (t.id, t.name.clone(), input_index(&t.mixer.input)))
            .collect()
    };
    egui::Window::new(tr("Audio Interface")).open(&mut open).default_size(vec2(560.0, 520.0)).resizable(true).show(ctx, |ui| {
        let st = &app.extra.audio_setup;
        ui.label(RichText::new(tr("Devices")).strong());
        egui::Grid::new("audio_devices").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label(tr("Microphones (input)"));
            in_changed = device_combo(ui, "in_dev", &mut in_dev, &ins);
            ui.end_row();
            ui.label(tr("Headphones/speakers (output)"));
            out_changed = device_combo(ui, "out_dev", &mut out_dev, &outs);
            ui.end_row();
        });
        let now = app
            .recorder
            .as_ref()
            .map_or_else(|| tr("No input open").to_string(), |r| format!("{} · {} ch · {} Hz", r.device_name, r.channels, r.sample_rate));
        ui.label(RichText::new(now).small().color(t.text_dim));
        ui.label(RichText::new(tr("TalkCraft remembers the interface and uses it whenever it is connected.")).small().color(t.text_dim));
        if let Some(p) = app.recorder.as_ref().and_then(|r| profile(&r.device_name)) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(profile_tip(&p, speakers.len())).color(Color32::from_rgb(140, 200, 240)));
                if let Kind::Interface { mics } = p.kind
                    && ui.button(tr("Assign in order")).on_hover_text(tr("First speaker on input 1, the next on input 2 …")).clicked()
                {
                    in_order = Some(mics);
                }
                if p.kind == Kind::UsbMic && cfg!(target_os = "macos") && ui.button(tr("Open Audio MIDI Setup")).clicked() {
                    let _ = std::process::Command::new("open").args(["-a", "Audio MIDI Setup"]).spawn();
                }
            });
        }
        ui.separator();

        ui.label(RichText::new(tr("Inputs")).strong());
        ui.horizontal_wrapped(|ui| {
            for (i, m) in st.meters.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("In {}", i + 1)).small());
                    meter(ui, *m, 60.0);
                });
            }
        });
        ui.separator();

        ui.label(RichText::new(tr("Microphones")).strong());
        egui::Grid::new("mic_assign").num_columns(4).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            for (id, name, input) in &speakers {
                ui.label(name);
                let shown = input.map_or_else(|| "–".to_string(), |i| format!("In {}", i + 1));
                egui::ComboBox::from_id_salt(("mic_in", id.0)).width(70.0).selected_text(shown).show_ui(ui, |ui| {
                    for i in 0..channels.max(1) {
                        if ui.selectable_label(*input == Some(i), format!("In {}", i + 1)).clicked() {
                            assign = Some((*id, i));
                        }
                    }
                });
                meter(ui, input.and_then(|i| st.meters.get(i)).copied().unwrap_or(0.0), 120.0);
                match &st.detect {
                    Some((t2, since, _)) if t2 == id => {
                        let left = DETECT.saturating_sub(since.elapsed()).as_secs_f32();
                        ui.label(RichText::new(format!("{} ({left:.0} s)", tr("Please speak now…"))).color(Color32::from_rgb(240, 200, 90)));
                    }
                    _ => {
                        if ui.add_enabled(st.detect.is_none() && st.check.is_none(), egui::Button::new(tr("Detect"))).on_hover_text(tr("The speaker talks for 3 s; TalkCraft picks the input their voice is on")).clicked() {
                            detect = Some(*id);
                        }
                    }
                }
                ui.end_row();
            }
        });
        ui.separator();

        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Soundcheck")).strong());
            match &st.check {
                Some((since, _)) => {
                    let left = SOUNDCHECK.saturating_sub(since.elapsed()).as_secs_f32();
                    ui.label(RichText::new(format!("{} ({left:.0} s)", tr("Everyone talk normally now…"))).color(Color32::from_rgb(240, 200, 90)));
                }
                None => {
                    if ui.add_enabled(st.detect.is_none(), egui::Button::new(tr("Start (10 s)"))).clicked() {
                        soundcheck = true;
                    }
                }
            }
        });
        for (name, input, peak) in &st.results {
            let (text, col) = advice_text(&advice(*peak));
            ui.horizontal(|ui| {
                ui.label(format!("{name} (In {}): {peak:.0} dBFS", input.map_or(0, |i| i + 1)));
                ui.label(RichText::new(text).color(col));
            });
        }
        ui.label(RichText::new(tr("Aim for speech peaks around −12 dBFS (the white mark). Monitor through the interface's direct monitoring: no delay in the headphones.")).small().color(t.text_dim));
    });
    app.ui.show_playback_engine = open;
    if app.extra.audio_setup.detect.is_some() || app.extra.audio_setup.check.is_some() || !app.extra.audio_setup.meters.is_empty() {
        ctx.request_repaint_after(Duration::from_millis(50));
    }
    if in_changed || out_changed {
        app.ui.input_device = in_dev;
        app.ui.output_device = out_dev;
        soundcraft_playback::record::set_preferred_devices(app.ui.output_device.clone(), app.ui.input_device.clone());
        if out_changed && !app.engine.transport.playing {
            app.replace_player();
        }
        if in_changed && !app.engine.transport.recording {
            app.recorder = None;
            app.recorder_failed = false;
            app.extra.audio_setup.meters.clear();
        }
    }
    if let Some(mics) = in_order {
        for (n, (id, _, _)) in speakers.iter().enumerate().take(mics.max(1)) {
            let _ = app.run("track.input", json!({"tracks": [id.0], "input": format!("In {}", n + 1)}));
        }
    }
    if let Some((track, i)) = assign {
        let _ = app.run("track.input", json!({"tracks": [track.0], "input": format!("In {}", i + 1)}));
    }
    if let Some(track) = detect {
        app.extra.audio_setup.detect = Some((track, Instant::now(), Vec::new()));
    }
    if soundcheck {
        app.extra.audio_setup.results.clear();
        app.extra.audio_setup.check = Some((Instant::now(), Vec::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_parse_from_routes() {
        assert_eq!(input_index(&Route::Hardware("In 1".into())), Some(0));
        assert_eq!(input_index(&Route::Hardware("In 3-4".into())), Some(2));
        assert_eq!(input_index(&Route::None), None);
    }

    #[test]
    fn the_voice_is_found_on_the_clearly_loudest_input() {
        assert_eq!(detect_channel(&[0.001, 0.9, 0.05, 0.0]), Some(1));
        assert_eq!(detect_channel(&[0.5, 0.4]), None, "two inputs equally loud: unclear (bleed or a stereo mic)");
        assert_eq!(detect_channel(&[0.0, 0.00001]), None, "silence");
        assert_eq!(detect_channel(&[]), None);
    }

    #[test]
    fn podcast_hardware_is_recognised_by_name() {
        assert_eq!(profile("Scarlett 2i2 USB").map(|p| p.kind), Some(Kind::Interface { mics: 2 }));
        assert_eq!(profile("RØDECaster Pro II Multitrack").map(|p| p.kind), Some(Kind::Mixer));
        assert_eq!(profile("ZOOM PodTrak P4").map(|p| p.name), Some("Zoom PodTrak"));
        assert_eq!(profile("Shure MV7").map(|p| p.kind), Some(Kind::UsbMic));
        assert_eq!(profile("MacBook Pro-Mikrofon"), None);
    }

    #[test]
    fn soundcheck_advice_follows_the_peak() {
        assert_eq!(advice(-70.0), Advice::NoSignal);
        assert_eq!(advice(-30.0), Advice::TooQuiet { raise_db: 18.0 });
        assert_eq!(advice(-12.0), Advice::Good);
        assert_eq!(advice(-3.0), Advice::TooLoud { lower_db: 9.0 });
        assert_eq!(advice(0.0), Advice::Clipping);
    }
}
