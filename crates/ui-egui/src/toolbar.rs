//! The Edit window toolbar.

use crate::theme::{Tokens, bold, mono, regular};
use crate::widgets::icon_button;
use crate::{SoundApp, icons};
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;
use soundcraft_model::{EditMode, Tool};
use soundcraft_time::{TimeFormat, format_length, format_position};

pub const HEIGHT: f32 = 78.0;

fn group(ui: &mut Ui, w: f32, h: f32, add: impl FnOnce(&mut Ui, Rect)) {
    let t = Tokens::current();
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    ui.painter().rect(r, CornerRadius::same(4), t.toolbar_group, Stroke::new(1.0, t.group_border), StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(5.0)).layout(egui::Layout::left_to_right(egui::Align::Min)));
    add(&mut child, r);
}

pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, t.toolbar_bg);
    ui.add_space(4.0);
    // On a narrow window the toolbar scrolls sideways (trackpad or Shift+wheel) instead of cutting off.
    egui::ScrollArea::horizontal()
        .id_salt("toolbar_scroll")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| toolbar_row(app, ui));
}

fn toolbar_row(app: &mut SoundApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        group(ui, 96.0, 64.0, |ui, _| crate::workspace::toolbar_switch(app, ui));
        let editing = app.ui.workspace.edits();
        if editing {
            edit_modes(app, ui);
        }
        zoom_group(app, ui);
        if editing {
            tools(app, ui);
        }
        counters(app, ui);
        if editing {
            grid_nudge(app, ui);
        }
        transport(app, ui);
        marker_group(app, ui);
        loudness(app, ui);
        output_meter(app, ui);
    });
}

fn edit_modes(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let cur = app.engine.session().edit.edit_mode;
    group(ui, 112.0, 52.0, |ui, r| {
        let inner = r.shrink(6.0);
        let cw = inner.width() / 2.0;
        let ch = inner.height() / 2.0;
        // TalkCraft leaves out Spot (timecode spotting); Grid spans the bottom row.
        let modes = [(EditMode::Shuffle, 0, 0, 1, "SHUFFLE", "F1"), (EditMode::Slip, 1, 0, 1, "SLIP", "F2"), (EditMode::Grid, 0, 1, 2, "GRID", "F4")];
        for (m, cx, cy, span, label, key) in modes {
            let cell = Rect::from_min_size(pos2(inner.min.x + cw * cx as f32, inner.min.y + ch * cy as f32), vec2(cw * span as f32, ch)).shrink(1.0);
            let resp = ui.interact(cell, ui.id().with(("mode", label)), Sense::click());
            let on = cur == m || (m == EditMode::Grid && cur == EditMode::GridRelative);
            ui.painter().rect_filled(cell, 1.0, if on { t.mode_on } else { t.mode_off });
            let txt = if m == EditMode::Grid && cur == EditMode::GridRelative { "REL GRID" } else { label };
            ui.painter().text(cell.center(), Align2::CENTER_CENTER, txt, bold(10.5), if on { t.mode_on_text } else { t.mode_off_text });
            if resp.clicked() {
                let id = match m {
                    EditMode::Grid if cur == EditMode::Grid => "relative_grid",
                    EditMode::Shuffle => "shuffle",
                    EditMode::Spot => "spot",
                    EditMode::Slip => "slip",
                    _ => "grid",
                };
                let _ = app.run("edit.mode", json!({"mode": id}));
            }
            resp.on_hover_text(crate::i18n::trf("{} mode ({})", &[&label.to_lowercase(), &key]));
        }
    });
}

fn zoom_group(app: &mut SoundApp, ui: &mut Ui) {
    group(ui, 128.0, 52.0, |ui, _| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 3.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icon_button(ui, vec2(28.0, 22.0), "minus", false, "Zoom out (Cmd+[)").clicked() {
                    let _ = app.run("view.zoom_out", json!({}));
                }
                if icon_button(ui, vec2(30.0, 22.0), "wave", false, "Waveform zoom in (Cmd+Alt+])").clicked() {
                    let _ = app.run("view.waveform_zoom", json!({"factor": 2.0}));
                }
                if icon_button(ui, vec2(28.0, 22.0), "plus", false, "Zoom in (Cmd+])").clicked() {
                    let _ = app.run("view.zoom_in", json!({}));
                }
                if icon_button(ui, vec2(28.0, 22.0), "zoom", false, "Fill window with session (Alt+A)").clicked() {
                    let w = ui.ctx().content_rect().width() - 420.0;
                    let _ = app.run("view.zoom_fit", json!({"width_px": w}));
                }
            });
        });
    });
}

