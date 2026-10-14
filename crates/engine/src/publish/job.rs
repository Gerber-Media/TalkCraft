//! One publish: the exported episode files to the chosen target. Runs off the UI thread; reports
//! progress through `log` and returns where the episode can be found.

use super::feed::{Channel, Feed, Item};
use super::hosts::{Episode, Host, HostConfig};
use super::http::Transport;
use super::storage::{Storage, public_url, upload};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "host")]
pub enum Target {
    /// The own RSS feed on the storage.
    #[default]
    OwnFeed,
    Host(Host),
}

/// The show's details for the own feed that the episode does not carry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShowInfo {
    pub description: String,
    pub email: String,
    pub website: String,
    pub category: String,
    pub explicit: bool,
}

/// Everything the user set up for publishing (kept by the app, with the keys, outside sessions).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub target: Target,
    pub storage: Storage,
    pub show: ShowInfo,
    pub hosts: BTreeMap<String, HostConfig>,
    /// Publish right away; off: hosts get a draft to check first.
    pub publish_now: bool,
}

/// The exported files and the episode's details.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prepared {
    pub audio: PathBuf,
    pub audio_mime: String,
    pub duration_s: u64,
    pub show: String,
    pub author: String,
    pub language: String,
    pub title: String,
    pub description: String,
    pub number: u32,
    pub season: u32,
    pub cover: Option<PathBuf>,
    pub chapters: Option<PathBuf>,
    pub transcript: Option<PathBuf>,
}

/// A file name for the web: letters, digits and dashes.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = match c {
            'ä' | 'Ä' => "ae".to_string(),
            'ö' | 'Ö' => "oe".to_string(),
            'ü' | 'Ü' => "ue".to_string(),
            'ß' => "ss".to_string(),
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase().to_string(),
            _ => "-".to_string(),
        };
        out.push_str(&c);
    }
    let mut s = out.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    s.truncate(60);
    if s.is_empty() { "folge".into() } else { s }
}

