//! Window > Remote Guests: guests join from their browser (the guest page on the web), the host
//! page runs in the host's browser and is served from here, on loopback only. Record in TalkCraft
//! starts and stops every guest's own lossless recording; when a take ends, the host page hands
//! over each guest's file and the call as heard in the studio. Both land on the guest's track,
//! the guest's own file synced onto the call (`podcast.import_remote`).

use crate::SoundApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use egui::{RichText, vec2};
use serde_json::{Value, json};
use soundcraft_time::Samples;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Where guests open their page; the room follows after `#`.
pub const GUEST_PAGE: &str = "https://gerbermedia.codefloe.page/talkcraft-gast/";
const HOST_PAGE: &str = include_str!("../../../remote/host/index.html");
/// The parrot for the studio page (the same files as the guest page's).
const LOGO: &[u8] = include_bytes!("../../../remote/guest/logo.png");
const FAVICON: &[u8] = include_bytes!("../../../remote/guest/favicon.ico");
const FAVICON_32: &[u8] = include_bytes!("../../../remote/guest/favicon-32.png");
/// Largest upload accepted: three hours of 48 kHz 16-bit mono.
const MAX_UPLOAD: usize = 3 * 3600 * 48_000 * 2 + 1024;
/// A guest the host page has not mentioned for this long has left.
const GUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// One file handed over by the host page.
#[derive(Debug, Clone, PartialEq)]
pub struct Upload {
    pub guest: String,
    /// The guest's own recording (`true`) or the call as heard in the studio.
    pub hq: bool,
    pub take: u32,
    /// When the host page started that take (ms since the epoch, the host's clock).
    pub started_ms: i64,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct Guest {
    pub name: String,
    pub level_db: f32,
    pub connected: bool,
    /// The guest muted themselves.
    pub self_muted: bool,
}

#[derive(Debug, Default)]
struct Shared {
    recording: bool,
    take: u32,
    guests: Vec<Guest>,
    guests_seen: Option<Instant>,
    /// Guests muted in the call (their track's mute button).
    muted: Vec<String>,
    /// Guests to send away, collected until the host page next asks.
    kick: Vec<String>,
    /// Where uploads are written.
    dir: PathBuf,
}

#[derive(Debug)]
struct Server {
    port: u16,
    room: String,
    token: String,
    shared: Arc<Mutex<Shared>>,
    uploads: Mutex<Receiver<Upload>>,
}

#[derive(Debug, Clone, Default)]
pub struct RemoteStudio {
    pub show: bool,
    server: Option<Arc<Server>>,
    /// Per take: where the recording started on the timeline, and when (ms since the epoch).
    takes: HashMap<u32, (Samples, i64)>,
    /// Call recordings waiting for the guest's own file: (guest, take) → upload.
    live: HashMap<(String, u32), Upload>,
    was_recording: bool,
    take: u32,
    pub log: Vec<String>,
}

/// `window.remote`: opens or closes the window.
pub fn toggle(app: &mut SoundApp, p: &Value) -> Value {
    let w = &mut app.extra.remote;
    w.show = p.get("value").and_then(Value::as_bool).unwrap_or(!w.show);
    json!({"value": w.show})
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
}

/// Random lowercase letters and digits, from the system's hasher seeds (no extra crate).
fn random_id(len: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    const ABC: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut out = String::new();
    while out.len() < len {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_i64(now_ms());
        let mut x = h.finish();
        for _ in 0..8 {
            if out.len() >= len {
                break;
            }
            out.push(char::from(ABC.get((x % ABC.len() as u64) as usize).copied().unwrap_or(b'x')));
            x /= ABC.len() as u64;
        }
    }
    out
}

/// Decodes `%XX` and `+` in a query value.
fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(v) = hex {
                    out.push(v);
                    i += 2;
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query(q: &str) -> HashMap<String, String> {
    q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (url_decode(k), url_decode(v))).collect()
}

/// A guest's name made safe for a file name.
fn file_safe(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(40).collect();
    if s.is_empty() { "Gast".into() } else { s }
}

/// What the server answers: status, content type, body.
type Reply = (u16, &'static str, Vec<u8>);

/// Handles one request. `path` has the token already removed.
fn route(method: &str, path: &str, body: &[u8], srv: &Server, tx: &Sender<Upload>) -> Reply {
    let (path, q) = path.split_once('?').unwrap_or((path, ""));
    // The shared state is locked only for a moment: the UI thread takes it every frame, so slow
    // work (writing an upload) happens outside.
    let busy = || (500, "text/plain", b"busy".to_vec());
    match (method, path) {
        ("GET", "" | "/") => {
            let page = HOST_PAGE.replace("{{ROOM}}", &srv.room).replace("{{TOKEN}}", &srv.token).replace("{{GUEST_URL}}", GUEST_PAGE);
            (200, "text/html; charset=utf-8", page.into_bytes())
        }
        ("GET", "/logo.png") => (200, "image/png", LOGO.to_vec()),
        ("GET", "/favicon.ico") => (200, "image/x-icon", FAVICON.to_vec()),
        ("GET", "/favicon-32.png") => (200, "image/png", FAVICON_32.to_vec()),
        ("GET", "/state") => {
            let Ok(mut sh) = srv.shared.lock() else { return busy() };
            let kick = std::mem::take(&mut sh.kick);
            let state = json!({"recording": sh.recording, "take": sh.take, "room": srv.room, "muted": sh.muted, "kick": kick});
            (200, "application/json", state.to_string().into_bytes())
        }
        ("POST", "/guests") => {
            let list: Vec<Value> = serde_json::from_slice(body).unwrap_or_default();
            let Ok(mut sh) = srv.shared.lock() else { return busy() };
            sh.guests = list
                .iter()
                .take(32)
                .map(|g| Guest {
                    name: g["name"].as_str().unwrap_or("Gast").chars().take(60).collect(),
                    level_db: g["level"].as_f64().unwrap_or(-60.0) as f32,
                    connected: g["connected"].as_bool().unwrap_or(false),
                    self_muted: g["self_muted"].as_bool().unwrap_or(false),
                })
                .collect();
            sh.guests_seen = Some(Instant::now());
            (200, "application/json", b"{}".to_vec())
        }
        ("POST", "/upload") => {
            let q = query(q);
            let guest =
                q.get("guest").map(|g| g.trim().chars().take(60).collect::<String>()).filter(|g| !g.is_empty()).unwrap_or_else(|| "Gast".into());
            let hq = q.get("kind").is_some_and(|k| k == "hq");
            let take = q.get("take").and_then(|t| t.parse::<u32>().ok()).unwrap_or(0);
            let started_ms = q.get("started").and_then(|t| t.parse::<i64>().ok()).unwrap_or_else(now_ms);
            let Ok(dir) = srv.shared.lock().map(|sh| sh.dir.clone()) else { return busy() };
            if std::fs::create_dir_all(&dir).is_err() {
                return (500, "text/plain", b"cannot create folder".to_vec());
            }
            let path = dir.join(format!("{}-take{take}-{}.wav", file_safe(&guest), if hq { "hq" } else { "live" }));
            if let Err(e) = std::fs::write(&path, body) {
                return (500, "text/plain", e.to_string().into_bytes());
            }
            let _ = tx.send(Upload { guest, hq, take, started_ms, path });
            (200, "application/json", b"{}".to_vec())
        }
        _ => (404, "text/plain", b"not found".to_vec()),
    }
}

fn serve(stream: TcpStream, srv: &Server, tx: &Sender<Upload>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
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
    let prefix = format!("/{}", srv.token);
    let reply = if len > MAX_UPLOAD {
        (413, "text/plain", b"too large".to_vec())
    } else if let Some(rest) = target.strip_prefix(&prefix) {
        let mut body = vec![0; len];
        rd.read_exact(&mut body)?;
        route(&method, rest, &body, srv, tx)
    } else {
        (404, "text/plain", b"not found".to_vec())
    };
    let (status, kind, body) = reply;
    write!(
        out,
        "HTTP/1.1 {status} OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    out.write_all(&body)?;
    out.flush()
}

fn start_server(dir: PathBuf) -> Result<Arc<Server>, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (tx, rx) = channel();
    let srv = Arc::new(Server {
        port,
        room: format!("talkcraft-{}", random_id(12)),
        token: random_id(24),
        shared: Arc::new(Mutex::new(Shared { dir, ..Default::default() })),
        uploads: Mutex::new(rx),
    });
    let s = srv.clone();
    std::thread::Builder::new()
        .name("talkcraft-remote".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let (s, tx) = (s.clone(), tx.clone());
                let _ = std::thread::Builder::new().name("talkcraft-remote-conn".into()).spawn(move || {
                    if let Err(e) = serve(stream, &s, &tx) {
                        log::warn!("remote guests: {e}");
                    }
                });
            }
        })
        .map_err(|e| e.to_string())?;
    log::info!("remote guests: studio page on 127.0.0.1:{port}");
    Ok(srv)
}

