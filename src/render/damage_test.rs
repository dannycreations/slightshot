use tiny_skia::Pixmap;

use super::{framed_area, panels_area, Shown};
use crate::{
  annotate::{History, Tool},
  geom::Rect,
  render::{
    chrome::{build, tooltip_rect, Side},
    surface::dimmed_into,
    Backdrop, Chrome, Hotspot, Scene,
  },
  text::TextEngine,
};

fn dimmed(frame: &Pixmap) -> Pixmap {
  let mut out =
    Pixmap::new(frame.width(), frame.height()).expect("valid dimensions");
  dimmed_into(&mut out, frame);
  out
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

#[test]
fn a_picked_box_that_moves_damages_both_places_it_was() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let canvas = Pixmap::new(600, 600).unwrap();
  let backdrop = dimmed(&canvas);
  let bounds = Rect::new(0.0, 0.0, 600.0, 600.0);
  let sel = Rect::new(20.0, 20.0, 80.0, 80.0);
  let mut chrome = Chrome::new();
  build(
    &mut chrome,
    Some(sel),
    bounds,
    Tool::Select,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  let mut scene =
    scene_of((600, 600), &canvas, &backdrop, &chrome, &engine, Some(sel));

  let from = Rect::new(100.0, 100.0, 60.0, 24.0);
  let to = Rect::new(300.0, 300.0, 60.0, 24.0);
  scene.picked = Some(from);
  let before = Shown::capture(&scene);
  scene.picked = Some(to);
  let after = Shown::capture(&scene);

  let area = before.settled_from(&after, bounds, &engine);
  assert!(
    area.contains_rect(framed_area(from))
      && area.contains_rect(framed_area(to)),
    "the frame and its handles have to be painted out where they were and \
     in where they are, got {area:?}"
  );
  scene.picked = None;
  let dropped = Shown::capture(&scene);
  assert!(
    before
      .settled_from(&dropped, bounds, &engine)
      .contains_rect(framed_area(from)),
    "putting a run down has to take its frame off the screen"
  );
}

#[test]
fn an_unchanged_state_settles_nowhere() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let canvas = Pixmap::new(400, 400).unwrap();
  let backdrop = dimmed(&canvas);
  let bounds = Rect::new(0.0, 0.0, 400.0, 400.0);
  let sel = Rect::new(20.0, 20.0, 120.0, 120.0);
  let mut chrome = Chrome::new();
  build(
    &mut chrome,
    Some(sel),
    bounds,
    Tool::Pen,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  let scene =
    scene_of((400, 400), &canvas, &backdrop, &chrome, &engine, Some(sel));
  let shown = Shown::capture(&scene);
  let same = Shown::capture(&scene);
  assert!(
    shown.settled_from(&same, bounds, &engine).is_empty(),
    "a frame with nothing new to show should not repaint anything"
  );
}

#[test]
fn a_state_that_moves_the_selection_covers_both_regions() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let canvas = Pixmap::new(600, 600).unwrap();
  let backdrop = dimmed(&canvas);
  let bounds = Rect::new(0.0, 0.0, 600.0, 600.0);
  let history = History::default();
  let from = Rect::new(20.0, 20.0, 80.0, 80.0);
  let to = Rect::new(400.0, 400.0, 80.0, 80.0);

  // The panels hang off the region, so the one built for each of them has to
  // be part of the damage: the old ones have to be painted out and the new
  // ones in.
  let mut old_chrome = Chrome::new();
  build(
    &mut old_chrome,
    Some(from),
    bounds,
    Tool::Pen,
    &history,
    true,
    Backdrop::Frozen,
  );
  let mut new_chrome = Chrome::new();
  build(
    &mut new_chrome,
    Some(to),
    bounds,
    Tool::Pen,
    &history,
    true,
    Backdrop::Frozen,
  );

  let before = Shown::capture(&scene_of(
    (600, 600),
    &canvas,
    &backdrop,
    &old_chrome,
    &engine,
    Some(from),
  ));
  let after = Shown::capture(&scene_of(
    (600, 600),
    &canvas,
    &backdrop,
    &new_chrome,
    &engine,
    Some(to),
  ));
  let area = before.settled_from(&after, bounds, &engine);
  assert!(
    area.contains_rect(from) && area.contains_rect(to),
    "the damage must cover both regions, got {area:?}"
  );
  assert!(
    area.contains_rect(old_chrome.tools[0].area),
    "the panels that moved away, got {area:?}"
  );
  assert!(
    area.contains_rect(new_chrome.tools[0].area),
    "and the panels that moved in, got {area:?}"
  );
}

#[test]
fn an_unlaid_out_panel_damages_nothing() {
  // `Rect::ZERO` is how the chrome says "no button here". Inflating that
  // sentinel would make it a live box at the origin, and every union with a
  // real region would then reach back across the screen to the corner.
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let chrome = Chrome::new();
  let area = panels_area(
    &chrome,
    None,
    None,
    Rect::new(0.0, 0.0, 400.0, 400.0),
    &engine,
  );
  assert!(
    area.is_empty(),
    "no button is laid out, so the panel claims no ground, got {area:?}"
  );
}

#[test]
fn a_hover_only_change_covers_the_buttons_and_the_tooltip() {
  let Ok(engine) = TextEngine::load() else {
    return;
  };
  let canvas = Pixmap::new(600, 600).unwrap();
  let backdrop = dimmed(&canvas);
  let bounds = Rect::new(0.0, 0.0, 600.0, 600.0);
  let sel = Rect::new(200.0, 200.0, 120.0, 120.0);
  let mut chrome = Chrome::new();
  build(
    &mut chrome,
    Some(sel),
    bounds,
    Tool::Pen,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  let mut cold =
    scene_of((600, 600), &canvas, &backdrop, &chrome, &engine, Some(sel));
  let before = Shown::capture(&cold);
  cold.hotspot = Some(Hotspot::Tool(1));
  let after = Shown::capture(&cold);
  let area = before.settled_from(&after, bounds, &engine);
  let hovered = &chrome.tools[1];
  assert!(
    area.contains_rect(hovered.area),
    "the hovered button must be repainted, got {area:?}"
  );
  let tooltip = tooltip_rect(
    hovered.area,
    hovered.command.label(),
    bounds,
    &engine,
    Side::Left,
  );
  assert!(
    area.contains_rect(tooltip),
    "and the tooltip it brings up, got {area:?} missing {tooltip:?}"
  );
}
