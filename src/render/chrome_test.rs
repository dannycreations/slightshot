use super::{
  build, hotspot_at, Button, Chrome, Command, Hotspot, PANEL_PAD,
  SCREEN_MARGIN, TOOL_GAP,
};
use crate::{
  action::Deliverable,
  annotate::{History, Shape, Tool},
  geom::{Point, Rect},
  render::Backdrop,
};

#[test]
fn hotspot_at_finds_button_under_point() {
  let mut chrome = Chrome::new(Backdrop::Frozen);
  chrome.actions[0].area = Rect::new(10.0, 10.0, 30.0, 30.0);
  assert_eq!(
    hotspot_at(&chrome, Point::new(20.0, 20.0)),
    Some(Hotspot::Action(0))
  );
  assert_eq!(hotspot_at(&chrome, Point::new(100.0, 100.0)), None);
}

#[test]
fn command_label_names_every_button() {
  let cases = [
    (Command::Tool(Tool::Select), "Select"),
    (Command::Tool(Tool::Pen), "Pen"),
    (Command::Tool(Tool::Line), "Line"),
    (Command::Tool(Tool::Arrow), "Arrow"),
    (Command::Tool(Tool::Box), "Rectangle"),
    (Command::Tool(Tool::Marker), "Marker"),
    (Command::Tool(Tool::Label), "Text"),
    (Command::NextColor, "Next color"),
    (Command::Undo, "Undo"),
    (Command::Close, "Close"),
    (Command::Deliver(Deliverable::Upload), "Upload"),
    (Command::Deliver(Deliverable::Copy), "Copy"),
    (Command::Deliver(Deliverable::Save), "Save"),
  ];
  for (command, expected) in cases {
    assert_eq!(command.label(), expected);
  }
}

