//! The podcast studio's slimmer surface: music, notation and picture features stay in the
//! engine (and the CLI, control channel and MCP) but leave the menus and dialogs.

use crate::SoundApp;
use crate::widgets::{icon_button, rec_toggle};
use serde_json::json;
use soundcraft_model::{Track, TrackKind};

/// The product name shown in menus, titles and the About window.
pub const APP_NAME: &str = "TalkCraft";

/// Menu roots a podcast never needs: MIDI/tempo events.
const HIDDEN_ROOTS: &[&str] = &["Event"];

/// Menu paths containing any of these are hidden (matched case-sensitively, so "HEAT" does not
/// hide "Repeat" and "Meter" stays visible as "Meters and Faders").
const HIDDEN_PARTS: &[&str] = &[
    "MIDI",
    "Score",
    "Sibelius",
    "Video",
    "Instrument",
    "Sketch",
    "Project",
    "Tempo",
    "Change Meter",
    "Rulers > Meter",
    "Key Signature",
    "Chord",
    "Bars|Beats",
    "Feet+Frames",
    "Dolby Atmos",
    "HEAT",
    "Machine Track",
    "VCA",
    "Beat Detective",
    "Renumber Bars",
    "Quantize",
    "Note Overlay",
    "Collaboration",
    "Synchronization",
    "Clip Groups",
    // Setup entries without a window yet (or with nothing to set in a podcast).
    "Disk Allocation",
    "Peripherals",
    "Timecode",
    "Click/Countoff",
    "Transcription Settings",
];

/// Single entries a podcast does not need: items the fork never implemented (Dashboard, AAF,
/// Melodyne …), music and film tools (punch modes, click, elastic audio, timecode spotting) and
/// big-studio views (mic preamps, object, ten sends).
const HIDDEN_EXACT: &[&str] = &[
    "Setup > Hardware...",
    "AudioSuite > Delay > Mod Delay III",
    "AudioSuite > Harmonic > Eleven Lite",
    "AudioSuite > Harmonic > Lo-Fi",
    "AudioSuite > Harmonic > Recti-Fi",
    "AudioSuite > Harmonic > SansAmp PSA-1",
    "AudioSuite > Modulation > Sci-Fi",
    "AudioSuite > Other > Signal Generator",
    "AudioSuite > Pitch Shift > Pitch II",
    "AudioSuite > Pitch Shift > Pitch Shift Legacy",
    "AudioSuite > Pitch Shift > Vari-Fi",
    "Clip > Capture...",
    "Clip > Elastic Properties",
    "Clip > Identify Sync Point",
    "Clip > Loop...",
    "Clip > Remove Pitch Shift",
    "Clip > Remove Warp",
    "Clip > Time Lock/Unlock",
    "Clip > Unloop...",
    "Edit > Automation > Coalesce Pan Guide Automation",
    "Edit > Automation > Glide Pan Only",
    "Edit > Automation > Glide to All Enabled",
    "Edit > Automation > Glide to Current",
    "Edit > Clear Special > Pan Automation",
    "Edit > Clear Special > Plugin Automation",
    "Edit > Copy Selection to... > Target Playlist",
    "Edit > Copy Special > Pan Automation",
    "Edit > Copy Special > Plugin Automation",
    "Edit > Cut Special > Pan Automation",
    "Edit > Cut Special > Plugin Automation",
    "Edit > Move Selection to... > Target Playlist",
    "Edit > Selection > Double Edit",
    "Edit > Selection > Duplicate and Extend Edit",
    "Edit > Selection > Extend Edit Selection To Marker Lane Above",
    "Edit > Selection > Extend Edit Selection To Marker Lane Below",
    "Edit > Selection > Halve Edit",
    "Edit > Selection > Move Edit Selection To Marker Lane Above",
    "Edit > Selection > Move Edit Selection To Marker Lane Below",
    "Edit > Separate > At Transients",
    "Edit > Separate > On Grid",
    "Edit > TCE Edit to Timeline Selection",
    "File > Dashboard...",
    "File > Export > Selected Tracks as New AAF/OMF...",
    "File > Export > Selected Tracks as New Media Composer Compatible Session...",
    "File > Export > Session Info as Text...",
    "File > Get Info...",
    "File > Import > VENUE Channel Names as Track Names",
    "File > Sign In",
    "Options > Calibration Mode",
    "Options > Click",
    "Options > Destructive Record",
    "Options > DestructivePunch",
    "Options > Dynamic Transport",
    "Options > Edit/Tool Mode Keyboard Lock",
    "Options > Layered Editing",
    "Options > Prepare DPE Tracks",
    "Options > QuickPunch",
    "Options > Solo Mode > AFL (After Fader Listen)",
    "Options > Solo Mode > PFL (Pre Fader Listen)",
    "Options > Solo Mode > X-OR (Cancels Previous Solo)",
    "Options > Tab to Transient",
    "Options > TrackPunch",
    "Options > Transport Online",
    "Track > Commit...",
    "Track > Convert Aux to Routing Folder",
    "Track > Create Click Track",
    "Track > Designate as Target Playlist",
    "Track > Freeze",
    "Track > Set Record Tracks to Input Only",
    "Track > Show Target Playlist",
    "Track > Split into Mono",
    "View > Clip > Original Time Stamp",
    "View > Clip > Scene And Take",
    "View > Clip > Sync Point",
    "View > Clip > User Time Stamp",
    "View > Edit Window Views > Inserts F-J",
    "View > Edit Window Views > Mic Preamps",
    "View > Edit Window Views > Object",
    "View > Edit Window Views > Real-Time Properties",
    "View > Edit Window Views > Sends F-J",
    "View > Main Counter > Samples",
    "View > Mix Window Views > Delay Compensation",
    "View > Mix Window Views > Inserts F-J",
    "View > Mix Window Views > Mic Preamps",
    "View > Mix Window Views > Object",
    "View > Mix Window Views > Sends F-J",
    "View > Rulers > Markers 2",
    "View > Rulers > Markers 3",
    "View > Rulers > Markers 4",
    "View > Rulers > Markers 5",
    "View > Rulers > Samples",
    "View > Transport > Ableton Link",
    "Window > ARA Plugins",
    "Window > Learn",
    "Window > Renderer",
    "Window > SoundFlow",
    "Window > Splice",
    "Window > Task Manager",
    "Window > Transcript",
    "Window > Trim and Downmix",
];