fn tools(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let cur = app.engine.session().edit.tool;
    group(ui, 268.0 + 38.0 * 2.0, 52.0, |ui, r| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 3.0);
            let row = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (tool, icon) in [
                    (Tool::Zoom, "zoom"),
                    (Tool::Trim, "trim"),
                    (Tool::Selector, "selector"),
                    (Tool::Grabber, "grabber"),
                    (Tool::Scrubber, "scrubber"),
                    (Tool::Pencil, "pencil"),
                    (Tool::Blade, "blade"),
                ] {
                    let smart_member = cur == Tool::Smart && matches!(tool, Tool::Trim | Tool::Selector | Tool::Grabber);
                    let tip = format!("{}{}", tool.label(), tool.fkey().map(|k| format!(" ({k})")).unwrap_or_default());
                    if icon_button(ui, vec2(36.0, 26.0), icon, cur == tool || smart_member, &tip).clicked() {
                        let _ = app.run("edit.tool", json!({"tool": tool.id()}));
                    }
                }
            });
            // Smart tool bracket above Trim/Selector/Grabber.
            let rr = row.response.rect;
            let bx0 = rr.min.x + 38.0;
            let bx1 = bx0 + 38.0 * 3.0 - 2.0;
            let bracket = Rect::from_min_max(pos2(bx0 - 2.0, r.min.y + 1.0), pos2(bx1 + 2.0, r.min.y + 5.0));
            let resp = ui.interact(bracket.expand2(vec2(0.0, 2.0)), ui.id().with("smart"), Sense::click());
            ui.painter().rect_filled(bracket, 1.0, if cur == Tool::Smart { t.accent } else { t.tool_bracket_off });
            if resp.on_hover_text(crate::i18n::tr("Smart Tool (F6+F7 / F7+F8)")).clicked() {
                let _ = app.run("edit.tool", json!({"tool": "smart"}));
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let e = app.engine.session().edit.clone();
                // TalkCraft leaves out the music toggles (Tab to Transient, Mirrored MIDI).
                let toggles: [(&str, &str, bool, &str); 3] = [
                    ("options.link_timeline_edit", "link", e.link_timeline_edit, "Link Timeline and Edit Selection"),
                    ("options.link_track_edit", "link", e.link_track_edit, "Link Track and Edit Selection"),
                    ("options.automation_follows_edit", "wave", e.automation_follows_edit, "Automation Follows Edit"),
                ];
                for (id, icon, on, tip) in toggles {
                    if icon_button(ui, vec2(36.0, 18.0), icon, on, tip).clicked() {
                        let _ = app.run(id, json!({}));
                    }
                }
            });
        });
    });
}

fn fmt_pos(app: &SoundApp, at: i64, f: TimeFormat) -> String {
    let s = app.engine.session();
    format_position(at, f, s.sample_rate, &s.tempo, s.frame_rate, s.timecode_start)
}

