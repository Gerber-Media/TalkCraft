//! TalkCraft's running version for the app, the CLI and the About window:
//! `TALKCRAFT_MINOR.<commits since TALKCRAFT_FORK_BASE>` from `packaging/version.env` and git at
//! build time. `TALKCRAFT_VERSION` in the environment overrides it (the packaging scripts pass the
//! number they put on the installers); without git, `packaging/talkcraft-version` (written by the
//! packaging scripts), else `<minor>.0-dev`.

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env_file = root.join("packaging/version.env");
    println!("cargo::rerun-if-env-changed=TALKCRAFT_VERSION");
    let generated = root.join("packaging/talkcraft-version");
    // Cargo treats a watched file that does not exist as always changed (a rebuild every time),
    // so only existing files are watched.
    for f in [env_file.clone(), generated.clone(), root.join(".git/HEAD"), root.join(".git/refs/heads"), root.join(".git/packed-refs")] {
        if f.exists() {
            println!("cargo::rerun-if-changed={}", f.display());
        }
    }
    let text = std::fs::read_to_string(&env_file).unwrap_or_default();
    let get = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('=').map(|v| v.trim().to_string()));
    let minor = get("TALKCRAFT_MINOR").unwrap_or_else(|| "0.1".into());
    let base = get("TALKCRAFT_FORK_BASE").unwrap_or_default();
    let build = std::process::Command::new("git")
        .args(["rev-list", "--count", &format!("{base}..HEAD")])
        .current_dir(&root)
        .output()
        .ok()
        .filter(|o| o.status.success() && !base.is_empty())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok());
    // Order: the packaging scripts' environment, git, the number they left for sandboxed builds.
    let version = std::env::var("TALKCRAFT_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| build.map(|n| format!("{minor}.{n}")))
        .or_else(|| std::fs::read_to_string(&generated).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| {
            println!("cargo::warning=TalkCraft version: the fork commit is not in this checkout (shallow clone?); building as {minor}.0-dev");
            format!("{minor}.0-dev")
        });
    let build = build.unwrap_or(0);
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("talkcraft_version.rs");
    let code = format!(
        "/// TalkCraft's version (`minor.build`).\npub const VERSION: &str = {version:?};\n/// Commits since the fork.\npub const BUILD: u32 = {build};\n"
    );
    if let Err(e) = std::fs::write(&out, code) {
        println!("cargo::warning=writing {}: {e}", out.display());
    }
}
