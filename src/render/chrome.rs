use tiny_skia::Pixmap;

use super::{deliverable_region, Backdrop, Scene};
use crate::{
  action::Deliverable,
  annotate::{active_color, History, Tool},
  draw,
  geom::{clamp_span, Point, Rect},
  text::TextEngine,
};

#[cfg(test)]
#[path = "chrome_test.rs"]
mod chrome_test;

const BUTTON: f32 = 30.0;
const TOOL_GAP: f32 = 2.0;
const PANEL_PAD: f32 = 5.0;
const COLUMN_GAP: f32 = 6.0;
const ROW_GAP: f32 = 8.0;
const SCREEN_MARGIN: f32 = 4.0;
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
  pub(super) fn shown(&self) -> bool {
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

const TOOL_BUTTONS: [Button; 9] = [
  button(Command::Tool(Tool::Pen), draw::Icon::Pen),
  button(Command::Tool(Tool::Line), draw::Icon::Line),
  button(Command::Tool(Tool::Arrow), draw::Icon::Arrow),
  button(Command::Tool(Tool::Box), draw::Icon::Outline),
  button(Command::Tool(Tool::Marker), draw::Icon::Marker),
  button(Command::Tool(Tool::Label), draw::Icon::Letter),
  button(Command::NextColor, draw::Icon::Letter),
  button(Command::Undo, draw::Icon::Undo),
  button(Command::Close, draw::Icon::Close),
];

const FROZEN_TOOLS: usize = 8;

const ACTION_BUTTONS: [Button; 4] = [
  button(Command::Deliver(Deliverable::Upload), draw::Icon::Upload),
  button(Command::Deliver(Deliverable::Copy), draw::Icon::CopyImage),
  button(Command::Deliver(Deliverable::Save), draw::Icon::Save),
  button(Command::Close, draw::Icon::Close),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
  pub tools: [Button; TOOL_BUTTONS.len()],
  pub actions: [Button; ACTION_BUTTONS.len()],
}

impl Chrome {
  pub fn new() -> Self {
    Self {
      tools: TOOL_BUTTONS,
      actions: ACTION_BUTTONS,
    }
  }

  pub(super) fn buttons(&self) -> impl Iterator<Item = &Button> {
    self.tools.iter().chain(&self.actions)
  }

  pub(super) fn buttons_mut(&mut self) -> impl Iterator<Item = &mut Button> {
    self.tools.iter_mut().chain(&mut self.actions)
  }

  fn hovered(&self, hotspot: Option<Hotspot>) -> Option<(&Button, Side)> {
    match hotspot {
      Some(Hotspot::Tool(i)) => self.tools.get(i).map(|b| (b, Side::Left)),
      Some(Hotspot::Action(i)) => self.actions.get(i).map(|b| (b, Side::Above)),
      None => None,
    }
  }

  pub(super) fn tooltips<'a>(
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

  for b in chrome.buttons_mut() {
    b.enabled = b.command.enabled(ready, can_undo);
    b.active = matches!(b.command, Command::Tool(active) if active == tool);
    b.area = Rect::ZERO;
  }

  let Some(sel) = selection.filter(|_| show_chrome) else {
    return;
  };
  let column = if live {
    TOOL_BUTTONS.len()
  } else {
    FROZEN_TOOLS
  };
  layout_column(&mut chrome.tools[..column], sel, bounds, live);
  if !live {
    layout_row(&mut chrome.actions, sel, bounds);
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
enum Axis {
  Across,
  Down,
}

fn panel_span(count: usize) -> f32 {
  let count = count as f32;
  count * BUTTON + (count - 1.0) * TOOL_GAP + PANEL_PAD * 2.0
}

fn layout_column(buttons: &mut [Button], sel: Rect, bounds: Rect, live: bool) {
  let width = BUTTON + PANEL_PAD * 2.0;
  let height = panel_span(buttons.len());
  let mut x = sel.right() + COLUMN_GAP;
  if x + width > bounds.right() {
    x = sel.right() - width - COLUMN_GAP;
  }
  let x = clamp_span(x, width, bounds.x, bounds.right());
  let wanted = if live {
    sel.center().y - height * 0.5
  } else {
    sel.bottom() - height
  };
  let y = clamp_span(
    wanted,
    height,
    bounds.y + SCREEN_MARGIN,
    bounds.bottom() - SCREEN_MARGIN,
  );
  lay_out_buttons(buttons, (x, y), Axis::Down);
}

fn layout_row(buttons: &mut [Button], sel: Rect, bounds: Rect) {
  let width = panel_span(buttons.len());
  let height = BUTTON + PANEL_PAD * 2.0;
  let x = (sel.right() - width).max(bounds.x);
  let mut y = sel.bottom() + ROW_GAP;
  if y + height > bounds.bottom() {
    y = sel.y - height - ROW_GAP;
  }
  let y = clamp_span(y, height, bounds.y, bounds.bottom());
  lay_out_buttons(buttons, (x, y), Axis::Across);
}

fn lay_out_buttons(buttons: &mut [Button], at: (f32, f32), axis: Axis) {
  let step = BUTTON + TOOL_GAP;
  let (x, y) = at;
  for (index, button) in buttons.iter_mut().enumerate() {
    let offset = index as f32 * step;
    let (dx, dy) = match axis {
      Axis::Down => (0.0, offset),
      Axis::Across => (offset, 0.0),
    };
    button.area =
      Rect::new(x + PANEL_PAD + dx, y + PANEL_PAD + dy, BUTTON, BUTTON);
  }
}

#[derive(Clone, Copy)]
pub(super) enum Side {
  Left,
  Above,
}

pub(super) fn tooltip_rect(
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

pub(super) fn draw_panels(pm: &mut Pixmap, scene: &Scene) {
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