fn counters(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let (main_fmt, sub_fmt, sel) = {
        let s = app.engine.session();
        (s.edit.main_counter, s.edit.sub_counter.unwrap_or(TimeFormat::Samples), s.edit.selection)
    };
    let pos = app.position();
    let (r, _) = ui.allocate_exact_size(vec2(318.0, 64.0), Sense::hover());
    ui.painter().rect(r, CornerRadius::same(4), t.counter_bg, Stroke::new(1.0, t.counter_border), StrokeKind::Inside);
    let main_r = Rect::from_min_max(r.min, pos2(r.min.x + 180.0, r.min.y + 44.0));
    let txt = fmt_pos(app, pos, main_fmt);
    ui.painter().text(pos2(main_r.max.x - 18.0, main_r.center().y), Align2::RIGHT_CENTER, txt, mono(26.0), t.counter_text);
    let resp = ui.interact(main_r, ui.id().with("main_counter"), Sense::click());
    icons::draw(ui.painter(), Rect::from_center_size(pos2(main_r.max.x - 9.0, main_r.center().y), vec2(10.0, 10.0)), "triangle_down", t.text_dim);
    egui::Popup::menu(&resp).show(|ui| {
        for f in TimeFormat::ALL {
            if ui.selectable_label(main_fmt == f, f.label()).clicked() {
                let _ = app.run("view.main_counter", json!({"format": f.id()}));
            }
        }
    });
    // Start / End / Length.
    let s = app.engine.session();
    let rows = [
        ("Start", fmt_pos(app, sel.start, main_fmt)),
        ("End", fmt_pos(app, sel.end, main_fmt)),
        ("Length", format_length(sel.start, sel.len(), main_fmt, s.sample_rate, &s.tempo, s.frame_rate)),
    ];
    for (i, (label, val)) in rows.iter().enumerate() {
        let y = r.min.y + 10.0 + i as f32 * 13.0;
        ui.painter().text(pos2(r.min.x + 214.0, y), Align2::RIGHT_CENTER, crate::i18n::tr(label), regular(11.0), t.counter_label);
        ui.painter().text(pos2(r.max.x - 8.0, y), Align2::RIGHT_CENTER, val, mono(11.0), t.counter_text);
    }
    // Sub counter row.
    let y = r.max.y - 11.0;
    ui.painter().line_segment([pos2(r.min.x + 6.0, r.max.y - 21.0), pos2(r.max.x - 6.0, r.max.y - 21.0)], Stroke::new(1.0, t.counter_rule));
    ui.painter().text(pos2(r.min.x + 10.0, y), Align2::LEFT_CENTER, crate::i18n::tr("Sub"), regular(11.0), t.counter_label);
    ui.painter().text(pos2(r.min.x + 150.0, y), Align2::RIGHT_CENTER, fmt_pos(app, pos, sub_fmt), mono(11.0), t.counter_text);
    let st = crate::i18n::tr(if app.engine.is_dirty() { "modified" } else { "saved" });
    let dev = crate::i18n::tr(app.player.as_ref().map_or("no audio engine", |p| if p.silent { "silent clock" } else { "audio on" }));
    ui.painter().text(pos2(r.max.x - 8.0, y), Align2::RIGHT_CENTER, format!("{st} · {dev}"), regular(10.0), t.text_dim);
}

/// A grid or nudge value in the interface language ("1/16 note" → "1/16 Note",
/// "1.000 sec" → "1,000 Sek.").
fn value_label(v: &str) -> String {
    if !crate::i18n::german() {
        return v.to_string();
    }
    if let Some(n) = v.strip_suffix(" sec") {
        return format!("{} Sek.", n.replace('.', ","));
    }
    v.replace(" notes", " Noten").replace(" note", " Note").replace(" bar", " Takt").replace(" samples", " Samples").replace(" frame", " Frame")
}

