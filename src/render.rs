use tiny_skia::Pixmap;

use crate::{
  actions::{Deliverable, Shot},
  annotate::{
    History, Shape, Tool, LABEL_SIZE, MARKER_ALPHA, MARKER_WIDTH, PALETTE,
  },
  draw,
  geom::{handle_anchor, Point, Rect, HANDLES},
  text::TextEngine,
};

pub const HANDLE_SLOP: f32 = 7.0;

const MIN_REGION: f32 = 6.0;
const DIM_ALPHA: u8 = 105;
const BUTTON: f32 = 30.0;
const TOOL_GAP: f32 = 2.0;
const PANEL_PAD: f32 = 5.0;
const BADGE_TEXT: f32 = 13.0;
const ICON_BOX: f32 = 18.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
  Tool(Tool),
  NextColor,
  Undo,
  Deliver(Deliverable),
}

#[derive(Clone, Copy, Debug)]
pub struct Button {
  pub command: Command,
  pub icon: draw::Icon,
  pub area: Rect,
  pub enabled: bool,
  pub active: bool,
}

#[derive(Debug)]
pub struct Chrome {
  pub tools: Vec<Button>,
  pub actions: Vec<Button>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hotspot {
  Tool(usize),
  Action(usize),
}

fn button(
  command: Command,
  icon: draw::Icon,
  enabled: bool,
  active: bool,
) -> Button {
  Button {
    command,
    icon,
    area: Rect::default(),
    enabled,
    active,
  }
}

pub fn deliverable_region(selection: Option<Rect>) -> Option<Rect> {
  selection.filter(|sel| sel.w >= MIN_REGION && sel.h >= MIN_REGION)
}

pub fn build(
  selection: Option<Rect>,
  bounds: Rect,
  tool: Tool,
  history: &History,
) -> Chrome {
  let ready = deliverable_region(selection).is_some();

  let mut tools: Vec<Button> = [
    (Tool::Pen, draw::Icon::Pen),
    (Tool::Line, draw::Icon::Line),
    (Tool::Arrow, draw::Icon::Arrow),
    (Tool::Box, draw::Icon::Outline),
    (Tool::Marker, draw::Icon::Marker),
    (Tool::Label, draw::Icon::Letter),
  ]
  .iter()
  .map(|&(item, icon)| button(Command::Tool(item), icon, true, item == tool))
  .collect();

  tools.push(button(Command::NextColor, draw::Icon::Letter, true, false));
  tools.push(button(
    Command::Undo,
    draw::Icon::Undo,
    history.can_undo(),
    false,
  ));

  let mut actions = [
    (Command::Deliver(Deliverable::Upload), draw::Icon::Upload),
    (Command::Deliver(Deliverable::Copy), draw::Icon::CopyImage),
    (Command::Deliver(Deliverable::Save), draw::Icon::Save),
    (Command::Deliver(Deliverable::Close), draw::Icon::Close),
  ]
  .iter()
  .map(|&(command, icon)| {
    let always_enabled = command == Command::Deliver(Deliverable::Close);
    button(command, icon, ready || always_enabled, false)
  })
  .collect::<Vec<_>>();

  layout_vertical(&mut tools, selection, bounds);
  layout_horizontal(&mut actions, selection, bounds);

  Chrome { tools, actions }
}

pub fn hotspot_at(chrome: &Chrome, p: Point) -> Option<Hotspot> {
  chrome
    .tools
    .iter()
    .position(|b| b.area.contains(p))
    .map(Hotspot::Tool)
    .or_else(|| {
      chrome
        .actions
        .iter()
        .position(|b| b.area.contains(p))
        .map(Hotspot::Action)
    })
}

fn hide_all(buttons: &mut [Button]) {
  for b in buttons {
    b.area = Rect::default();
  }
}

fn layout_vertical(
  buttons: &mut [Button],
  selection: Option<Rect>,
  bounds: Rect,
) {
  let Some(sel) = selection else {
    hide_all(buttons);
    return;
  };
  let count = buttons.len() as f32;
  let height = count * BUTTON + (count - 1.0) * TOOL_GAP + PANEL_PAD * 2.0;
  let width = BUTTON + PANEL_PAD * 2.0;

  let mut x = sel.right() + 6.0;
  if x + width > bounds.right() {
    x = sel.right() - width - 6.0;
  }
  x = x.clamp(bounds.x, (bounds.right() - width).max(bounds.x));
  let y = (sel.bottom() - height).clamp(
    bounds.y + 4.0,
    (bounds.bottom() - height - 4.0).max(bounds.y),
  );

  for (index, button) in buttons.iter_mut().enumerate() {
    button.area = Rect::new(
      x + PANEL_PAD,
      y + PANEL_PAD + index as f32 * (BUTTON + TOOL_GAP),
      BUTTON,
      BUTTON,
    );
  }
}

fn layout_horizontal(
  buttons: &mut [Button],
  selection: Option<Rect>,
  bounds: Rect,
) {
  let Some(sel) = selection else {
    hide_all(buttons);
    return;
  };
  let count = buttons.len() as f32;
  let width = count * BUTTON + (count - 1.0) * TOOL_GAP + PANEL_PAD * 2.0;
  let height = BUTTON + PANEL_PAD * 2.0;

  let x = (sel.right() - width).max(bounds.x);
  let mut y = sel.bottom() + 8.0;
  if y + height > bounds.bottom() {
    y = sel.y - height - 8.0;
  }
  let y = y.clamp(bounds.y, (bounds.bottom() - height).max(bounds.y));

  for (index, button) in buttons.iter_mut().enumerate() {
    button.area = Rect::new(
      x + PANEL_PAD + index as f32 * (BUTTON + TOOL_GAP),
      y + PANEL_PAD,
      BUTTON,
      BUTTON,
    );
  }
}

pub struct Scene<'a> {
  pub frame: &'a Pixmap,
  pub backdrop: &'a Pixmap,
  pub bounds: Rect,
  pub selection: Option<Rect>,
  pub ants_phase: f32,
  pub shapes: &'a [Shape],
  pub draft: Option<&'a Shape>,
  pub typing: Option<(Point, &'a str)>,
  pub palette_index: usize,
  pub chrome: &'a Chrome,
  pub hotspot: Option<Hotspot>,
  pub text: &'a TextEngine,
}

pub fn paint(pm: &mut Pixmap, scene: &Scene) {
  pm.data_mut().copy_from_slice(scene.backdrop.data());
  draw_annotations(pm, scene);
  if let Some(sel) = scene.selection {
    draw_border(pm, sel, scene.ants_phase);
    draw_handles(pm, sel);
    draw_badge(pm, sel, scene.bounds, scene.text);
    if let Some((at, buffer)) = scene.typing {
      let caret_x = at.x + scene.text.width(buffer, LABEL_SIZE);
      draw::polyline(
        pm,
        &[
          Point::new(caret_x, at.y),
          Point::new(caret_x, at.y + LABEL_SIZE),
        ],
        [255, 255, 255],
        1.5,
        255,
      );
    }
    draw_panels(pm, scene);
  }
}

pub fn dimmed_copy(frame: &Pixmap) -> Pixmap {
  let mut pm = frame.clone();
  let keep = u32::from(255 - DIM_ALPHA);
  for pixel in pm.data_mut().as_chunks_mut::<4>().0 {
    for channel in &mut pixel[..3] {
      *channel = ((u32::from(*channel) * keep + 127) / 255) as u8;
    }
  }
  pm
}

fn annotated_layer(
  source: &Pixmap,
  sel: Rect,
  shapes: &[Shape],
  draft: Option<&Shape>,
  typing: Option<(Point, &str, [u8; 3])>,
  engine: &TextEngine,
) -> Option<(Pixmap, Point)> {
  let px_w = source.width() as i32;
  let px_h = source.height() as i32;
  if px_w == 0 || px_h == 0 {
    return None;
  }
  let (x0, y0, width, height) = region_pixels(sel, px_w, px_h);
  let mut layer = Pixmap::new(width as u32, height as u32)?;
  copy_region(&mut layer, source.data(), px_w as usize, x0, y0);
  let origin = Point::new(x0 as f32, y0 as f32);
  for shape in shapes {
    draw_shape(&mut layer, shape, origin, engine);
  }
  if let Some(draft) = draft {
    draw_shape(&mut layer, draft, origin, engine);
  }
  if let Some((at, buffer, ink)) = typing {
    engine.draw(
      &mut layer,
      buffer,
      at.x - origin.x,
      at.y - origin.y,
      LABEL_SIZE,
      ink,
    );
  }
  Some((layer, origin))
}

fn draw_annotations(pm: &mut Pixmap, scene: &Scene) {
  let Some(sel) = scene.selection else {
    return;
  };
  let ink = PALETTE[scene.palette_index % PALETTE.len()];
  let typing = scene.typing.map(|(at, buffer)| (at, buffer, ink));
  let Some((layer, origin)) = annotated_layer(
    scene.frame,
    sel,
    scene.shapes,
    scene.draft,
    typing,
    scene.text,
  ) else {
    return;
  };
  let stride = layer.width() as usize * 4;
  let canvas_w = pm.width() as usize;
  let source = layer.data();
  let target = pm.data_mut();
  for row in 0..layer.height() as usize {
    let from = row * stride;
    let to = ((origin.y as usize + row) * canvas_w + origin.x as usize) * 4;
    target[to..to + stride].copy_from_slice(&source[from..from + stride]);
  }
}

fn copy_region(
  layer: &mut Pixmap,
  source: &[u8],
  source_w: usize,
  x0: i32,
  y0: i32,
) {
  let width = layer.width() as usize;
  let height = layer.height() as usize;
  let stride = width * 4;
  let target = layer.data_mut();
  for row in 0..height {
    let from = ((y0 as usize + row) * source_w + x0 as usize) * 4;
    target[row * stride..][..stride]
      .copy_from_slice(&source[from..from + stride]);
  }
}

fn region_pixels(sel: Rect, px_w: i32, px_h: i32) -> (i32, i32, i32, i32) {
  let x0 = sel.x.floor().max(0.0) as i32;
  let y0 = sel.y.floor().max(0.0) as i32;
  let width = ((sel.right().ceil() as i32) - x0).clamp(1, px_w - x0);
  let height = ((sel.bottom().ceil() as i32) - y0).clamp(1, px_h - y0);
  (x0, y0, width, height)
}

pub fn flatten(
  frame: &Pixmap,
  sel: Rect,
  shapes: &[Shape],
  text: &TextEngine,
) -> Shot {
  let Some((layer, _)) = annotated_layer(frame, sel, shapes, None, None, text)
  else {
    return Shot::empty();
  };
  Shot {
    width: layer.width(),
    height: layer.height(),
    rgba: layer.data().to_vec(),
  }
}

fn draw_shape(
  pm: &mut Pixmap,
  shape: &Shape,
  origin: Point,
  engine: &TextEngine,
) {
  let rel = |p: &Point| Point::new(p.x - origin.x, p.y - origin.y);
  match shape {
    Shape::Freehand {
      points,
      color,
      width,
    } => {
      let mapped: Vec<Point> = points.iter().map(rel).collect();
      draw::polyline(pm, &mapped, *color, *width, 255);
    }
    Shape::Marker { points, color } => {
      let mapped: Vec<Point> = points.iter().map(rel).collect();
      draw::polyline(pm, &mapped, *color, MARKER_WIDTH, MARKER_ALPHA);
    }
    Shape::Segment {
      from,
      to,
      color,
      width,
    } => {
      draw::polyline(pm, &[rel(from), rel(to)], *color, *width, 255);
    }
    Shape::Arrow {
      tail,
      head,
      color,
      width,
    } => {
      let (start, end) = (rel(tail), rel(head));
      draw::polyline(pm, &[start, end], *color, *width, 255);
      draw::arrow_head(pm, start, end, (*width * 3.5).max(6.0), *color, 255);
    }
    Shape::Outline { rect, color, width } => {
      let shifted = rect.translated(-origin.x, -origin.y);
      draw::rect_stroke(pm, shifted, *color, *width, 255);
    }
    Shape::Caption { at, text, color } => {
      engine.draw(
        pm,
        text,
        at.x - origin.x,
        at.y - origin.y,
        LABEL_SIZE,
        *color,
      );
    }
  }
}

fn draw_border(pm: &mut Pixmap, sel: Rect, phase: f32) {
  draw::dashed_rect(pm, sel, [255, 255, 255], 1.0, phase);
}

fn draw_handles(pm: &mut Pixmap, sel: Rect) {
  for &handle in &HANDLES {
    let anchor = handle_anchor(sel, handle);
    let square = Rect::new(anchor.x - 3.0, anchor.y - 3.0, 6.0, 6.0);
    draw::rect_fill(pm, square, [255, 255, 255], 255);
    draw::rect_stroke(pm, square, [20, 20, 20], 1.0, 255);
  }
}

fn draw_badge(pm: &mut Pixmap, sel: Rect, bounds: Rect, engine: &TextEngine) {
  let label = format!("{}x{}", sel.w.round() as i64, sel.h.round() as i64);
  let text_width = engine.width(&label, BADGE_TEXT);
  let pad = 6.0;
  let box_w = text_width + pad * 2.0;
  let box_h = BADGE_TEXT + 7.0;

  let mut bx = sel.x;
  let mut by = sel.y - box_h - 3.0;
  if by < bounds.y {
    by = sel.y + 3.0;
  }
  bx = bx.clamp(bounds.x, (bounds.right() - box_w).max(bounds.x));

  draw::rounded_fill(
    pm,
    Rect::new(bx, by, box_w, box_h),
    4.0,
    [10, 10, 10],
    210,
  );
  engine.draw(pm, &label, bx + pad, by + 3.5, BADGE_TEXT, [255, 255, 255]);
}

fn draw_panels(pm: &mut Pixmap, scene: &Scene) {
  let swatch = PALETTE[scene.palette_index % PALETTE.len()];
  for (index, button) in scene.chrome.tools.iter().enumerate() {
    let hovered = scene.hotspot == Some(Hotspot::Tool(index));
    draw_button(pm, button, hovered, swatch);
  }
  for (index, button) in scene.chrome.actions.iter().enumerate() {
    let hovered = scene.hotspot == Some(Hotspot::Action(index));
    draw_button(pm, button, hovered, swatch);
  }
}

fn draw_button(
  pm: &mut Pixmap,
  button: &Button,
  hovered: bool,
  swatch: [u8; 3],
) {
  if button.area.w <= 0.0 || button.area.h <= 0.0 {
    return;
  }
  let bg_alpha = if hovered { 225 } else { 175 };
  draw::rounded_fill(pm, button.area, 5.0, [12, 12, 12], bg_alpha);
  if button.active {
    draw::rounded_stroke(
      pm,
      button.area.inflated(1.5),
      6.0,
      [255, 255, 255],
      1.5,
      220,
    );
  }
  let ink = if button.enabled {
    [240, 240, 240]
  } else {
    [120, 120, 120]
  };
  if button.command == Command::NextColor {
    let inner = button.area.inflated(-7.0);
    draw::rounded_fill(pm, inner, 3.0, swatch, 255);
  } else {
    button.icon.paint(pm, button.area.center(), ICON_BOX, ink);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geom::Rect;

  #[test]
  fn dimmed_copy_darkens_rgb_and_keeps_alpha() {
    let mut pm = Pixmap::new(2, 1).unwrap();
    pm.data_mut()
      .copy_from_slice(&[200, 100, 50, 255, 10, 20, 30, 128]);
    let dimmed = dimmed_copy(&pm);
    let px = dimmed.data();
    assert_eq!(px[0], ((200 * 150 + 127) / 255) as u8);
    assert_eq!(px[3], 255);
    assert_eq!(px[7], 128);
  }

  #[test]
  fn hotspot_at_finds_button_under_point() {
    let mut chrome = Chrome {
      tools: vec![],
      actions: vec![],
    };
    let mut b = button(Command::Undo, draw::Icon::Undo, true, false);
    b.area = Rect::new(10.0, 10.0, 30.0, 30.0);
    chrome.actions.push(b);
    assert_eq!(
      hotspot_at(&chrome, Point::new(20.0, 20.0)),
      Some(Hotspot::Action(0))
    );
    assert_eq!(hotspot_at(&chrome, Point::new(0.0, 0.0)), None);
  }

  #[test]
  fn build_hides_buttons_without_selection() {
    let chrome = build(
      None,
      Rect::new(0.0, 0.0, 1920.0, 1080.0),
      Tool::Pen,
      &History::default(),
    );
    assert!(chrome.tools.iter().all(|b| b.area.w == 0.0));
    assert!(chrome.actions.iter().all(|b| b.area.w == 0.0));
  }

  #[test]
  fn build_shows_deliver_buttons_when_selection_is_ready() {
    let sel = Rect::new(10.0, 10.0, 200.0, 150.0);
    let chrome = build(
      Some(sel),
      Rect::new(0.0, 0.0, 1920.0, 1080.0),
      Tool::Pen,
      &History::default(),
    );
    assert!(chrome.tools.iter().all(|b| b.area.w > 0.0));
    assert!(chrome.actions.iter().all(|b| b.area.w > 0.0));
    let close = chrome
      .actions
      .iter()
      .find(|b| b.command == Command::Deliver(Deliverable::Close));
    assert!(close.is_some_and(|b| b.enabled));
  }

  #[test]
  fn deliverable_region_requires_minimum_size() {
    assert!(
      deliverable_region(Some(Rect::new(0.0, 0.0, 100.0, 100.0))).is_some()
    );
    assert!(deliverable_region(Some(Rect::new(0.0, 0.0, 3.0, 3.0))).is_none());
    assert!(deliverable_region(None).is_none());
  }

  #[test]
  fn region_pixels_clamps_to_source_bounds() {
    let sel = Rect::new(-5.0, -5.0, 100.0, 100.0);
    let (x0, y0, w, h) = region_pixels(sel, 50, 40);
    assert_eq!((x0, y0, w, h), (0, 0, 50, 40));
  }

  #[test]
  fn flatten_crops_region_and_keeps_rgba_stride() {
    let Ok(engine) = TextEngine::load() else {
      return;
    };
    let mut frame = Pixmap::new(100, 100).unwrap();
    frame.data_mut().iter_mut().for_each(|p| *p = 255);
    let sel = Rect::new(10.0, 10.0, 20.0, 30.0);
    let shot = flatten(&frame, sel, &[], &engine);
    assert_eq!((shot.width, shot.height), (20, 30));
    assert_eq!(shot.rgba.len(), 20 * 30 * 4);
  }

  #[test]
  fn flatten_draws_committed_shapes_into_the_region() {
    let Ok(engine) = TextEngine::load() else {
      return;
    };
    let mut frame = Pixmap::new(100, 100).unwrap();
    frame.data_mut().iter_mut().for_each(|p| *p = 100);
    let sel = Rect::new(10.0, 10.0, 20.0, 30.0);
    let stroke = Shape::Segment {
      from: Point::new(15.0, 15.0),
      to: Point::new(25.0, 35.0),
      color: [239, 68, 68],
      width: 2.5,
    };
    let shot = flatten(&frame, sel, &[stroke], &engine);
    let painted = shot
      .rgba
      .as_chunks::<4>()
      .0
      .iter()
      .any(|p| p[0] == 239 && p[1] == 68 && p[2] == 68);
    assert!(painted, "flattened region should contain the drawn stroke");
  }
}
