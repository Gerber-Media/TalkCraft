//! The podcast studio's slimmer surface: music, notation and picture features stay in the
//! engine (and the CLI, control channel and MCP) but leave the menus and dialogs.

use crate::SoundApp;
use crate::widgets::{icon_button, rec_toggle};
use serde_json::json;
use soundcraft_model::{Track, TrackKind};

/// The product name shown in menus, titles and the About window.
pub const APP_NAME: &str = "PodCraft";

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

/// Whether a menu catalog line (`Root > … > Leaf`) is left out of the podcast menus.
pub fn hidden_menu(path: &str) -> bool {
    let root = path.split(" > ").next().unwrap_or(path);
    HIDDEN_ROOTS.contains(&root) || HIDDEN_PARTS.iter().any(|p| path.contains(p))
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
