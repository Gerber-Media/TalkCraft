//! Podcast hosts: upload the episode and create it (as a draft or published) through each host's
//! API, with the key the user enters. Each flow follows the host's own documentation; where only
//! secondary sources were available that is said in `Host::verified`.

use super::http::{Part, Request, Transport, call};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    Buzzsprout,
    Transistor,
    Podigee,
    Podbean,
    Spreaker,
    Captivate,
    RssCom,
    Castos,
    Megaphone,
}

impl Host {
    pub const ALL: [Host; 9] = [
        Host::Buzzsprout,
        Host::Transistor,
        Host::Podigee,
        Host::Podbean,
        Host::Spreaker,
        Host::Captivate,
        Host::RssCom,
        Host::Castos,
        Host::Megaphone,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Host::Buzzsprout => "Buzzsprout",
            Host::Transistor => "Transistor",
            Host::Podigee => "Podigee",
            Host::Podbean => "Podbean",
            Host::Spreaker => "Spreaker",
            Host::Captivate => "Captivate",
            Host::RssCom => "RSS.com",
            Host::Castos => "Castos",
            Host::Megaphone => "Megaphone",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Host::Buzzsprout => "buzzsprout",
            Host::Transistor => "transistor",
            Host::Podigee => "podigee",
            Host::Podbean => "podbean",
            Host::Spreaker => "spreaker",
            Host::Captivate => "captivate",
            Host::RssCom => "rss_com",
            Host::Castos => "castos",
            Host::Megaphone => "megaphone",
        }
    }

    pub fn from_id(s: &str) -> Option<Host> {
        Host::ALL.into_iter().find(|h| h.id() == s)
    }

    /// The fields the user fills in: (key in `HostConfig`, label, secret?).
    pub fn fields(self) -> &'static [(&'static str, &'static str, bool)] {
        match self {
            Host::Buzzsprout => &[("api_key", "API token", true), ("show_id", "Podcast ID", false)],
            Host::Transistor => &[("api_key", "API key", true), ("show_id", "Show ID", false)],
            Host::Podigee => &[("api_key", "API key", true), ("show_id", "Podcast ID", false)],
            Host::Podbean => &[("user", "Client ID", false), ("api_key", "Client secret", true)],
            Host::Spreaker => &[("api_key", "OAuth token", true), ("show_id", "Show ID", false)],
            Host::Captivate => &[("user", "User ID", false), ("api_key", "API token", true), ("show_id", "Show ID", false)],
            Host::RssCom => &[("api_key", "API key", true), ("show_id", "Podcast ID", false)],
            Host::Castos => &[("api_key", "API token", true), ("show_id", "Podcast ID", false)],
            Host::Megaphone => &[("api_key", "API token", true), ("user", "Network ID", false), ("show_id", "Podcast ID", false)],
        }
    }

    /// Whether the request format was checked against the host's own documentation (or its
    /// published API description); otherwise it comes from secondary sources and may need adjusting.
    pub fn verified(self) -> bool {
        !matches!(self, Host::Castos | Host::Megaphone)
    }

    /// Hosts that take the audio only as a public URL (the file goes to your own storage first).
    pub fn needs_public_url(self) -> bool {
        matches!(self, Host::Castos | Host::Megaphone)
    }
}

/// What the user entered for a host.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostConfig {
    pub api_key: String,
    pub show_id: String,
    /// Second identifier where a host needs one (client ID, user ID, network ID).
    pub user: String,
}

/// The episode to publish.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Episode {
    pub audio: PathBuf,
    pub audio_mime: String,
    pub title: String,
    pub description: String,
    pub number: u32,
    pub season: u32,
    /// Publish now, or leave it as a draft to check on the host first.
    pub publish: bool,
    /// The audio's public URL, for hosts that only take one.
    pub audio_url: Option<String>,
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("episode.m4a").to_string()
}

fn size(p: &std::path::Path) -> Result<u64, String> {
    std::fs::metadata(p).map(|m| m.len()).map_err(|e| format!("{}: {e}", p.display()))
}

fn need<'a>(v: &'a str, what: &str) -> Result<&'a str, String> {
    let v = v.trim();
    if v.is_empty() { Err(format!("{what} is missing")) } else { Ok(v) }
}

