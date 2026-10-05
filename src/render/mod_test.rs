use tiny_skia::Pixmap;

use super::{
  badge_rect, build, deliverable_region, dimmed_into, ink, paint,
  region_pixels, repaint, shape_area, typed_area, Backdrop, Chrome, Hotspot,
  Scene,
};
use crate::{
  annotate::{active_color, History, Shape, Tool},
  geom::{Point, Rect},
  text::TextEngine,
};

fn dimmed(frame: &Pixmap) -> Pixmap {
  let mut out =
    Pixmap::new(frame.width(), frame.height()).expect("valid dimensions");
  dimmed_into(&mut out, frame);
  out
}

fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
  left.iter().zip(right).position(|(a, b)| a != b)
}

fn busy_scene(sel: Rect, bounds: Rect) -> (Pixmap, Pixmap, Chrome) {
  let mut canvas = Pixmap::new(400, 400).unwrap();
  for (index, pixel) in canvas
    .data_mut()
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .enumerate()
  {
    let v = (index % 251) as u8;
    pixel.copy_from_slice(&[v, v.wrapping_add(7), v.wrapping_add(31), 255]);
  }
  let backdrop = dimmed(&canvas);
  let mut chrome = Chrome::new(Backdrop::Frozen);
  let mut history = History::default();
  history.push(Shape::Text {
    at: Point::new(40.0, 60.0),
    text: "note".to_string(),
    color: [255, 255, 255],
    size: 14.0,
  });
  build(
    &mut chrome,
    Some(sel),
    bounds,
    Tool::Pen,
    &history,
    true,
    Backdrop::Frozen,
  );
  (canvas, backdrop, chrome)
}

fn scene_of<'a>(
  size: (u32, u32),
  canvas: &'a Pixmap,
  backdrop: &'a Pixmap,
  chrome: &'a Chrome,
  engine: &'a TextEngine,
  selection: Option<Rect>,
) -> Scene<'a> {
  Scene {
    backdrop,
    canvas,
    bounds: Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32),
    selection,
    picked: None,
    kind: Backdrop::Frozen,
    draft: None,
    typing: None,
    caret: false,
    palette_index: 0,
    chrome,
    hotspot: None,
    text: engine,
    hint: None,
  }
}

fn painted_in(
  pm: &Pixmap,
  size: (u32, u32),
  box_: Rect,
  rgb: [u8; 3],
) -> usize {
  let (x, y, width, height) = region_pixels(box_, size.0, size.1);
  (0..height)
    .flat_map(|row| (0..width).map(move |col| (col, row)))
    .filter(|&(col, row)| {
      let i = ((y + row) * size.0 + x + col) as usize * 4;
      pm.data()[i..i + 3] == rgb
    })
    .count()
}

#[test]
fn deliverable_region_requires_minimum_size() {
  assert!(deliverable_region(Some(Rect::new(0.0, 0.0, 100.0, 100.0))).is_some());
  assert!(deliverable_region(Some(Rect::new(0.0, 0.0, 3.0, 3.0))).is_none());
  assert!(deliverable_region(None).is_none());
}

#[test]
fn draft_is_stroked_at_absolute_coordinates() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let mut canvas = Pixmap::new(40, 80).unwrap();
  for px in canvas.data_mut().as_chunks_mut::<4>().0 {
    px.copy_from_slice(&[200, 200, 200, 255]);
  }
  let backdrop = dimmed(&canvas);
  let bounds = Rect::new(0.0, 0.0, 40.0, 80.0);
  let sel = Rect::new(5.0, 5.0, 30.0, 70.0);
  let selection = Some(sel);
  let history = History::default();
  let mut chrome = Chrome::new(Backdrop::Frozen);
  build(
    &mut chrome,
    selection,
    bounds,
    Tool::Select,
    &history,
    false,
    Backdrop::Frozen,
  );

  let draft = Shape::Line {
    from: Point::new(10.0, 50.0),
    to: Point::new(30.0, 70.0),
    color: [255, 0, 0],
    width: 3.0,
    arrow: false,
  };

  let mut pm = backdrop.clone();

  let scene = Scene {
    backdrop: &backdrop,
    canvas: &canvas,
    bounds,
    selection,
    picked: None,
    kind: Backdrop::Frozen,
    draft: Some(&draft),
    typing: None,
    caret: false,
    palette_index: 0,
    chrome: &chrome,
    hotspot: None,
    text: &engine,
    hint: None,
  };

  paint(&mut pm, &scene, bounds);

  let idx = (70 * 40 + 30) * 4;
  let px = &pm.data()[idx..idx + 4];
  assert!(
    px[0] > 150 && px[2] < 100,
    "draft should appear at (30, 70), got {px:?}"
  );
}