/// Whole submenus a podcast does not need.
const HIDDEN_PREFIXES: &[&str] = &[
    "Clip > Rating",
    "View > Expanded Sends",
    "Window > Configurations",
    "Window > Workspaces",
    "Window > New Workspace",
    "Track > ARA Plugins",
    "Clip > ARA Plugins",
];

/// Track widths a podcast uses; the surround and Ambisonics formats are left out.
const KEPT_WIDTHS: &[&str] = &["Mono", "Stereo"];

/// Whether a menu catalog line (`Root > … > Leaf`) is left out of the podcast menus.
pub fn hidden_menu(path: &str) -> bool {
    let root = path.split(" > ").next().unwrap_or(path);
    if let Some(width) = path.strip_prefix("Track > Change Track Width > ") {
        return !KEPT_WIDTHS.contains(&width);
    }
    HIDDEN_ROOTS.contains(&root)
        || HIDDEN_PARTS.iter().any(|p| path.contains(p))
        || HIDDEN_EXACT.contains(&path)
        || HIDDEN_PREFIXES.iter().any(|p| path.starts_with(p))
}

/// Neutral names for menu items the clean-room original named after Avid products.
pub fn menu_label(label: &str) -> &str {
    match label {
        "EQ3 1-Band" => "EQ 1-Band",
        "EQ3 7-Band" => "EQ 7-Band",
        "BF-76" => "FET Compressor",
        "Dyn3 Compressor/Limiter" => "Compressor/Limiter",
        "Dyn3 De-Esser" => "De-Esser",
        "Dyn3 Expander/Gate" => "Expander/Gate",
        "Maxim" => "Maximizer",
        "D-Verb" => "Reverb",
        "Pro Tools Search" => "Search",
        "Playback Engine..." => "Audio Interface...",
        other => other,
    }
}

/// Track kinds the New Tracks dialog does not offer.
pub fn hidden_track_kind(k: TrackKind) -> bool {
    matches!(k, TrackKind::Midi | TrackKind::Instrument | TrackKind::Vca | TrackKind::Video)
}

/// A track's record button (records this speaker now; lit while it records) and its lock
/// (left out of recording).
pub fn record_buttons(app: &mut SoundApp, ui: &mut egui::Ui, track: &Track, size: egui::Vec2) {
    let id = track.id;
    let recording = app.engine.transport.recording && track.mixer.record_arm;
    let locked = track.mixer.record_lock;
    let tip = if recording {
        "Recording — click to stop"
    } else if locked {
        "Locked against recording (unlock with the lock button)"
    } else {
        "Record this track now"
    };
    let resp = ui.add_enabled_ui(!locked || recording, |ui| rec_toggle(ui, size, recording, tip)).inner;
    if resp.clicked() {
        let r = if app.engine.transport.recording {
            app.run("transport.record", json!({}))
        } else {
            app.run("transport.record_track", json!({"track": id.0}))
        };
        if let Err(e) = r {
            app.ui.status = e;
        }
    }
    let lock_tip = if locked { "Locked: Record leaves this track out (click to unlock)" } else { "Lock: leave this track out when recording" };
    if icon_button(ui, size, "lock", locked, lock_tip).clicked() {
        let _ = app.run("mix.record_lock", json!({"track": id.0}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_and_picture_menus_are_hidden_and_podcast_essentials_stay() {
        for p in [
            "Event > Beat Detective",
            "File > Import > MIDI...",
            "File > Print Score...",
            "Window > Video",
            "View > Rulers > Bars|Beats",
            "Clip > Quantize to Grid",
        ] {
            assert!(hidden_menu(p), "{p} is hidden");
        }
        for p in [
            "File > Bounce Mix...",
            "File > Import > Audio...",
            "Edit > Strip Silence",
            "View > Mix Window Views > Meters and Faders",
            "Track > New...",
        ] {
            assert!(!hidden_menu(p), "{p} stays");
        }
    }

    #[test]
    fn new_tracks_offer_audio_aux_and_folders() {
        let shown: Vec<_> = TrackKind::ALL.into_iter().filter(|k| !hidden_track_kind(*k)).collect();
        assert!(shown.contains(&TrackKind::Audio) && shown.contains(&TrackKind::Aux));
        assert!(!shown.contains(&TrackKind::Midi) && !shown.contains(&TrackKind::Instrument));
    }
}
