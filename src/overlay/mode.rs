use std::time::{Duration, Instant};

use crate::{
  annotate::{stroke_alpha, Segment, Shape},
  geom::{Handle, Point, Rect},
  render::Run,
};

#[cfg(test)]
#[path = "mode_test.rs"]
mod mode_test;

pub(super) const CARET_BLINK: Duration = Duration::from_millis(530);

pub(super) enum Mode {
  Idle,
  Rubber(Point),
  Draw(Shape, Point),
  MoveRegion(Point),
  MoveText {
    index: usize,
    last: Point,
  },
  ResizeRegion(Handle, Rect),
  ResizeText {
    index: usize,
    handle: Handle,
    origin: Rect,
  },
  Type(Typing),
}

pub(super) struct Typing {
  pub(super) buffer: String,
  pub(super) at: Point,
  pub(super) size: f32,
  pub(super) color: [u8; 3],
  pub(super) editing: Option<usize>,
  pub(super) caret: Caret,
}

impl Typing {
  pub(super) fn new(at: Point, size: f32, color: [u8; 3]) -> Self {
    Self {
      buffer: String::new(),
      at,
      size,
      color,
      editing: None,
      caret: Caret::new(Instant::now()),
    }
  }
}

pub(super) struct Caret {
  lit: bool,
  due: Instant,
}

impl Caret {
  pub(super) fn new(now: Instant) -> Self {
    Self {
      lit: true,
      due: now + CARET_BLINK,
    }
  }

  pub(super) fn flip(&mut self, now: Instant) -> bool {
    if now < self.due {
      return false;
    }
    self.lit = !self.lit;
    self.due = now + CARET_BLINK;
    true
  }

  pub(super) fn restart(&mut self, now: Instant) {
    self.lit = true;
    self.due = now + CARET_BLINK;
  }

  pub(super) fn lit(&self) -> bool {
    self.lit
  }
}

impl Mode {
  #[inline(always)]
  pub(super) fn draft<'a>(
    &'a self,
    lifted: Option<&'a Shape>,
  ) -> Option<&'a Shape> {
    match self {
      Mode::Draw(shape, _) if !shape.is_stroke() => Some(shape),
      Mode::MoveText { .. } | Mode::ResizeText { .. } => lifted,
      _ => None,
    }
  }

  #[inline(always)]
  pub(super) fn text_index(&self) -> Option<usize> {
    match self {
      Mode::MoveText { index, .. } | Mode::ResizeText { index, .. } => {
        Some(*index)
      }
      _ => None,
    }
  }

  #[inline(always)]
  pub(super) fn dragging_text(&self) -> bool {
    matches!(self, Mode::MoveText { .. } | Mode::ResizeText { .. })
  }

  #[inline(always)]
  pub(super) fn typing(&self) -> Option<Run<'_>> {
    match self {
      Mode::Type(typing) => Some(Run {
        at: typing.at,
        text: &typing.buffer,
        size: typing.size,
        color: typing.color,
      }),
      _ => None,
    }
  }

  #[inline(always)]
  pub(super) fn caret(&self) -> Option<&Caret> {
    match self {
      Mode::Type(typing) => Some(&typing.caret),
      _ => None,
    }
  }

  #[inline(always)]
  pub(super) fn caret_due(&self) -> Option<Instant> {
    Some(self.caret()?.due)
  }

  #[inline(always)]
  pub(super) fn shows_chrome(&self) -> bool {
    matches!(self, Mode::Idle | Mode::Draw(_, _) | Mode::Type(_))
  }

  #[inline(always)]
  pub(super) fn stroking(&self) -> bool {
    matches!(self, Mode::Draw(shape, _) if shape.is_stroke())
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Step {
  Repaint,
  Segment(Segment),
}

pub(super) fn extend_draft(
  anchor: Point,
  draft: &mut Shape,
  p: Point,
) -> Option<Step> {
  match draft {
    Shape::Stroke {
      points,
      color,
      width,
      marker,
    } => {
      let segment = Segment {
        from: points.last().copied().unwrap_or(p),
        to: p,
        color: *color,
        width: *width,
        alpha: stroke_alpha(*marker),
      };
      points.push(p);
      Some(Step::Segment(segment))
    }
    Shape::Line { to, .. } => {
      if *to == p {
        return None;
      }
      *to = p;
      Some(Step::Repaint)
    }
    Shape::Outline { rect, .. } => {
      let new_rect = Rect::spanning(anchor, p);
      if *rect == new_rect {
        return None;
      }
      *rect = new_rect;
      Some(Step::Repaint)
    }
    Shape::Text { .. } => {
      unreachable!("a run of text is typed, never dragged as a draft")
    }
  }
}
