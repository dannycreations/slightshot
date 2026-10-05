use crate::geom::{Point, Rect};

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
    Rect::spanning(self.from, self.to).inflated(self.width * 0.5 + 1.0)
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
  pub fn push(&mut self, shape: Shape) {
    self.applied.push(shape);
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

#[cfg(test)]
mod tests {
  use super::*;

  fn dot(x: f32, y: f32) -> Shape {
    Shape::Text {
      at: Point::new(x, y),
      text: "x".to_string(),
      color: PALETTE[0],
      size: LABEL_SIZE,
    }
  }

  #[test]
  fn undo_empties_the_history_in_reverse_order() {
    let mut history = History::default();
    history.push(dot(1.0, 1.0));
    history.push(dot(2.0, 2.0));
    assert!(history.undo());
    assert_eq!(history.shapes().len(), 1);
    assert_eq!(history.shapes()[0], dot(1.0, 1.0));
    assert!(history.undo());
    assert!(history.shapes().is_empty());
    assert!(!history.undo());
  }

  #[test]
  fn only_the_marker_paints_translucent() {
    assert_eq!(stroke_alpha(true), MARKER_ALPHA);
    assert_eq!(stroke_alpha(false), 255);
  }

  #[test]
  fn segment_bounds_cover_the_round_caps() {
    let segment = Segment {
      from: Point::new(10.0, 10.0),
      to: Point::new(30.0, 10.0),
      color: [0, 0, 0],
      width: 6.0,
      alpha: 255,
    };
    // A round cap reaches half the width past the end point, and the padded
    // box has to hold it plus the antialiased pixel outside it.
    assert_eq!(segment.bounds(), Rect::new(6.0, 6.0, 28.0, 8.0));
  }

  #[test]
  fn incomplete_shapes_are_rejected() {
    let stray = Shape::Stroke {
      points: vec![Point::new(0.0, 0.0)],
      color: PALETTE[1],
      width: LINE_WIDTH,
      marker: false,
    };
    let stub = Shape::Line {
      from: Point::new(0.0, 0.0),
      to: Point::new(1.5, 0.0),
      color: PALETTE[2],
      width: LINE_WIDTH,
      arrow: false,
    };
    assert!(!stray.is_complete());
    assert!(!stub.is_complete());
  }

  #[test]
  fn only_the_marker_blends_and_a_blank_run_is_no_run() {
    let marker = Shape::Stroke {
      points: vec![Point::new(0.0, 0.0), Point::new(4.0, 4.0)],
      color: PALETTE[0],
      width: MARKER_WIDTH,
      marker: true,
    };
    let pen = Shape::Stroke {
      marker: false,
      points: vec![Point::new(0.0, 0.0), Point::new(4.0, 4.0)],
      color: PALETTE[0],
      width: LINE_WIDTH,
    };
    // Only ink that is laid down twice in the same place can darken, and a
    // rebuild has to know the difference to avoid redoing whole shapes.
    assert!(marker.blends());
    assert!(!pen.blends());
    assert!(!dot(1.0, 1.0).blends());
    assert!(!Shape::Line {
      from: Point::new(0.0, 0.0),
      to: Point::new(4.0, 4.0),
      color: PALETTE[0],
      width: LINE_WIDTH,
      arrow: false,
    }
    .blends());

    let mut blank = dot(1.0, 1.0);
    let Shape::Text { text, .. } = &mut blank else {
      panic!("a label is a text shape")
    };
    text.clear();
    assert!(!blank.is_complete(), "an empty run has nothing to draw");
  }

  #[test]
  fn removing_from_the_middle_renumbers_the_shapes_after_it() {
    let mut history = History::default();
    history.push(dot(1.0, 1.0));
    history.push(dot(2.0, 2.0));
    history.push(dot(3.0, 3.0));

    assert_eq!(history.remove(1), Some(dot(2.0, 2.0)));
    assert_eq!(history.shapes(), &[dot(1.0, 1.0), dot(3.0, 3.0)]);
    assert_eq!(history.remove(7), None, "there is no such slot");
  }
}