#[test]
fn a_frame_painted_in_pieces_is_the_frame_a_whole_paint_makes() {
  // A session repaints one box at a time, so tiling the screen has to land on
  // exactly the frame a single paint would have made. Anything the clipped
  // paint gets wrong about what belongs where shows up as a seam.
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let bounds = Rect::new(0.0, 0.0, 400.0, 400.0);
  let sel = Rect::new(30.0, 30.0, 300.0, 200.0);
  let (canvas, backdrop, chrome) = busy_scene(sel, bounds);
  let mut scene =
    scene_of((400, 400), &canvas, &backdrop, &chrome, &engine, Some(sel));
  scene.hotspot = Some(Hotspot::Action(2));

  let mut whole = Pixmap::new(400, 400).unwrap();
  paint(&mut whole, &scene, bounds);

  let mut tiled = Pixmap::new(400, 400).unwrap();
  let tile = Rect::new(0.0, 0.0, 133.0, 117.0);
  for row in 0..4 {
    for column in 0..4 {
      paint(
        &mut tiled,
        &scene,
        Rect::new(
          tile.x + tile.w * column as f32,
          tile.y + tile.h * row as f32,
          tile.w,
          tile.h,
        ),
      );
    }
  }
  if first_difference(tiled.data(), whole.data()).is_some() {
    for y in 0..6 {
      let mut line = String::new();
      for x in 26..40 {
        let i = (y * 400 + x) * 4;
        line.push_str(&format!(
          "{:>3},{:>3},{:>3}|",
          tiled.data()[i],
          tiled.data()[i + 1],
          tiled.data()[i + 2]
        ));
      }
      eprintln!("tiled y={y} {line}");
      let mut line = String::new();
      for x in 26..40 {
        let i = (y * 400 + x) * 4;
        line.push_str(&format!(
          "{:>3},{:>3},{:>3}|",
          whole.data()[i],
          whole.data()[i + 1],
          whole.data()[i + 2]
        ));
      }
      eprintln!("whole y={y} {line}");
    }
  }
  assert_eq!(
    first_difference(tiled.data(), whole.data()),
    None,
    "painting the screen in tiles has to land on the same frame"
  );
}

#[test]
fn a_damaged_frame_leaves_the_rest_of_the_buffer_alone() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let bounds = Rect::new(0.0, 0.0, 400.0, 400.0);
  let sel = Rect::new(30.0, 30.0, 300.0, 200.0);
  let (canvas, backdrop, chrome) = busy_scene(sel, bounds);
  let scene =
    scene_of((400, 400), &canvas, &backdrop, &chrome, &engine, Some(sel));

  // A frame holding what the last frame drew, repainted over one box: only
  // that box may change, because the pixels around it are already right.
  let mut frame = Pixmap::new(400, 400).unwrap();
  paint(&mut frame, &scene, bounds);
  let before = frame.clone();
  paint(&mut frame, &scene, Rect::new(100.0, 100.0, 40.0, 40.0));

  assert_eq!(
    first_difference(frame.data(), before.data()),
    None,
    "repainting the same scene over a box of it must change nothing"
  );
}

#[test]
fn damage_that_clips_the_badge_repaints_the_whole_of_it() {
  // The badge is laid down in one piece, so a box that clips a corner of it
  // has to bring the whole thing back rather than leave the rest of it
  // standing on screen for the rest of the session. The chrome is left
  // unlaid out so the badge is the only piece the damage can reach: a panel
  // spans the middle of the screen and would carry it either way.
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let bounds = Rect::new(0.0, 0.0, 400.0, 400.0);
  let sel = Rect::new(200.0, 200.0, 100.0, 100.0);
  let canvas = Pixmap::new(400, 400).unwrap();
  let backdrop = dimmed(&canvas);
  let mut chrome = Chrome::new(Backdrop::Frozen);
  build(
    &mut chrome,
    Some(sel),
    bounds,
    Tool::Pen,
    &History::default(),
    false,
    Backdrop::Frozen,
  );
  let scene =
    scene_of((400, 400), &canvas, &backdrop, &chrome, &engine, Some(sel));
  let badge = badge_rect(sel, bounds, &engine);

  let mut whole = Pixmap::new(400, 400).unwrap();
  paint(&mut whole, &scene, bounds);
  let mut clipped = Pixmap::new(400, 400).unwrap();
  paint(&mut clipped, &scene, bounds);
  // The bottom right corner of the badge, clear of the region's own frame.
  paint(
    &mut clipped,
    &scene,
    Rect::new(badge.right() - 8.0, badge.bottom() - 4.0, 8.0, 4.0),
  );

  assert_eq!(
    first_difference(clipped.data(), whole.data()),
    None,
    "the badge has to come back whole, got {badge:?}"
  );
}

