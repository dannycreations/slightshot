use tiny_skia::Pixmap;

use crate::{
  action::{Deliverable, Shot},
  annotate::{active_color, stroke_alpha, History, Segment, Shape, Tool},
  draw,
  geom::{clamp_span, handle_anchor, Point, Rect, HANDLES},
  text::TextEngine,
};

const MIN_REGION: f32 = 6.0;
const DIM_ALPHA: u8 = 105;
const BUTTON: f32 = 30.0;
const TOOL_GAP: f32 = 2.0;
const PANEL_PAD: f32 = 5.0;
const COLUMN_GAP: f32 = 6.0;
const ROW_GAP: f32 = 8.0;
const SCREEN_MARGIN: f32 = 4.0;
const BADGE_TEXT: f32 = 18.0;
const BADGE_GAP: f32 = 5.0;
const ICON_BOX: f32 = 18.0;
const TOOLTIP_TEXT: f32 = 14.0;
const TOOLTIP_PAD: f32 = 5.0;
const TOOLTIP_GAP: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backdrop {
  Frozen,
  Live,
}

impl Backdrop {
  pub fn picks_region(self) -> bool {
    matches!(self, Backdrop::Frozen)
  }
}

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

impl Button {
  fn shown(&self) -> bool {
    self.area.w > 0.0 && self.area.h > 0.0
  }
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

impl Chrome {
  pub fn new(backdrop: Backdrop) -> Self {
    let mut tools = TOOL_BUTTONS.to_vec();
    if !backdrop.picks_region() {
      tools.push(button(Command::Close, draw::Icon::Close));
    }
    Self {
      tools,
      actions: ACTION_BUTTONS.to_vec(),
    }
  }

