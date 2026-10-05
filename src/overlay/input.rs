use std::{
  mem,
  time::{Duration, Instant},
};

use winit::{dpi::PhysicalPosition, window::CursorIcon};

use super::{
  mode::{extend_draft, Caret, Mode, Step, Typing},
  resize::{resize_cursor, resize_text},
  session::Session,
  Outcome,
};
use crate::{
  action::Deliverable,
  annotate::{
    active_color, Shape, Tool, MAX_SIZE, MIN_SIZE, PALETTE, SIZE_STEP,
  },
  geom::{hit_handle, resized, Handle, Point, Rect, HANDLE_SLOP},
  render::{self, Hotspot},
};

#[cfg(test)]
#[path = "input_test.rs"]
mod input_test;

const HINT_DURATION: Duration = Duration::from_millis(800);
const GRAB_SLOP: f32 = 4.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_SLOP: f32 = 4.0;
const MAX_STRETCH: f32 = 8.0;

impl Session {
  pub(super) fn mouse_move(&mut self, position: PhysicalPosition<f64>) {
    let p = Point::new(position.x as f32, position.y as f32);
    if self.mode.stroking() {
      self.place_reports(p);
    }
    self.pointer_at(p);
  }

  pub(super) fn pointer_motion(&mut self, delta: (f64, f64)) {
    if !self.mode.stroking() {
      return;
    }
    let from = self.raw_path.last().copied().unwrap_or(self.cursor);
    self
      .raw_path
      .push(Point::new(from.x + delta.0 as f32, from.y + delta.1 as f32));
  }

  fn place_reports(&mut self, to: Point) {
    let from = self.cursor;
    let mut reports = mem::take(&mut self.raw_path);
    if place_burst(&mut reports, from, to) {
      for point in reports {
        self.pointer_at(point);
      }
    }
  }

  fn pointer_at(&mut self, p: Point) {
    if self.cursor == p {
      return;
    }
    self.cursor = p;
    let previous = self.hover;
    self.hover = if self.mode.shows_chrome() && self.selection.is_some() {
      render::hotspot_at(&self.chrome, p)
    } else {
      None
    };
    if self.hover != previous {
      self.window.request_redraw();
    }
    // A drag of a run of text is dispatched on its own, because it has to take
    // the run off the canvas and the borrow of the mode in hand would not allow
    // it to.
    if self.mode.dragging_text() {
      if self.drag_text(p) {
        self.window.request_redraw();
      }
      return;
    }
    let mut step = None;
    match &mut self.mode {
      Mode::Idle => {
        let icon = self.cursor_at(p);
        self.update_cursor(icon);
      }
      Mode::Rubber(anchor) => {
        let drawn = Rect::spanning(*anchor, p);
        self.set_selection(drawn);
      }
      Mode::Draw(draft, anchor) => {
        step = extend_draft(*anchor, draft, p);
      }
      Mode::MoveRegion(last) => {
        let sel = self
          .selection
          .expect("a region is held to move it, and there is one");
        let delta = Point::new(p.x - last.x, p.y - last.y);
        *last = p;
        self.set_selection(sel.moved_inside(self.bounds, delta));
      }
      Mode::ResizeRegion(handle, rect) => {
        let target = p.clamped_inside(self.bounds);
        let dragged = resized(*rect, *handle, target);
        self.set_selection(dragged);
      }
      // A run being dragged was dispatched above, and typing follows no
      // path of its own.
      Mode::MoveText { .. } | Mode::ResizeText { .. } | Mode::Type(_) => {}
    }
    if let Some(step) = step {
      if let Step::Segment(segment) = step {
        self.ink_segment(segment);
      }
      self.window.request_redraw();
    }
  }

  fn drag_text(&mut self, p: Point) -> bool {
    self.lift_dragged();
    let Some(Shape::Text { at, text, size, .. }) = self.lifted.as_mut() else {
      return false;
    };
    match &mut self.mode {
      Mode::MoveText { last, .. } => {
        let delta = Point::new(p.x - last.x, p.y - last.y);
        *last = p;
        // The whole box has to stay on screen, or the run cannot be pressed
        // again to bring it back.
        let box_ = self
          .engine
          .bounds(text, Point::new(at.x + delta.x, at.y + delta.y), *size)
          .clamped_inside(self.bounds);
        *at = Point::new(box_.x, box_.y);
        true
      }
      Mode::ResizeText { handle, origin, .. } => {
        let target = p.clamped_inside(self.bounds);
        let (next_at, next_size) =
          resize_text(&self.engine, text, *origin, *handle, target);
        let moved = *at != next_at || *size != next_size;
        *at = next_at;
        *size = next_size;
        moved
      }
      _ => false,
    }
  }