#[test]
fn repaint_puts_a_moved_run_where_it_went_and_takes_nothing_with_it() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  const SIZE: (u32, u32) = (300, 200);
  const WHITE: [u8; 3] = [255, 255, 255];
  const RED: [u8; 3] = [239, 68, 68];
  let mut base = Pixmap::new(SIZE.0, SIZE.1).unwrap();
  base
    .data_mut()
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .for_each(|px| px.copy_from_slice(&[40, 60, 80, 255]));

  let written = |x: f32| Shape::Text {
    at: Point::new(x, 40.0),
    text: "movable".to_string(),
    color: WHITE,
    size: 20.0,
  };
  let stroke = Shape::Line {
    from: Point::new(10.0, 46.0),
    to: Point::new(290.0, 46.0),
    color: RED,
    width: 4.0,
    arrow: false,
  };
  let was = shape_area(Some(&written(20.0)), &engine);
  let now = shape_area(Some(&written(160.0)), &engine);

  let mut canvas = base.clone();
  let mut backdrop = dimmed(&base);
  let whole = Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32);
  repaint(
    &mut canvas,
    &mut backdrop,
    &base,
    &[written(20.0), stroke.clone()],
    whole,
    &engine,
  );
  assert!(
    painted_in(&canvas, SIZE, was, WHITE) > 0,
    "the run has to be written where it was left, {was:?}"
  );

  repaint(
    &mut canvas,
    &mut backdrop,
    &base,
    &[written(160.0), stroke],
    was.union(now),
    &engine,
  );
  assert_eq!(
    painted_in(&canvas, SIZE, was, WHITE),
    0,
    "and it has to leave {was:?} clear once it has moved"
  );
  assert!(
    painted_in(&canvas, SIZE, now, WHITE) > 0,
    "the run has to show up where it was moved to, {now:?}"
  );
  assert!(
    painted_in(&canvas, SIZE, was.union(now), RED) > 0,
    "the stroke it moved across has to survive the move"
  );
}

#[test]
fn a_rebuild_leaves_translucent_ink_exactly_as_it_was_laid() {
  // A rebuild lays every shape that reaches into the box down again. Ink
  // laid down twice is darker than ink laid once, so the marker under the
  // box has to go back to the capture whole rather than have its ink put
  // over its own.
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let size = (400u32, 200u32);
  let mut base = Pixmap::new(size.0, size.1).unwrap();
  base
    .data_mut()
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .for_each(|px| px.copy_from_slice(&[40, 60, 80, 255]));
  let marker = Shape::Stroke {
    points: vec![Point::new(10.0, 120.0), Point::new(380.0, 120.0)],
    color: [250, 204, 21],
    width: 16.0,
    marker: true,
  };
  let text = Shape::Text {
    at: Point::new(150.0, 108.0),
    text: "over the marker".to_string(),
    color: [255, 255, 255],
    size: 20.0,
  };

  // The ground as one pass over both shapes lays it, which is what the
  // rebuild has to arrive at.
  let mut once = base.clone();
  ink(&mut once, &marker, &engine);
  ink(&mut once, &text, &engine);
  let mut rebuilt = base.clone();
  ink(&mut rebuilt, &marker, &engine);
  let mut backdrop = dimmed(&base);
  let laid = repaint(
    &mut rebuilt,
    &mut backdrop,
    &base,
    &[marker.clone(), text.clone()],
    shape_area(Some(&text), &engine),
    &engine,
  );

  assert_eq!(
    first_difference(rebuilt.data(), once.data()),
    None,
    "the marker has to come out of the rebuild exactly as it went in"
  );
  assert!(
    laid.contains_rect(shape_area(Some(&marker), &engine)),
    "and the caller has to repaint all of it, got {laid:?}"
  );
}

