use crate::geom::{Point, Rect};

#[cfg(test)]
#[path = "annotate_test.rs"]
mod annotate_test;

pub const PALETTE: [[u8; 3]; 7] = [
  [239, 68, 68],
  [249, 115, 22],
  [250, 204, 21],
  [34, 197, 94],
  [59, 130, 246],
  [255, 255, 255],
  [15, 23, 42],
];

#[inline(always)]
pub fn active_color(index: usize) -> [u8; 3] {
  PALETTE[index % PALETTE.len()]
}

pub const LINE_WIDTH: f32 = 4.0;
pub const MARKER_WIDTH: f32 = 16.0;
pub const MARKER_ALPHA: u8 = 80;
pub const LABEL_SIZE: f32 = 20.0;

pub const MIN_SIZE: f32 = 1.0;
pub const MAX_SIZE: f32 = 100.0;
pub const SIZE_STEP: f32 = 1.0;

const MIN_DRAG: f32 = 3.0;

#[inline(always)]
pub fn stroke_alpha(marker: bool) -> u8 {
  if marker {
    MARKER_ALPHA
  } else {
    255
  }
}

#[inline(always)]
pub fn stroke_bounds(area: Rect, width: f32) -> Rect {
  area.inflated(width * 0.5 + 1.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
  pub from: Point,
  pub to: Point,
  pub color: [u8; 3],
  pub width: f32,
  pub alpha: u8,
}

impl Segment {
  pub fn bounds(&self) -> Rect {
    stroke_bounds(Rect::spanning(self.from, self.to), self.width)
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(usize)]
pub enum Tool {
  Select = 0,
  Pen,
  Line,
  Arrow,
  Box,
  Marker,
  Label,
}

pub const TOOLS: [Tool; 7] = [
  Tool::Select,
  Tool::Pen,
  Tool::Line,
  Tool::Arrow,
  Tool::Box,
  Tool::Marker,
  Tool::Label,
];

impl Tool {
  #[inline(always)]
  pub fn default_size(self) -> f32 {
    match self {
      Tool::Select => 0.0,
      Tool::Pen | Tool::Line | Tool::Arrow | Tool::Box => LINE_WIDTH,
      Tool::Marker => MARKER_WIDTH,
      Tool::Label => LABEL_SIZE,
    }
  }

  #[inline(always)]
  pub fn is_annotation(self) -> bool {
    !matches!(self, Tool::Select)
  }

  pub fn label(self) -> &'static str {
    match self {
      Tool::Select => "Select",
      Tool::Pen => "Pen",
      Tool::Line => "Line",
      Tool::Arrow => "Arrow",
      Tool::Box => "Rectangle",
      Tool::Marker => "Marker",
      Tool::Label => "Text",
    }
  }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
  Stroke {
    points: Vec<Point>,
    color: [u8; 3],
    width: f32,
    marker: bool,
  },
  Line {
    from: Point,
    to: Point,
    color: [u8; 3],
    width: f32,
    arrow: bool,
  },
  Outline {
    rect: Rect,
    color: [u8; 3],
    width: f32,
  },
  Text {
    at: Point,
    text: String,
    color: [u8; 3],
    size: f32,
  },
}

impl Shape {
  pub fn is_stroke(&self) -> bool {
    matches!(self, Shape::Stroke { .. })
  }

  pub fn blends(&self) -> bool {
    match self {
      Shape::Stroke { marker, .. } => *marker,
      Shape::Line { .. } | Shape::Outline { .. } | Shape::Text { .. } => false,
    }
  }

  pub fn is_complete(&self) -> bool {
    match self {
      Shape::Stroke { points, .. } => points.len() >= 2,
      Shape::Line { from, to, .. } => {
        from.distance_squared(*to) >= MIN_DRAG * MIN_DRAG
      }
      Shape::Outline { rect, .. } => rect.w >= MIN_DRAG && rect.h >= MIN_DRAG,
      Shape::Text { text, .. } => !text.trim().is_empty(),
    }
  }
}

#[derive(Default)]
pub struct History {
  applied: Vec<Shape>,
}

impl History {
  pub fn push(&mut self, shape: Shape) -> usize {
    self.applied.push(shape);
    self.applied.len() - 1
  }

  pub fn undo(&mut self) -> bool {
    self.applied.pop().is_some()
  }

  pub fn shapes(&self) -> &[Shape] {
    &self.applied
  }

  pub fn can_undo(&self) -> bool {
    !self.applied.is_empty()
  }

  pub fn shape(&self, index: usize) -> Option<&Shape> {
    self.applied.get(index)
  }

  pub fn shape_mut(&mut self, index: usize) -> Option<&mut Shape> {
    self.applied.get_mut(index)
  }

  pub fn remove(&mut self, index: usize) -> Option<Shape> {
    (index < self.applied.len()).then(|| self.applied.remove(index))
  }
}
