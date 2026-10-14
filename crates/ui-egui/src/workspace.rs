//! Work modes: "I want to … record a podcast / edit a podcast". Recording shows what a session
//! needs (Record, inputs, remote guests, jingles, markers); editing shows the cutting tools,
//! subtitles and export; All shows both. Only menus, shortcuts, toolbar and track headers
//! follow the mode: the session and every command stay the same, so switching never loses
//! anything. TalkCraft asks at start (Setup can turn that off: then it starts with All).

use crate::SoundApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use egui::{RichText, vec2};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Workspace {
    #[default]
    All,
    Record,
    Edit,
}

impl Workspace {
    pub const ALL: [Workspace; 3] = [Workspace::Record, Workspace::Edit, Workspace::All];

    pub fn id(self) -> &'static str {
        match self {
            Workspace::All => "all",
            Workspace::Record => "record",
            Workspace::Edit => "edit",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.id() == s)
    }

    /// The switch's label ("Edit" and "All" already mean the menu and "Alle" in the table).
    pub fn label(self) -> &'static str {
        let de = crate::i18n::german();
        match self {
            Workspace::All if de => "Alles",
            Workspace::All => "All",
            Workspace::Record if de => "Aufnehmen",
            Workspace::Record => "Record",
            Workspace::Edit if de => "Schneiden",
            Workspace::Edit => "Edit",
        }
    }

    pub fn records(self) -> bool {
        self != Workspace::Edit
    }

    pub fn edits(self) -> bool {
        self != Workspace::Record
    }
}

/// The mode the menus are built for (set every frame, like the interface language).
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(w: Workspace) {
    CURRENT.store(w as u8, Ordering::Relaxed);
}

pub fn current() -> Workspace {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Workspace::Record,
        2 => Workspace::Edit,
        _ => Workspace::All,
    }
}

/// Menu items only an editing session needs (hidden while recording).
const EDIT_ONLY_PREFIXES: &[&str] =
    &["Clip >", "AudioSuite >", "Event >", "File > Export", "File > Bounce", "Window > Subtitles", "Window > Edit by Text"];
/// Edit menu items that stay while recording.
const EDIT_KEPT_WHILE_RECORDING: &[&str] = &["Edit > Can't Undo", "Edit > Can't Redo", "Edit > Select All"];
/// Menu items only a recording session needs (hidden while editing).
const RECORD_ONLY: &[&str] = &[
    "Window > Remote Guests",
    "Setup > Hardware...",
    "Setup > Playback Engine...",
    "Setup > I/O...",
    "Options > Destructive Record",
    "Options > Loop Record",
    "Options > QuickPunch",
    "Options > TrackPunch",
    "Options > DestructivePunch",
    "Options > Pre/Post-Roll",
    "Options > Low Latency Monitoring",
    "Track > Set Record Tracks to Input Only",
];

/// Whether the current mode leaves the menu item `path` out.
pub fn hidden_menu(path: &str) -> bool {
    hidden_menu_in(current(), path)
}

pub fn hidden_menu_in(w: Workspace, path: &str) -> bool {
    match w {
        Workspace::All => false,
        Workspace::Record => {
            EDIT_ONLY_PREFIXES.iter().any(|p| path.starts_with(p)) || (path.starts_with("Edit >") && !EDIT_KEPT_WHILE_RECORDING.contains(&path))
        }
        Workspace::Edit => RECORD_ONLY.contains(&path),
    }
}

/// Whether the current mode ignores the command's keyboard shortcut.
pub fn hidden_command(id: &str) -> bool {
    hidden_command_in(current(), id)
}

pub fn hidden_command_in(w: Workspace, id: &str) -> bool {
    match w {
        Workspace::All => false,
        Workspace::Record => {
            (id.starts_with("edit.") && !matches!(id, "edit.undo" | "edit.redo" | "edit.select_all"))
                || id.starts_with("clip.")
                || id.starts_with("audiosuite.")
                || id == "file.export_podcast"
        }
        Workspace::Edit => matches!(id, "transport.record" | "transport.record_track"),
    }
}

/// `ui.workspace {mode: record|edit|all}`: switches the mode (no value: report it).
pub fn run(app: &mut SoundApp, p: &Value) -> Result<Value, String> {
    if let Some(m) = p.get("mode").and_then(Value::as_str) {
        let w = Workspace::from_id(m).ok_or_else(|| format!("unknown mode `{m}` (record, edit or all)"))?;
        switch(app, w);
    }
    Ok(json!({"mode": app.ui.workspace.id()}))
}

