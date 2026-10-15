//! The On Air palette: a big display for the studio, e.g. on a second screen. A lamp (On Air while
//! recording, the 3-2-1 before it, else ready or what to check), the running time large, the
//! current topic against its plan and the next one. In a window of its own it also has big buttons
//! for a touchscreen. The remote control on a phone shows the same (`status`).

use crate::SoundApp;
use crate::i18n::{tr, trf};
use crate::readiness::Light;
use crate::theme::Tokens;
use egui::{Color32, FontId, RichText, vec2};
use serde_json::{Value, json};

/// What is going on in the studio right now.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub recording: bool,
    /// Seconds left of the 3-2-1.
    pub countdown: Option<u32>,
    pub take_s: f64,
    /// Current topic: title, seconds spent, seconds planned (0 = open).
    pub topic: Option<(String, f64, f64)>,
    pub next: Option<String>,
    pub light: Light,
    /// What to check before recording (not green).
    pub checks: Vec<String>,
}

pub fn status(app: &mut SoundApp) -> Live {
    let list = crate::readiness::checks(app);
    let light = list.first().map_or(Light::Green, |(l, _)| *l);
    Live {
        recording: app.engine.transport.recording,
        countdown: app.countdown.as_ref().map(|(at, _)| at.saturating_duration_since(std::time::Instant::now()).as_secs_f64().ceil() as u32),
        take_s: app.take_seconds(),
        topic: crate::rundown_window::current_topic(app),
        next: crate::rundown_window::next_title(app),
        light,
        checks: list.into_iter().filter(|(l, _)| *l != Light::Green).map(|(_, t)| t).collect(),
    }
}

pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// `window.onair`: opens or closes the palette.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    crate::palettes::toggle(app, "onair", p)
}

/// `studio.status`: the same as JSON (for agents and the remote control).
pub fn status_json(app: &mut SoundApp) -> Value {
    let l = status(app);
    json!({
        "recording": l.recording,
        "countdown": l.countdown,
        "take": clock(l.take_s),
        "take_s": l.take_s,
        "topic": l.topic.as_ref().map(|(t, spent, planned)| json!({"title": t, "spent": clock(*spent), "planned": if *planned > 0.0 { clock(*planned) } else { String::new() }, "over": *planned > 0.0 && spent > planned})),
        "next": l.next,
        "light": match l.light { Light::Green => "green", Light::Yellow => "yellow", Light::Red => "red" },
        "checks": l.checks,
    })
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !crate::palettes::is_open(app, "onair") {
        return;
    }
    let live = status(app);
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
    let mut run: Option<&str> = None;
    crate::palettes::show(app, ctx, "onair", tr("On Air"), vec2(420.0, 300.0), |app, ui, own_window| {
        let t = Tokens::current();
        let w = ui.available_width();
        // Everything grows with the window: readable from across the room.
        let scale = (w / 420.0).clamp(0.8, 3.0);
        let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
        let (lamp_text, lamp_fill, lamp_ink) = if let Some(n) = live.countdown {
            (format!("{}  {n}", tr("Recording in")), Color32::from_rgb(120, 20, 20), Color32::WHITE)
        } else if live.recording {
            (format!("{} ON AIR", if blink { "●" } else { " " }), Color32::from_rgb(200, 30, 30), Color32::WHITE)
        } else {
            let label = match live.light {
                Light::Green => tr("Ready"),
                Light::Yellow => tr("Check"),
                Light::Red => tr("Not ready"),
            };
            (label.to_string(), t.counter_bg, live.light.color())
        };
        let (lamp, _) = ui.allocate_exact_size(vec2(w, 56.0 * scale), egui::Sense::hover());
        ui.painter().rect_filled(lamp, 8.0, lamp_fill);
        ui.painter().text(lamp.center(), egui::Align2::CENTER_CENTER, lamp_text, FontId::proportional(30.0 * scale), lamp_ink);
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(clock(live.take_s)).font(FontId::monospace(44.0 * scale)).color(if live.recording { t.text } else { t.text_dim }));
        });
        ui.add_space(4.0);
        match &live.topic {
            Some((title, spent, planned)) => {
                let over = *planned > 0.0 && spent > planned;
                ui.label(RichText::new(tr("Now")).size(11.0 * scale).color(t.text_dim));
                ui.label(RichText::new(title).size(22.0 * scale).strong());
                let time = if *planned > 0.0 { format!("{} / {}", clock(*spent), clock(*planned)) } else { clock(*spent) };
                ui.label(RichText::new(time).font(FontId::monospace(16.0 * scale)).color(if over { Color32::from_rgb(230, 70, 60) } else { t.text }));
            }
            None if !live.recording => {
                for c in live.checks.iter().take(4) {
                    ui.label(RichText::new(format!("• {c}")).size(13.0 * scale).color(t.text_dim));
                }
            }
            None => {}
        }
        if let Some(next) = &live.next {
            ui.label(RichText::new(trf("Next: {}", &[next])).size(14.0 * scale).color(t.text_dim));
        }
        if own_window {
            // Big buttons for a touchscreen.
            ui.add_space(8.0);
            let bw = ((w - 3.0 * 8.0) / 4.0).max(70.0);
            let size = vec2(bw, 48.0 * scale.min(1.6));
            ui.horizontal(|ui| {
                let rec = if live.recording || live.countdown.is_some() { tr("Stop") } else { tr("Record") };
                let fill = if live.recording { Color32::from_rgb(60, 60, 60) } else { Color32::from_rgb(170, 30, 30) };
                if ui.add(egui::Button::new(RichText::new(rec).size(16.0 * scale.min(1.6)).color(Color32::WHITE)).fill(fill).min_size(size)).clicked()
                {
                    run = Some(if live.recording { "transport.stop" } else { "transport.record" });
                }
                if ui.add(egui::Button::new(RichText::new(tr("+ Chapter")).size(15.0 * scale.min(1.6))).min_size(size)).clicked() {
                    run = Some("markers.add");
                }
                if ui.add(egui::Button::new(RichText::new(tr("Mistake")).size(15.0 * scale.min(1.6))).min_size(size)).clicked() {
                    run = Some("markers.add_cut");
                }
                if ui
                    .add_enabled(live.next.is_some(), egui::Button::new(RichText::new(tr("Next topic")).size(15.0 * scale.min(1.6))).min_size(size))
                    .clicked()
                {
                    run = Some("next_topic");
                }
            });
        }
        let _ = app;
    });
    match run {
        Some("next_topic") => {
            crate::rundown_window::next_topic(app);
        }
        Some(cmd) => {
            if let Err(e) = app.run(cmd, json!({})) {
                app.ui.status = e;
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    #[test]
    fn the_status_names_the_topic_and_what_comes_next() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 1})).unwrap();
        app.engine.execute("podcast.set", &json!({"rundown": [{"title": "Intro", "minutes": 2}, {"title": "Energie", "minutes": 10}]})).unwrap();
        let v = app.run("studio.status", json!({})).unwrap();
        assert_eq!(v["recording"], json!(false));
        assert_eq!(v["next"], json!("Intro"));
        crate::rundown_window::start_topic(&mut app, 0);
        let v = app.run("studio.status", json!({})).unwrap();
        assert_eq!(v["topic"]["title"], json!("Intro"));
        assert_eq!(v["topic"]["planned"], json!("00:02:00"));
        assert_eq!(v["next"], json!("Energie"));
        assert_eq!(clock(3725.0), "01:02:05");
    }
}
