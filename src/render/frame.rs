use tiny_skia::Pixmap;

use crate::{
  draw,
  geom::{clamp_span, handle_anchor, Handle, Rect, HANDLES},
  text::TextEngine,
};

pub(super) const EDGE: f32 = 3.0;
const BADGE_TEXT: f32 = 18.0;
const BADGE_GAP: f32 = 5.0;
const BADGE_PAD: f32 = 6.0;
const BADGE_H: f32 = BADGE_TEXT + 7.0;

pub(super) fn outline_boxes(sel: Rect) -> [Rect; 4] {
  let edge = sel.inflated(EDGE);
  [
    Rect::new(edge.x, edge.y, edge.w, EDGE),
    Rect::new(edge.x, edge.bottom() - EDGE, edge.w, EDGE),
    Rect::new(edge.x, edge.y, EDGE, edge.h),
    Rect::new(edge.right() - EDGE, edge.y, EDGE, edge.h),
  ]
}

pub(super) fn handle_boxes(sel: Rect) -> [Rect; 8] {
  HANDLES.map(|handle| handle_square(sel, handle).inflated(EDGE))
}

pub(super) fn handle_square(sel: Rect, handle: Handle) -> Rect {
  let anchor = handle_anchor(sel, handle);
  Rect::new(anchor.x - 3.0, anchor.y - 3.0, 6.0, 6.0)
}

pub(super) fn draw_handles(pm: &mut Pixmap, sel: Rect) {
  for &handle in &HANDLES {
    let square = handle_square(sel, handle);
    draw::rect_fill(pm, square, [255, 255, 255], 255);
    draw::rect_stroke(pm, square, [20, 20, 20], 1.0, 255);
  }
}

pub(super) fn badge_label(sel: Rect) -> String {
  format!("{}x{}", sel.w.round() as i64, sel.h.round() as i64)
}

pub(super) fn badge_rect(sel: Rect, bounds: Rect, engine: &TextEngine) -> Rect {
  let box_w = engine.width(&badge_label(sel), BADGE_TEXT) + BADGE_PAD * 2.0;
  let mut bx = sel.x;
  let mut by = sel.y - BADGE_H - BADGE_GAP;
  if by < bounds.y {
    by = sel.y + BADGE_GAP;
  }
  bx = clamp_span(bx, box_w, bounds.x, bounds.right());
  Rect::new(bx, by, box_w, BADGE_H)
}

pub(super) fn draw_badge(
  pm: &mut Pixmap,
  plate: Rect,
  label: &str,
  engine: &TextEngine,
) {
  draw::rounded_fill(pm, plate, 4.0, [10, 10, 10], 210);
  engine.draw(
    pm,
    label,
    plate.x + BADGE_PAD,
    plate.y + 3.5,
    BADGE_TEXT,
    [255, 255, 255],
  );
}