#[test]
fn the_text_being_written_is_drawn_in_the_colour_it_will_land_in() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let bounds = Rect::new(0.0, 0.0, 200.0, 120.0);
  let canvas = Pixmap::new(200, 120).unwrap();
  let backdrop = dimmed(&canvas);
  let chrome = Chrome::new(Backdrop::Frozen);
  let mut scene =
    scene_of((200, 120), &canvas, &backdrop, &chrome, &engine, None);
  // The active colour is the palette's first entry, so a preview drawn in the
  // wrong one is easy to tell from the right one.
  scene.typing = Some((Point::new(20.0, 70.0), "Hg", 20.0, [0, 255, 0]));

  let mut pm = Pixmap::new(200, 120).unwrap();
  paint(&mut pm, &scene, bounds);
  let lit = |rgb: [u8; 3]| {
    pm.data()
      .as_chunks::<4>()
      .0
      .iter()
      .filter(|px| [px[0], px[1], px[2]] == rgb)
      .count()
  };
  assert!(lit([0, 255, 0]) > 0, "the preview has to show at all");
  assert_eq!(
    lit(active_color(scene.palette_index)),
    0,
    "and it has to be in the colour the run will land in"
  );
}

#[test]
fn rebuilding_a_runs_own_ink_takes_every_letterform_with_it() {
  // The line a run sits on does not hold its ink: a descender reaches below
  // the line and a hook can reach left of it, so the box a rebuild is asked
  // for has to be the ink and not the line.
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let screen = (200u32, 120u32);
  let mut base = Pixmap::new(screen.0, screen.1).unwrap();
  base
    .data_mut()
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .for_each(|px| px.copy_from_slice(&[20, 30, 40, 255]));

  let run = Shape::Text {
    at: Point::new(40.0, 40.0),
    text: "jjgjpqy".to_string(),
    color: [255, 255, 255],
    size: 24.0,
  };
  let Shape::Text {
    at, ref text, size, ..
  } = run
  else {
    panic!("a run is a text shape")
  };
  let line = engine.bounds(text, at, size);
  let lettered = shape_area(Some(&run), &engine);
  assert!(
    !line.contains_rect(lettered),
    "the test is only worth anything if the line misses the ink: \
     {line:?} vs {lettered:?}"
  );

  let mut canvas = base.clone();
  ink(&mut canvas, &run, &engine);
  let mut backdrop = dimmed(&base);
  // The run as a lift leaves it: on the canvas, but out of the history.
  repaint(&mut canvas, &mut backdrop, &base, &[], lettered, &engine);

  assert_eq!(
    painted_in(
      &canvas,
      screen,
      Rect::new(0.0, 0.0, 200.0, 120.0),
      [255, 255, 255]
    ),
    0,
    "a lifted run must leave nothing of itself on the canvas"
  );
}

#[test]
fn the_caret_paints_inside_the_damage_box_reserved_for_it() {
  // The caret is a round-capped stroke, so it reaches half its width above
  // the anchor. Whatever it paints outside the typed area never gets
  // repainted, so it would sit there for the rest of the session.
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let bounds = Rect::new(0.0, 0.0, 200.0, 120.0);
  let canvas = Pixmap::new(200, 120).unwrap();
  let backdrop = dimmed(&canvas);
  let mut chrome = Chrome::new(Backdrop::Frozen);
  build(
    &mut chrome,
    None,
    bounds,
    Tool::Label,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  // Nothing typed yet and no region picked, so the caret is the only thing
  // the paint can put on screen.
  let at = Point::new(100.0, 70.0);
  let mut scene =
    scene_of((200, 120), &canvas, &backdrop, &chrome, &engine, None);
  scene.typing = Some((at, "", 20.0, [255, 255, 255]));
  scene.caret = true;

  let mut pm = Pixmap::new(200, 120).unwrap();
  paint(&mut pm, &scene, bounds);

  let area = typed_area(scene.typing, &engine);
  for (index, pixel) in pm.data().as_chunks::<4>().0.iter().enumerate() {
    if pixel[3] == 0 {
      continue;
    }
    let x = (index % 200) as f32;
    let y = (index / 200) as f32;
    assert!(
      area.contains(Point::new(x, y)),
      "the caret lit ({x}, {y}), outside the {area:?} it reserved"
    );
  }
}
