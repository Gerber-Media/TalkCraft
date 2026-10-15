//! The user's own working preferences (scrolling, waveform and clip display, pre/post-roll, loop
//! recording, …). They live in the session's edit state, as in the incumbent, but a user expects
//! them to stay: so TalkCraft remembers the last choice in its settings and brings it to every
//! session that is created or opened.

use serde_json::{Map, Value};
use soundcraft_model::EditState;

/// Edit-state fields that are preferences rather than part of the episode.
const FIELDS: &[&str] = &[
    "scrolling",
    "insertion_follows_playback",
    "link_timeline_edit",
    "automation_follows_edit",
    "pre_post_roll",
    "pre_roll",
    "post_roll",
    "loop_playback",
    "loop_record",
    "delay_compensation",
    "rulers",
    "main_counter",
    "sub_counter",
    "grid",
    "nudge",
    "grid_lines",
];

/// View flags that are preferences (by prefix).
const FLAG_PREFIXES: &[&str] = &["waveform.", "view.clip.", "view.marker.", "view.transport.", "edit_view.track_number"];

fn pref_flag(f: &str) -> bool {
    FLAG_PREFIXES.iter().any(|p| f.starts_with(p))
}

/// The preferences in `edit`.
pub fn capture(edit: &EditState) -> Value {
    let all = serde_json::to_value(edit).unwrap_or(Value::Null);
    let mut out = Map::new();
    for k in FIELDS {
        if let Some(v) = all.get(*k) {
            out.insert((*k).to_string(), v.clone());
        }
    }
    let flags: Vec<Value> = edit.flags.iter().filter(|f| pref_flag(f)).map(|f| Value::String(f.clone())).collect();
    out.insert("flags".into(), Value::Array(flags));
    Value::Object(out)
}

/// `edit` with the preferences `p` (unknown or unreadable values are left as they are).
pub fn apply(edit: &EditState, p: &Value) -> EditState {
    let Ok(mut all) = serde_json::to_value(edit) else { return edit.clone() };
    for k in FIELDS {
        if let (Some(v), Some(slot)) = (p.get(*k), all.get_mut(*k)) {
            *slot = v.clone();
        }
    }
    let mut out: EditState = serde_json::from_value(all).unwrap_or_else(|_| edit.clone());
    if let Some(flags) = p.get("flags").and_then(Value::as_array) {
        out.flags.retain(|f| !pref_flag(f));
        out.flags.extend(flags.iter().filter_map(Value::as_str).filter(|f| pref_flag(f)).map(str::to_string));
    }
    out
}

/// Once per frame: a newly opened or created session gets the remembered preferences (without
/// becoming "changed"); otherwise a change the user made is remembered.
pub fn sync(app: &mut crate::SoundApp) {
    let generation = app.engine.generation;
    if app.prefs_generation != Some(generation) {
        app.prefs_generation = Some(generation);
        if let Some(p) = app.ui.edit_prefs.clone() {
            let edit = &app.engine.session().edit;
            let want = apply(edit, &p);
            if &want != edit {
                let clean = !app.engine.is_dirty();
                app.engine.session_mut().edit = want;
                if clean {
                    app.engine.mark_clean();
                }
            }
            return;
        }
    }
    // Only when the document changed: comparing costs a serialization.
    if app.prefs_revision != app.engine.revision {
        app.prefs_revision = app.engine.revision;
        let now = capture(&app.engine.session().edit);
        if app.ui.edit_prefs.as_ref() != Some(&now) {
            app.ui.edit_prefs = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Services, SoundApp};
    use serde_json::json;

    #[test]
    fn preferences_follow_the_user_into_new_sessions_without_making_them_dirty() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        sync(&mut app);
        app.engine.session_mut().edit.scrolling = "center".into();
        app.engine.session_mut().edit.set_flag("waveform.rectified", true);
        app.engine.session_mut().edit.loop_record = true;
        sync(&mut app);
        app.engine.execute("session.new", &json!({"name": "Neu", "template": "podcast", "speakers": 1})).unwrap();
        assert_ne!(app.engine.session().edit.scrolling, "center");
        sync(&mut app);
        let e = &app.engine.session().edit;
        assert_eq!(e.scrolling, "center");
        assert!(e.flag("waveform.rectified") && e.loop_record);
        assert!(!app.engine.is_dirty(), "a fresh session stays unchanged");
        // The remembered preferences survive a restart (they are in UiState).
        let saved: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(saved.edit_prefs, app.ui.edit_prefs);
    }
}