fn ext(p: &Path) -> String {
    p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Publishes; `feed_state` is where the own feed's list of episodes is kept, `work` a folder for
/// the storage (Git checkout).
pub fn run(
    settings: &Settings,
    ep: &Prepared,
    feed_state: &Path,
    work: &Path,
    t: &mut dyn Transport,
    log: &mut dyn FnMut(String),
) -> Result<String, String> {
    match settings.target {
        Target::OwnFeed => own_feed(settings, ep, feed_state, work, log),
        Target::Host(h) => {
            let cfg = settings.hosts.get(h.id()).cloned().unwrap_or_default();
            let mut episode = Episode {
                audio: ep.audio.clone(),
                audio_mime: ep.audio_mime.clone(),
                title: ep.title.clone(),
                description: ep.description.clone(),
                number: ep.number,
                season: ep.season,
                publish: settings.publish_now,
                audio_url: None,
            };
            if h.needs_public_url() {
                let name = format!("{}.{}", base_name(ep), ext(&ep.audio));
                log(format!("{}: putting the audio on your storage first", h.name()));
                upload(&settings.storage, &[(ep.audio.clone(), name.clone())], work, &format!("Audio: {}", ep.title), log)?;
                episode.audio_url = Some(public_url(&settings.storage, &name));
            }
            super::hosts::publish(h, &cfg, &episode, t, log)
        }
    }
}

fn base_name(ep: &Prepared) -> String {
    if ep.number > 0 { format!("{:03}-{}", ep.number, slug(&ep.title)) } else { slug(&ep.title) }
}

fn own_feed(settings: &Settings, ep: &Prepared, feed_state: &Path, work: &Path, log: &mut dyn FnMut(String)) -> Result<String, String> {
    let st = &settings.storage;
    if st.public_base.trim().is_empty() {
        return Err("the storage needs its public web address (where feed.xml will be reachable)".into());
    }
    let mut feed: Feed = std::fs::read_to_string(feed_state).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let base = base_name(ep);
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let audio_name = format!("{base}.{}", ext(&ep.audio));
    files.push((ep.audio.clone(), audio_name.clone()));
    let mut item = Item {
        guid: format!("talkcraft-{base}"),
        title: ep.title.clone(),
        description: ep.description.clone(),
        audio_url: public_url(st, &audio_name),
        audio_bytes: std::fs::metadata(&ep.audio).map(|m| m.len()).map_err(|e| format!("{}: {e}", ep.audio.display()))?,
        audio_type: ep.audio_mime.clone(),
        duration_s: ep.duration_s,
        pub_date: super::feed::rfc2822(super::unix_now()),
        episode: ep.number,
        season: ep.season,
        ..Default::default()
    };
    // A re-publish of the same episode keeps its date.
    if let Some(old) = feed.items.iter().find(|i| i.guid == item.guid) {
        item.pub_date = old.pub_date.clone();
    }
    if let Some(c) = &ep.chapters {
        let n = format!("{base}.chapters.json");
        item.chapters_url = public_url(st, &n);
        files.push((c.clone(), n));
    }
    if let Some(v) = &ep.transcript {
        let n = format!("{base}.vtt");
        item.transcript_url = public_url(st, &n);
        files.push((v.clone(), n));
    }
    let mut channel = Channel {
        title: if ep.show.trim().is_empty() { ep.title.clone() } else { ep.show.clone() },
        description: settings.show.description.clone(),
        author: ep.author.clone(),
        email: settings.show.email.clone(),
        link: settings.show.website.clone(),
        image: feed.channel.image.clone(),
        language: ep.language.clone(),
        category: settings.show.category.clone(),
        explicit: settings.show.explicit,
        feed_url: public_url(st, "feed.xml"),
    };
    if channel.description.trim().is_empty() {
        channel.description = channel.title.clone();
    }
    if let Some(c) = &ep.cover {
        let n = format!("cover.{}", ext(c));
        channel.image = public_url(st, &n);
        files.push((c.clone(), n));
    }
    feed.channel = channel;
    feed.add(item);
    let xml_path = work.join("feed.xml");
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;
    std::fs::write(&xml_path, feed.to_xml()).map_err(|e| e.to_string())?;
    files.push((xml_path, "feed.xml".into()));
    upload(st, &files, work, &format!("Folge: {}", ep.title), log)?;
    if let Some(dir) = feed_state.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(feed_state, serde_json::to_string_pretty(&feed).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(public_url(st, "feed.xml"))
}

#[cfg(test)]
mod tests {
    use super::super::http::{Fake, ok};
    use super::super::storage::StorageKind;
    use super::*;
    use std::process::Command;

    #[test]
    fn slugs_for_the_web() {
        assert_eq!(slug("Energie für Dänemark – Teil 2!"), "energie-fuer-daenemark-teil-2");
        assert_eq!(slug("???"), "folge");
    }

    fn files(root: &Path) -> Prepared {
        std::fs::create_dir_all(root).unwrap();
        let audio = root.join("ep.m4a");
        std::fs::write(&audio, vec![1u8; 500]).unwrap();
        let ch = root.join("ep.chapters.json");
        std::fs::write(&ch, "{}").unwrap();
        let cover = root.join("cover.jpg");
        std::fs::write(&cover, "jpg").unwrap();
        Prepared {
            audio,
            audio_mime: "audio/mp4".into(),
            duration_s: 61,
            show: "Europa 2.0".into(),
            author: "Dirk & Sven".into(),
            language: "de".into(),
            title: "Energie".into(),
            description: "Worum es geht".into(),
            number: 12,
            season: 1,
            cover: Some(cover),
            chapters: Some(ch),
            transcript: None,
        }
    }

    #[test]
    fn the_own_feed_goes_to_git_with_the_audio_and_lists_every_episode() {
        let root = std::env::temp_dir().join(format!("tc-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let remote = root.join("remote.git");
        assert!(Command::new("git").args(["init", "--bare", "-q"]).arg(&remote).status().unwrap().success());
        let settings = Settings {
            storage: Storage {
                kind: StorageKind::Git,
                url: remote.to_string_lossy().into(),
                branch: "pages".into(),
                public_base: "https://x.example/".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut ep = files(&root.join("export"));
        let state = root.join("feeds/europa.json");
        let mut t = Fake::default();
        let url = run(&settings, &ep, &state, &root.join("work"), &mut t, &mut |_| {}).unwrap();
        assert_eq!(url, "https://x.example/feed.xml");
        ep.number = 13;
        ep.title = "Wasser".into();
        run(&settings, &ep, &state, &root.join("work"), &mut t, &mut |_| {}).unwrap();
        let show = |p: &str| {
            String::from_utf8_lossy(&Command::new("git").arg("--git-dir").arg(&remote).args(["show", &format!("pages:{p}")]).output().unwrap().stdout)
                .into_owned()
        };
        let xml = show("feed.xml");
        assert!(xml.contains("<title>Europa 2.0</title>") && xml.contains("012-energie.m4a") && xml.contains("013-wasser.m4a"), "{xml}");
        assert!(xml.contains("https://x.example/cover.jpg") && xml.contains("012-energie.chapters.json"));
        assert_eq!(show("012-energie.m4a").len(), 500, "the audio is on the server");
        assert!(t.sent.is_empty(), "no host API involved");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_host_gets_the_episode_and_url_only_hosts_get_the_storage_url() {
        let root = std::env::temp_dir().join(format!("tc-job-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ep = files(&root);
        let mut settings = Settings { target: Target::Host(Host::Spreaker), publish_now: true, ..Default::default() };
        settings.hosts.insert("spreaker".into(), HostConfig { api_key: "T".into(), show_id: "9".into(), user: String::new() });
        let mut t = Fake { sent: Vec::new(), answers: vec![ok(200, serde_json::json!({"response": {"episode": {"site_url": "https://s/e"}}}))] };
        assert_eq!(run(&settings, &ep, &root.join("s.json"), &root.join("w"), &mut t, &mut |_| {}), Ok("https://s/e".into()));
        // Castos without a storage: refused before any request.
        settings.target = Target::Host(Host::Castos);
        settings.hosts.insert("castos".into(), HostConfig { api_key: "T".into(), show_id: "9".into(), user: String::new() });
        let mut t = Fake::default();
        assert!(run(&settings, &ep, &root.join("s.json"), &root.join("w"), &mut t, &mut |_| {}).is_err());
        assert!(t.sent.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