impl Server {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/{}/", self.port, self.token)
    }
}

/// A remote guest's track in the session.
pub fn track_name(guest: &str) -> String {
    format!("Ferngast – {guest}")
}

/// Makes the guest's track when they join: mono, locked against local recording (their sound
/// comes from their browser).
fn ensure_track(app: &mut SoundApp, guest: &str) {
    let name = track_name(guest);
    if app.engine.session().track_by_name(&name).is_some() {
        return;
    }
    let made = app
        .run("track.new", json!({"name": name, "format": "mono"}))
        .and_then(|_| app.run("mix.record_lock", json!({"tracks": [name], "value": true})));
    if let Err(e) = made {
        app.ui.status = format!("{name}: {e}");
    }
}

/// Folder for guest files: next to the saved session, else in the autosave folder.
fn upload_dir(app: &SoundApp) -> PathBuf {
    let base = app.engine.path.as_ref().and_then(|p| std::path::Path::new(p).parent().map(|d| d.join("Remote Guests")));
    base.unwrap_or_else(|| app.autosave_dir.clone().unwrap_or_else(std::env::temp_dir).join("Remote Guests"))
}

/// Where a file started at `started_ms` belongs, for a take recorded from `start` at `wall_ms`.
pub fn timeline_position(start: Samples, wall_ms: i64, started_ms: i64, rate: u32) -> Samples {
    let late_ms = started_ms.saturating_sub(wall_ms).clamp(-60_000, 60_000);
    start.saturating_add(late_ms.saturating_mul(i64::from(rate)) / 1000).max(0)
}

