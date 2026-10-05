use std::time::{Duration, Instant};

use crate::{
  annotate::{stroke_alpha, Segment, Shape},
  geom::{Handle, Point, Rect},
};

#[cfg(test)]
#[path = "mode_test.rs"]
mod mode_test;

pub(super) const CARET_BLINK: Duration = Duration::from_millis(530);

/// What a press on the overlay turned into, and so what every later input has
/// to be read against.
pub(super) enum Mode {
  Idle,
  Rubber(Point),
  Draw(Shape, Point),
  MoveRegion(Point),
  MoveText {
    index: usize,
    lifted: Option<Shape>,
    last: Point,
  },
  ResizeRegion(Handle, Rect),
  ResizeText {
    index: usize,
    lifted: Option<Shape>,
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

/// The blinking cursor, which is the one part of the overlay on a clock of its
/// own rather than on the session's.
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

  /// Whether the caret changed, so only a flip is worth a frame.
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
  /// The shape still following the pointer, which the frame has to draw but
  /// the history does not yet hold.
  #[inline(always)]
  pub(super) fn draft(&self) -> Option<&Shape> {
    match self {
      Mode::Draw(shape, _) if !shape.is_stroke() => Some(shape),
      Mode::MoveText { lifted, .. } | Mode::ResizeText { lifted, .. } => {
        lifted.as_ref()
      }
      _ => None,
    }
  }

  #[inline(always)]
  pub(super) fn dragging_text(&self) -> bool {
    matches!(self, Mode::MoveText { .. } | Mode::ResizeText { .. })
  }

  #[inline(always)]
  pub(super) fn typing(&self) -> Option<(Point, &str, f32, [u8; 3])> {
    match self {
      Mode::Type(typing) => {
        Some((typing.at, typing.buffer.as_str(), typing.size, typing.color))
      }
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

  /// Only a stroke follows the pointer, so only a stroke wants the raw feed.
  #[inline(always)]
  pub(super) fn stroking(&self) -> bool {
    matches!(self, Mode::Draw(shape, _) if shape.is_stroke())
  }
}

/// What the ink of a growing draft asks of the session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Step {
  Repaint,
  Segment(Segment),
}

/// Follow the pointer with a draft, and report what has to be drawn.
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