fn switch(app: &mut SoundApp, w: Workspace) {
    app.ui.workspace = w;
    set(w);
    app.extra.ask_workspace = false;
    app.extra.offer_edit = false;
    // Recording: nothing on the timeline reacts like an editing tool.
    if w == Workspace::Record {
        let _ = app.run("edit.tool", json!({"tool": "selector"}));
    }
}

/// Mode switch at the start of the toolbar.
pub fn toolbar_switch(app: &mut SoundApp, ui: &mut egui::Ui) {
    let t = Tokens::current();
    let now = app.ui.workspace;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.spacing_mut().button_padding = vec2(6.0, 1.0);
        for w in Workspace::ALL {
            let on = now == w;
            // On the accent: the theme's text colour for selections (white).
            let text = RichText::new(w.label()).size(11.0).color(if on { ui.visuals().selection.stroke.color } else { t.text_dim });
            if ui.add(egui::Button::selectable(on, text).min_size(vec2(84.0, 18.0))).clicked() && !on {
                switch(app, w);
            }
        }
    });
}

/// The start question, and after a recording the offer to switch to editing.
pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    let t = Tokens::current();
    if app.extra.ask_workspace {
        let mut chosen = None;
        egui::Modal::new(egui::Id::new("workspace_ask")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(tr("I want to …"));
            ui.add_space(8.0);
            for (w, text, hint) in [
                (Workspace::Record, "record a podcast", "Record, microphones, remote guests, jingles and markers"),
                (Workspace::Edit, "edit a podcast", "Cutting, pauses, filler words, subtitles, levels and export"),
                (Workspace::All, "see everything", "Recording and editing tools together"),
            ] {
                let b = egui::Button::new(RichText::new(tr(text)).size(16.0)).min_size(vec2(360.0, 34.0));
                if ui.add(b).on_hover_text(tr(hint)).clicked() {
                    chosen = Some(w);
                }
                ui.label(RichText::new(tr(hint)).small().color(t.text_dim));
                ui.add_space(4.0);
            }
            ui.separator();
            let mut dont_ask = !app.ui.ask_workspace;
            ui.checkbox(&mut dont_ask, tr("Don't ask again (start with everything)"));
            app.ui.ask_workspace = !dont_ask;
            ui.label(RichText::new(tr("You can switch at any time at the left of the toolbar.")).small().color(t.text_dim));
        });
        if let Some(w) = chosen {
            switch(app, w);
        }
    }
    if app.extra.offer_edit && app.ui.workspace == Workspace::Record && !app.engine.transport.recording {
        let mut close = false;
        egui::Window::new(tr("Recording finished")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_TOP, vec2(0.0, 120.0)).show(
            ctx,
            |ui| {
                ui.label(tr("Switch to editing now?"));
                ui.horizontal(|ui| {
                    if ui.button(Workspace::Edit.label()).clicked() {
                        switch(app, Workspace::Edit);
                        close = true;
                    }
                    if ui.button(tr("Keep recording")).clicked() {
                        close = true;
                    }
                });
            },
        );
        if close {
            app.extra.offer_edit = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_filter_menus_and_shortcuts() {
        let (rec, edit, all) = (Workspace::Record, Workspace::Edit, Workspace::All);
        assert!(hidden_menu_in(rec, "Clip > Normalize..."));
        assert!(hidden_menu_in(rec, "Edit > Cut"));
        assert!(!hidden_menu_in(rec, "Edit > Can't Undo"));
        assert!(hidden_menu_in(rec, "File > Export Episode..."));
        assert!(!hidden_menu_in(rec, "Window > Remote Guests"));
        assert!(hidden_command_in(rec, "edit.delete") && !hidden_command_in(rec, "edit.undo") && !hidden_command_in(rec, "transport.record"));
        assert!(!hidden_menu_in(edit, "Edit > Cut") && hidden_menu_in(edit, "Window > Remote Guests"));
        assert!(hidden_command_in(edit, "transport.record") && !hidden_command_in(edit, "edit.delete"));
        assert!(!hidden_menu_in(all, "Clip > Normalize...") && !hidden_command_in(all, "transport.record"));
        assert_eq!(Workspace::from_id("edit"), Some(Workspace::Edit));
    }
}
