use winit::window::CursorIcon;

use super::{resize_cursor, resize_text};
use crate::{
  annotate::{MAX_SIZE, MIN_SIZE},
  geom::{handle_anchor, Handle, Handle::*, Point, HANDLES},
  text::TextEngine,
};

#[test]
fn resize_cursor_maps_each_handle_to_its_icon() {
  let cases = [
    (TopLeft, CursorIcon::NwseResize),
    (BottomRight, CursorIcon::NwseResize),
    (BottomLeft, CursorIcon::NeswResize),
    (TopRight, CursorIcon::NeswResize),
    (Top, CursorIcon::NsResize),
    (Bottom, CursorIcon::NsResize),
    (Left, CursorIcon::EwResize),
    (Right, CursorIcon::EwResize),
  ];
  for (handle, expected) in cases {
    assert_eq!(resize_cursor(handle), expected);
  }
}

#[test]
fn a_run_of_text_takes_the_size_the_handle_drag_reports() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let (at, size, text) = (Point::new(40.0, 60.0), 20.0, "Hg");
  let origin = engine.bounds(text, at, size);

  // A handle taken back to where the drag began has to leave the run exactly
  // where it found it, so a press on a handle without a drag does nothing.
  for &handle in &HANDLES {
    assert_eq!(
      resize_text(&engine, text, origin, handle, handle_anchor(origin, handle)),
      (at, size),
      "the {handle:?} handle has to leave the run where it found it"
    );
  }

  // A corner leaves the one opposite it alone, and takes the scale from the
  // way it was pulled further: as wide again is no change, as tall again is
  // double.
  assert_eq!(
    resize_text(
      &engine,
      text,
      origin,
      Handle::BottomRight,
      Point::new(origin.right(), origin.bottom() + origin.h),
    ),
    (at, 40.0)
  );

  // Every report of a drag is measured against the box the drag started on,
  // so a corner pulled further and further keeps growing by the step the
  // pointer took rather than chasing the box it just produced.
  let mut walked = size;
  for step in 1..=4 {
    let target = Point::new(
      origin.right() + origin.w * step as f32,
      origin.bottom() + origin.h * step as f32,
    );
    let (_, next) =
      resize_text(&engine, text, origin, Handle::BottomRight, target);
    assert_eq!(
      next,
      (size * (1.0 + step as f32)).min(MAX_SIZE),
      "step {step} of a corner drag has to be the scale the pointer asked \
       for"
    );
    assert!(next >= walked, "and a drag that keeps going keeps growing");
    walked = next;
  }

  // A side handle reads its own axis, ignores the other one, and grows the
  // run out of the anchor it already had.
  let across = Point::new(origin.right() + origin.w, origin.y - 100.0);
  let resized = resize_text(&engine, text, origin, Handle::Right, across);
  assert_eq!(resized.0, at);
  assert!(resized.1 > size);

  // A handle dragged past its anchor bottoms out rather than turning it
  // inside out.
  assert_eq!(
    resize_text(
      &engine,
      text,
      origin,
      Handle::BottomRight,
      handle_anchor(origin, Handle::TopLeft),
    )
    .1,
    MIN_SIZE
  );
}
