use tiny_skia::Pixmap;

use super::Icon;
use crate::{
  draw::rounded_fill,
  geom::{Point, Rect},
};

#[test]
fn icon_renders_at_large_coordinates() {
  let w = 1920u32;
  let h = 1080u32;
  let mut pm = Pixmap::new(w, h).expect("alloc");
  let bx = 1485.0;
  let by = 285.0;
  rounded_fill(
    &mut pm,
    Rect::new(bx, by, 30.0, 30.0),
    5.0,
    [12, 12, 12],
    175,
  );
  Icon::Upload.paint(
    &mut pm,
    Point::new(bx + 15.0, by + 15.0),
    18.0,
    [240, 240, 240],
  );
  let mut lit = 0usize;
  for y in (by as usize)..(by as usize + 30) {
    for x in (bx as usize)..(bx as usize + 30) {
      if pm.data()[(y * w as usize + x) * 4] > 100 {
        lit += 1;
      }
    }
  }
  assert!(lit > 20, "icon missing at large coords: lit={lit}");
}
