//! Ready to record? Before a take: every speaker's microphone has signal and none clips, the
//! remote guests are connected, and the disk holds enough. While recording: a big "On Air" with
//! the running time. Record with the countdown on counts 3-2-1 first (Settings › Audio).

use crate::SoundApp;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use egui::{Color32, RichText, vec2};
use std::time::{Duration, Instant};

/// How long Record counts down.
pub const COUNTDOWN: Duration = Duration::from_secs(3);
/// A microphone counts as heard when it reached this within the last `HEARD_FOR`.
const HEARD_DB: f32 = -50.0;
const HEARD_FOR: Duration = Duration::from_secs(10);
/// A peak at or above this is clipping.
const CLIP_DB: f32 = -0.5;
/// Free disk space is asked for this often.
const DISK_EVERY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default)]
pub struct Readiness {
    /// Per input: when it was last heard, and when it last clipped.
    heard: Vec<Option<Instant>>,
    clipped: Vec<Option<Instant>>,
    /// Free bytes where takes are written, and when that was asked.
    free: Option<(Instant, Option<u64>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Light {
    Green,
    Yellow,
    Red,
}

impl Light {
    fn color(self) -> Color32 {
        match self {
            Light::Green => Color32::from_rgb(60, 175, 90),
            Light::Yellow => Color32::from_rgb(225, 170, 40),
            Light::Red => Color32::from_rgb(210, 60, 55),
        }
    }
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-6).log10()
}

/// Per frame: remembers which inputs were heard or clipped, and starts a counted-down take.
pub fn tick(app: &mut SoundApp) {
    let now = Instant::now();
    // The readiness check listens to the microphones whenever recording is on the menu.
    if app.ui.workspace.records() && app.player.is_some() {
        app.ensure_input();
    }
    let r = &mut app.extra.readiness;
    let n = app.input_peaks.len();
    r.heard.resize(n, None);
    r.clipped.resize(n, None);
    for (i, p) in app.input_peaks.iter().enumerate() {
        let d = db(*p);
        if d >= HEARD_DB
            && let Some(h) = r.heard.get_mut(i)
        {
            *h = Some(now);
        }
        if d >= CLIP_DB
            && let Some(c) = r.clipped.get_mut(i)
        {
            *c = Some(now);
        }
    }
    if let Some((at, _)) = &app.countdown
        && now >= *at
        && let Some((_, tracks)) = app.countdown.take()
    {
        app.start_recording(tracks);
    }
}

/// Free bytes on the disk holding `dir` (`df`; `None` where that is not available).
fn free_bytes(dir: &std::path::Path) -> Option<u64> {
    #[cfg(unix)]
    {
        let out = std::process::Command::new("df").arg("-Pk").arg(dir).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().nth(1)?;
        let kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
        Some(kb.saturating_mul(1024))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

/// Where takes are written: next to the saved session, else the autosave folder.
fn take_dir(app: &SoundApp) -> std::path::PathBuf {
    app.engine
        .path
        .as_ref()
        .and_then(|p| std::path::Path::new(p).parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| app.autosave_dir.clone().unwrap_or_else(std::env::temp_dir))
}

/// The checks, worst first: (light, text).
pub fn checks(app: &mut SoundApp) -> Vec<(Light, String)> {
    let now = Instant::now();
    let mut out = Vec::new();
    let s = app.engine.session();
    let targets = soundcraft_engine::podcast::record_targets(s);
    let guests: Vec<crate::remote::Guest> = crate::remote::guests(app);
    let r = &app.extra.readiness;
    if app.recorder.is_none() && targets.iter().any(|t| s.track(*t).is_some_and(|t| !t.name.starts_with("Ferngast"))) {
        out.push((Light::Red, tr("No audio input is open (Setup › Audio Interface)").to_string()));
    }
    for id in &targets {
        let Some(t) = s.track(*id) else { continue };
        if let Some(guest) = t.name.strip_prefix(crate::remote::TRACK_PREFIX) {
            match guests.iter().find(|g| g.name == guest) {
                Some(g) if g.connected => out.push((Light::Green, trf("{} is connected", &[&t.name]))),
                _ => out.push((Light::Red, trf("{} is not connected", &[&t.name]))),
            }
            continue;
        }
        if app.recorder.is_none() {
            continue;
        }
        let Some(input) = crate::audio_setup::input_index(&t.mixer.input) else { continue };
        let recent = |v: &[Option<Instant>]| v.get(input).copied().flatten().is_some_and(|at| now.duration_since(at) < HEARD_FOR);
        if recent(&r.clipped) {
            out.push((Light::Red, trf("{} clips: turn the gain down", &[&t.name])));
        } else if recent(&r.heard) {
            out.push((Light::Green, trf("{}: microphone has signal", &[&t.name])));
        } else {
            out.push((Light::Yellow, trf("{}: no signal yet, please say something", &[&t.name])));
        }
    }
    // Disk: 24-bit mono per recorded track (remote guests arrive later, as files).
    let due = r.free.as_ref().is_none_or(|(at, _)| now.duration_since(*at) > DISK_EVERY);
    if due {
        let free = free_bytes(&take_dir(app));
        app.extra.readiness.free = Some((now, free));
    }
    let local = targets.iter().filter(|t| s.track(**t).is_some_and(|t| !t.name.starts_with(crate::remote::TRACK_PREFIX))).count().max(1);
    if let Some((_, Some(free))) = app.extra.readiness.free {
        let per_hour = u64::from(s.sample_rate.hz()) * 3 * 3600 * local as u64;
        let hours = free as f64 / per_hour.max(1) as f64;
        let light = if hours < 1.0 {
            Light::Red
        } else if hours < 3.0 {
            Light::Yellow
        } else {
            Light::Green
        };
        let h = hours.min(99.0);
        out.push((light, trf("Disk space for {} h {} min", &[&(h as u64), &((h.fract() * 60.0) as u64)])));
    }
    out.sort_by_key(|(l, _)| std::cmp::Reverse(*l));
    out
}

/// Toolbar block in the recording modes: the readiness light before a take, "On Air" during it.
pub fn toolbar(app: &mut SoundApp, ui: &mut egui::Ui) {
    let t = Tokens::current();
    if app.engine.transport.recording {
        let secs = app.engine.session().sample_rate.seconds(app.position().saturating_sub(app.record_start).max(0)).max(0.0) as u64;
        let (r, _) = ui.allocate_exact_size(vec2(150.0, 64.0), egui::Sense::hover());
        let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
        ui.painter().rect_filled(r, 4.0, Color32::from_rgb(170, 30, 30));
        ui.painter().text(
            egui::pos2(r.center().x, r.min.y + 18.0),
            egui::Align2::CENTER_CENTER,
            if blink { "● ON AIR" } else { "  ON AIR" },
            egui::FontId::proportional(16.0),
            Color32::WHITE,
        );
        ui.painter().text(
            egui::pos2(r.center().x, r.min.y + 44.0),
            egui::Align2::CENTER_CENTER,
            format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60),
            egui::FontId::monospace(20.0),
            Color32::WHITE,
        );
        ui.ctx().request_repaint_after(Duration::from_millis(250));
        return;
    }
    let list = checks(app);
    let worst = list.first().map_or(Light::Green, |(l, _)| *l);
    let (r, resp) = ui.allocate_exact_size(vec2(118.0, 64.0), egui::Sense::hover());
    ui.painter().rect(r, 4.0, t.counter_bg, egui::Stroke::new(1.0, t.counter_border), egui::StrokeKind::Inside);
    ui.painter().circle_filled(egui::pos2(r.min.x + 18.0, r.center().y - 8.0), 8.0, worst.color());
    let label = match worst {
        Light::Green => tr("Ready"),
        Light::Yellow => tr("Check"),
        Light::Red => tr("Not ready"),
    };
    ui.painter().text(egui::pos2(r.min.x + 32.0, r.center().y - 8.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), t.text);
    let problems = list.iter().filter(|(l, _)| *l != Light::Green).count();
    let sub = if problems == 0 { tr("all checks passed").to_string() } else { trf("{} to check", &[&problems]) };
    ui.painter().text(egui::pos2(r.min.x + 10.0, r.center().y + 14.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(10.5), t.text_dim);
    resp.on_hover_ui(|ui| {
        ui.label(RichText::new(tr("Ready to record?")).strong());
        for (l, text) in &list {
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(l.color()));
                ui.label(text);
            });
        }
    });
    ui.ctx().request_repaint_after(Duration::from_millis(300));
}