fn grid_nudge(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let (grid, nudge, lines) = {
        let e = &app.engine.session().edit;
        (e.grid, e.nudge, e.grid_lines)
    };
    let (r, _) = ui.allocate_exact_size(vec2(170.0, 64.0), Sense::hover());
    ui.painter().rect(r, CornerRadius::same(4), t.counter_bg, Stroke::new(1.0, t.counter_border), StrokeKind::Inside);
    let gl = Rect::from_min_size(pos2(r.min.x + 6.0, r.min.y + 8.0), vec2(48.0, 16.0));
    ui.painter().rect_filled(gl, 1.0, if lines { t.mode_on } else { t.counter_off });
    ui.painter().text(gl.center(), Align2::CENTER_CENTER, crate::i18n::tr("Grid"), bold(11.0), if lines { t.mode_on_text } else { t.mode_off_text });
    if ui.interact(gl, ui.id().with("grid_lines"), Sense::click()).on_hover_text(crate::i18n::tr("Show grid lines")).clicked() {
        let _ = app.run("view.grid_lines", json!({}));
    }
    let gr = Rect::from_min_max(pos2(r.min.x + 60.0, r.min.y + 6.0), pos2(r.max.x - 4.0, r.min.y + 26.0));
    ui.painter().text(pos2(gr.max.x - 14.0, gr.center().y), Align2::RIGHT_CENTER, value_label(&grid.label()), mono(11.0), t.counter_text);
    let gresp = ui.interact(gr, ui.id().with("grid_value"), Sense::click());
    value_menu(app, &gresp, "view.grid");
    ui.painter().text(pos2(r.min.x + 10.0, r.min.y + 44.0), Align2::LEFT_CENTER, crate::i18n::tr("Nudge"), regular(11.0), t.counter_label);
    // What Nudge is, on the label.
    let label_r = Rect::from_min_size(pos2(r.min.x + 4.0, r.min.y + 34.0), vec2(54.0, 20.0));
    let _ = ui
        .interact(label_r, ui.id().with("nudge_label"), Sense::hover())
        .on_hover_text(crate::i18n::tr("Step for the + and − keys: moves the selected clips (or the selection) by this much"));
    let nr = Rect::from_min_max(pos2(r.min.x + 60.0, r.min.y + 34.0), pos2(r.max.x - 4.0, r.min.y + 54.0));
    ui.painter().text(pos2(nr.max.x - 14.0, nr.center().y), Align2::RIGHT_CENTER, value_label(&nudge.label()), mono(11.0), t.counter_text);
    let nresp = ui.interact(nr, ui.id().with("nudge_value"), Sense::click());
    value_menu(app, &nresp, "view.nudge");
    for rr in [gr, nr] {
        icons::draw(ui.painter(), Rect::from_center_size(pos2(rr.max.x - 6.0, rr.center().y), vec2(10.0, 10.0)), "triangle_down", t.text_dim);
    }
}

fn value_menu(app: &mut SoundApp, resp: &egui::Response, cmd: &str) {
    egui::Popup::menu(resp).show(|ui| {
        for v in ["1 bar", "1/2", "1/4", "1/8", "1/16", "1/32", "1/64", "1/8t", "1/16t", "1/4."] {
            if ui.button(value_label(v)).clicked() {
                let _ = app.run(cmd, json!({"value": v}));
            }
        }
        ui.separator();
        for (label, v) in [
            ("1 msec", json!({"seconds": 0.001})),
            ("10 msec", json!({"seconds": 0.01})),
            ("100 msec", json!({"seconds": 0.1})),
            ("1 second", json!({"seconds": 1.0})),
            ("1 frame", json!({"frames": 1})),
            ("100 samples", json!({"samples": 100})),
        ] {
            if ui.button(crate::i18n::tr(label)).clicked() {
                let _ = app.run(cmd, json!({"value": v}));
            }
        }
    });
}

