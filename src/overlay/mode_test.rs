use std::time::{Duration, Instant};

use super::{extend_draft, Caret, Mode, Step, CARET_BLINK};
use crate::{
  annotate::{stroke_alpha, Segment, Shape},
  geom::{Point, Rect},
};

#[test]
fn extend_draft_reports_the_segment_a_stroke_gained() {
  let mut stroke = Shape::Stroke {
    points: vec![Point::new(1.0, 1.0)],
    color: [7, 8, 9],
    width: 3.0,
    marker: true,
  };
  assert_eq!(
    extend_draft(Point::new(1.0, 1.0), &mut stroke, Point::new(5.0, 5.0)),
    Some(Step::Segment(Segment {
      from: Point::new(1.0, 1.0),
      to: Point::new(5.0, 5.0),
      color: [7, 8, 9],
      width: 3.0,
      alpha: stroke_alpha(true),
    }))
  );
  assert_eq!(
    stroke,
    Shape::Stroke {
      points: vec![Point::new(1.0, 1.0), Point::new(5.0, 5.0)],
      color: [7, 8, 9],
      width: 3.0,
      marker: true,
    }
  );
}

#[test]
fn extend_draft_logs_moves_smaller_than_a_pixel() {
  // Sampling used to drop anything under a pixel. The log is raw now, so a
  // stroke taken slowly still follows the cursor.
  let mut stroke = Shape::Stroke {
    points: vec![Point::new(1.0, 1.0)],
    color: [0, 0, 0],
    width: 2.0,
    marker: false,
  };
  let nudge = Point::new(1.2, 1.0);
  assert!(matches!(
    extend_draft(Point::new(1.0, 1.0), &mut stroke, nudge),
    Some(Step::Segment(_))
  ));
  let Shape::Stroke { points, .. } = &stroke else {
    panic!("the draft is a stroke");
  };
  assert_eq!(points, &[Point::new(1.0, 1.0), nudge]);
}

#[test]
fn extend_draft_reports_no_step_when_the_cursor_does_not_move() {
  let mut line = Shape::Line {
    from: Point::new(0.0, 0.0),
    to: Point::new(4.0, 4.0),
    color: [0, 0, 0],
    width: 2.0,
    arrow: false,
  };
  assert_eq!(
    extend_draft(Point::new(0.0, 0.0), &mut line, Point::new(4.0, 4.0)),
    None
  );

  let mut boxed = Shape::Outline {
    rect: Rect::new(10.0, 20.0, 0.0, 0.0),
    color: [0, 0, 0],
    width: 2.0,
  };
  assert_eq!(
    extend_draft(Point::new(10.0, 20.0), &mut boxed, Point::new(40.0, 60.0)),
    Some(Step::Repaint)
  );
  assert_eq!(
    extend_draft(Point::new(10.0, 20.0), &mut boxed, Point::new(40.0, 60.0)),
    None,
    "a box that has not moved should not ask for another frame"
  );
}

#[test]
fn only_a_stroke_takes_the_pointer_from_the_raw_feed() {
  // Windows merges the mouse positions it cannot deliver in time, so a
  // stroke has to be built from the raw reports instead. A tool that
  // settles on a position rather than following a path reads fine from the
  // merged feed, and collecting reports for it as well would drag it
  // around for motion the user never made.
  let stroke = || {
    Mode::Draw(
      Shape::Stroke {
        points: vec![Point::new(0.0, 0.0)],
        color: [0, 0, 0],
        width: 2.0,
        marker: false,
      },
      Point::new(0.0, 0.0),
    )
  };
  assert!(stroke().stroking(), "a stroke follows the raw reports");
  assert!(!Mode::Idle.stroking());
  assert!(!Mode::Draw(
    Shape::Line {
      from: Point::new(0.0, 0.0),
      to: Point::new(4.0, 4.0),
      color: [0, 0, 0],
      width: 2.0,
      arrow: false,
    },
    Point::new(0.0, 0.0)
  )
  .stroking());
  assert!(!Mode::Rubber(Point::new(0.0, 0.0)).stroking());
}

#[test]
fn the_caret_holds_each_state_for_a_blink_and_a_keystroke_brings_it_back() {
  let start = Instant::now();
  let mut caret = Caret::new(start);
  assert!(caret.lit(), "a caret comes up lit");

  // Nothing changes the caret until its time is up, so a frame redrawn for
  // any other reason cannot leave it in the wrong state.
  assert!(!caret.flip(start));
  assert!(!caret.flip(start + CARET_BLINK / 2));
  assert!(caret.lit());

  assert!(caret.flip(start + CARET_BLINK), "then it goes dark");
  assert!(!caret.lit());
  assert!(
    !caret.flip(start + CARET_BLINK + CARET_BLINK / 2),
    "and stays dark for a whole blink"
  );
  assert!(caret.flip(start + 3 * CARET_BLINK), "until it comes back");
  assert!(caret.lit());

  // A keystroke starts the blink again from lit, so the caret is never dark
  // while there is still typing going on.
  caret.restart(start + 3 * CARET_BLINK);
  assert!(caret.lit());
  assert!(
    !caret.flip(start + 3 * CARET_BLINK + Duration::from_millis(1)),
    "and holds steady for a blink after the last keystroke"
  );
  assert!(caret.flip(start + 4 * CARET_BLINK), "then blinks as before");
  assert!(!caret.lit());
}
