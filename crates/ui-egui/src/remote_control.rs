//! Window › Remote Control: a phone or tablet in the same Wi-Fi becomes the studio's remote. The
//! window shows a QR code; the page it opens (`remote/control/`, English or German after the
//! browser) has the On Air display, Record/Stop, chapter and mistake markers, the next topic and
//! the jingle pads with their pictures. Nothing to install.
//!
//! TalkCraft serves the page itself on the local network, under a random token in the address:
//! only who has the link (the QR code) can use it. It runs only while switched on.

use crate::SoundApp;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use egui::{Color32, RichText, vec2};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PAGE: &str = include_str!("../../../remote/control/index.html");
/// Larger requests are refused (the page only sends tiny commands).
const MAX_BODY: usize = 4096;
/// A device counts as connected while it asked within this time.
const SEEN: Duration = Duration::from_secs(4);

#[derive(Debug, Default)]
struct Shared {
    /// The status as the page gets it (rebuilt by the UI thread a few times a second).
    state: String,
    /// Pad pictures by pad number (1-based).
    pictures: HashMap<usize, String>,
    /// What the page asked for, run by the UI thread.
    commands: Vec<Value>,
    /// Devices by id, and when each last asked.
    devices: HashMap<String, Instant>,
}

#[derive(Debug)]
pub struct Server {
    url: String,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, Clone, Default)]
pub struct RemoteControl {
    server: Option<Arc<Server>>,
    last_state: Option<Instant>,
    pub note: String,
}

/// `window.remote_control`: opens or closes the palette.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    crate::palettes::toggle(app, "remote_control", p)
}

/// This computer's address in the local network (no packet is sent: a UDP "connect" only picks
/// the route).
fn lan_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn reply(out: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 {status} OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    out.write_all(body)?;
    out.flush()
}

fn picture_type(path: &str) -> &'static str {
    match std::path::Path::new(path).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
}

fn serve(stream: TcpStream, token: &str, shared: &Mutex<Shared>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut out = stream.try_clone()?;
    let mut rd = BufReader::new(stream);
    let mut line = String::new();
    rd.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if rd.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let prefix = format!("/{token}/");
    let Some(rest) = target.strip_prefix(&prefix) else {
        return reply(&mut out, 404, "text/plain", b"not found");
    };
    if len > MAX_BODY {
        return reply(&mut out, 413, "text/plain", b"too large");
    }
    let mut body = vec![0; len];
    rd.read_exact(&mut body)?;
    let (path, q) = rest.split_once('?').unwrap_or((rest, ""));
    match (method.as_str(), path) {
        ("GET", "") => reply(&mut out, 200, "text/html; charset=utf-8", PAGE.as_bytes()),
        ("GET", "state") => {
            let q = crate::remote::query(q);
            let state = match shared.lock() {
                Ok(mut sh) => {
                    if let Some(d) = q.get("d").filter(|d| !d.is_empty() && d.len() <= 32) {
                        sh.devices.insert(d.clone(), Instant::now());
                    }
                    sh.state.clone()
                }
                Err(_) => return reply(&mut out, 500, "text/plain", b"busy"),
            };
            reply(&mut out, 200, "application/json", state.as_bytes())
        }
        ("GET", p) if p.starts_with("pad/") => {
            let n = p.trim_start_matches("pad/").parse::<usize>().unwrap_or(0);
            let file = shared.lock().ok().and_then(|sh| sh.pictures.get(&n).cloned());
            match file.and_then(|f| std::fs::read(&f).ok().map(|b| (picture_type(&f), b))) {
                Some((kind, bytes)) => reply(&mut out, 200, kind, &bytes),
                None => reply(&mut out, 404, "text/plain", b"no picture"),
            }
        }
        ("POST", "do") => {
            let cmd: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            if cmd.get("a").and_then(Value::as_str).is_some()
                && let Ok(mut sh) = shared.lock()
            {
                sh.commands.push(cmd);
            }
            reply(&mut out, 200, "application/json", b"{}")
        }
        _ => reply(&mut out, 404, "text/plain", b"not found"),
    }
}