/// Per frame: tells the host page whether TalkCraft records, and files the guests' takes.
pub fn update(app: &mut SoundApp) {
    let Some(srv) = app.extra.remote.server.clone() else { return };
    let recording = app.engine.transport.recording;
    let w = &mut app.extra.remote;
    if recording && !w.was_recording {
        w.take = w.take.saturating_add(1);
        w.takes.insert(w.take, (app.record_start, now_ms()));
    }
    w.was_recording = recording;
    if let Ok(mut sh) = srv.shared.lock() {
        sh.recording = recording;
        sh.take = w.take;
        sh.dir = upload_dir(app);
        if sh.guests_seen.is_some_and(|t| t.elapsed() > GUEST_TIMEOUT) {
            sh.guests.clear();
        }
    }
    let guests = srv.shared.lock().map(|sh| sh.guests.clone()).unwrap_or_default();
    for g in &guests {
        ensure_track(app, &g.name);
        // The guest's level on their track's meter.
        if let Some(id) = app.engine.session().track_by_name(&track_name(&g.name)).map(|t| t.id) {
            let lin = if g.connected { 10f32.powf(g.level_db.clamp(-120.0, 0.0) / 20.0) } else { 0.0 };
            let m = app.meters.entry(id).or_default();
            for (l, h) in m.level.iter_mut().zip(m.hold.iter_mut()) {
                *l = l.max(lin);
                *h = h.max(lin);
            }
        }
    }
    let muted: Vec<String> = guests
        .iter()
        .filter(|g| app.engine.session().track_by_name(&track_name(&g.name)).is_some_and(|t| t.mixer.mute))
        .map(|g| g.name.clone())
        .collect();
    if let Ok(mut sh) = srv.shared.lock() {
        sh.muted = muted;
    }
    let got: Vec<Upload> = srv.uploads.lock().map(|rx| rx.try_iter().collect()).unwrap_or_default();
    for up in got {
        if up.hq {
            file_take(app, up);
        } else {
            app.extra.remote.live.insert((up.guest.clone(), up.take), up);
        }
    }
}

