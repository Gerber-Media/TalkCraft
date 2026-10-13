//! TalkCraft: with the Grabber, a click puts the insertion point where it lands, on a clip or
//! beside it (dragging still moves clips); Shift-click selects the clip.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use serde_json::json;
use soundcraft_ui_egui::{Services, SoundApp, edit_window};

fn frame(ctx: &egui::Context, app: &mut SoundApp, events: Vec<Event>) {
    let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))), events, ..Default::default() };
    ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    })
    .textures_delta
    .clear();
}

fn click(ctx: &egui::Context, app: &mut SoundApp, at: Pos2, modifiers: Modifiers) {
    frame(ctx, app, vec![Event::PointerMoved(at)]);
    frame(ctx, app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers }]);
    frame(ctx, app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]);
    frame(ctx, app, vec![]);
}

#[test]
fn a_grabber_click_places_the_insertion_point() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.engine.execute("edit.tool", &json!({"tool": "grabber"})).unwrap();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, vec![]);
    frame(&ctx, &mut app, vec![]);
    let s = app.engine.session().clone();
    let kick = s.track_by_name("Kick").unwrap();
    let clip = kick.clips()[0].clone();
    let [x0, y0, x1, y1] = app.edit_layout.timeline;
    let tl = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    let row = app.edit_layout.rows.iter().find(|(id, _)| *id == kick.id.0).unwrap().1;
    let y = row[1] + kick.height.points() * 0.7;
    let x = (edit_window::x_of(&s, tl, clip.start) + edit_window::x_of(&s, tl, clip.end())) / 2.0;
    let x = x.min(tl.max.x - 20.0);
    let expect = edit_window::sample_at(&s, tl, x);

    click(&ctx, &mut app, pos2(x, y), Modifiers::NONE);
    let ed = &app.engine.session().edit;
    assert!(ed.selection.is_empty(), "a point, not the clip's range: {:?}", ed.selection);
    assert!((ed.selection.start - expect).abs() < s.sample_rate.samples(0.05), "{} vs {expect}", ed.selection.start);
    assert!(ed.selected_clips.is_empty());

    click(&ctx, &mut app, pos2(x, y), Modifiers::SHIFT);
    assert_eq!(app.engine.session().edit.selected_clips, vec![clip.id], "Shift-click selects the clip");
}