fn transport(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let playing = app.is_playing();
    let recording = app.engine.transport.recording;
    let looped = app.engine.session().edit.loop_playback;
    group(ui, 222.0, 64.0, |ui, _| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                if icon_button(ui, vec2(38.0, 28.0), "loop", looped, "Loop Playback (Cmd+Shift+L)").clicked() {
                    let _ = app.run("options.loop_playback", json!({}));
                }
                let (r, resp) = ui.allocate_exact_size(vec2(38.0, 28.0), Sense::click());
                ui.painter().rect(
                    r,
                    CornerRadius::same(3),
                    if resp.hovered() { t.button_hi } else { t.button },
                    Stroke::new(1.0, t.button_border),
                    StrokeKind::Inside,
                );
                icons::draw(ui.painter(), r.shrink(5.0), "stop", if playing { t.text } else { t.accent });
                if resp.on_hover_text(crate::i18n::tr("Stop (Space)")).clicked() {
                    let _ = app.run("transport.stop", json!({}));
                }
                let (r, resp) = ui.allocate_exact_size(vec2(38.0, 28.0), Sense::click());
                ui.painter().rect(
                    r,
                    CornerRadius::same(3),
                    if playing {
                        Color32::from_rgb(40, 90, 50)
                    } else if resp.hovered() {
                        t.button_hi
                    } else {
                        t.button
                    },
                    Stroke::new(1.0, t.button_border),
                    StrokeKind::Inside,
                );
                icons::draw(ui.painter(), r.shrink(5.0), "play", if playing { Color32::from_rgb(120, 255, 140) } else { t.counter_text });
                if resp.on_hover_text(crate::i18n::tr("Play (Space)")).clicked() {
                    let _ = app.run("transport.play", json!({}));
                }
                if !app.ui.workspace.records() {
                    return;
                }
                let (r, resp) = ui.allocate_exact_size(vec2(38.0, 28.0), Sense::click());
                ui.painter().rect(
                    r,
                    CornerRadius::same(3),
                    if recording {
                        Color32::from_rgb(110, 30, 30)
                    } else if resp.hovered() {
                        t.button_hi
                    } else {
                        t.button
                    },
                    Stroke::new(1.0, t.button_border),
                    StrokeKind::Inside,
                );
                icons::draw(ui.painter(), r.shrink(5.0), "record", t.rec);
                if resp.on_hover_text(crate::i18n::tr("Record (Cmd+Space)")).clicked() {
                    let _ = app.run("transport.record", json!({}));
                }
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for (icon, cmd, tip) in [
                    ("rtz", "transport.rtz", "Return to Zero (Home)"),
                    ("rewind", "transport.rewind", "Rewind"),
                    ("ffwd", "transport.fast_forward", "Fast Forward"),
                    ("end", "transport.go_to_end", "Go to End (End)"),
                ] {
                    if icon_button(ui, vec2(38.0, 22.0), icon, false, tip).clicked() {
                        let _ = app.run(cmd, json!({}));
                    }
                }
                rate_button(app, ui);
            });
        });
    });
}

/// Recording: set a chapter (Enter) or mark a mistake (Shift+Enter) without looking.
/// Editing: step from mistake to mistake and tick each one off once it is cut.
fn marker_group(app: &mut SoundApp, ui: &mut Ui) {
    let tr = crate::i18n::tr;
    let (rec, edit) = (app.ui.workspace.records(), app.ui.workspace.edits());
    let cuts = app.engine.session().markers.iter().filter(|m| soundcraft_engine::podcast::is_cut_marker(m)).count();
    let mistake_fill = egui::Color32::from_rgb(190, 55, 55);
    let mut run: Option<&str> = None;
    // Recording only: two big buttons. Editing (and All): one narrow block, record row on top.
    let width = if rec && !edit { 120.0 } else { 112.0 };
    group(ui, width, 64.0, |ui, _| {
        ui.spacing_mut().button_padding = vec2(4.0, 1.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 4.0);
            if rec && !edit {
                if ui
                    .add(egui::Button::new(tr("+ Chapter")).min_size(vec2(104.0, 24.0)))
                    .on_hover_text(tr("Chapter marker at the playhead (Enter)"))
                    .clicked()
                {
                    run = Some("markers.add");
                }
                let b = egui::Button::new(egui::RichText::new(tr("✂ Mistake")).color(egui::Color32::WHITE)).fill(mistake_fill);
                if ui.add(b.min_size(vec2(104.0, 24.0))).on_hover_text(tr("Mark a mistake to cut later (Shift+Enter)")).clicked() {
                    run = Some("markers.add_cut");
                }
                return;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                if rec {
                    if ui.add(egui::Button::new("+").min_size(vec2(24.0, 22.0))).on_hover_text(tr("Chapter marker at the playhead (Enter)")).clicked()
                    {
                        run = Some("markers.add");
                    }
                    let b = egui::Button::new(egui::RichText::new("✂").color(egui::Color32::WHITE)).fill(mistake_fill).min_size(vec2(24.0, 22.0));
                    if ui.add(b).on_hover_text(tr("Mark a mistake to cut later (Shift+Enter)")).clicked() {
                        run = Some("markers.add_cut");
                    }
                }
                ui.label(egui::RichText::new(crate::i18n::trf("{} ✂", &[&cuts])).size(11.0))
                    .on_hover_text(crate::i18n::trf("{} mistake(s) to cut", &[&cuts]));
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let size = vec2(32.0, 22.0);
                if ui.add_enabled(cuts > 0, egui::Button::new("◀").min_size(size)).on_hover_text(tr("Previous mistake")).clicked() {
                    run = Some("markers.previous_cut");
                }
                if ui.add_enabled(cuts > 0, egui::Button::new("▶").min_size(size)).on_hover_text(tr("Next mistake")).clicked() {
                    run = Some("markers.next_cut");
                }
                if ui
                    .add_enabled(cuts > 0, egui::Button::new("✓").min_size(size))
                    .on_hover_text(tr("Fixed: remove the mistake marker here"))
                    .clicked()
                {
                    run = Some("markers.done_cut");
                }
            });
        });
    });
    if let Some(cmd) = run
        && let Err(e) = app.run(cmd, json!({}))
    {
        app.ui.status = e;
    }
}

