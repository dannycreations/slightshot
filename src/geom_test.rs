use super::{
  clamp_span, handle_anchor, hit_handle, resized, Handle, Point, Rect, HANDLES,
  HANDLE_SLOP,
};

const SCREEN: Rect = Rect::new(0.0, 0.0, 1920.0, 1080.0);

#[test]
fn spanning_normalizes_reversed_drags() {
  let from = Point::new(500.0, 400.0);
  let to = Point::new(100.0, 900.0);
  assert_eq!(
    Rect::spanning(from, to),
    Rect::new(100.0, 400.0, 400.0, 500.0)
  );
}

#[test]
fn hit_handle_matches_only_near_anchors() {
  let rect = Rect::new(10.0, 10.0, 90.0, 90.0);
  let corner = handle_anchor(rect, Handle::TopRight);
  assert_eq!(hit_handle(rect, corner, 5.0), Some(Handle::TopRight));
  assert_eq!(hit_handle(rect, Point::new(55.0, 55.0), 5.0), None);
}

#[test]
fn around_covers_every_point_in_any_order() {
  // The order a stroke was drawn in cannot move where its ink is claimed to
  // be, so the box has to come out the same whichever end was drawn first.
  let forwards = [
    Point::new(30.0, -10.0),
    Point::new(-5.0, 40.0),
    Point::new(12.0, 12.0),
  ];
  assert_eq!(
    Rect::around(&forwards),
    Rect::around(&forwards.iter().rev().copied().collect::<Vec<_>>()),
    "the box does not depend on the order of the samples"
  );
  assert_eq!(
    Rect::around(&forwards),
    Rect::new(-5.0, -10.0, 35.0, 50.0),
    "and it has to hold all three, corners and middle alike"
  );
  assert_eq!(Rect::around(&[]), Rect::ZERO, "no samples claim no ground");
  assert_eq!(
    Rect::around(&[Point::new(4.0, 7.0)]),
    Rect::new(4.0, 7.0, 0.0, 0.0),
    "one sample is a box of no size at that point"
  );
}

#[test]
fn a_handle_wins_over_the_interior_it_overlaps() {
  // The cursor under a point and the press that follows it both resolve what
  // is under the pointer, so a point inside the region but within reach of a
  // handle has to come back as the same answer for both.
  let sel = Rect::new(100.0, 100.0, 100.0, 100.0);
  assert_eq!(hit_handle(sel, Point::new(150.0, 150.0), HANDLE_SLOP), None);
  assert_eq!(
    hit_handle(sel, Point::new(100.0, 150.0), HANDLE_SLOP),
    Some(Handle::Left),
    "a handle grabs the interior it overlaps"
  );
  assert!(
    !sel.contains(Point::new(90.0, 150.0)),
    "and nothing is held outside the region"
  );
}

#[test]
fn every_handle_is_hit_at_its_anchor() {
  let sel = Rect::new(10.0, 10.0, 100.0, 100.0);
  for &h in &HANDLES {
    let anchor = handle_anchor(sel, h);
    assert_eq!(hit_handle(sel, anchor, HANDLE_SLOP), Some(h));
  }
}

#[test]
fn resized_pins_the_opposite_corner() {
  let rect = Rect::new(10.0, 20.0, 30.0, 40.0);
  let grown = resized(rect, Handle::BottomRight, Point::new(80.0, 90.0));
  assert_eq!(grown, Rect::new(10.0, 20.0, 70.0, 70.0));
}

#[test]
fn resized_flips_when_dragged_across() {
  let rect = Rect::new(10.0, 10.0, 30.0, 30.0);
  let flipped = resized(rect, Handle::Right, Point::new(5.0, 99.0));
  assert_eq!(flipped, Rect::new(5.0, 10.0, 5.0, 30.0));
}

#[test]
fn moved_inside_keeps_the_region_on_screen() {
  let rect = Rect::new(1800.0, 1000.0, 300.0, 200.0);
  let moved = rect.moved_inside(SCREEN, Point::new(500.0, 500.0));
  assert!(moved.right() <= SCREEN.right());
  assert!(moved.bottom() <= SCREEN.bottom());
}

#[test]
fn inflated_grows_and_shrinks() {
  let rect = Rect::new(10.0, 10.0, 100.0, 100.0);
  let grown = rect.inflated(5.0);
  assert_eq!(grown, Rect::new(5.0, 5.0, 110.0, 110.0));
  let shrunk = rect.inflated(-5.0);
  assert_eq!(shrunk, Rect::new(15.0, 15.0, 90.0, 90.0));
}

#[test]
fn clamp_span_keeps_the_span_on_screen() {
  assert_eq!(clamp_span(2300.0, 300.0, 0.0, 1920.0), 1620.0);
  assert_eq!(clamp_span(-50.0, 300.0, 0.0, 1920.0), 0.0);
}

#[test]
fn union_ignores_a_rect_with_no_area() {
  // `Rect::ZERO` is how the chrome says "no button here", so joining it in
  // must not stretch the result down to the origin.
  let area = Rect::new(100.0, 200.0, 30.0, 40.0);
  assert_eq!(area.union(Rect::ZERO), area);
  assert_eq!(Rect::ZERO.union(area), area);
  assert_eq!(Rect::ZERO.union(Rect::ZERO), Rect::ZERO);
  assert_eq!(
    area.union(Rect::new(90.0, 210.0, 30.0, 40.0)),
    Rect::new(90.0, 200.0, 40.0, 50.0)
  );
}

#[test]
fn overlaps_touches_neither_its_edge_nor_an_empty_rect() {
  let area = Rect::new(10.0, 10.0, 20.0, 20.0);
  assert!(area.overlaps(Rect::new(25.0, 25.0, 10.0, 10.0)));
  assert!(
    !area.overlaps(Rect::new(30.0, 10.0, 10.0, 10.0)),
    "edge to edge"
  );
  assert!(!area.overlaps(Rect::ZERO), "an empty rect is nowhere");
  assert!(!Rect::ZERO.overlaps(area));
  assert!(area.contains_rect(Rect::new(12.0, 12.0, 4.0, 4.0)));
  assert!(area.contains_rect(area));
  assert!(!area.contains_rect(Rect::new(9.0, 10.0, 4.0, 4.0)));
  assert!(!area.contains_rect(Rect::ZERO));
}
