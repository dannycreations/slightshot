use std::time::Duration;

use super::{is_double_click, place_burst, stretch, DOUBLE_CLICK};
use crate::geom::Point;

#[test]
fn a_burst_of_reports_is_pinned_to_both_cursor_positions() {
  // The reports know the shape of the path, not where the cursor ended up:
  // Windows scales the position it reports for pointer speed, so a fast
  // stroke arrives short and the ink would trail the cursor. Anchoring only
  // one end leaves a gap, so both ends have to land on the cursor.
  let from = Point::new(100.0, 100.0);
  let mut reports =
    vec![from, Point::new(104.0, 101.0), Point::new(108.0, 104.0)];
  assert!(place_burst(&mut reports, from, Point::new(120.0, 120.0)));
  assert_eq!(reports[0], from, "the burst starts on the old cursor");
  assert_eq!(
    reports[2],
    Point::new(120.0, 120.0),
    "and ends on the new one, or the ink drifts off the cursor"
  );
  assert_eq!(
    reports[1],
    Point::new(110.0, 105.0),
    "the turn between the ends is the hand's, and has to survive"
  );
}

#[test]
fn a_burst_that_says_nothing_about_the_gap_is_not_drawn() {
  let from = Point::new(100.0, 100.0);
  // The reports went nowhere while the cursor crossed the screen. The
  // straight line is the honest answer, and drawing the reports where they
  // landed would put ink off the cursor.
  let mut still = vec![from, from, from];
  assert!(!place_burst(&mut still, from, Point::new(400.0, 100.0)));

  // A drag straight along one axis is rejected too, because there is no
  // direction to stretch and the straight line already is its path. A rule
  // that looks like a defect here is the whole difference between a level
  // line and a wiggle drawn through it.
  let mut level = vec![from, Point::new(110.0, 100.0)];
  assert!(!place_burst(&mut level, from, Point::new(110.0, 100.0)));

  // A drag that already matches the cursor keeps the ink where it is.
  let mut agreed =
    vec![from, Point::new(105.0, 105.0), Point::new(110.0, 110.0)];
  let expected = agreed.clone();
  assert!(place_burst(&mut agreed, from, Point::new(110.0, 110.0),));
  assert_eq!(agreed, expected);
}

#[test]
fn stretch_rejects_a_gap_the_reports_do_not_account_for() {
  // A mouse held still reports no distance at all, and a report the app
  // never received leaves the survivors far shorter than the cursor moved.
  // Either way the straight line between two cursor positions is the honest
  // answer, and a rejected factor must not reach the path maths as a NaN.
  assert_eq!(stretch(0.0, 0.0), None);
  assert_eq!(stretch(0.0, 40.0), None);
  assert_eq!(stretch(2.0, 100.0), None, "a factor of 50 is a lost report");
  assert_eq!(stretch(20.0, 30.0), Some(1.5), "pointer speed, or reports");
  assert_eq!(
    stretch(30.0, 20.0),
    Some(20.0 / 30.0),
    "a path that doubled back"
  );
}

#[test]
fn two_presses_make_a_pair_only_on_the_same_spot_inside_the_window() {
  let at = Point::new(120.0, 80.0);
  // The rule is the whole of a double click: the same place, inside the
  // window. Anything else is a first press, so a run is only opened for
  // rewriting by a gesture the user meant as one.
  assert!(is_double_click(
    Duration::from_millis(80),
    at,
    Point::new(122.0, 81.0)
  ));
  assert!(
    is_double_click(DOUBLE_CLICK, at, at),
    "the window is as long as it says"
  );
  assert!(
    !is_double_click(DOUBLE_CLICK + Duration::from_millis(1), at, at),
    "a press too late is a first press"
  );
  assert!(
    !is_double_click(Duration::from_millis(80), at, Point::new(160.0, 80.0)),
    "and so is one on the run next door"
  );
}
