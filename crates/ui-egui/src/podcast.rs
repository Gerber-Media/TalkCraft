//! The podcast studio's slimmer surface: music, notation and picture features stay in the
//! engine (and the CLI, control channel and MCP) but leave the menus and dialogs.

use soundcraft_model::TrackKind;

/// The product name shown in menus, titles and the About window.
pub const APP_NAME: &str = "Podcast Studio";

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
