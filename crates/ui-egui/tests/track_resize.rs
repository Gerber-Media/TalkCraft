//! TalkCraft: a track's height is changed by dragging the bottom edge of its header, freely and
//! live, as one undo step; the Zoom Toggle without a range does nothing but say why.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use serde_json::json;
use soundcraft_model::TrackHeight;
use soundcraft_ui_egui::{Services, SoundApp};

fn frame(ctx: &egui::Context, app: &mut SoundApp, events: Vec<Event>) {
    let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))), events, ..Default::default() };
    ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    })
    .textures_delta
    .clear();
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
fn dragging_a_track_headers_bottom_edge_resizes_it_live_in_one_undo_step() {
    let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
    app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 2})).unwrap();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]);
    }
    let (id, row) = app.edit_layout.rows[0];
    let before = app.engine.session().track(soundcraft_model::TrackId(id)).unwrap().height.points();
    let [x0, ..] = app.edit_layout.timeline;
    let edge = pos2(x0 - 120.0, row[3] - 1.5);
    let undo = app.engine.undo_history().len();
    frame(&ctx, &mut app, vec![Event::PointerMoved(edge)]);
    frame(&ctx, &mut app, vec![button(edge, true)]);
    for k in 1..=5 {
        frame(&ctx, &mut app, vec![Event::PointerMoved(edge + vec2(0.0, 10.0 * k as f32))]);
    }
    let live = app.engine.session().track(soundcraft_model::TrackId(id)).unwrap().height.points();
    assert!((live - (before + 50.0)).abs() < 2.0, "follows the pointer while dragging, not in steps: {before} → {live}");
    frame(&ctx, &mut app, vec![button(edge + vec2(0.0, 50.0), false)]);
    frame(&ctx, &mut app, vec![]);
    assert_eq!(app.engine.undo_history().len(), undo + 1, "one undo step for the whole drag");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.engine.session().track(soundcraft_model::TrackId(id)).unwrap().height.points(), before);
    assert!(TrackHeight::from_points(before + 50.0).points() > before);
}

#[test]
fn the_zoom_toggle_without_a_range_explains_instead_of_zooming() {
    let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
    app.engine.execute("session.new", &json!({"name": "S", "template": "podcast", "speakers": 1})).unwrap();
    let zoom = app.engine.session().edit.zoom.samples_per_px;
    soundcraft_ui_egui::edit_window::zoom_toggle(&mut app);
    assert_eq!(app.engine.session().edit.zoom.samples_per_px, zoom, "no zoom into a few samples");
    assert!(app.engine.session().edit.zoom_toggle.is_none());
    assert!(app.ui.status.contains("select a range"), "{}", app.ui.status);
}