  fn lift_dragged(&mut self) {
    if self.lifted.is_some() {
      return;
    }
    let Some(index) = self.mode.text_index() else {
      return;
    };
    if let Some(run) = self.lift_text(index) {
      self.lifted = Some(run);
    }
  }

  fn set_selection(&mut self, sel: Rect) {
    if self.selection != Some(sel) {
      self.selection = Some(sel);
      self.window.request_redraw();
    }
  }

  pub(super) fn mouse_down(&mut self) -> Option<Outcome> {
    let p = self.cursor;
    // Whatever is half typed lands first, so no click can drop text the user
    // can still see being typed.
    self.commit_typing();
    let double = self.double_clicked(p);
    if let Some(hotspot) = render::hotspot_at(&self.chrome, p) {
      return self.activate(hotspot);
    }
    // A run of text is an object: a press on it picks it up, whatever tool is
    // in hand, so it can be moved and scaled long after it was written.
    if let Some((index, box_)) = self.text_at(p) {
      let handle = self.handle_on(index, box_, p);
      self.picked = Some(index);
      // The second press of a pair asks the run for its text back, so it opens
      // the run for rewriting instead of picking it up all over again.
      if double {
        self.edit_picked();
        self.window.request_redraw();
        return None;
      }
      self.mode = match handle {
        Some(handle) => Mode::ResizeText {
          index,
          handle,
          origin: box_,
        },
        None => Mode::MoveText { index, last: p },
      };
      self.window.request_redraw();
      return None;
    }
    // A press that reaches no run at all is the only way to put one down.
    self.picked = None;
    if self.tool == Tool::Select {
      if let Some(region) = self.region_at(p) {
        self.mode = region;
        self.window.request_redraw();
        return None;
      }
    }
    self.mode = match self.tool {
      // A new region starts only where there is one to pick. A live backdrop is
      // the whole screen already, so the select tool has nothing to start
      // there, and a rubber band would shrink the live canvas to it.
      Tool::Select if self.backdrop_kind.picks_region() => {
        self.selection = None;
        Mode::Rubber(p)
      }
      Tool::Select => Mode::Idle,
      Tool::Label => Mode::Type(Typing::new(
        p,
        self.size(Tool::Label),
        active_color(self.palette_index),
      )),
      tool => Mode::Draw(self.new_shape(tool, p), p),
    };
    if self.mode.stroking() {
      // A stroke that ended before the system reported a position left
      // reports behind. The next one starts from the cursor, not from them.
      self.raw_path.clear();
    }
    self.window.request_redraw();
    None
  }

  pub(super) fn mouse_up(&mut self) {
    match mem::replace(&mut self.mode, Mode::Idle) {
      Mode::Rubber(_) => {
        self.selection = render::deliverable_region(self.selection);
      }
      Mode::Draw(draft, _) if draft.is_complete() => {
        let area = render::shape_area(Some(&draft), &self.engine);
        let stroke = draft.is_stroke();
        self.history.push(draft);
        // A stroke is already on the ink, segment by segment. Rebuilding it
        // here would composite the whole log over itself a second time.
        if !stroke {
          self.rebuild(area);
        }
      }
      // A run that never left the canvas is already where it belongs.
      Mode::MoveText { index, .. } | Mode::ResizeText { index, .. } => {
        if let Some(run) = self.lifted.take() {
          self.land_text(index, run);
        }
      }
      // Typing hands its buffer and anchor straight back: placing a run of
      // text is a click that opens it, so that one mode outlives the release.
      Mode::Type(typing) => self.mode = Mode::Type(typing),
      _ => {}
    }
    self.window.request_redraw();
  }

  pub(super) fn character(&mut self, ch: &str) -> Option<Outcome> {
    if ch.eq_ignore_ascii_case("c") && self.modifiers.control_key() {
      // A run the pointer is in the middle of moving, or the one the caret is
      // sitting in, is not on the canvas, so it has to land before the image is
      // handed over.
      if self.mode.dragging_text() {
        self.mouse_up();
      }
      self.commit_typing();
      return self.deliver(Deliverable::Copy);
    }
    if self.mode.typing().is_none() && matches!(ch, "[" | "]") {
      let step = if ch == "]" { SIZE_STEP } else { -SIZE_STEP };
      self.adjust_size(step);
      return None;
    }
    self.type_char(ch);
    None
  }

