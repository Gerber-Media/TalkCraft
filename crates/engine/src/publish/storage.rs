//! Where the own feed and its files live: a Git repository served as a website (e.g. Codefloe or
//! GitHub Pages), an SFTP server (key login), FTP/FTPS or WebDAV. Uploads go through the system's
//! `git`, `sftp` and `curl` programs.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    #[default]
    Git,
    Sftp,
    Ftp,
    WebDav,
}

impl StorageKind {
    pub const ALL: [StorageKind; 4] = [StorageKind::Git, StorageKind::Sftp, StorageKind::Ftp, StorageKind::WebDav];
    pub fn name(self) -> &'static str {
        match self {
            StorageKind::Git => "Git (e.g. Codefloe/GitHub Pages)",
            StorageKind::Sftp => "SFTP (key login)",
            StorageKind::Ftp => "FTP / FTPS",
            StorageKind::WebDav => "WebDAV",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Storage {
    pub kind: StorageKind,
    /// Git: the repository URL (HTTPS). SFTP: the host. FTP/WebDAV: the server URL
    /// (ftp://, ftps://, https://).
    pub url: String,
    pub user: String,
    /// FTP/WebDAV password (SFTP uses the SSH key, Git the system's credential helper).
    pub password: String,
    /// SFTP port (0 = 22).
    pub port: u16,
    /// Folder on the server or in the repository ("" = top level).
    pub dir: String,
    /// Git branch the site is served from (Codefloe Pages: "pages").
    pub branch: String,
    /// Public web address of that folder, e.g. https://name.codefloe.page/podcast/.
    pub public_base: String,
}

/// A file's public URL.
pub fn public_url(st: &Storage, name: &str) -> String {
    format!("{}/{}", st.public_base.trim_end_matches('/'), super::hosts::url_encode(name))
}

fn run(c: &mut Command, what: &str) -> Result<String, String> {
    let out = c.output().map_err(|e| format!("{what}: could not start ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { err };
        return Err(format!("{what} failed: {}", msg.chars().take(400).collect::<String>()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Uploads `files` (local path, name on the server). `work` is a folder TalkCraft keeps for this
/// storage (the Git checkout).
pub fn upload(st: &Storage, files: &[(PathBuf, String)], work: &Path, message: &str, log: &mut dyn FnMut(String)) -> Result<(), String> {
    if st.url.trim().is_empty() {
        return Err("the storage has no address".into());
    }
    match st.kind {
        StorageKind::Git => git(st, files, work, message, log),
        StorageKind::Sftp => sftp(st, files, log),
        StorageKind::Ftp => curl_put(st, files, log, false),
        StorageKind::WebDav => curl_put(st, files, log, true),
    }
}

fn git(st: &Storage, files: &[(PathBuf, String)], work: &Path, message: &str, log: &mut dyn FnMut(String)) -> Result<(), String> {
    let repo = work.join("repo");
    let branch = if st.branch.trim().is_empty() { "pages" } else { st.branch.trim() };
    let g = |args: &[&str]| {
        let mut c = Command::new("git");
        // Never wait for a password prompt nobody sees: credentials come from the system.
        c.env("GIT_TERMINAL_PROMPT", "0").arg("-C").arg(&repo).args(args);
        c
    };
    if repo.join(".git").is_dir() {
        log("Git: fetching the latest state".into());
        run(&mut g(&["fetch", "origin"]), "git fetch")?;
        // The branch may not exist yet on the server (first publish).
        if run(&mut g(&["rev-parse", "--verify", &format!("origin/{branch}")]), "git rev-parse").is_ok() {
            run(&mut g(&["checkout", "-B", branch, &format!("origin/{branch}")]), "git checkout")?;
        }
    } else {
        log("Git: cloning the repository".into());
        std::fs::create_dir_all(work).map_err(|e| e.to_string())?;
        let mut c = Command::new("git");
        c.env("GIT_TERMINAL_PROMPT", "0").args(["clone", st.url.trim()]).arg(&repo);
        run(&mut c, "git clone")?;
        let has = run(&mut g(&["rev-parse", "--verify", &format!("origin/{branch}")]), "git rev-parse").is_ok();
        if has {
            run(&mut g(&["checkout", "-B", branch, &format!("origin/{branch}")]), "git checkout")?;
        } else {
            run(&mut g(&["checkout", "--orphan", branch]), "git checkout")?;
        }
    }
    let target = repo.join(st.dir.trim_matches('/'));
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    for (local, name) in files {
        std::fs::copy(local, target.join(name)).map_err(|e| format!("{}: {e}", local.display()))?;
    }
    run(&mut g(&["add", "-A"]), "git add")?;
    let mut commit = g(&["-c", "user.name=TalkCraft", "-c", "user.email=talkcraft@localhost", "commit", "-m", message]);
    // Nothing new (a re-publish of identical files) is fine.
    if let Err(e) = run(&mut commit, "git commit")
        && !e.contains("nothing to commit")
    {
        return Err(e);
    }
    log("Git: pushing".into());
    run(&mut g(&["push", "origin", &format!("HEAD:{branch}")]), "git push")?;
    Ok(())
}

fn sftp(st: &Storage, files: &[(PathBuf, String)], log: &mut dyn FnMut(String)) -> Result<(), String> {
    let port = if st.port == 0 { 22 } else { st.port };
    let target = if st.user.trim().is_empty() { st.url.trim().to_string() } else { format!("{}@{}", st.user.trim(), st.url.trim()) };
    // A leading "-" lets a batch line fail without stopping (mkdir of an existing folder).
    let mut batch = String::new();
    let dir = st.dir.trim_matches('/');
    if !dir.is_empty() {
        let mut path = String::new();
        for part in dir.split('/') {
            path = if path.is_empty() { part.to_string() } else { format!("{path}/{part}") };
            batch.push_str(&format!("-mkdir \"{path}\"\n"));
        }
        batch.push_str(&format!("cd \"{dir}\"\n"));
    }
    for (local, name) in files {
        batch.push_str(&format!("put \"{}\" \"{name}\"\n", local.display()));
    }
    log("SFTP: uploading".into());
    let mut child = Command::new("sftp")
        .args(["-b", "-", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=accept-new", "-P", &port.to_string(), &target])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("sftp: could not start ({e})"))?;
    if let Some(mut i) = child.stdin.take() {
        i.write_all(batch.as_bytes()).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("sftp failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

fn curl_put(st: &Storage, files: &[(PathBuf, String)], log: &mut dyn FnMut(String), webdav: bool) -> Result<(), String> {
    let base = st.url.trim().trim_end_matches('/');
    let dir = st.dir.trim_matches('/');
    let folder = if dir.is_empty() { base.to_string() } else { format!("{base}/{dir}") };
    let auth = format!("{}:{}", st.user, st.password);
    if webdav && !dir.is_empty() {
        // Each level of the folder; an existing one answers 405, which is fine.
        let mut path = base.to_string();
        for part in dir.split('/') {
            path = format!("{path}/{part}");
            let _ = Command::new("curl").args(["-sS", "-o", "/dev/null", "-X", "MKCOL", "--user", &auth]).arg(format!("{path}/")).output();
        }
    }
    for (local, name) in files {
        log(format!("Uploading {name}"));
        let mut c = Command::new("curl");
        c.args(["-sS", "--fail", "--connect-timeout", "30", "--max-time", "7200", "--user", &auth, "-T"]).arg(local);
        if !webdav {
            c.arg("--ftp-create-dirs");
        }
        c.arg(format!("{folder}/{}", super::hosts::url_encode(name)));
        run(&mut c, &format!("upload of {name}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_ok(args: &[&str], dir: &Path) {
        let st = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
    }

    #[test]
    fn git_storage_publishes_to_a_branch_and_updates_it_next_time() {
        let root = std::env::temp_dir().join(format!("tc-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let remote = root.join("remote.git");
        git_ok(&["init", "--bare", "-q", remote.to_str().unwrap()], &root);
        let st = Storage {
            kind: StorageKind::Git,
            url: remote.to_string_lossy().into(),
            dir: "podcast".into(),
            branch: "pages".into(),
            public_base: "https://x.example/podcast/".into(),
            ..Default::default()
        };
        let f1 = root.join("feed.xml");
        std::fs::write(&f1, "<rss>1</rss>").unwrap();
        let work = root.join("work");
        upload(&st, &[(f1.clone(), "feed.xml".into())], &work, "Folge 1", &mut |_| {}).unwrap();
        std::fs::write(&f1, "<rss>2</rss>").unwrap();
        upload(&st, &[(f1.clone(), "feed.xml".into())], &work, "Folge 2", &mut |_| {}).unwrap();
        // What the server has on the branch.
        let show = Command::new("git").args(["--git-dir", remote.to_str().unwrap(), "show", "pages:podcast/feed.xml"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&show.stdout), "<rss>2</rss>");
        assert_eq!(public_url(&st, "folge 1.m4a"), "https://x.example/podcast/folge%201.m4a");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_address_is_refused() {
        let st = Storage::default();
        assert!(upload(&st, &[], Path::new("/tmp"), "x", &mut |_| {}).is_err());
    }
}
