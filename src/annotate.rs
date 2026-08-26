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

pub const PEN_WIDTH: f32 = 2.5;
pub const LINE_WIDTH: f32 = 2.0;
pub const MARKER_WIDTH: f32 = 16.0;
pub const MARKER_ALPHA: u8 = 80;
pub const LABEL_SIZE: f32 = 20.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
  Select,
  Pen,
  Line,
  Arrow,
  Box,
  Marker,
  Label,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
  Freehand {
    points: Vec<Point>,
    color: [u8; 3],
    width: f32,
  },
  Segment {
    from: Point,
    to: Point,
    color: [u8; 3],
    width: f32,
  },
  Arrow {
    tail: Point,
    head: Point,
    color: [u8; 3],
    width: f32,
  },
  Outline {
    rect: Rect,
    color: [u8; 3],
    width: f32,
  },
  Marker {
    points: Vec<Point>,
    color: [u8; 3],
  },
  Caption {
    at: Point,
    text: String,
    color: [u8; 3],
  },
}

impl Shape {
  pub fn is_complete(&self) -> bool {
    match self {
      Shape::Freehand { points, .. } | Shape::Marker { points, .. } => {
        points.len() >= 2
      }
      Shape::Segment { from, to, .. }
      | Shape::Arrow {
        tail: from,
        head: to,
        ..
      } => from.distance(*to) >= 3.0,
      Shape::Outline { rect, .. } => rect.w >= 3.0 && rect.h >= 3.0,
      Shape::Caption { text, .. } => !text.trim().is_empty(),
    }
  }

  pub fn translate(&mut self, dx: f32, dy: f32) {
    let bump = |p: &mut Point| {
      p.x += dx;
      p.y += dy;
    };
    match self {
      Shape::Freehand { points, .. } | Shape::Marker { points, .. } => {
        points.iter_mut().for_each(bump);
      }
      Shape::Segment { from, to, .. }
      | Shape::Arrow {
        tail: from,
        head: to,
        ..
      } => {
        bump(from);
        bump(to);
      }
      Shape::Outline { rect, .. } => *rect = rect.translated(dx, dy),
      Shape::Caption { at, .. } => bump(at),
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

  pub fn translate_all(&mut self, dx: f32, dy: f32) {
    for shape in &mut self.applied {
      shape.translate(dx, dy);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn dot(x: f32, y: f32) -> Shape {
    Shape::Caption {
      at: Point::new(x, y),
      text: "x".to_string(),
      color: PALETTE[0],
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
  fn incomplete_shapes_are_rejected() {
    let stray = Shape::Freehand {
      points: vec![Point::new(0.0, 0.0)],
      color: PALETTE[1],
      width: PEN_WIDTH,
    };
    let stub = Shape::Segment {
      from: Point::new(0.0, 0.0),
      to: Point::new(1.5, 0.0),
      color: PALETTE[2],
      width: LINE_WIDTH,
    };
    assert!(!stray.is_complete());
    assert!(!stub.is_complete());
  }

  #[test]
  fn translate_moves_every_variant() {
    let mut shape = Shape::Segment {
      from: Point::new(0.0, 0.0),
      to: Point::new(4.0, 0.0),
      color: PALETTE[2],
      width: LINE_WIDTH,
    };
    shape.translate(10.0, 5.0);
    assert_eq!(
      shape,
      Shape::Segment {
        from: Point::new(10.0, 5.0),
        to: Point::new(14.0, 5.0),
        color: PALETTE[2],
        width: LINE_WIDTH,
      }
    );
  }

  #[test]
  fn history_translate_all_moves_shapes_together() {
    let mut history = History::default();
    history.push(Shape::Segment {
      from: Point::new(0.0, 0.0),
      to: Point::new(4.0, 0.0),
      color: PALETTE[2],
      width: LINE_WIDTH,
    });
    history.push(Shape::Caption {
      at: Point::new(2.0, 2.0),
      text: "hi".to_string(),
      color: PALETTE[0],
    });
    history.translate_all(1.0, 1.0);
    assert_eq!(
      history.shapes()[0],
      Shape::Segment {
        from: Point::new(1.0, 1.0),
        to: Point::new(5.0, 1.0),
        color: PALETTE[2],
        width: LINE_WIDTH,
      }
    );
    assert_eq!(
      history.shapes()[1],
      Shape::Caption {
        at: Point::new(3.0, 3.0),
        text: "hi".to_string(),
        color: PALETTE[0],
      }
    );
  }
}