  pub(super) fn press_enter(&mut self) {
    if self.mode.typing().is_some() {
      self.commit_typing();
    } else if matches!(self.mode, Mode::Idle) && self.tool == Tool::Select {
      self.edit_picked();
    }
    self.window.request_redraw();
  }

  pub(super) fn backspace(&mut self) {
    if let Mode::Type(typing) = &mut self.mode {
      typing.buffer.pop();
      typing.caret.restart(Instant::now());
    }
    self.repaint_typing();
  }

  fn type_char(&mut self, ch: &str) {
    if let Mode::Type(typing) = &mut self.mode {
      typing.buffer.push_str(ch);
      typing.caret.restart(Instant::now());
    }
    self.repaint_typing();
  }

  fn double_clicked(&mut self, p: Point) -> bool {
    let now = Instant::now();
    let second = self.press.is_some_and(|(when, at)| {
      is_double_click(now.duration_since(when), at, p)
    });
    self.press = Some((now, p));
    second
  }

  fn region_at(&self, p: Point) -> Option<Mode> {
    // A live backdrop is the whole screen, which is not a thing to move, so
    // there the select tool is only ever a way of picking up what is on it.
    let sel = self
      .selection
      .filter(|_| self.backdrop_kind.picks_region())?;
    match hit_handle(sel, p, HANDLE_SLOP) {
      Some(handle) => Some(Mode::ResizeRegion(handle, sel)),
      None if sel.contains(p) => Some(Mode::MoveRegion(p)),
      None => None,
    }
  }

  fn text_at(&self, p: Point) -> Option<(usize, Rect)> {
    self
      .history
      .shapes()
      .iter()
      .enumerate()
      .rev()
      .find_map(|(index, shape)| {
        let Shape::Text { at, text, size, .. } = shape else {
          return None;
        };
        let box_ = self.text_box(*at, text, *size)?;
        let grab = box_.inflated(GRAB_SLOP);
        grab.contains(p).then_some((index, box_))
      })
  }

  pub(super) fn picked_box(&self) -> Option<Rect> {
    if let Some(Shape::Text { at, text, size, .. }) =
      self.mode.draft(self.lifted.as_ref())
    {
      return self.text_box(*at, text, *size);
    }
    let Shape::Text { at, text, size, .. } =
      self.history.shape(self.picked?)?
    else {
      return None;
    };
    self.text_box(*at, text, *size)
  }

  fn text_box(&self, at: Point, text: &str, size: f32) -> Option<Rect> {
    let box_ = self.engine.bounds(text, at, size);
    (!box_.is_empty()).then_some(box_)
  }

  fn cursor_at(&self, p: Point) -> CursorIcon {
    if let Some((index, box_)) = self.text_at(p) {
      return match self.handle_on(index, box_, p) {
        Some(handle) => resize_cursor(handle),
        None => CursorIcon::Move,
      };
    }
    if self.tool != Tool::Select {
      return CursorIcon::default();
    }
    match self.region_at(p) {
      Some(Mode::ResizeRegion(handle, _)) => resize_cursor(handle),
      Some(Mode::MoveRegion(_)) => CursorIcon::Move,
      _ => CursorIcon::default(),
    }
  }

  fn handle_on(&self, index: usize, box_: Rect, p: Point) -> Option<Handle> {
    (self.picked == Some(index))
      .then(|| hit_handle(box_, p, HANDLE_SLOP))
      .flatten()
  }

  fn new_shape(&self, tool: Tool, p: Point) -> Shape {
    let color = active_color(self.palette_index);
    let width = self.size(tool);
    match tool {
      Tool::Pen | Tool::Marker => Shape::Stroke {
        points: vec![p],
        color,
        width,
        marker: tool == Tool::Marker,
      },
      Tool::Line | Tool::Arrow => Shape::Line {
        from: p,
        to: p,
        color,
        width,
        arrow: tool == Tool::Arrow,
      },
      Tool::Box => Shape::Outline {
        rect: Rect::new(p.x, p.y, 0.0, 0.0),
        color,
        width,
      },
      Tool::Select | Tool::Label => {
        unreachable!("Select and Label are handled directly in mouse_down")
      }
    }
  }

