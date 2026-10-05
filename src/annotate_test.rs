use super::{
  stroke_alpha, History, Segment, Shape, LABEL_SIZE, LINE_WIDTH, MARKER_ALPHA,
  MARKER_WIDTH, PALETTE,
};
use crate::geom::{Point, Rect};

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