/// The big 3-2-1 over the window while a take is about to start.
pub fn countdown_overlay(app: &SoundApp, ctx: &egui::Context) {
    let Some((at, _)) = &app.countdown else { return };
    let left = at.saturating_duration_since(Instant::now()).as_secs_f32().ceil().max(1.0) as u32;
    egui::Area::new(egui::Id::new("countdown")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(220.0, 220.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 105.0, Color32::from_rgba_unmultiplied(170, 30, 30, 230));
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, left.to_string(), egui::FontId::proportional(120.0), Color32::WHITE);
    });
    ctx.request_repaint_after(Duration::from_millis(50));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use serde_json::json;

    fn app() -> SoundApp {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 2})).unwrap();
        app
    }

    fn pump(app: &mut SoundApp) {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).textures_delta.clear();
    }

    #[test]
    fn record_counts_down_first_and_record_again_cancels() {
        let mut app = app();
        app.ui.countdown = true;
        app.engine.execute("transport.record", &json!({})).unwrap();
        pump(&mut app);
        assert!(app.countdown.is_some(), "counting down, not recording yet");
        assert!(!app.engine.transport.recording);
        app.engine.execute("transport.record", &json!({})).unwrap();
        pump(&mut app);
        assert!(app.countdown.is_none(), "cancelled");
        // Started when the count reaches zero (no input here, so it reports why it cannot record).
        app.engine.execute("transport.record", &json!({})).unwrap();
        pump(&mut app);
        if let Some((at, _)) = &mut app.countdown {
            *at = Instant::now() - Duration::from_millis(1);
        }
        pump(&mut app);
        assert!(app.countdown.is_none(), "the take was started");
    }

    #[test]
    fn without_an_input_the_check_says_why_and_heard_microphones_turn_green() {
        let mut app = app();
        let list = checks(&mut app);
        assert_eq!(list.first().map(|(l, _)| *l), Some(Light::Red), "{list:?}");
        // With levels coming in: input 1 heard, input 2 clipping.
        app.input_peaks = vec![0.1, 1.0];
        tick(&mut app);
        let r = &app.extra.readiness;
        assert!(r.heard.first().copied().flatten().is_some() && r.clipped.get(1).copied().flatten().is_some());
        assert!(r.clipped.first().copied().flatten().is_none(), "input 1 does not clip");
    }
}