#[test]
fn build_hides_buttons_without_selection() {
  let sel = Rect::new(10.0, 10.0, 200.0, 150.0);
  let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
  let mut chrome = Chrome::new(Backdrop::Frozen);
  build(
    &mut chrome,
    Some(sel),
    bounds,
    Tool::Pen,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  assert!(chrome.tools.iter().all(|b| b.area.w > 0.0));
  build(
    &mut chrome,
    None,
    bounds,
    Tool::Pen,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  assert!(chrome.tools.iter().all(|b| b.area.w == 0.0));
  assert!(chrome.actions.iter().all(|b| b.area.w == 0.0));
}

#[test]
fn build_shows_deliver_buttons_when_selection_is_ready() {
  // Tall enough for the whole column, so it can hang off the region's own
  // bottom edge rather than being clamped back onto the screen.
  let sel = Rect::new(10.0, 10.0, 200.0, 400.0);
  let mut chrome = Chrome::new(Backdrop::Frozen);
  build(
    &mut chrome,
    Some(sel),
    Rect::new(0.0, 0.0, 1920.0, 1080.0),
    Tool::Pen,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  assert!(chrome.tools.iter().all(|b| b.area.w > 0.0));
  assert!(chrome.actions.iter().all(|b| b.area.w > 0.0));
  let close = chrome.actions.iter().find(|b| b.command == Command::Close);
  assert!(close.is_some_and(|b| b.enabled));
  let lowest = chrome
    .tools
    .iter()
    .map(|b| b.area.bottom())
    .fold(f32::MIN, f32::max);
  assert_eq!(
    lowest,
    sel.bottom() - PANEL_PAD,
    "a picked region keeps its column hanging off its own bottom edge"
  );
}

#[test]
fn build_hides_buttons_when_not_idle() {
  let sel = Rect::new(10.0, 10.0, 200.0, 150.0);
  let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
  let mut chrome = Chrome::new(Backdrop::Frozen);
  build(
    &mut chrome,
    Some(sel),
    bounds,
    Tool::Pen,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  build(
    &mut chrome,
    Some(sel),
    bounds,
    Tool::Pen,
    &History::default(),
    false,
    Backdrop::Frozen,
  );
  assert!(chrome.tools.iter().all(|b| b.area.w == 0.0));
  assert!(chrome.actions.iter().all(|b| b.area.w == 0.0));
}

#[test]
fn a_panel_taller_than_the_screen_hangs_off_the_top_of_it() {
  // A screen too short to hold the whole column used to ask `clamp` for a
  // range with its ends the wrong way round, which panics.
  let bounds = Rect::new(0.0, 0.0, 1920.0, 200.0);
  let mut chrome = Chrome::new(Backdrop::Frozen);
  build(
    &mut chrome,
    Some(Rect::new(10.0, 10.0, 200.0, 150.0)),
    bounds,
    Tool::Pen,
    &History::default(),
    true,
    Backdrop::Frozen,
  );
  let top = chrome
    .tools
    .iter()
    .map(|b| b.area.y)
    .fold(f32::MAX, f32::min);
  assert_eq!(
    top,
    bounds.y + SCREEN_MARGIN + PANEL_PAD,
    "the column sits against the top edge, which is all there is room for"
  );
}

#[test]
fn build_derives_button_state_from_the_session() {
  let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
  let ready = Rect::new(10.0, 10.0, 200.0, 150.0);
  let mut history = History::default();
  let mut chrome = Chrome::new(Backdrop::Frozen);
  build(
    &mut chrome,
    Some(ready),
    bounds,
    Tool::Arrow,
    &history,
    true,
    Backdrop::Frozen,
  );

  fn find(chrome: &Chrome, command: Command) -> &Button {
    chrome
      .tools
      .iter()
      .chain(&chrome.actions)
      .find(|b| b.command == command)
      .expect("the chrome lists every command")
  }
  assert!(find(&chrome, Command::Tool(Tool::Arrow)).active);
  assert!(!find(&chrome, Command::Tool(Tool::Pen)).active);
  assert!(!find(&chrome, Command::Undo).enabled);
  assert!(find(&chrome, Command::Deliver(Deliverable::Copy)).enabled);

  history.push(Shape::Text {
    at: Point::new(5.0, 5.0),
    text: "x".to_string(),
    color: [0, 0, 0],
    size: 10.0,
  });
  build(
    &mut chrome,
    Some(ready),
    bounds,
    Tool::Arrow,
    &history,
    true,
    Backdrop::Frozen,
  );
  assert!(find(&chrome, Command::Undo).enabled);

  let tiny = Rect::new(10.0, 10.0, 1.0, 1.0);
  build(
    &mut chrome,
    Some(tiny),
    bounds,
    Tool::Arrow,
    &history,
    true,
    Backdrop::Frozen,
  );
  assert!(!find(&chrome, Command::Deliver(Deliverable::Copy)).enabled);
  assert!(find(&chrome, Command::Close).enabled);
}

#[test]
fn a_live_backdrop_offers_nothing_to_deliver() {
  // Drawing over the live desktop never captured anything, so the three
  // actions that hand an image off have nothing to hand. Closing the overlay
  // still has to work.
  let sel = Rect::new(10.0, 10.0, 200.0, 150.0);
  let mut history = History::default();
  history.push(Shape::Text {
    at: Point::new(5.0, 5.0),
    text: "x".to_string(),
    color: [0, 0, 0],
    size: 10.0,
  });
  let mut chrome = Chrome::new(Backdrop::Live);
  build(
    &mut chrome,
    Some(sel),
    Rect::new(0.0, 0.0, 1920.0, 1080.0),
    Tool::Pen,
    &history,
    true,
    Backdrop::Live,
  );
  for button in chrome.tools.iter().chain(&chrome.actions) {
    if matches!(button.command, Command::Deliver(_)) {
      assert!(!button.enabled, "{button:?} cannot deliver a live overlay");
    }
  }
  // The close button the live overlay shows is the one in the column.
  let close = chrome.tools.iter().find(|b| b.command == Command::Close);
  assert!(close.is_some_and(|b| b.enabled));
}

#[test]
fn a_live_backdrop_shows_its_tools_before_anything_is_picked() {
  // The whole screen is the area, so the panel has to be laid out from the
  // moment the overlay opens rather than after a drag. Nothing was captured,
  // so the row of delivery actions stays hidden and Close ends the column.
  let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
  let mut chrome = Chrome::new(Backdrop::Live);
  build(
    &mut chrome,
    Some(bounds),
    bounds,
    Tool::Pen,
    &History::default(),
    true,
    Backdrop::Live,
  );

  fn find(buttons: &[Button], command: Command) -> &Button {
    buttons
      .iter()
      .find(|b| b.command == command)
      .expect("the column carries every command it shows")
  }

  assert!(chrome.tools.iter().all(|b| b.area.w > 0.0));
  assert!(
    chrome.actions.iter().all(|b| b.area.w == 0.0),
    "a live overlay has nothing to hand over, so its row stays hidden"
  );

  let top = chrome
    .tools
    .iter()
    .map(|b| b.area.y)
    .fold(f32::MAX, f32::min);
  let bottom = chrome
    .tools
    .iter()
    .map(|b| b.area.bottom())
    .fold(f32::MIN, f32::max);
  assert!(
    (top + bottom - (bounds.y + bounds.bottom())).abs() < 1.0,
    "the live column is centred on the screen, got {top}..{bottom}"
  );

  let undo = find(&chrome.tools, Command::Undo);
  let close = find(&chrome.tools, Command::Close);
  assert_eq!(close.area.x, undo.area.x);
  assert_eq!(
    close.area.y,
    undo.area.bottom() + TOOL_GAP,
    "close sits one slot below undo"
  );
  assert!(close.enabled);
  assert_eq!(
    hotspot_at(&chrome, close.area.center()),
    Some(Hotspot::Tool(chrome.tools.len() - 1)),
    "the close button at the foot of the column has to be clickable"
  );
  for button in &chrome.tools {
    if matches!(button.command, Command::Tool(Tool::Pen)) {
      assert!(button.active, "the pen is the tool in hand");
    }
  }
}
