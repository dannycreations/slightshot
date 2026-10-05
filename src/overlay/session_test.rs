use super::{opening, Backdrop};
use crate::{annotate::Tool, geom::Rect};

#[test]
fn a_live_overlay_opens_covering_the_screen_with_the_pen_out() {
  let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
  // The whole screen is the area and the pen is already in hand, so the very
  // first click after the hotkey draws instead of dragging out a region or
  // hunting for the pen in the toolbar.
  assert_eq!(opening(bounds, Backdrop::Live), (Some(bounds), Tool::Pen));
  assert_eq!(opening(bounds, Backdrop::Frozen), (None, Tool::Select));
}
