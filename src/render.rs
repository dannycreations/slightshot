use tiny_skia::Pixmap;

use crate::{
  action::{Deliverable, Shot},
  annotate::{active_color, History, Shape, Tool, MARKER_ALPHA},
  draw,
  geom::{clamp_span, handle_anchor, Point, Rect, HANDLES},
  text::TextEngine,
};

pub const HANDLE_SLOP: f32 = 7.0;

const MIN_REGION: f32 = 6.0;
const DIM_ALPHA: u8 = 105;
const BUTTON: f32 = 30.0;
const TOOL_GAP: f32 = 2.0;
const PANEL_PAD: f32 = 5.0;
const BADGE_TEXT: f32 = 18.0;
const BADGE_GAP: f32 = 5.0;
const ICON_BOX: f32 = 18.0;
const TOOLTIP_TEXT: f32 = 14.0;
const TOOLTIP_PAD: f32 = 5.0;
const TOOLTIP_GAP: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
  Tool(Tool),
  NextColor,
  Undo,
  Close,
  Deliver(Deliverable),
}

impl Command {
  pub fn label(self) -> &'static str {
    match self {
      Command::Tool(tool) => tool.label(),
      Command::NextColor => "Next color",
      Command::Undo => "Undo",
      Command::Close => "Close",
      Command::Deliver(deliverable) => deliverable.label(),
    }
  }

  /// Whether a button carrying this command accepts a click right now.
  fn enabled(self, ready: bool, can_undo: bool) -> bool {
    match self {
      Command::Tool(_) | Command::NextColor | Command::Close => true,
      Command::Undo => can_undo,
      Command::Deliver(_) => ready,
    }
  }
}

#[derive(Clone, Copy, Debug)]
pub struct Button {
  pub command: Command,
  pub icon: draw::Icon,
  pub area: Rect,
  pub enabled: bool,
  pub active: bool,
}

const fn button(command: Command, icon: draw::Icon) -> Button {
  Button {
    command,
    icon,
    area: Rect::ZERO,
    enabled: false,
    active: false,
  }
}

const TOOL_BUTTONS: [Button; 8] = [
  button(Command::Tool(Tool::Pen), draw::Icon::Pen),
  button(Command::Tool(Tool::Line), draw::Icon::Line),
  button(Command::Tool(Tool::Arrow), draw::Icon::Arrow),
  button(Command::Tool(Tool::Box), draw::Icon::Outline),
  button(Command::Tool(Tool::Marker), draw::Icon::Marker),
  button(Command::Tool(Tool::Label), draw::Icon::Letter),
  button(Command::NextColor, draw::Icon::Letter),
  button(Command::Undo, draw::Icon::Undo),
];

const ACTION_BUTTONS: [Button; 4] = [
  button(Command::Deliver(Deliverable::Upload), draw::Icon::Upload),
  button(Command::Deliver(Deliverable::Copy), draw::Icon::CopyImage),
  button(Command::Deliver(Deliverable::Save), draw::Icon::Save),
  button(Command::Close, draw::Icon::Close),
];

#[derive(Debug)]
pub struct Chrome {
  pub tools: Vec<Button>,
  pub actions: Vec<Button>,
}

