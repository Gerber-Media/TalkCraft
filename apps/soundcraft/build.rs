//! Windows only: embed the app icon and version info (VERSIONINFO) into `talkcraft.exe`, so it
//! shows in Explorer, the taskbar, the Start menu and Alt-Tab.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `TALKCRAFT_REQUIRE_WINRES=1` turns it
//! into an error (for release builds).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/talkcraft.ico");
    println!("cargo:rerun-if-env-changed=TALKCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/talkcraft.ico")
        .set("ProductName", "TalkCraft")
        .set("FileDescription", "TalkCraft podcast studio")
        .set("CompanyName", "GERBER.MEDIA")
        .set("LegalCopyright", "Copyright (c) GERBER.MEDIA and the SoundCraft authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "talkcraft.exe")
        .set("InternalName", "talkcraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("TALKCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=talkcraft.exe built without icon/version resources: {e}");
    }
}
