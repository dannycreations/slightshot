use tiny_skia::Pixmap;

use super::{
  dim_region_into, dimmed_into, flatten, ink, ink_segment, region_pixels,
};
use crate::{
  annotate::{Segment, Shape},
  geom::{Point, Rect},
  text::TextEngine,
};

fn dimmed(frame: &Pixmap) -> Pixmap {
  let mut out =
    Pixmap::new(frame.width(), frame.height()).expect("valid dimensions");
  dimmed_into(&mut out, frame);
  out
}

#[test]
fn dimming_darkens_rgb_and_keeps_alpha() {
  let mut pm = Pixmap::new(2, 1).unwrap();
  pm.data_mut()
    .copy_from_slice(&[200, 100, 50, 255, 10, 20, 30, 128]);
  let dimmed = dimmed(&pm);
  let px = dimmed.data();
  assert_eq!(px[0], ((200 * 150 + 127) / 255) as u8);
  assert_eq!(px[3], 255);
  assert_eq!(px[7], 128);
}

#[test]
fn dimmed_into_overwrites_an_existing_buffer() {
  let mut src = Pixmap::new(2, 1).unwrap();
  src
    .data_mut()
    .copy_from_slice(&[200, 100, 50, 255, 10, 20, 30, 128]);
  let mut dst = Pixmap::new(2, 1).unwrap();
  dst.data_mut().iter_mut().for_each(|b| *b = 77);
  dimmed_into(&mut dst, &src);
  assert_eq!(dst.data(), dimmed(&src).data());
}

#[test]
fn dim_region_into_leaves_the_rest_of_the_backdrop_alone() {
  let mut src = Pixmap::new(2, 1).unwrap();
  src
    .data_mut()
    .copy_from_slice(&[200, 200, 200, 255, 200, 200, 200, 255]);
  let mut dst = Pixmap::new(2, 1).unwrap();
  dst.data_mut().iter_mut().for_each(|b| *b = 77);
  dim_region_into(&mut dst, &src, Rect::new(0.0, 0.0, 1.0, 1.0));
  assert_eq!(dst.data()[0], ((200 * 150 + 127) / 255) as u8);
  assert_eq!(
    dst.data()[4],
    77,
    "a pixel outside the stamped box must not be dimmed"
  );
}

#[test]
fn ink_segment_keeps_the_backdrop_equal_to_a_full_dim() {
  // A live stroke dims one segment's box at a time instead of the whole
  // screen, which is only correct while that box covers every pixel the
  // segment touched, round caps included.
  let mut canvas = Pixmap::new(32, 32).unwrap();
  canvas.data_mut().iter_mut().for_each(|b| *b = 200);
  let mut backdrop = Pixmap::new(32, 32).unwrap();
  dimmed_into(&mut backdrop, &canvas);

  ink_segment(
    &mut canvas,
    &mut backdrop,
    Segment {
      from: Point::new(4.0, 4.0),
      to: Point::new(24.0, 4.0),
      color: [255, 0, 0],
      width: 4.0,
      alpha: 255,
    },
  );

  assert_eq!(
    backdrop.data(),
    dimmed(&canvas).data(),
    "the segment's bounds should cover every pixel it inked"
  );
}

#[test]
fn region_pixels_clamps_to_source_bounds() {
  let sel = Rect::new(-5.0, -5.0, 100.0, 100.0);
  let (x0, y0, w, h) = region_pixels(sel, 50, 40);
  assert_eq!((x0, y0, w, h), (0, 0, 50, 40));
}

#[test]
fn region_pixels_drains_a_selection_past_the_source() {
  // A drag can leave the window, so the rubber band can start past the
  // right and bottom edges. That must report an empty region, not panic.
  let sel = Rect::new(45.0, 38.0, 40.0, 20.0);
  let (x0, y0, w, h) = region_pixels(sel, 50, 40);
  assert_eq!((x0, y0, w, h), (45, 38, 5, 2));
  let beyond = Rect::new(80.0, 90.0, 10.0, 10.0);
  let (x0, y0, w, h) = region_pixels(beyond, 50, 40);
  assert_eq!((x0, y0, w, h), (50, 40, 0, 0));
}

#[test]
fn flatten_crops_the_exact_pixels_of_the_region() {
  let mut frame = Pixmap::new(4, 4).unwrap();
  for (row, px) in frame
    .data_mut()
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .enumerate()
  {
    px.copy_from_slice(&[row as u8, 0, 0, 255]);
  }
  let shot = flatten(&frame, Rect::new(1.0, 1.0, 2.0, 2.0));
  assert_eq!((shot.width, shot.height), (2, 2));
  let rows: Vec<u8> =
    shot.rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
  assert_eq!(rows, vec![5, 6, 9, 10]);
}

#[test]
fn flatten_rebases_a_full_width_region_onto_the_origin() {
  let mut frame = Pixmap::new(4, 4).unwrap();
  for (row, px) in frame
    .data_mut()
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .enumerate()
  {
    px.copy_from_slice(&[row as u8, 0, 0, 255]);
  }
  // Rows 2 and 3, full width: one straight copy, but it must start at
  // source row 2 rather than row 0.
  let shot = flatten(&frame, Rect::new(0.0, 2.0, 4.0, 2.0));
  assert_eq!((shot.width, shot.height), (4, 2));
  let rows: Vec<u8> =
    shot.rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
  assert_eq!(rows, vec![8, 9, 10, 11, 12, 13, 14, 15]);
}

#[test]
fn flatten_carries_ink_already_committed_to_the_canvas() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let mut canvas = Pixmap::new(100, 100).unwrap();
  canvas.data_mut().iter_mut().for_each(|p| *p = 100);
  let stroke = Shape::Line {
    from: Point::new(15.0, 15.0),
    to: Point::new(25.0, 35.0),
    color: [239, 68, 68],
    width: 2.5,
    arrow: false,
  };
  ink(&mut canvas, &stroke, &engine);

  let shot = flatten(&canvas, Rect::new(10.0, 10.0, 20.0, 30.0));
  let painted = shot
    .rgba
    .as_chunks::<4>()
    .0
    .iter()
    .any(|p| p[0] == 239 && p[1] == 68 && p[2] == 68);
  assert!(painted, "flattened region should contain the drawn stroke");
}

#[test]
fn arrow_line_stops_at_the_head_and_tapers_to_a_point() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let mut canvas = Pixmap::new(100, 100).unwrap();
  let arrow = Shape::Line {
    from: Point::new(10.0, 50.0),
    to: Point::new(90.0, 50.0),
    color: [255, 0, 0],
    width: 4.0,
    arrow: true,
  };
  ink(&mut canvas, &arrow, &engine);

  let painted_in_column = |x: usize| -> usize {
    canvas
      .data()
      .as_chunks::<4>()
      .0
      .iter()
      .skip(x)
      .step_by(100)
      .filter(|p| p[0] == 255 && p[3] == 255)
      .count()
  };

  let tip = painted_in_column(90);
  let body = painted_in_column(80);
  assert!(
    tip <= 2,
    "arrow tip should taper to a point, got {tip} painted pixels at the head"
  );
  assert!(
    body > 4,
    "arrowhead body should be wider than the line, got {body} painted pixels"
  );
}
