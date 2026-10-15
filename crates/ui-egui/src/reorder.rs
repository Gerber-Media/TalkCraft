//! Sorting by hand: every list that has an order of its own (jingle pads, run sheet topics,
//! tracks) gets a grip. Dragging it shows the new order live, the other items making room, and
//! letting go snaps the item into its place. The History is not sortable: it is a record.

use egui::{Pos2, Rect, Sense, vec2};

#[derive(Debug, Clone, Default)]
pub struct Reorder {
    /// The item being dragged (its index in the list) and where it would land.
    drag: Option<(usize, usize)>,
    /// Where each place was drawn (last frame), to find the place under the pointer.
    places: Vec<Rect>,
}

impl Reorder {
    /// The items in drawing order: the dragged one already at the place it would land.
    pub fn order(&self, n: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        if let Some((from, to)) = self.drag
            && from < n
            && to < n
        {
            let item = order.remove(from);
            order.insert(to, item);
        }
        order
    }

    /// Whether item `i` is the one being dragged.
    pub fn dragging(&self, i: usize) -> bool {
        self.drag.is_some_and(|(from, _)| from == i)
    }

    pub fn active(&self) -> bool {
        self.drag.is_some()
    }

    /// Remembers where place `k` was drawn.
    pub fn place(&mut self, k: usize, r: Rect) {
        if self.places.len() <= k {
            self.places.resize(k + 1, Rect::NOTHING);
        }
        if let Some(p) = self.places.get_mut(k) {
            *p = r;
        }
    }

    /// Where place `k` was drawn last.
    pub fn place_rect(&self, k: usize) -> Option<Rect> {
        self.places.get(k).copied()
    }

    /// Drops places beyond `n` (the list got shorter).
    pub fn places_for(&mut self, n: usize) {
        self.places.truncate(n);
    }

    fn nearest(&self, at: Pos2) -> Option<usize> {
        if let Some(k) = self.places.iter().position(|r| r.contains(at)) {
            return Some(k);
        }
        self.places
            .iter()
            .enumerate()
            .filter(|(_, r)| r.is_positive())
            .min_by(|a, b| a.1.center().distance_sq(at).total_cmp(&b.1.center().distance_sq(at)))
            .map(|(k, _)| k)
    }

    /// The grip of item `i` drawn in `r`. Returns `(from, to)` when the item was dropped in a new
    /// place.
    pub fn grip(&mut self, ui: &mut egui::Ui, r: Rect, i: usize, salt: &str) -> Option<(usize, usize)> {
        let t = crate::theme::Tokens::current();
        let resp = ui.interact(r, ui.id().with(("reorder-grip", salt, i)), Sense::drag());
        let hot = resp.hovered() || resp.dragged();
        crate::icons::draw(ui.painter(), r, "grip", if hot { t.text } else { t.text_dim });
        if hot {
            ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        let resp = resp.on_hover_text(crate::i18n::tr("Drag to sort"));
        if resp.drag_started() {
            self.drag = Some((i, i));
        }
        if resp.dragged()
            && let (Some((from, _)), Some(at)) = (self.drag, ui.ctx().pointer_latest_pos())
            && let Some(to) = self.nearest(at)
        {
            self.drag = Some((from, to));
            ui.ctx().request_repaint();
        }
        if resp.drag_stopped() {
            let done = self.drag.take();
            return done.filter(|(from, to)| from != to);
        }
        None
    }

    /// Outlines the place where the dragged item will land.
    pub fn mark(&self, ui: &egui::Ui, i: usize, r: Rect) {
        if self.dragging(i) {
            let t = crate::theme::Tokens::current();
            ui.painter().rect_stroke(r.expand(2.0), 4.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
        }
    }
}

/// The size of a grip.
pub const GRIP: egui::Vec2 = vec2(14.0, 18.0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dragged_item_is_shown_where_it_would_land() {
        let mut r = Reorder::default();
        assert_eq!(r.order(4), vec![0, 1, 2, 3]);
        r.drag = Some((3, 0));
        assert_eq!(r.order(4), vec![3, 0, 1, 2]);
        r.drag = Some((0, 2));
        assert_eq!(r.order(4), vec![1, 2, 0, 3]);
        for k in 0..3 {
            r.place(k, Rect::from_min_size(egui::pos2(0.0, k as f32 * 20.0), vec2(100.0, 18.0)));
        }
        assert_eq!(r.nearest(egui::pos2(50.0, 45.0)), Some(2));
        assert_eq!(r.nearest(egui::pos2(50.0, 500.0)), Some(2), "below the list: the last place");
    }
}