/// Listening rates offered; the pitch stays the same.
pub const PLAYBACK_RATES: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

fn rate_label(r: f32) -> String {
    let s = format!("{r}×");
    if crate::i18n::tr("Playback Speed") == "Playback Speed" { s } else { s.replace('.', ",") }
}

/// Faster or slower playback without changing the voices' pitch.
fn rate_button(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let rate = app.ui.playback_rate;
    let label = egui::RichText::new(rate_label(rate)).size(12.0).color(if (rate - 1.0).abs() > 0.001 { t.accent } else { t.counter_text });
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(4.0, 1.0);
        let r = ui.menu_button(label, |ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Playback Speed")).strong());
            for r in PLAYBACK_RATES {
                if ui.selectable_label((rate - r).abs() < 0.001, rate_label(r)).clicked() {
                    let _ = app.run("transport.rate", json!({"rate": r}));
                    ui.close();
                }
            }
        });
        r.response.on_hover_text(crate::i18n::tr("Playback speed (the pitch stays the same)"));
    });
}

/// The episode's integrated loudness and true peak against the export target.
fn loudness(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let target = app.engine.session().podcast.export.loudness_lufs;
    let (r, resp) = ui.allocate_exact_size(vec2(132.0, 64.0), Sense::hover());
    ui.painter().rect(r, CornerRadius::same(4), t.counter_bg, Stroke::new(1.0, t.counter_border), StrokeKind::Inside);
    ui.painter().text(pos2(r.min.x + 8.0, r.min.y + 12.0), Align2::LEFT_CENTER, crate::i18n::tr("EPISODE"), bold(9.5), t.counter_label);
    let stale = app.loudness.stale();
    let (main, sub, col) = match app.loudness.value {
        Some((lufs, tp)) => {
            let ok = target.is_none_or(|g| (lufs - g).abs() <= 1.0) && tp <= -1.0;
            let col = if stale {
                t.text_dim
            } else if ok {
                Color32::from_rgb(120, 230, 140)
            } else {
                Color32::from_rgb(240, 180, 80)
            };
            (format!("{lufs:.1} LUFS"), format!("TP {tp:.1} dBTP"), col)
        }
        None => ("– LUFS".to_string(), crate::i18n::tr(if stale { "measuring…" } else { "no audio" }).to_string(), t.text_dim),
    };
    ui.painter().text(pos2(r.min.x + 8.0, r.min.y + 33.0), Align2::LEFT_CENTER, main, mono(15.0), col);
    ui.painter().text(pos2(r.min.x + 8.0, r.min.y + 52.0), Align2::LEFT_CENTER, sub, mono(10.5), t.counter_text);
    let goal = target.map_or_else(|| "no loudness target set".to_string(), |g| format!("export target {g:.0} LUFS"));
    resp.on_hover_text(crate::i18n::trf("Loudness of the whole mix (ITU BS.1770), measured a moment after each change. Green: within 1 LU of the {} and true peak at or below -1 dBTP. The episode export normalises to the target anyway; this shows how far the mix is from it.", &[&goal]));
}

fn output_meter(app: &mut SoundApp, ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(26.0, 64.0), Sense::hover());
    let m = app.main_meter;
    let w = (r.width() - 6.0) / 2.0;
    for c in 0..2 {
        let rr = Rect::from_min_size(pos2(r.min.x + 2.0 + c as f32 * (w + 2.0), r.min.y), vec2(w, r.height()));
        crate::widgets::meter(ui, rr, m.level[c], m.hold[c], m.clip);
    }
}
