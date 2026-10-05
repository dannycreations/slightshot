use tiny_skia::Pixmap;

use super::TextEngine;
use crate::geom::Point;

#[test]
fn empty_string_has_zero_width() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  assert_eq!(engine.width("", 20.0), 0.0);
}

#[test]
fn draw_blends_a_glyph_without_panicking() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let mut pm = Pixmap::new(60, 30).unwrap();
  pm.data_mut().iter_mut().for_each(|p| *p = 0);
  engine.draw(&mut pm, "Hi", 4.0, 22.0, 20.0, [255, 255, 255]);
  let lit = pm.data().as_chunks::<4>().0.iter().any(|p| p[3] > 0);
  assert!(lit, "expected at least one lit pixel after drawing text");
}

#[test]
fn width_scales_with_font_size() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let small = engine.width("MM", 10.0);
  let large = engine.width("MM", 40.0);
  assert!(large > small, "larger text should be wider");
}

#[test]
fn inked_holds_every_pixel_draw_would_touch() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  assert!(
    engine.inked("", Point::new(10.0, 10.0), 20.0).is_empty(),
    "nothing to draw means nothing to damage"
  );

  let at = Point::new(40.0, 60.0);
  let area = engine.inked("Hg", at, 20.0);
  let mut pm = Pixmap::new(160, 160).unwrap();
  engine.draw(&mut pm, "Hg", at.x, at.y, 20.0, [255, 255, 255]);

  for (index, pixel) in pm.data().as_chunks::<4>().0.iter().enumerate() {
    if pixel[3] == 0 {
      continue;
    }
    let x = (index % 160) as f32;
    let y = (index / 160) as f32;
    assert!(
      area.contains(Point::new(x, y)),
      "a lit pixel at ({x}, {y}) sits outside the reported area {area:?}"
    );
  }
  assert!(
    !area.is_empty() && area.w > 20.0,
    "a two letter run has to report a box, got {area:?}"
  );
}

#[test]
fn the_box_a_run_fills_is_anchored_at_its_own_corner() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let at = Point::new(40.0, 60.0);
  let box_ = engine.bounds("Hg", at, 20.0);
  // A resize measures from a corner of this box and moves the anchor to it,
  // so a corner that is not the anchor would shift the run on every drag.
  assert_eq!(
    (box_.x, box_.y),
    (at.x, at.y),
    "the box's top left corner is the anchor"
  );
  assert_eq!(box_.h, 20.0, "the box is the line the run sits on");
  assert!(
    box_.w > engine.bounds("H", at, 20.0).w,
    "a longer run fills a wider box"
  );
  assert!(
    engine.bounds("", at, 20.0).is_empty(),
    "a run with nothing in it fills no box, or it would sit at the origin \
     waiting for a click"
  );
}

#[test]
fn repeated_glyphs_reuse_the_atlas_without_regrowing_it() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let _ = engine.width("M", 20.0);
  let after_first = engine.atlas.borrow().len();
  let _ = engine.width("M", 20.0);
  assert_eq!(engine.atlas.borrow().len(), after_first);
}