impl Default for Chrome {
  fn default() -> Self {
    Self {
      tools: TOOL_BUTTONS.to_vec(),
      actions: ACTION_BUTTONS.to_vec(),
    }
  }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hotspot {
  Tool(usize),
  Action(usize),
}

#[inline(always)]
pub fn deliverable_region(selection: Option<Rect>) -> Option<Rect> {
  selection.filter(|sel| sel.w >= MIN_REGION && sel.h >= MIN_REGION)
}

pub fn build(
  chrome: &mut Chrome,
  selection: Option<Rect>,
  bounds: Rect,
  tool: Tool,
  history: &History,
  show_chrome: bool,
) {
  let ready = deliverable_region(selection).is_some();
  let can_undo = history.can_undo();

  for b in chrome.tools.iter_mut().chain(&mut chrome.actions) {
    b.enabled = b.command.enabled(ready, can_undo);
    if let Command::Tool(active) = b.command {
      b.active = active == tool;
    }
  }

  let Some(sel) = selection.filter(|_| show_chrome) else {
    for b in chrome.tools.iter_mut().chain(&mut chrome.actions) {
      b.area = Rect::ZERO;
    }
    return;
  };
  layout_panel(&mut chrome.tools, sel, bounds, Axis::Vertical);
  layout_panel(&mut chrome.actions, sel, bounds, Axis::Horizontal);
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

#[derive(Clone, Copy)]
enum Axis {
  Vertical,
  Horizontal,
}

fn layout_panel(buttons: &mut [Button], sel: Rect, bounds: Rect, axis: Axis) {
  let count = buttons.len() as f32;
  let step = BUTTON + TOOL_GAP;
  let main = count * BUTTON + (count - 1.0) * TOOL_GAP + PANEL_PAD * 2.0;
  let cross = BUTTON + PANEL_PAD * 2.0;
  let (width, height) = match axis {
    Axis::Vertical => (cross, main),
    Axis::Horizontal => (main, cross),
  };

  let (x, y) = match axis {
    Axis::Vertical => {
      let mut x = sel.right() + 6.0;
      if x + width > bounds.right() {
        x = sel.right() - width - 6.0;
      }
      x = clamp_span(x, width, bounds.x, bounds.right());
      let y = (sel.bottom() - height).clamp(
        bounds.y + 4.0,
        (bounds.bottom() - height - 4.0).max(bounds.y),
      );
      (x, y)
    }
    Axis::Horizontal => {
      let x = (sel.right() - width).max(bounds.x);
      let mut y = sel.bottom() + 8.0;
      if y + height > bounds.bottom() {
        y = sel.y - height - 8.0;
      }
      let y = clamp_span(y, height, bounds.y, bounds.bottom());
      (x, y)
    }
  };

  for (index, button) in buttons.iter_mut().enumerate() {
    let (dx, dy) = match axis {
      Axis::Vertical => (0.0, index as f32 * step),
      Axis::Horizontal => (index as f32 * step, 0.0),
    };
    button.area =
      Rect::new(x + PANEL_PAD + dx, y + PANEL_PAD + dy, BUTTON, BUTTON);
  }
}

pub struct Scene<'a> {
  pub inked_backdrop: &'a Pixmap,
  pub inked_canvas: &'a Pixmap,
  pub bounds: Rect,
  pub selection: Option<Rect>,
  pub draft: Option<&'a Shape>,
  pub typing: Option<(Point, &'a str, f32)>,
  pub palette_index: usize,
  pub chrome: &'a Chrome,
  pub hotspot: Option<Hotspot>,
  pub text: &'a TextEngine,
  pub hint: Option<&'a str>,
}

pub fn paint(pm: &mut Pixmap, scene: &Scene) {
  pm.data_mut().copy_from_slice(scene.inked_backdrop.data());
  let Some(sel) = scene.selection else {
    draw_live(pm, scene);
    return;
  };
  let (x0, y0, width, height) = region_pixels(sel, pm.width(), pm.height());
  if width == 0 || height == 0 {
    return;
  }
  let px_w = pm.width() as usize;
  blit(
    pm.data_mut(),
    px_w,
    x0 as usize,
    y0 as usize,
    scene.inked_canvas.data(),
    scene.inked_canvas.width() as usize,
    x0 as usize,
    y0 as usize,
    width as usize,
    height as usize,
  );
  draw_live(pm, scene);
  draw::dashed_rect(pm, sel, [255, 255, 255]);
  draw_handles(pm, sel);
  draw_badge(pm, sel, scene.bounds, scene.text);
  if let Some((at, buffer, size)) = scene.typing {
    let caret_x = at.x + scene.text.width(buffer, size);
    draw::polyline(
      pm,
      &[Point::new(caret_x, at.y), Point::new(caret_x, at.y + size)],
      [255, 255, 255],
      1.5,
      255,
      Point::new(0.0, 0.0),
    );
  }
  draw_panels(pm, scene);
}

const DIM_LUT: [u8; 256] = {
  let keep = 255 - DIM_ALPHA as u32;
  let mut lut = [0u8; 256];
  let mut i = 0;
  while i < 256 {
    lut[i] = ((i as u32 * keep + 127) / 255) as u8;
    i += 1;
  }
  lut
};

pub fn dimmed_into(dst: &mut Pixmap, src: &Pixmap) {
  let dst_chunks = dst.data_mut().as_chunks_mut::<4>().0;
  let src_chunks = src.data().as_chunks::<4>().0;
  for (dst_px, src_px) in dst_chunks.iter_mut().zip(src_chunks) {
    dst_px[0] = DIM_LUT[src_px[0] as usize];
    dst_px[1] = DIM_LUT[src_px[1] as usize];
    dst_px[2] = DIM_LUT[src_px[2] as usize];
    dst_px[3] = src_px[3];
  }
}

pub fn dimmed_copy(frame: &Pixmap) -> Pixmap {
  let mut out =
    Pixmap::new(frame.width(), frame.height()).expect("valid frame dimensions");
  dimmed_into(&mut out, frame);
  out
}

/// Copies a `width` by `height` pixel block. Both callers reject an empty
/// region before calling, so the block always covers at least one pixel.
#[allow(clippy::too_many_arguments)]
fn blit(
  dest: &mut [u8],
  dest_stride_px: usize,
  dest_x: usize,
  dest_y: usize,
  source: &[u8],
  source_stride_px: usize,
  source_x: usize,
  source_y: usize,
  width: usize,
  height: usize,
) {
  let row_bytes = width * 4;
  let src_stride_bytes = source_stride_px * 4;
  let dst_stride_bytes = dest_stride_px * 4;

  if row_bytes == src_stride_bytes
    && row_bytes == dst_stride_bytes
    && dest_x == 0
    && source_x == 0
  {
    let total = row_bytes * height;
    let dst_start = dest_y * dst_stride_bytes;
    let src_start = source_y * src_stride_bytes;
    dest[dst_start..dst_start + total]
      .copy_from_slice(&source[src_start..src_start + total]);
    return;
  }

  let mut src = (source_y * source_stride_px + source_x) * 4;
  let mut dst = (dest_y * dest_stride_px + dest_x) * 4;
  for _ in 0..height {
    dest[dst..dst + row_bytes].copy_from_slice(&source[src..src + row_bytes]);
    src += src_stride_bytes;
    dst += dst_stride_bytes;
  }
}

fn draw_live(pm: &mut Pixmap, scene: &Scene) {
  let origin = Point::new(0.0, 0.0);
  if let Some(draft) = scene.draft {
    ink(pm, draft, origin, scene.text);
  }
  if let Some((at, buffer, size)) = scene.typing {
    let color = active_color(scene.palette_index);
    scene.text.draw(pm, buffer, at.x, at.y, size, color);
  }
}

/// The pixel rectangle covering `sel`, clipped to a `px_w` by `px_h` image.
/// Every edge is clipped, because a drag can start or end outside the image. A
/// selection that falls outside reports a zero width or height. `f32 as u32`
/// saturates, so negative and NaN edges land on zero.
#[inline(always)]
fn region_pixels(sel: Rect, px_w: u32, px_h: u32) -> (u32, u32, u32, u32) {
  let x0 = (sel.x.floor() as u32).min(px_w);
  let y0 = (sel.y.floor() as u32).min(px_h);
  let width = ((sel.right().ceil() as u32).saturating_sub(x0)).min(px_w - x0);
  let height = ((sel.bottom().ceil() as u32).saturating_sub(y0)).min(px_h - y0);
  (x0, y0, width, height)
}

/// Crops `sel` out of `frame`. Annotations are expected to be already inked
/// into `frame`, which is what the overlay does before delivering a shot.
pub fn flatten(frame: &Pixmap, sel: Rect) -> Shot {
  let (x0, y0, width, height) =
    region_pixels(sel, frame.width(), frame.height());
  if width == 0 || height == 0 {
    return Shot::empty();
  }
  let Some(mut layer) = Pixmap::new(width, height) else {
    return Shot::empty();
  };
  blit(
    layer.data_mut(),
    width as usize,
    0,
    0,
    frame.data(),
    frame.width() as usize,
    x0 as usize,
    y0 as usize,
    width as usize,
    height as usize,
  );
  Shot {
    width,
    height,
    rgba: layer.take(),
  }
}

pub(crate) fn ink(
  pm: &mut Pixmap,
  shape: &Shape,
  origin: Point,
  engine: &TextEngine,
) {
  match shape {
    Shape::Stroke {
      points,
      color,
      width,
      marker,
    } => {
      let alpha = if *marker { MARKER_ALPHA } else { 255 };
      draw::polyline(pm, points, *color, *width, alpha, origin);
    }
    Shape::Line {
      from,
      to,
      color,
      width,
      arrow,
    } => {
      let start = Point::new(from.x - origin.x, from.y - origin.y);
      let end = Point::new(to.x - origin.x, to.y - origin.y);
      if *arrow {
        let size = (*width * 3.5).max(6.0);
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        let len = dx.hypot(dy);
        let base = if len > 0.0 {
          let k = size.min(len) / len;
          Point::new(end.x - dx * k, end.y - dy * k)
        } else {
          end
        };
        draw::polyline(
          pm,
          &[start, base],
          *color,
          *width,
          255,
          Point::new(0.0, 0.0),
        );
        draw::arrow_head(pm, start, end, size, *color, 255);
      } else {
        draw::polyline(
          pm,
          &[start, end],
          *color,
          *width,
          255,
          Point::new(0.0, 0.0),
        );
      }
    }
    Shape::Outline { rect, color, width } => {
      let shifted = rect.translated(-origin.x, -origin.y);
      draw::rect_stroke(pm, shifted, *color, *width, 255);
    }
    Shape::Caption {
      at,
      text,
      color,
      size,
    } => {
      engine.draw(pm, text, at.x - origin.x, at.y - origin.y, *size, *color);
    }
  }
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
  let mut by = sel.y - box_h - BADGE_GAP;
  if by < bounds.y {
    by = sel.y + BADGE_GAP;
  }
  bx = clamp_span(bx, box_w, bounds.x, bounds.right());

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
  let swatch = active_color(scene.palette_index);
  for (index, button) in scene.chrome.tools.iter().enumerate() {
    let hovered = scene.hotspot == Some(Hotspot::Tool(index));
    draw_button(pm, button, hovered, swatch);
  }
  for (index, button) in scene.chrome.actions.iter().enumerate() {
    let hovered = scene.hotspot == Some(Hotspot::Action(index));
    draw_button(pm, button, hovered, swatch);
  }

  let (hovered_btn, side) = match scene.hotspot {
    Some(Hotspot::Tool(i)) => (scene.chrome.tools.get(i), Side::Left),
    Some(Hotspot::Action(i)) => (scene.chrome.actions.get(i), Side::Above),
    None => (None, Side::Left),
  };
  if let Some(button) = hovered_btn {
    draw_tooltip(
      pm,
      button.area,
      button.command.label(),
      scene.bounds,
      scene.text,
      side,
    );
  }
  if let Some(text) = scene.hint {
    if let Some(button) = scene.chrome.tools.iter().find(|b| b.active) {
      draw_tooltip(pm, button.area, text, scene.bounds, scene.text, Side::Left);
    }
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

enum Side {
  Left,
  Above,
}

fn tooltip_rect(
  area: Rect,
  label: &str,
  bounds: Rect,
  engine: &TextEngine,
  side: Side,
) -> Rect {
  let w = engine.width(label, TOOLTIP_TEXT) + TOOLTIP_PAD * 2.0;
  let h = TOOLTIP_TEXT + TOOLTIP_PAD * 2.0;
  let (mut bx, by) = match side {
    Side::Left => {
      let x = if area.x - TOOLTIP_GAP - w >= bounds.x {
        area.x - TOOLTIP_GAP - w
      } else {
        area.right() + TOOLTIP_GAP
      };
      (x, area.center().y - h * 0.5)
    }
    Side::Above => {
      let y = if area.y - TOOLTIP_GAP - h >= bounds.y {
        area.y - TOOLTIP_GAP - h
      } else {
        area.bottom() + TOOLTIP_GAP
      };
      (area.center().x - w * 0.5, y)
    }
  };
  bx = clamp_span(bx, w, bounds.x, bounds.right());
  let by = clamp_span(by, h, bounds.y, bounds.bottom());
  Rect::new(bx, by, w, h)
}

fn draw_tooltip(
  pm: &mut Pixmap,
  area: Rect,
  label: &str,
  bounds: Rect,
  engine: &TextEngine,
  side: Side,
) {
  if label.is_empty() {
    return;
  }
  let rect = tooltip_rect(area, label, bounds, engine, side);
  draw::rounded_fill(pm, rect, 4.0, [10, 10, 10], 220);
  engine.draw(
    pm,
    label,
    rect.x + TOOLTIP_PAD,
    rect.y + TOOLTIP_PAD,
    TOOLTIP_TEXT,
    [240, 240, 240],
  );
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
  fn dimmed_into_overwrites_an_existing_buffer() {
    let mut src = Pixmap::new(2, 1).unwrap();
    src
      .data_mut()
      .copy_from_slice(&[200, 100, 50, 255, 10, 20, 30, 128]);
    let mut dst = Pixmap::new(2, 1).unwrap();
    dst.data_mut().iter_mut().for_each(|b| *b = 77);
    dimmed_into(&mut dst, &src);
    assert_eq!(dst.data(), dimmed_copy(&src).data());
  }

  #[test]
  fn hotspot_at_finds_button_under_point() {
    let mut chrome = Chrome::default();
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
    let mut chrome = Chrome::default();
    build(
      &mut chrome,
      Some(sel),
      bounds,
      Tool::Pen,
      &History::default(),
      true,
    );
    assert!(chrome.tools.iter().all(|b| b.area.w > 0.0));
    build(
      &mut chrome,
      None,
      bounds,
      Tool::Pen,
      &History::default(),
      true,
    );
    assert!(chrome.tools.iter().all(|b| b.area.w == 0.0));
    assert!(chrome.actions.iter().all(|b| b.area.w == 0.0));
  }

  #[test]
  fn build_shows_deliver_buttons_when_selection_is_ready() {
    let sel = Rect::new(10.0, 10.0, 200.0, 150.0);
    let mut chrome = Chrome::default();
    build(
      &mut chrome,
      Some(sel),
      Rect::new(0.0, 0.0, 1920.0, 1080.0),
      Tool::Pen,
      &History::default(),
      true,
    );
    assert!(chrome.tools.iter().all(|b| b.area.w > 0.0));
    assert!(chrome.actions.iter().all(|b| b.area.w > 0.0));
    let close = chrome.actions.iter().find(|b| b.command == Command::Close);
    assert!(close.is_some_and(|b| b.enabled));
  }

  #[test]
  fn build_hides_buttons_when_not_idle() {
    let sel = Rect::new(10.0, 10.0, 200.0, 150.0);
    let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
    let mut chrome = Chrome::default();
    build(
      &mut chrome,
      Some(sel),
      bounds,
      Tool::Pen,
      &History::default(),
      true,
    );
    build(
      &mut chrome,
      Some(sel),
      bounds,
      Tool::Pen,
      &History::default(),
      false,
    );
    assert!(chrome.tools.iter().all(|b| b.area.w == 0.0));
    assert!(chrome.actions.iter().all(|b| b.area.w == 0.0));
  }

  #[test]
  fn build_derives_button_state_from_the_session() {
    let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
    let ready = Rect::new(10.0, 10.0, 200.0, 150.0);
    let mut history = History::default();
    let mut chrome = Chrome::default();
    build(
      &mut chrome,
      Some(ready),
      bounds,
      Tool::Arrow,
      &history,
      true,
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

    history.push(Shape::Caption {
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
    );
    assert!(find(&chrome, Command::Undo).enabled);

    let tiny = Rect::new(10.0, 10.0, 1.0, 1.0);
    build(&mut chrome, Some(tiny), bounds, Tool::Arrow, &history, true);
    assert!(!find(&chrome, Command::Deliver(Deliverable::Copy)).enabled);
    assert!(find(&chrome, Command::Close).enabled);
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
    ink(&mut canvas, &stroke, Point::default(), &engine);

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
    ink(&mut canvas, &arrow, Point::default(), &engine);

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

  #[test]
  fn draft_is_stroked_at_absolute_coordinates() {
    let Ok(engine) = TextEngine::load() else {
      return;
    };
    let mut canvas = Pixmap::new(40, 80).unwrap();
    for px in canvas.data_mut().as_chunks_mut::<4>().0 {
      px.copy_from_slice(&[200, 200, 200, 255]);
    }
    let backdrop = dimmed_copy(&canvas);
    let bounds = Rect::new(0.0, 0.0, 40.0, 80.0);
    let sel = Rect::new(5.0, 5.0, 30.0, 70.0);
    let selection = Some(sel);
    let history = History::default();
    let mut chrome = Chrome::default();
    build(
      &mut chrome,
      selection,
      bounds,
      Tool::Select,
      &history,
      false,
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
      inked_backdrop: &backdrop,
      inked_canvas: &canvas,
      bounds,
      selection,
      draft: Some(&draft),
      typing: None,
      palette_index: 0,
      chrome: &chrome,
      hotspot: None,
      text: &engine,
      hint: None,
    };

    paint(&mut pm, &scene);

    let idx = (70 * 40 + 30) * 4;
    let px = &pm.data()[idx..idx + 4];
    assert!(
      px[0] > 150 && px[2] < 100,
      "draft should appear at (30, 70), got {px:?}"
    );
  }
}