fn file_take(app: &mut SoundApp, hq: Upload) {
    let rate = app.engine.session().sample_rate.hz();
    let (start, wall) = app.extra.remote.takes.get(&hq.take).copied().unwrap_or((app.record_start, hq.started_ms));
    let live = app.extra.remote.live.remove(&(hq.guest.clone(), hq.take));
    let mut p = json!({
        "guest": track_name(&hq.guest),
        "path": hq.path.to_string_lossy(),
        "at": timeline_position(start, wall, hq.started_ms, rate),
    });
    if let Some(l) = &live {
        p["live"] = json!(l.path.to_string_lossy());
        p["live_at"] = json!(timeline_position(start, wall, l.started_ms, rate));
    }
    let msg = match app.run("podcast.import_remote", p) {
        Ok(v) if v["sync"]["error"].is_string() => {
            format!("{}: take {} placed, not synced ({})", hq.guest, hq.take, v["sync"]["error"].as_str().unwrap_or(""))
        }
        Ok(v) if live.is_some() => {
            format!("{}: take {} placed and synced ({:+.3} s)", hq.guest, hq.take, v["sync"]["moved_s"].as_f64().unwrap_or(0.0))
        }
        Ok(_) => format!("{}: take {} placed", hq.guest, hq.take),
        Err(e) => format!("{}: take {} failed: {e}", hq.guest, hq.take),
    };
    app.ui.status = msg.clone();
    app.extra.remote.log.push(msg);
}