  fn hovered(&self, hotspot: Option<Hotspot>) -> Option<(&Button, Side)> {
    match hotspot {
      Some(Hotspot::Tool(i)) => self.tools.get(i).map(|b| (b, Side::Left)),
      Some(Hotspot::Action(i)) => self.actions.get(i).map(|b| (b, Side::Above)),
      None => None,
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
  backdrop: Backdrop,
) {
  let live = !backdrop.picks_region();
  let ready =
    backdrop.picks_region() && deliverable_region(selection).is_some();
  let can_undo = history.can_undo();

  for b in chrome.tools.iter_mut().chain(&mut chrome.actions) {
    b.enabled = b.command.enabled(ready, can_undo);
    b.active = matches!(b.command, Command::Tool(active) if active == tool);
    b.area = Rect::ZERO;
  }

  let Some(sel) = selection.filter(|_| show_chrome) else {
    return;
  };
  layout_panel(&mut chrome.tools, sel, bounds, Panel::Column { live });
  if !live {
    layout_panel(&mut chrome.actions, sel, bounds, Panel::Row);
  }
}

fn under(buttons: &[Button], p: Point) -> Option<usize> {
  buttons.iter().position(|b| b.shown() && b.area.contains(p))
}

pub fn hotspot_at(chrome: &Chrome, p: Point) -> Option<Hotspot> {
  under(&chrome.tools, p)
    .map(Hotspot::Tool)
    .or_else(|| under(&chrome.actions, p).map(Hotspot::Action))
}

#[derive(Clone, Copy)]
enum Panel {
  Column { live: bool },
  Row,
}

fn layout_panel(buttons: &mut [Button], sel: Rect, bounds: Rect, panel: Panel) {
  let count = buttons.len() as f32;
  let long = count * BUTTON + (count - 1.0) * TOOL_GAP + PANEL_PAD * 2.0;
  let short = BUTTON + PANEL_PAD * 2.0;

  let Panel::Column { live } = panel else {
    let (width, height) = (long, short);
    let x = (sel.right() - width).max(bounds.x);
    let mut y = sel.bottom() + ROW_GAP;
    if y + height > bounds.bottom() {
      y = sel.y - height - ROW_GAP;
    }
    let y = clamp_span(y, height, bounds.y, bounds.bottom());
    lay_out_buttons(buttons, (x, y), Panel::Row);
    return;
  };

  let (width, height) = (short, long);
  let mut x = sel.right() + COLUMN_GAP;
  if x + width > bounds.right() {
    x = sel.right() - width - COLUMN_GAP;
  }
  let x = clamp_span(x, width, bounds.x, bounds.right());
  let y = if live {
    clamp_span(
      sel.center().y - height * 0.5,
      height,
      bounds.y + SCREEN_MARGIN,
      bounds.bottom() - SCREEN_MARGIN,
    )
  } else {
    (sel.bottom() - height).clamp(
      bounds.y + SCREEN_MARGIN,
      (bounds.bottom() - height - SCREEN_MARGIN).max(bounds.y),
    )
  };
  lay_out_buttons(buttons, (x, y), Panel::Column { live });
}

fn lay_out_buttons(buttons: &mut [Button], at: (f32, f32), panel: Panel) {
  let step = BUTTON + TOOL_GAP;
  let (x, y) = at;
  for (index, button) in buttons.iter_mut().enumerate() {
    let offset = index as f32 * step;
    let (dx, dy) = match panel {
      Panel::Column { .. } => (0.0, offset),
      Panel::Row => (offset, 0.0),
    };
    button.area =
      Rect::new(x + PANEL_PAD + dx, y + PANEL_PAD + dy, BUTTON, BUTTON);
  }
}

pub struct Scene<'a> {
  pub backdrop: &'a Pixmap,
  pub canvas: &'a Pixmap,
  pub bounds: Rect,
  pub selection: Option<Rect>,
  pub kind: Backdrop,
  pub draft: Option<&'a Shape>,
  pub typing: Option<(Point, &'a str, f32)>,
  pub palette_index: usize,
  pub chrome: &'a Chrome,
  pub hotspot: Option<Hotspot>,
  pub text: &'a TextEngine,
  pub hint: Option<&'a str>,
}

pub fn paint(pm: &mut Pixmap, scene: &Scene) {
  pm.data_mut().copy_from_slice(scene.backdrop.data());
  let Some(sel) = scene.selection else {
    draw_live(pm, scene);
    return;
  };
  let (x0, y0, width, height) = region_pixels(sel, pm.width(), pm.height());
  if width == 0 || height == 0 {
    return;
  }
  let at = (x0, y0);
  copy_region(pm, scene.canvas, at, at, (width, height));
  draw_live(pm, scene);
  // An area the user picked can be adjusted, so it gets the outline and the
  // handles that say so. A live overlay's area is the window itself and cannot
  // be adjusted, so those would only promise something that is not there.
  if scene.kind.picks_region() {
    draw::dashed_rect(pm, sel, [255, 255, 255]);
    draw_handles(pm, sel);
    draw_badge(pm, sel, scene.bounds, scene.text);
  }
  if let Some((at, buffer, size)) = scene.typing {
    let caret_x = at.x + scene.text.width(buffer, size);
    draw::polyline(
      pm,
      &[Point::new(caret_x, at.y), Point::new(caret_x, at.y + size)],
      [255, 255, 255],
      1.5,
      255,
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

#[inline(always)]
fn dim_pixel(dst: &mut [u8], src: &[u8]) {
  dst[0] = DIM_LUT[src[0] as usize];
  dst[1] = DIM_LUT[src[1] as usize];
  dst[2] = DIM_LUT[src[2] as usize];
  dst[3] = src[3];
}

pub fn dimmed_into(dst: &mut Pixmap, src: &Pixmap) {
  let width = src.width().min(dst.width());
  let height = src.height().min(dst.height());
  let whole = Rect::new(0.0, 0.0, width as f32, height as f32);
  dim_region_into(dst, src, whole);
}

fn dim_region_into(dst: &mut Pixmap, src: &Pixmap, rect: Rect) {
  let (x0, y0, width, height) = region_pixels(
    rect,
    src.width().min(dst.width()),
    src.height().min(dst.height()),
  );
  if width == 0 || height == 0 {
    return;
  }
  let row_bytes = width as usize * 4;
  let (src_stride, dst_stride) =
    (src.width() as usize * 4, dst.width() as usize * 4);
  let (src_data, dst_data) = (src.data(), dst.data_mut());
  for row in 0..height as usize {
    let y = y0 as usize + row;
    let from = y * src_stride + x0 as usize * 4;
    let to = y * dst_stride + x0 as usize * 4;
    let (src_pixels, dst_pixels) = (
      &src_data[from..from + row_bytes],
      &mut dst_data[to..to + row_bytes],
    );
    for (src_px, dst_px) in src_pixels
      .as_chunks::<4>()
      .0
      .iter()
      .zip(dst_pixels.as_chunks_mut::<4>().0)
    {
      dim_pixel(dst_px, src_px);
    }
  }
}

pub(crate) fn ink_segment(
  canvas: &mut Pixmap,
  backdrop: &mut Pixmap,
  segment: Segment,
) {
  draw::polyline(
    canvas,
    &[segment.from, segment.to],
    segment.color,
    segment.width,
    segment.alpha,
  );
  dim_region_into(backdrop, canvas, segment.bounds());
}

fn copy_region(
  dest: &mut Pixmap,
  source: &Pixmap,
  from: (u32, u32),
  to: (u32, u32),
  size: (u32, u32),
) {
  let (from_x, from_y) = from;
  let (to_x, to_y) = to;
  let (width, height) = size;
  let row_bytes = width as usize * 4;
  let src_stride = source.width() as usize * 4;
  let dst_stride = dest.width() as usize * 4;
  let (src, dst) = (source.data(), dest.data_mut());
  let src_row = from_y as usize * src_stride + from_x as usize * 4;
  let dst_row = to_y as usize * dst_stride + to_x as usize * 4;

  if from_x == 0
    && to_x == 0
    && row_bytes == src_stride
    && row_bytes == dst_stride
  {
    let total = row_bytes * height as usize;
    dst[dst_row..dst_row + total]
      .copy_from_slice(&src[src_row..src_row + total]);
    return;
  }

  for row in 0..height as usize {
    let s = src_row + row * src_stride;
    let d = dst_row + row * dst_stride;
    dst[d..d + row_bytes].copy_from_slice(&src[s..s + row_bytes]);
  }
}

fn draw_live(pm: &mut Pixmap, scene: &Scene) {
  if let Some(draft) = scene.draft {
    ink(pm, draft, scene.text);
  }
  if let Some((at, buffer, size)) = scene.typing {
    let color = active_color(scene.palette_index);
    scene.text.draw(pm, buffer, at.x, at.y, size, color);
  }
}

#[inline(always)]
fn region_pixels(sel: Rect, px_w: u32, px_h: u32) -> (u32, u32, u32, u32) {
  let x0 = (sel.x.floor() as u32).min(px_w);
  let y0 = (sel.y.floor() as u32).min(px_h);
  let width = ((sel.right().ceil() as u32).saturating_sub(x0)).min(px_w - x0);
  let height = ((sel.bottom().ceil() as u32).saturating_sub(y0)).min(px_h - y0);
  (x0, y0, width, height)
}

pub fn flatten(frame: &Pixmap, sel: Rect) -> Shot {
  let (x0, y0, width, height) =
    region_pixels(sel, frame.width(), frame.height());
  if width == 0 || height == 0 {
    return Shot::empty();
  }
  let Some(mut layer) = Pixmap::new(width, height) else {
    return Shot::empty();
  };
  copy_region(&mut layer, frame, (x0, y0), (0, 0), (width, height));
  Shot {
    width,
    height,
    rgba: layer.take(),
  }
}

pub(crate) fn ink(pm: &mut Pixmap, shape: &Shape, engine: &TextEngine) {
  match shape {
    Shape::Stroke {
      points,
      color,
      width,
      marker,
    } => {
      draw::polyline(pm, points, *color, *width, stroke_alpha(*marker));
    }
    Shape::Line {
      from,
      to,
      color,
      width,
      arrow,
    } => {
      // An arrow stops its shaft where the head begins, so the two meet
      // without the head overlapping the line. A plain line runs the full
      // distance, which is that same shaft with nothing pulled back.
      let size = if *arrow { (*width * 3.5).max(6.0) } else { 0.0 };
      let shaft_end = arrow_base(*from, *to, size);
      draw::polyline(pm, &[*from, shaft_end], *color, *width, 255);
      if *arrow {
        draw::arrow_head(pm, *from, *to, size, *color, 255);
      }
    }
    Shape::Outline { rect, color, width } => {
      draw::rect_stroke(pm, *rect, *color, *width, 255);
    }
    Shape::Caption {
      at,
      text,
      color,
      size,
    } => {
      engine.draw(pm, text, at.x, at.y, *size, *color);
    }
  }
}

fn arrow_base(from: Point, to: Point, head_size: f32) -> Point {
  let (dx, dy) = (to.x - from.x, to.y - from.y);
  let len = dx.hypot(dy);
  if len <= 0.0 {
    return to;
  }
  let back = head_size.min(len) / len;
  Point::new(to.x - dx * back, to.y - dy * back)
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

  if let Some((button, side)) = scene.chrome.hovered(scene.hotspot) {
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
  if !button.shown() {
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
    history.push(Shape::Caption {
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
      kind: Backdrop::Frozen,
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
