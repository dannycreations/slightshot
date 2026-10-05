use tiny_skia::Pixmap;

use crate::{
  action::{Deliverable, Shot},
  annotate::{active_color, stroke_alpha, History, Segment, Shape, Tool},
  draw,
  geom::{clamp_span, handle_anchor, Handle, Point, Rect, HANDLES},
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
const BADGE_PAD: f32 = 6.0;
const BADGE_H: f32 = BADGE_TEXT + 7.0;
const EDGE: f32 = 3.0;
const ICON_BOX: f32 = 18.0;
const TOOLTIP_TEXT: f32 = 14.0;
const TOOLTIP_PAD: f32 = 5.0;
const TOOLTIP_GAP: f32 = 6.0;
const CARET_WIDTH: f32 = 1.5;

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

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

  fn tooltips<'a>(
    &'a self,
    hotspot: Option<Hotspot>,
    hint: Option<&'a str>,
  ) -> impl Iterator<Item = (Rect, &'a str, Side)> {
    let hovered = self
      .hovered(hotspot)
      .map(|(button, side)| (button.area, button.command.label(), side));
    let raised = hint.and_then(|hint| {
      self
        .tools
        .iter()
        .find(|button| button.active)
        .map(|button| (button.area, hint, Side::Left))
    });
    hovered.into_iter().chain(raised)
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
  pub picked: Option<Rect>,
  pub kind: Backdrop,
  pub draft: Option<&'a Shape>,
  pub typing: Option<(Point, &'a str, f32, [u8; 3])>,
  pub caret: bool,
  pub palette_index: usize,
  pub chrome: &'a Chrome,
  pub hotspot: Option<Hotspot>,
  pub text: &'a TextEngine,
  pub hint: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
pub struct Shown {
  selection: Option<Rect>,
  picked: Option<Rect>,
  chrome: Chrome,
  hotspot: Option<Hotspot>,
  hint: Option<String>,
  palette: usize,
  draft: Option<Shape>,
  typing: Option<(Point, String, f32, [u8; 3])>,
}

impl Shown {
  pub fn capture(scene: &Scene) -> Self {
    Self {
      selection: scene.selection,
      picked: scene.picked,
      chrome: scene.chrome.clone(),
      hotspot: scene.hotspot,
      hint: scene.hint.map(str::to_string),
      palette: scene.palette_index,
      draft: scene.draft.cloned(),
      typing: scene
        .typing
        .map(|(at, buffer, size, color)| (at, buffer.to_string(), size, color)),
    }
  }

  pub fn settled_from(
    &self,
    next: &Shown,
    bounds: Rect,
    engine: &TextEngine,
  ) -> Rect {
    let mut area = Rect::ZERO;
    if self.selection != next.selection {
      area = area.union(region_area(self.selection, bounds, engine));
      area = area.union(region_area(next.selection, bounds, engine));
    }
    if self.picked != next.picked {
      area = area.union(picked_area(self.picked));
      area = area.union(picked_area(next.picked));
    }
    // A new colour repaints the panels as well, because the swatch on the
    // colour button is the one thing on them that takes it.
    let panels_moved = self.chrome != next.chrome
      || self.hotspot != next.hotspot
      || self.hint != next.hint
      || self.palette != next.palette;
    if panels_moved {
      area = area.union(self.panels_area(bounds, engine));
      area = area.union(next.panels_area(bounds, engine));
    }
    if self.draft != next.draft {
      area = area.union(shape_area(self.draft.as_ref(), engine));
      area = area.union(shape_area(next.draft.as_ref(), engine));
    }
    // The text being typed is the other thing that takes the active colour.
    if self.typing != next.typing || self.palette != next.palette {
      area = area.union(typed_area(self.typed(), engine));
      area = area.union(typed_area(next.typed(), engine));
    }
    area
  }

  fn panels_area(&self, bounds: Rect, engine: &TextEngine) -> Rect {
    panels_area(
      &self.chrome,
      self.hotspot,
      self.hint.as_deref(),
      bounds,
      engine,
    )
  }

  fn typed(&self) -> Option<(Point, &str, f32, [u8; 3])> {
    self
      .typing
      .as_ref()
      .map(|(at, buffer, size, color)| (*at, buffer.as_str(), *size, *color))
  }
}

fn region_area(sel: Option<Rect>, bounds: Rect, engine: &TextEngine) -> Rect {
  match sel {
    Some(sel) => sel
      .union(framed_area(sel))
      .union(badge_rect(sel, bounds, engine).inflated(EDGE)),
    None => Rect::ZERO,
  }
}

fn framed_area(sel: Rect) -> Rect {
  bounding(&outline_boxes(sel)).union(bounding(&handle_boxes(sel)))
}

fn picked_area(sel: Option<Rect>) -> Rect {
  sel.map(framed_area).unwrap_or(Rect::ZERO)
}

fn outline_boxes(sel: Rect) -> [Rect; 4] {
  let edge = sel.inflated(EDGE);
  [
    Rect::new(edge.x, edge.y, edge.w, EDGE),
    Rect::new(edge.x, edge.bottom() - EDGE, edge.w, EDGE),
    Rect::new(edge.x, edge.y, EDGE, edge.h),
    Rect::new(edge.right() - EDGE, edge.y, EDGE, edge.h),
  ]
}

fn handle_boxes(sel: Rect) -> [Rect; 8] {
  HANDLES.map(|handle| handle_square(sel, handle).inflated(EDGE))
}

fn panels_area(
  chrome: &Chrome,
  hotspot: Option<Hotspot>,
  hint: Option<&str>,
  bounds: Rect,
  engine: &TextEngine,
) -> Rect {
  let mut area = Rect::ZERO;
  for button in chrome.tools.iter().chain(&chrome.actions) {
    // A button that was never laid out sits on `Rect::ZERO`, which is how the
    // chrome says "no button here". Inflating that sentinel would turn it
    // into a live box at the origin, and the union would then stretch every
    // repaint back to the top left of the screen.
    if button.shown() {
      area = area.union(button.area.inflated(EDGE));
    }
  }
  for (at, label, side) in chrome.tooltips(hotspot, hint) {
    area = area.union(tooltip_rect(at, label, bounds, engine, side));
  }
  area
}

pub fn shape_area(shape: Option<&Shape>, engine: &TextEngine) -> Rect {
  match shape {
    Some(Shape::Stroke { points, width, .. }) => {
      let (first, last) = (
        points.first().copied().unwrap_or_default(),
        points.last().copied().unwrap_or_default(),
      );
      let corners =
        points
          .iter()
          .fold(Rect::spanning(first, last), |area, point| {
            Rect::spanning(
              Point::new(area.x.min(point.x), area.y.min(point.y)),
              Point::new(area.right().max(point.x), area.bottom().max(point.y)),
            )
          });
      corners.inflated(width * 0.5 + 1.0)
    }
    Some(Shape::Line {
      from, to, width, ..
    }) => Rect::spanning(*from, *to).inflated(width * 0.5 + 1.0),
    Some(Shape::Outline { rect, width, .. }) => {
      rect.inflated(width * 0.5 + 1.0)
    }
    Some(Shape::Text { at, text, size, .. }) => engine.inked(text, *at, *size),
    None => Rect::ZERO,
  }
}

pub fn repaint(
  canvas: &mut Pixmap,
  backdrop: &mut Pixmap,
  base: &Pixmap,
  shapes: &[Shape],
  area: Rect,
  engine: &TextEngine,
) -> Rect {
  let boxes: Vec<Rect> = shapes
    .iter()
    .map(|shape| shape_area(Some(shape), engine))
    .collect();
  let mut region = area;
  loop {
    let grown = shapes.iter().zip(&boxes).fold(
      region,
      |region, (shape, box_)| match box_.overlaps(region) && shape.blends() {
        true => region.union(*box_),
        false => region,
      },
    );
    if grown == region {
      break;
    }
    region = grown;
  }

  let (x, y, width, height) =
    region_pixels(region, canvas.width(), canvas.height());
  if width == 0 || height == 0 {
    return Rect::ZERO;
  }
  let region = Rect::new(x as f32, y as f32, width as f32, height as f32);
  copy_region(canvas, base, (x, y), (x, y), (width, height));
  for (shape, box_) in shapes.iter().zip(&boxes) {
    if box_.overlaps(region) {
      ink(canvas, shape, engine);
    }
  }
  dim_region_into(backdrop, canvas, region);
  region
}

pub fn typed_area(
  typing: Option<(Point, &str, f32, [u8; 3])>,
  engine: &TextEngine,
) -> Rect {
  match typing {
    Some((at, buffer, size, _)) => {
      let x = at.x + engine.width(buffer, size);
      // The caret is a round-capped stroke, so like every other stroke it
      // paints half its width past each end, the one above the anchor
      // included, plus the antialiased pixel outside that. A box that stops at
      // the anchor would leave every cap behind as the caret steps right.
      let caret =
        Rect::spanning(Point::new(x, at.y), Point::new(x, at.y + size))
          .inflated(CARET_WIDTH * 0.5 + 1.0);
      engine.inked(buffer, at, size).union(caret)
    }
    None => Rect::ZERO,
  }
}

fn touches(piece: &[Rect], area: Rect) -> bool {
  piece.iter().any(|part| part.overlaps(area))
}

fn bounding(piece: &[Rect]) -> Rect {
  piece
    .iter()
    .fold(Rect::ZERO, |area, part| area.union(*part))
}

fn inside(piece: &[Rect], area: Rect) -> bool {
  piece.iter().all(|part| area.contains_rect(*part))
}

pub fn paint(pm: &mut Pixmap, scene: &Scene, damage: Rect) {
  let engine = scene.text;
  let region = scene
    .kind
    .picks_region()
    .then_some(scene.selection)
    .flatten();
  let outline = region.map(outline_boxes).unwrap_or([Rect::ZERO; 4]);
  let handles = region.map(handle_boxes).unwrap_or([Rect::ZERO; 8]);
  let badge = [region
    .map(|sel| badge_rect(sel, scene.bounds, engine).inflated(EDGE))
    .unwrap_or(Rect::ZERO)];
  let draft = [shape_area(scene.draft, engine)];
  let typing = [typed_area(scene.typing, engine)];
  let picked = [picked_area(scene.picked)];
  let panels = [panels_area(
    scene.chrome,
    scene.hotspot,
    scene.hint,
    scene.bounds,
    engine,
  )];

  let mut area = damage;
  for piece in [
    &outline[..],
    &handles[..],
    &badge[..],
    &draft[..],
    &typing[..],
    &picked[..],
    &panels[..],
  ] {
    if touches(piece, area) {
      area = area.union(bounding(piece));
    }
  }

  let (x0, y0, width, height) = region_pixels(area, pm.width(), pm.height());
  if width == 0 || height == 0 {
    return;
  }
  let at = (x0, y0);
  copy_region(pm, scene.backdrop, at, at, (width, height));
  if let Some(sel) = scene.selection {
    let (sx, sy, sw, sh) = region_pixels(sel, pm.width(), pm.height());
    let left = x0.max(sx);
    let top = y0.max(sy);
    let right = (x0 + width).min(sx + sw);
    let bottom = (y0 + height).min(sy + sh);
    if right > left && bottom > top {
      let inner = (left, top);
      copy_region(pm, scene.canvas, inner, inner, (right - left, bottom - top));
    }
  }
  if let Some(sel) = region {
    if inside(&outline, area) {
      draw::dashed_rect(pm, sel, [255, 255, 255]);
    }
    if inside(&handles, area) {
      draw_handles(pm, sel);
    }
    if inside(&badge, area) {
      draw_badge(pm, sel, scene.bounds, engine);
    }
  }
  if inside(&draft, area) {
    if let Some(shape) = scene.draft {
      ink(pm, shape, engine);
    }
  }
  if inside(&typing, area) {
    if let Some((at, buffer, size, color)) = scene.typing {
      engine.draw(pm, buffer, at.x, at.y, size, color);
      // The caret is dark for the half of the blink it is not lit for, which
      // is the same as not painting it: the text underneath stays.
      if scene.caret {
        let caret_x = at.x + engine.width(buffer, size);
        draw::polyline(
          pm,
          &[Point::new(caret_x, at.y), Point::new(caret_x, at.y + size)],
          [255, 255, 255],
          CARET_WIDTH,
          255,
        );
      }
    }
  }
  if inside(&picked, area) {
    if let Some(box_) = scene.picked {
      draw::dashed_rect(pm, box_, [255, 255, 255]);
      draw_handles(pm, box_);
    }
  }
  if inside(&panels, area) {
    draw_panels(pm, scene);
  }
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
    Shape::Text {
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
    let square = handle_square(sel, handle);
    draw::rect_fill(pm, square, [255, 255, 255], 255);
    draw::rect_stroke(pm, square, [20, 20, 20], 1.0, 255);
  }
}

fn handle_square(sel: Rect, handle: Handle) -> Rect {
  let anchor = handle_anchor(sel, handle);
  Rect::new(anchor.x - 3.0, anchor.y - 3.0, 6.0, 6.0)
}

fn badge_label(sel: Rect) -> String {
  format!("{}x{}", sel.w.round() as i64, sel.h.round() as i64)
}

fn badge_rect(sel: Rect, bounds: Rect, engine: &TextEngine) -> Rect {
  let box_w = engine.width(&badge_label(sel), BADGE_TEXT) + BADGE_PAD * 2.0;
  let mut bx = sel.x;
  let mut by = sel.y - BADGE_H - BADGE_GAP;
  if by < bounds.y {
    by = sel.y + BADGE_GAP;
  }
  bx = clamp_span(bx, box_w, bounds.x, bounds.right());
  Rect::new(bx, by, box_w, BADGE_H)
}

fn draw_badge(pm: &mut Pixmap, sel: Rect, bounds: Rect, engine: &TextEngine) {
  let label = badge_label(sel);
  let plate = badge_rect(sel, bounds, engine);
  draw::rounded_fill(pm, plate, 4.0, [10, 10, 10], 210);
  engine.draw(
    pm,
    &label,
    plate.x + BADGE_PAD,
    plate.y + 3.5,
    BADGE_TEXT,
    [255, 255, 255],
  );
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

  for (at, label, side) in scene.chrome.tooltips(scene.hotspot, scene.hint) {
    draw_tooltip(pm, at, label, scene.bounds, scene.text, side);
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
    // The active colour is the palette's first entry, so a preview drawn in
    // the wrong one is easy to tell from the right one.
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
  fn a_picked_box_that_moves_damages_both_places_it_was() {
    let Ok(engine) = TextEngine::load() else {
      return;
    };
    let canvas = Pixmap::new(600, 600).unwrap();
    let backdrop = dimmed(&canvas);
    let bounds = Rect::new(0.0, 0.0, 600.0, 600.0);
    let sel = Rect::new(20.0, 20.0, 80.0, 80.0);
    let mut chrome = Chrome::new(Backdrop::Frozen);
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

  fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
    left.iter().zip(right).position(|(a, b)| a != b)
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
    let mut old_chrome = Chrome::new(Backdrop::Frozen);
    build(
      &mut old_chrome,
      Some(from),
      bounds,
      Tool::Pen,
      &history,
      true,
      Backdrop::Frozen,
    );
    let mut new_chrome = Chrome::new(Backdrop::Frozen);
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

  #[test]
  fn an_unlaid_out_panel_damages_nothing() {
    // `Rect::ZERO` is how the chrome says "no button here". Inflating that
    // sentinel would make it a live box at the origin, and every union with a
    // real region would then reach back across the screen to the corner.
    let Ok(engine) = TextEngine::load() else {
      return;
    };
    let chrome = Chrome::new(Backdrop::Live);
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
}