/// Starts the studio (once) and opens the host page in the browser.
fn open_studio(app: &mut SoundApp) {
    if app.extra.remote.server.is_none() {
        match start_server(upload_dir(app)) {
            Ok(s) => app.extra.remote.server = Some(s),
            Err(e) => {
                app.ui.status = format!("Remote guests: {e}");
                return;
            }
        }
    }
    if let Some(s) = &app.extra.remote.server {
        open_in_browser(&s.url());
    }
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        log::warn!("cannot open {url}: {e}");
    }
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.extra.remote.show {
        if app.extra.remote.server.is_some() {
            update(app);
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        return;
    }
    update(app);
    let t = Tokens::current();
    let mut open = true;
    let mut start = false;
    let mut mute: Option<(String, bool)> = None;
    let mut kick: Option<String> = None;
    let muted = |name: &str| app.engine.session().track_by_name(&track_name(name)).is_some_and(|t| t.mixer.mute);
    let guests = app.extra.remote.server.as_ref().and_then(|s| s.shared.lock().ok().map(|sh| sh.guests.clone())).unwrap_or_default();
    egui::Window::new(tr("Remote Guests")).open(&mut open).default_size(vec2(460.0, 300.0)).resizable(true).show(ctx, |ui| {
        let w = &app.extra.remote;
        match &w.server {
            None => {
                ui.label(tr("Guests join from their browser: no installation, they only need the link and headphones."));
                ui.add_space(6.0);
                if ui.button(tr("Open Studio")).clicked() {
                    start = true;
                }
            }
            Some(s) => {
                let link = format!("{GUEST_PAGE}#{}", s.room);
                ui.label(RichText::new(tr("Invitation link for guests")).strong());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut link.as_str()).desired_width(320.0));
                    if ui.button(tr("Copy")).clicked() {
                        ui.ctx().copy_text(link.clone());
                    }
                });
                if ui.button(tr("Show Studio Page")).on_hover_text(tr("Keep the studio page open in your browser while guests are on.")).clicked() {
                    start = true;
                }
                ui.separator();
                if guests.is_empty() {
                    ui.label(RichText::new(tr("No guests yet.")).color(t.text_dim));
                }
                for g in &guests {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&g.name).strong());
                        let level = ((g.level_db + 60.0) / 60.0).clamp(0.0, 1.0);
                        ui.add(egui::ProgressBar::new(level).desired_width(120.0).desired_height(8.0));
                        if !g.connected {
                            ui.label(RichText::new(tr("connecting …")).color(t.text_dim));
                        } else if g.self_muted {
                            ui.label(RichText::new(tr("muted themselves")).color(t.text_dim));
                        }
                        let on = muted(&g.name);
                        if ui.selectable_label(on, tr("Mute")).on_hover_text(tr("Mutes the guest in the call (same as M on their track)")).clicked() {
                            mute = Some((g.name.clone(), !on));
                        }
                        if ui.button(tr("Remove")).on_hover_text(tr("Ends the call for this guest")).clicked() {
                            kick = Some(g.name.clone());
                        }
                    });
                }
            }
        }
        if !w.log.is_empty() {
            ui.separator();
            for l in w.log.iter().rev().take(6) {
                ui.label(RichText::new(l).small());
            }
        }
        ui.label(
            RichText::new(tr("Record starts every guest's own recording too. After Stop, their files arrive on their tracks, synced to the call."))
                .small()
                .color(t.text_dim),
        );
    });
    app.extra.remote.show = open;
    if let Some((name, value)) = mute
        && let Err(e) = app.run("mix.mute", json!({"tracks": [track_name(&name)], "value": value}))
    {
        app.ui.status = e;
    }
    if let (Some(name), Some(srv)) = (kick, &app.extra.remote.server)
        && let Ok(mut sh) = srv.shared.lock()
    {
        sh.kick.push(name);
    }
    if !guests.is_empty() {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    if start {
        open_studio(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_files_move_later_on_the_timeline() {
        assert_eq!(timeline_position(48_000, 1_000, 1_250, 48_000), 60_000);
        assert_eq!(timeline_position(0, 1_000, 500, 48_000), 0, "never before zero");
        assert_eq!(timeline_position(0, 0, i64::MAX, 48_000), 60 * 48_000, "absurd clocks are capped");
    }

    #[test]
    fn query_values_are_decoded() {
        let q = query("guest=J%C3%B6rg+M.&kind=hq&take=3");
        assert_eq!(q.get("guest").map(String::as_str), Some("Jörg M."));
        assert_eq!(file_safe("Jörg M./.."), "Jörg_M____");
        assert_eq!(url_decode("%zz"), "%zz");
    }

    #[test]
    fn the_server_files_uploads_and_reports_state() {
        let dir = std::env::temp_dir().join(format!("tc-remote-{}", std::process::id()));
        let srv = start_server(dir.clone()).unwrap();
        let ask = |req: String| {
            let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
            s.write_all(req.as_bytes()).unwrap();
            let mut out = Vec::new();
            s.read_to_end(&mut out).unwrap();
            String::from_utf8_lossy(&out).into_owned()
        };
        assert!(ask(format!("GET /{}/ HTTP/1.1\r\n\r\n", srv.token)).contains(&srv.room), "host page carries the room");
        assert!(ask("GET /state HTTP/1.1\r\n\r\n".into()).starts_with("HTTP/1.1 404"), "nothing without the token");
        assert!(ask(format!("GET /{}/state HTTP/1.1\r\n\r\n", srv.token)).contains("\"recording\":false"));
        assert!(ask(format!("GET /{}/favicon.ico HTTP/1.1\r\n\r\n", srv.token)).contains("image/x-icon"), "the parrot in the tab");
        let body = "RIFF";
        let r =
            ask(format!("POST /{}/upload?guest=Anna&kind=hq&take=2&started=5 HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", srv.token, body.len()));
        assert!(r.starts_with("HTTP/1.1 200"), "{r}");
        let up = srv.uploads.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((up.guest.as_str(), up.hq, up.take, up.started_ms), ("Anna", true, 2, 5));
        assert_eq!(std::fs::read(&up.path).unwrap(), body.as_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