  fn activate(&mut self, hotspot: Hotspot) -> Option<Outcome> {
    let command = match hotspot {
      Hotspot::Tool(i) => self.chrome.tools[i].command,
      Hotspot::Action(i) => self.chrome.actions[i].command,
    };
    match command {
      render::Command::Tool(tool) => {
        // Clicking the tool in hand puts it down and brings the region back,
        // which only means something where the user has a region to pick.
        self.tool = if self.tool == tool && self.backdrop_kind.picks_region() {
          Tool::Select
        } else {
          tool
        };
        None
      }
      render::Command::NextColor => {
        self.palette_index = (self.palette_index + 1) % PALETTE.len();
        None
      }
      render::Command::Undo => {
        if self.history.undo() {
          // Undo renumbers everything after the shape it dropped, so a pick
          // that was pointing at that shape has to let go rather than land on
          // whatever took its place.
          if self
            .picked
            .is_some_and(|pick| pick >= self.history.shapes().len())
          {
            self.picked = None;
          }
          self.rebuild(self.bounds);
        }
        None
      }
      render::Command::Close => Some(Outcome::Close),
      render::Command::Deliver(deliverable) => self.deliver(deliverable),
    }
  }

  fn deliver(&self, deliverable: Deliverable) -> Option<Outcome> {
    if !self.backdrop_kind.picks_region() {
      return None;
    }
    let sel = render::deliverable_region(self.selection)?;
    let canvas = self.buffers.canvas.get();
    Some(Outcome::Deliver {
      deliverable,
      shot: render::flatten(canvas, sel),
    })
  }

  fn commit_typing(&mut self) {
    let Mode::Type(typing) = mem::replace(&mut self.mode, Mode::Idle) else {
      return;
    };
    let label = Shape::Text {
      at: typing.at,
      text: typing.buffer,
      color: typing.color,
      size: typing.size,
    };
    let Some(index) = typing.editing else {
      if label.is_complete() {
        let area = render::shape_area(Some(&label), &self.engine);
        // A run that has just been written is picked straight away, so its
        // box is there to press on without reaching for the select tool.
        self.picked = Some(self.history.push(label));
        self.rebuild(area);
      }
      return;
    };
    if !label.is_complete() {
      // A run emptied out while it was being rewritten leaves nothing to
      // draw, and nothing to put back if the session is undone to here.
      self.history.remove(index);
      self.picked = None;
      return;
    }
    self.land_text(index, label);
  }

  fn edit_picked(&mut self) {
    let Some(index) = self.picked else {
      return;
    };
    let Some(Shape::Text {
      at,
      text,
      color,
      size,
    }) = self.lift_text(index)
    else {
      return;
    };
    self.mode = Mode::Type(Typing {
      buffer: text,
      at,
      size,
      color,
      editing: Some(index),
      caret: Caret::new(Instant::now()),
    });
  }

  #[inline(always)]
  fn size(&self, tool: Tool) -> f32 {
    self.sizes[tool as usize]
  }

  fn adjust_size(&mut self, delta: f32) {
    if !self.tool.is_annotation() {
      return;
    }
    let tool = self.tool;
    let next = (self.size(tool) + delta).clamp(MIN_SIZE, MAX_SIZE);
    self.sizes[tool as usize] = next;
    // A stroke keeps the width it was started with; the rest take the new one
    // mid-drag.
    if let Mode::Draw(
      Shape::Line { width, .. } | Shape::Outline { width, .. },
      _,
    ) = &mut self.mode
    {
      *width = next;
    }
    self.hint = Some((format_size(next), Instant::now() + HINT_DURATION));
    self.window.request_redraw();
  }
}

fn format_size(size: f32) -> String {
  if size.fract() == 0.0 {
    format!("{}", size as i32)
  } else {
    format!("{size:.1}")
  }
}

fn stretch(reported: f32, moved: f32) -> Option<f32> {
  let factor = moved / reported;
  (factor.is_finite() && factor.abs() <= MAX_STRETCH).then_some(factor)
}

fn place_burst(reports: &mut [Point], from: Point, to: Point) -> bool {
  let Some(last) = reports.last().copied() else {
    return false;
  };
  let (Some(kx), Some(ky)) = (
    stretch(last.x - from.x, to.x - from.x),
    stretch(last.y - from.y, to.y - from.y),
  ) else {
    return false;
  };
  for point in reports.iter_mut() {
    point.x = from.x + (point.x - from.x) * kx;
    point.y = from.y + (point.y - from.y) * ky;
  }
  true
}

fn is_double_click(elapsed: Duration, from: Point, to: Point) -> bool {
  elapsed <= DOUBLE_CLICK
    && from.distance_squared(to) <= DOUBLE_CLICK_SLOP * DOUBLE_CLICK_SLOP
}