fn start() -> Result<Arc<Server>, String> {
    let ip = lan_ip().ok_or_else(|| tr("This computer is not in a network (Wi-Fi or cable).").to_string())?;
    let listener = TcpListener::bind(("0.0.0.0", 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let token = crate::remote::random_id(20);
    let srv = Arc::new(Server {
        url: format!("http://{ip}:{port}/{token}/"),
        shared: Arc::new(Mutex::new(Shared::default())),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let (shared, stop) = (srv.shared.clone(), srv.stop.clone());
    std::thread::Builder::new()
        .name("talkcraft-remote-control".into())
        .spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let (shared, token) = (shared.clone(), token.clone());
                        let _ = std::thread::Builder::new().name("talkcraft-remote-control-conn".into()).spawn(move || {
                            if let Err(e) = serve(stream, &token, &shared) {
                                log::warn!("remote control: {e}");
                            }
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                    Err(e) => {
                        log::warn!("remote control: {e}");
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(srv)
}

/// `remote_control.start` / `.stop` / `.status`.
pub fn command(app: &mut SoundApp, id: &str) -> Result<Value, String> {
    match id {
        "remote_control.start" => {
            if app.extra.remote_control.server.is_none() {
                app.extra.remote_control.server = Some(start()?);
                update(app, true);
            }
        }
        "remote_control.stop" => {
            if let Some(s) = app.extra.remote_control.server.take() {
                s.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        _ => {}
    }
    let rc = &app.extra.remote_control;
    Ok(json!({"running": rc.server.is_some(), "url": rc.server.as_ref().map(|s| s.url.clone()), "devices": devices(app)}))
}

fn devices(app: &SoundApp) -> usize {
    app.extra
        .remote_control
        .server
        .as_ref()
        .and_then(|s| s.shared.lock().ok().map(|sh| sh.devices.values().filter(|t| t.elapsed() < SEEN).count()))
        .unwrap_or(0)
}

/// Per frame while switched on: the page's status, and what it asked for.
pub fn update(app: &mut SoundApp, now: bool) {
    let Some(srv) = app.extra.remote_control.server.clone() else { return };
    let commands = srv.shared.lock().map(|mut sh| std::mem::take(&mut sh.commands)).unwrap_or_default();
    for c in &commands {
        run(app, c);
    }
    let due = app.extra.remote_control.last_state.is_none_or(|t| t.elapsed() >= Duration::from_millis(200));
    if !(now || due || !commands.is_empty()) {
        return;
    }
    app.extra.remote_control.last_state = Some(Instant::now());
    let mut state = crate::onair::status_json(app);
    let pod = app.engine.session().podcast.clone();
    let count = soundcraft_engine::cmd::podcast::pad_count(&pod);
    let pads: Vec<Value> = (0..count)
        .map(|i| {
            // Only pads with a file can be played: a name alone does not count.
            let has_file = pod.jingles.get(i).is_some_and(|f| !f.trim().is_empty());
            let name = if has_file { soundcraft_engine::cmd::podcast::jingle_name(&pod, i) } else { String::new() };
            let image = pod.jingle_images.get(i).cloned().unwrap_or_default();
            json!({"name": name, "image": image})
        })
        .collect();
    state["pads"] = Value::Array(pads);
    if let Ok(mut sh) = srv.shared.lock() {
        sh.state = state.to_string();
        sh.pictures = pod.jingle_images.iter().enumerate().filter(|(_, p)| !p.trim().is_empty()).map(|(i, p)| (i + 1, p.clone())).collect();
        sh.devices.retain(|_, t| t.elapsed() < Duration::from_secs(60));
    }
}

/// One request of the page: `{a: record|stop|chapter|mistake|next|pad, n?}`.
fn run(app: &mut SoundApp, c: &Value) {
    let r = match c.get("a").and_then(Value::as_str).unwrap_or("") {
        "record" if !app.engine.transport.recording && app.countdown.is_none() => app.run("transport.record", json!({})).map(|_| ()),
        // Stop also cancels a countdown (Record again does that).
        "stop" if app.countdown.is_some() => app.run("transport.record", json!({})).map(|_| ()),
        "stop" if app.engine.transport.recording || app.is_playing() => app.run("transport.stop", json!({})).map(|_| ()),
        "chapter" => app.run("markers.add", json!({})).map(|_| ()),
        "mistake" => app.run("markers.add_cut", json!({})).map(|_| ()),
        "next" => {
            crate::rundown_window::next_topic(app);
            Ok(())
        }
        "pad" => {
            let n = c.get("n").and_then(Value::as_u64).unwrap_or(0) as usize;
            crate::jingles_window::fire(app, n);
            Ok(())
        }
        _ => Ok(()),
    };
    if let Err(e) = r {
        app.ui.status = e;
    }
}

/// A QR code drawn with the painter (black modules on white, with the quiet zone).
fn qr(ui: &mut egui::Ui, text: &str, size: f32) {
    let Ok(code) = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium) else {
        return;
    };
    let n = code.size();
    let (r, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(r, 6.0, Color32::WHITE);
    let cell = size / (n + 8) as f32;
    let origin = r.min + vec2(4.0 * cell, 4.0 * cell);
    for y in 0..n {
        for x in 0..n {
            if code.get_module(x, y) {
                let m = egui::Rect::from_min_size(origin + vec2(x as f32 * cell, y as f32 * cell), vec2(cell + 0.5, cell + 0.5));
                ui.painter().rect_filled(m, 0.0, Color32::BLACK);
            }
        }
    }
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if app.extra.remote_control.server.is_some() {
        update(app, false);
        ctx.request_repaint_after(Duration::from_millis(200));
    }
    if !crate::palettes::is_open(app, "remote_control") {
        return;
    }
    let mut action: Option<&str> = None;
    let url = app.extra.remote_control.server.as_ref().map(|s| s.url.clone());
    let count = devices(app);
    let note = app.extra.remote_control.note.clone();
    crate::palettes::show(app, ctx, "remote_control", tr("Remote Control"), vec2(360.0, 420.0), |_app, ui, _| {
        let t = Tokens::current();
        match &url {
            None => {
                ui.label(tr("A phone or tablet in the same Wi-Fi becomes the remote: On Air display, Record, markers, next topic and the jingle pads. Nothing to install."));
                ui.add_space(6.0);
                if ui.add(egui::Button::new(RichText::new(tr("Switch on")).strong()).fill(t.accent)).clicked() {
                    action = Some("remote_control.start");
                }
            }
            Some(url) => {
                ui.label(RichText::new(tr("Scan with the phone's camera:")).strong());
                let side = ui.available_width().clamp(160.0, 320.0);
                ui.vertical_centered(|ui| qr(ui, url, side));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut url.as_str()).desired_width(ui.available_width() - 70.0));
                    if ui.button(tr("Copy")).clicked() {
                        ui.ctx().copy_text(url.clone());
                    }
                });
                let devices = if count == 0 { tr("No device connected yet.").to_string() } else { trf("{} device(s) connected", &[&count]) };
                ui.label(RichText::new(devices).color(if count > 0 { Color32::from_rgb(60, 175, 90) } else { t.text_dim }));
                ui.label(
                    RichText::new(tr("Only devices in the same network that have this link can use it. If macOS asks whether TalkCraft may accept incoming connections, allow it. Company laptops often block such connections (security software); at home it works."))
                        .small()
                        .color(t.text_dim),
                );
                if ui.button(tr("Switch off")).clicked() {
                    action = Some("remote_control.stop");
                }
            }
        }
        if !note.is_empty() {
            ui.label(RichText::new(&note).color(t.rec));
        }
    });
    if let Some(id) = action {
        app.extra.remote_control.note = match command(app, id) {
            Ok(_) => String::new(),
            Err(e) => e,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    fn http(url: &str, method: &str, body: &str) -> (u16, String) {
        let rest = url.trim_start_matches("http://");
        let (host, path) = rest.split_once('/').unwrap();
        let mut s = TcpStream::connect(host).unwrap();
        write!(s, "{method} /{path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, out.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
    }

    #[test]
    fn a_phone_sees_the_studio_and_sets_markers() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 1})).unwrap();
        let v = match app.run("remote_control.start", json!({})) {
            Ok(v) => v,
            // A machine without any network (some CI sandboxes) cannot offer it.
            Err(e) => {
                assert!(e.contains("network") || e.contains("Netz"), "{e}");
                return;
            }
        };
        let url = v["url"].as_str().unwrap().to_string();
        // The local address works the same as the network one.
        let local = format!("http://127.0.0.1:{}", url.trim_start_matches("http://").split_once(':').unwrap().1);
        let (status, page) = http(&local, "GET", "");
        assert_eq!(status, 200);
        assert!(page.contains("Remote") || page.contains("TalkCraft"));
        let token = url.trim_end_matches('/').rsplit('/').next().unwrap();
        let wrong = local.replace(token, "guess");
        assert_eq!(http(&wrong, "GET", "").0, 404, "no token, no access");
        let (_, state) = http(&format!("{local}state?d=phone1"), "GET", "");
        let state: Value = serde_json::from_str(&state).unwrap();
        // Right after asking, the phone counts as connected (it stops counting a few seconds later).
        assert_eq!(app.run("remote_control.status", json!({})).unwrap()["devices"], json!(1));
        assert_eq!(state["recording"], json!(false));
        assert_eq!(state["pads"].as_array().unwrap().len(), 8);
        let markers = app.engine.session().markers.len();
        http(&format!("{local}do"), "POST", r#"{"a":"chapter"}"#);
        update(&mut app, true);
        assert_eq!(app.engine.session().markers.len(), markers + 1, "the chapter marker was set");
        app.run("remote_control.stop", json!({})).unwrap();
        assert!(app.extra.remote_control.server.is_none());
    }
}
