use tiny_skia::Pixmap;

use super::{dashed_rect, polyline, rect_stroke, rounded_fill};
use crate::geom::{Point, Rect};

#[test]
fn polyline_changes_pixels_on_the_canvas() {
  let mut pm = Pixmap::new(20, 20).expect("alloc");
  polyline(
    &mut pm,
    &[Point::new(2.0, 2.0), Point::new(18.0, 18.0)],
    [255, 0, 0],
    2.0,
    255,
  );
  let painted = pm
    .data()
    .as_chunks::<4>()
    .0
    .iter()
    .any(|p| p[0] == 255 && p[3] == 255);
  assert!(painted, "expected at least one red, opaque pixel");
}

#[test]
fn polyline_keeps_the_ink_on_the_line_between_two_samples() {
  fn alpha(pm: &Pixmap, x: u32, y: u32) -> u8 {
    pm.data()[(y * 64 + x) as usize * 4 + 3]
  }
  let mut pm = Pixmap::new(64, 64).expect("alloc");
  // A right angle. Smoothing fitted a curve through the samples that bowed
  // well clear of the first leg, so what pins the raw log is that the ink
  // stays on the segment joining two samples.
  polyline(
    &mut pm,
    &[
      Point::new(8.0, 8.0),
      Point::new(56.0, 8.0),
      Point::new(8.0, 56.0),
    ],
    [255, 0, 0],
    2.0,
    255,
  );
  assert!(
    alpha(&pm, 34, 8) > 200,
    "the first leg runs along y = 8 and should be inked there"
  );
  assert_eq!(
    alpha(&pm, 34, 4),
    0,
    "nothing four pixels off the first leg should be inked"
  );
}

#[test]
fn polyline_ignores_a_lone_sample() {
  let mut pm = Pixmap::new(24, 24).expect("alloc");
  polyline(&mut pm, &[Point::new(12.0, 12.0)], [255, 0, 0], 4.0, 255);
  assert_eq!(pm.data(), &[0u8; 24 * 24 * 4][..]);
}

#[test]
fn rounded_fill_paints_an_opaque_interior() {
  let mut pm = Pixmap::new(20, 20).expect("alloc");
  rounded_fill(
    &mut pm,
    Rect::new(4.0, 4.0, 12.0, 12.0),
    3.0,
    [0, 128, 255],
    255,
  );
  let center = &pm.data()[(10 * 20 + 10) as usize * 4..];
  assert_eq!(center[0], 0);
  assert_eq!(center[1], 128);
  assert_eq!(center[2], 255);
  assert_eq!(center[3], 255);
}

#[test]
fn dashed_rect_and_rect_stroke_share_the_same_path_helper() {
  let mut a = Pixmap::new(20, 20).expect("alloc");
  let mut b = Pixmap::new(20, 20).expect("alloc");
  let r = Rect::new(2.0, 2.0, 10.0, 10.0);
  rect_stroke(&mut a, r, [255, 255, 255], 1.0, 255);
  dashed_rect(&mut b, r, [255, 255, 255]);
  assert!(a.data().iter().any(|&p| p != 0));
  assert!(b.data().iter().any(|&p| p != 0));
}

#[test]
fn rounded_fill_paints_inside_the_requested_rect_only() {
  let mut pm = Pixmap::new(20, 20).expect("alloc");
  rounded_fill(
    &mut pm,
    Rect::new(5.0, 5.0, 10.0, 10.0),
    3.0,
    [1, 2, 3],
    255,
  );
  let alpha = |x: u32, y: u32| pm.data()[(y * 20 + x) as usize * 4 + 3];
  assert!(
    alpha(10, 10) > 0,
    "the centre of the rect should be painted"
  );
  assert_eq!(alpha(1, 1), 0, "nothing should be painted outside the rect");
}