/// A string or number field as text.
fn s(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Uploads and creates the episode; returns where to find it (a URL or the host's id).
pub fn publish(host: Host, cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    match host {
        Host::Buzzsprout => buzzsprout(cfg, ep, t, log),
        Host::Transistor => transistor(cfg, ep, t, log),
        Host::Podigee => podigee(cfg, ep, t, log),
        Host::Podbean => podbean(cfg, ep, t, log),
        Host::Spreaker => spreaker(cfg, ep, t, log),
        Host::Captivate => captivate(cfg, ep, t, log),
        Host::RssCom => rss_com(cfg, ep, t, log),
        Host::Castos => castos(cfg, ep, t, log),
        Host::Megaphone => megaphone(cfg, ep, t, log),
    }
}

/// Buzzsprout: create the episode (private), start an upload, PUT the file, complete, publish.
fn buzzsprout(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let key = need(&cfg.api_key, "API token")?;
    let pod = need(&cfg.show_id, "Podcast ID")?;
    let base = format!("https://www.buzzsprout.com/api/{pod}");
    let auth =
        |r: Request| r.header("Authorization", format!("Token token={key}")).header("User-Agent", "TalkCraft").header("Accept", "application/json");
    log("Buzzsprout: creating the episode".into());
    let created = call(
        t,
        auth(Request::new("POST", format!("{base}/episodes"))).json(json!({
            "title": ep.title, "description": ep.description, "episode_number": ep.number, "season_number": ep.season, "private": true
        })),
    )?;
    let id = s(&created["id"]);
    need(&id, "episode id in Buzzsprout's answer")?;
    let up = call(
        t,
        auth(Request::new("POST", format!("{base}/episodes/{id}/uploads"))).json(json!({
            "filename": file_name(&ep.audio), "type": ep.audio_mime, "byte_size": size(&ep.audio)?
        })),
    )?;
    let up = if up.get("upload").is_some() { up["upload"].clone() } else { up };
    let (up_id, url) = (s(&up["id"]), s(&up["upload_url"]));
    log("Buzzsprout: uploading the audio".into());
    call(t, Request::new("PUT", url).file(ep.audio.clone()))?;
    call(t, auth(Request::new("POST", format!("{base}/episodes/{id}/uploads/{up_id}/complete"))))?;
    if ep.publish {
        call(t, auth(Request::new("PATCH", format!("{base}/episodes/{id}"))).json(json!({"private": false})))?;
    }
    Ok(format!("https://www.buzzsprout.com/{pod}/episodes/{id}"))
}

/// Transistor: authorize an upload, PUT the file, create the episode with its audio URL, publish.
fn transistor(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let key = need(&cfg.api_key, "API key")?;
    let show = need(&cfg.show_id, "Show ID")?;
    let api = "https://api.transistor.fm/v1";
    let name = file_name(&ep.audio);
    let au = call(t, Request::new("GET", format!("{api}/episodes/authorize_upload?filename={}", url_encode(&name))).header("x-api-key", key))?;
    let a = &au["data"]["attributes"];
    let (upload_url, ctype, audio_url) = (s(&a["upload_url"]), s(&a["content_type"]), s(&a["audio_url"]));
    log("Transistor: uploading the audio".into());
    call(t, Request::new("PUT", upload_url).header("Content-Type", ctype).file(ep.audio.clone()))?;
    log("Transistor: creating the episode".into());
    let created = call(
        t,
        Request::new("POST", format!("{api}/episodes")).header("x-api-key", key).form(vec![
            ("episode[show_id]", show.to_string()),
            ("episode[title]", ep.title.clone()),
            ("episode[summary]", ep.description.clone()),
            ("episode[number]", ep.number.to_string()),
            ("episode[season]", ep.season.to_string()),
            ("episode[audio_url]", audio_url),
        ]),
    )?;
    let id = s(&created["data"]["id"]);
    if ep.publish {
        call(
            t,
            Request::new("PATCH", format!("{api}/episodes/{id}/publish"))
                .header("x-api-key", key)
                .form(vec![("episode[status]", "published".into())]),
        )?;
    }
    Ok(format!("Transistor episode {id}"))
}

/// Podigee: create the episode, get an upload URL, PUT, create a production with the file, start it.
fn podigee(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let key = need(&cfg.api_key, "API key")?;
    let pod = need(&cfg.show_id, "Podcast ID")?;
    let api = "https://app.podigee.com/api/v1";
    log("Podigee: creating the episode".into());
    let created = call(
        t,
        Request::new("POST", format!("{api}/episodes")).header("Token", key).json(json!({
            "title": ep.title, "podcast_id": pod.parse::<u64>().map_or(json!(pod), |n| json!(n)), "description": ep.description, "number": ep.number
        })),
    )?;
    let id = s(&created["id"]);
    let up = call(t, Request::new("POST", format!("{api}/uploads?filename={}", url_encode(&file_name(&ep.audio)))).header("Token", key))?;
    log("Podigee: uploading the audio".into());
    call(t, Request::new("PUT", s(&up["upload_url"])).header("Content-Type", s(&up["content_type"])).file(ep.audio.clone()))?;
    let prod = call(
        t,
        Request::new("POST", format!("{api}/productions")).header("Token", key).json(json!({
            "episode_id": id.parse::<u64>().map_or(json!(id), |n| json!(n)), "files": [{"url": s(&up["file_url"])}]
        })),
    )?;
    let pid = s(&prod["id"]);
    let start = if ep.publish { "?publish_episode=true" } else { "" };
    call(t, Request::new("POST", format!("{api}/productions/{pid}/start{start}")).header("Token", key))?;
    Ok(format!("Podigee episode {id}"))
}

/// Podbean: client-credentials token, authorize an upload, PUT, create the episode with the file key.
fn podbean(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let id = need(&cfg.user, "Client ID")?;
    let secret = need(&cfg.api_key, "Client secret")?;
    let api = "https://api.podbean.com/v1";
    let tok = call(
        t,
        Request::new("POST", format!("{api}/oauth/token")).form(vec![
            ("grant_type", "client_credentials".into()),
            ("client_id", id.to_string()),
            ("client_secret", secret.to_string()),
        ]),
    )?;
    let token = s(&tok["access_token"]);
    need(&token, "access token in Podbean's answer")?;
    let au = call(
        t,
        Request::new(
            "GET",
            format!(
                "{api}/files/uploadAuthorize?access_token={}&filename={}&filesize={}&content_type={}",
                url_encode(&token),
                url_encode(&file_name(&ep.audio)),
                size(&ep.audio)?,
                url_encode(&ep.audio_mime)
            ),
        ),
    )?;
    log("Podbean: uploading the audio".into());
    call(t, Request::new("PUT", s(&au["presigned_url"])).header("Content-Type", ep.audio_mime.clone()).file(ep.audio.clone()))?;
    log("Podbean: creating the episode".into());
    let created = call(
        t,
        Request::new("POST", format!("{api}/episodes")).form(vec![
            ("access_token", token),
            ("title", ep.title.clone()),
            ("content", ep.description.clone()),
            ("status", if ep.publish { "publish" } else { "draft" }.into()),
            ("type", "public".into()),
            ("media_key", s(&au["file_key"])),
        ]),
    )?;
    let link = s(&created["episode"]["permalink_url"]);
    Ok(if link.is_empty() { format!("Podbean episode {}", s(&created["episode"]["id"])) } else { link })
}

/// Spreaker: one multipart upload into the show (published), or a draft with the file attached.
fn spreaker(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let token = need(&cfg.api_key, "OAuth token")?;
    let show = need(&cfg.show_id, "Show ID")?;
    let api = "https://api.spreaker.com/v2";
    let auth = format!("Bearer {token}");
    log("Spreaker: uploading the episode".into());
    let v = if ep.publish {
        call(
            t,
            Request::new("POST", format!("{api}/shows/{show}/episodes")).header("Authorization", auth.clone()).multipart(vec![
                ("title", Part::Text(ep.title.clone())),
                ("description", Part::Text(ep.description.clone())),
                ("media_file", Part::File(ep.audio.clone())),
            ]),
        )?
    } else {
        let d = call(
            t,
            Request::new("POST", format!("{api}/episodes/drafts"))
                .header("Authorization", auth.clone())
                .multipart(vec![("title", Part::Text(ep.title.clone())), ("show_id", Part::Text(show.to_string()))]),
        )?;
        let id = s(&d["response"]["episode"]["episode_id"]);
        call(
            t,
            Request::new("POST", format!("{api}/episodes/{id}"))
                .header("Authorization", auth.clone())
                .multipart(vec![("description", Part::Text(ep.description.clone())), ("media_file", Part::File(ep.audio.clone()))]),
        )?
    };
    let e = &v["response"]["episode"];
    let url = s(&e["site_url"]);
    Ok(if url.is_empty() { format!("Spreaker episode {}", s(&e["episode_id"])) } else { url })
}

/// Captivate: token from user ID + API token, upload the media, create the episode with its id.
fn captivate(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let user = need(&cfg.user, "User ID")?;
    let key = need(&cfg.api_key, "API token")?;
    let show = need(&cfg.show_id, "Show ID")?;
    let api = "https://api.captivate.fm";
    let a = call(
        t,
        Request::new("POST", format!("{api}/authenticate/token"))
            .multipart(vec![("username", Part::Text(user.to_string())), ("token", Part::Text(key.to_string()))]),
    )?;
    let token = s(&a["user"]["token"]);
    need(&token, "token in Captivate's answer")?;
    let auth = format!("Bearer {token}");
    log("Captivate: uploading the audio".into());
    let m = call(
        t,
        Request::new("POST", format!("{api}/shows/{show}/media"))
            .header("Authorization", auth.clone())
            .multipart(vec![("file", Part::File(ep.audio.clone()))]),
    )?;
    let media = [s(&m["media"]["id"]), s(&m["id"])].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
    need(&media, "media id in Captivate's answer")?;
    log("Captivate: creating the episode".into());
    let now = super::now_utc();
    let created = call(
        t,
        Request::new("POST", format!("{api}/episodes")).header("Authorization", auth).multipart(vec![
            ("shows_id", Part::Text(show.to_string())),
            ("title", Part::Text(ep.title.clone())),
            ("itunes_title", Part::Text(ep.title.clone())),
            ("media_id", Part::Text(media)),
            ("date", Part::Text(now)),
            ("episode_number", Part::Text(ep.number.to_string())),
            ("episode_type", Part::Text("full".into())),
            ("shownotes", Part::Text(ep.description.clone())),
            ("summary", Part::Text(ep.description.clone())),
            ("status", Part::Text(if ep.publish { "Published" } else { "Draft" }.into())),
        ]),
    )?;
    Ok(format!("Captivate episode {}", [s(&created["episode"]["id"]), s(&created["id"])].concat()))
}

/// RSS.com: a pre-signed upload for the audio, then the episode with that upload's id.
fn rss_com(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let key = need(&cfg.api_key, "API key")?;
    let pod = need(&cfg.show_id, "Podcast ID")?;
    let api = format!("https://api.rss.com/v4/podcasts/{pod}");
    let up = call(
        t,
        Request::new("POST", format!("{api}/assets/presigned-uploads")).header("X-Api-Key", key).json(json!({
            "asset_type": "audio", "expected_mime": ep.audio_mime, "filename": file_name(&ep.audio)
        })),
    )?;
    log("RSS.com: uploading the audio".into());
    call(t, Request::new("PUT", s(&up["url"])).header("Content-Type", ep.audio_mime.clone()).file(ep.audio.clone()))?;
    log("RSS.com: creating the episode".into());
    let mut body = json!({
        "title": ep.title, "description": if ep.description.is_empty() { ep.title.clone() } else { ep.description.clone() },
        "itunes_episode_type": "full", "audio_upload_id": s(&up["id"]),
    });
    if ep.number > 0 {
        body["itunes_episode"] = json!(ep.number);
    }
    if ep.season > 0 {
        body["itunes_season"] = json!(ep.season);
    }
    if ep.publish {
        // "Set to the current timestamp for automatic publishing."
        body["schedule_datetime"] = json!(super::now_iso());
    }
    let created = call(t, Request::new("POST", format!("{api}/episodes")).header("X-Api-Key", key).json(body))?;
    let url = s(&created["dashboard_url"]);
    Ok(if url.is_empty() { format!("RSS.com episode {}", s(&created["id"])) } else { url })
}

/// Castos (from secondary sources): the episode with the audio's public URL.
fn castos(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let key = need(&cfg.api_key, "API token")?;
    let pod = need(&cfg.show_id, "Podcast ID")?;
    let url = ep.audio_url.clone().ok_or("Castos takes the audio as a public URL: choose a storage for the file")?;
    log("Castos: creating the episode".into());
    let mut body = json!({"title": ep.title, "audio_url": url, "description": ep.description});
    if ep.publish {
        body["publish_at"] = json!(super::now_iso());
    }
    let created = call(
        t,
        Request::new("POST", format!("https://app.castos.com/api/v2/podcasts/{pod}/episodes"))
            .header("Authorization", format!("Bearer {key}"))
            .json(body),
    )?;
    Ok(format!("Castos episode {}", [s(&created["data"]["id"]), s(&created["id"])].concat()))
}

/// Megaphone (from secondary sources): the episode in a network's podcast, audio fetched from a URL.
fn megaphone(cfg: &HostConfig, ep: &Episode, t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let key = need(&cfg.api_key, "API token")?;
    let net = need(&cfg.user, "Network ID")?;
    let pod = need(&cfg.show_id, "Podcast ID")?;
    let url = ep.audio_url.clone().ok_or("Megaphone takes the audio as a public URL: choose a storage for the file")?;
    log("Megaphone: creating the episode".into());
    let created = call(
        t,
        Request::new("POST", format!("https://cms.megaphone.fm/api/networks/{net}/podcasts/{pod}/episodes"))
            .header("Authorization", format!("Token token=\"{key}\""))
            .json(json!({"title": ep.title, "summary": ep.description, "pubdate": super::now_iso(), "backgroundAudioFileUrl": url, "draft": !ep.publish})),
    )?;
    Ok(format!("Megaphone episode {}", s(&created["id"])))
}

/// Percent-encodes a query value.
pub fn url_encode(v: &str) -> String {
    let mut o = String::new();
    for b in v.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(char::from(b)),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::super::http::{Body, Fake, ok};
    use super::*;

    fn episode() -> (Episode, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("tc-hosts-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("folge 7.m4a");
        std::fs::write(&audio, vec![0u8; 1234]).unwrap();
        let ep = Episode {
            audio,
            audio_mime: "audio/mp4".into(),
            title: "Folge 7".into(),
            description: "Worum es geht".into(),
            number: 7,
            season: 1,
            publish: true,
            audio_url: Some("https://files.example/folge7.m4a".into()),
        };
        (ep, dir)
    }

    fn rand_suffix() -> u64 {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u8(0);
        h.finish()
    }

    fn cfg() -> HostConfig {
        HostConfig { api_key: "KEY".into(), show_id: "42".into(), user: "U".into() }
    }

    fn run(host: Host, answers: Vec<super::super::http::Response>) -> (Result<String, String>, Vec<Request>) {
        let (ep, dir) = episode();
        let mut t = Fake { sent: Vec::new(), answers };
        let r = publish(host, &cfg(), &ep, &mut t, &mut |_| {});
        let _ = std::fs::remove_dir_all(dir);
        (r, t.sent)
    }

    fn calls(sent: &[Request]) -> Vec<String> {
        sent.iter().map(|r| format!("{} {}", r.method, r.url.split('?').next().unwrap_or(""))).collect()
    }

    #[test]
    fn buzzsprout_creates_uploads_completes_and_publishes() {
        let (r, sent) = run(
            Host::Buzzsprout,
            vec![
                ok(201, json!({"id": 9})),
                ok(201, json!({"upload": {"id": "u1", "upload_url": "https://s3.example/put"}})),
                ok(200, json!({})),
                ok(200, json!({})),
                ok(200, json!({})),
            ],
        );
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(
            calls(&sent),
            [
                "POST https://www.buzzsprout.com/api/42/episodes",
                "POST https://www.buzzsprout.com/api/42/episodes/9/uploads",
                "PUT https://s3.example/put",
                "POST https://www.buzzsprout.com/api/42/episodes/9/uploads/u1/complete",
                "PATCH https://www.buzzsprout.com/api/42/episodes/9"
            ]
        );
        assert!(sent[0].headers.contains(&("Authorization".into(), "Token token=KEY".into())));
        assert!(sent[2].headers.iter().all(|(k, _)| k != "Authorization"), "no API key on the storage upload");
        assert_eq!(sent[1].body, Body::Json(json!({"filename": "folge 7.m4a", "type": "audio/mp4", "byte_size": 1234})));
    }

    #[test]
    fn transistor_podigee_podbean_follow_their_flows() {
        let (r, sent) = run(
            Host::Transistor,
            vec![
                ok(
                    200,
                    json!({"data": {"attributes": {"upload_url": "https://s3/x", "content_type": "audio/mp4", "audio_url": "https://cdn/x.m4a"}}}),
                ),
                ok(200, json!({})),
                ok(201, json!({"data": {"id": "77"}})),
                ok(200, json!({})),
            ],
        );
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(
            calls(&sent),
            [
                "GET https://api.transistor.fm/v1/episodes/authorize_upload",
                "PUT https://s3/x",
                "POST https://api.transistor.fm/v1/episodes",
                "PATCH https://api.transistor.fm/v1/episodes/77/publish"
            ]
        );
        assert!(sent[0].url.contains("filename=folge%207.m4a"));

        let (r, sent) = run(
            Host::Podigee,
            vec![
                ok(201, json!({"id": 5})),
                ok(200, json!({"upload_url": "https://s3/p", "content_type": "audio/mp4", "file_url": "https://f/p"})),
                ok(200, json!({})),
                ok(201, json!({"id": 8})),
                ok(200, json!({})),
            ],
        );
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(sent[4].url, "https://app.podigee.com/api/v1/productions/8/start?publish_episode=true");
        assert_eq!(sent[3].body, Body::Json(json!({"episode_id": 5, "files": [{"url": "https://f/p"}]})));

        let (r, sent) = run(
            Host::Podbean,
            vec![
                ok(200, json!({"access_token": "T"})),
                ok(200, json!({"presigned_url": "https://s3/b", "file_key": "K"})),
                ok(200, json!({})),
                ok(200, json!({"episode": {"permalink_url": "https://pb/e"}})),
            ],
        );
        assert_eq!(r, Ok("https://pb/e".to_string()));
        assert!(sent[1].url.contains("filesize=1234") && sent[1].url.contains("content_type=audio%2Fmp4"));
        let Body::Form(f) = &sent[3].body else { panic!("form") };
        assert!(f.contains(&("media_key".into(), "K".into())) && f.contains(&("status".into(), "publish".into())));
    }

    #[test]
    fn spreaker_captivate_rss_com_follow_their_flows() {
        let (r, sent) = run(Host::Spreaker, vec![ok(200, json!({"response": {"episode": {"episode_id": 3, "site_url": "https://spreaker/e"}}}))]);
        assert_eq!(r, Ok("https://spreaker/e".to_string()));
        assert_eq!(calls(&sent), ["POST https://api.spreaker.com/v2/shows/42/episodes"]);

        let (r, sent) = run(
            Host::Captivate,
            vec![ok(200, json!({"user": {"token": "T"}})), ok(200, json!({"media": {"id": "m1"}})), ok(200, json!({"episode": {"id": "e1"}}))],
        );
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(
            calls(&sent),
            [
                "POST https://api.captivate.fm/authenticate/token",
                "POST https://api.captivate.fm/shows/42/media",
                "POST https://api.captivate.fm/episodes"
            ]
        );

        let (r, sent) = run(
            Host::RssCom,
            vec![
                ok(201, json!({"id": "up1", "url": "https://s3/r"})),
                ok(200, json!({})),
                ok(201, json!({"id": 4, "dashboard_url": "https://rss.com/d"})),
            ],
        );
        assert_eq!(r, Ok("https://rss.com/d".to_string()));
        let Body::Json(b) = &sent[2].body else { panic!("json") };
        assert_eq!(b["audio_upload_id"], "up1");
        assert!(b["schedule_datetime"].is_string(), "published now");
    }

    #[test]
    fn errors_name_the_step_and_missing_keys_are_caught_before_any_request() {
        let (r, sent) = run(Host::Buzzsprout, vec![ok(401, json!({"error": "bad token"}))]);
        let e = r.unwrap_err();
        assert!(e.contains("HTTP 401") && e.contains("bad token"), "{e}");
        assert_eq!(sent.len(), 1, "stops at the first failure");
        let (ep, dir) = episode();
        let mut t = Fake::default();
        let r = publish(Host::Transistor, &HostConfig::default(), &ep, &mut t, &mut |_| {});
        assert!(r.unwrap_err().contains("API key is missing"));
        assert!(t.sent.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
