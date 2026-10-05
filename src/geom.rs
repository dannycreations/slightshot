#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Point {
  pub x: f32,
  pub y: f32,
}

impl Point {
  #[inline(always)]
  pub const fn new(x: f32, y: f32) -> Self {
    Self { x, y }
  }

  #[inline(always)]
  pub fn distance_squared(self, other: Point) -> f32 {
    let dx = self.x - other.x;
    let dy = self.y - other.y;
    dx * dx + dy * dy
  }

  #[inline(always)]
  pub fn clamped_inside(self, bounds: Rect) -> Point {
    Point::new(
      self.x.clamp(bounds.x, bounds.right()),
      self.y.clamp(bounds.y, bounds.bottom()),
    )
  }
}

#[inline(always)]
pub fn clamp_span(value: f32, len: f32, min: f32, max: f32) -> f32 {
  value.clamp(min, (max - len).max(min))
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rect {
  pub x: f32,
  pub y: f32,
  pub w: f32,
  pub h: f32,
}

impl Rect {
  pub const ZERO: Rect = Rect::new(0.0, 0.0, 0.0, 0.0);

  #[inline(always)]
  pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
    Self { x, y, w, h }
  }

  #[inline(always)]
  pub fn spanning(a: Point, b: Point) -> Self {
    Self::new(
      a.x.min(b.x),
      a.y.min(b.y),
      (a.x - b.x).abs(),
      (a.y - b.y).abs(),
    )
  }

  pub fn around(points: &[Point]) -> Self {
    let Some((first, rest)) = points.split_first() else {
      return Rect::ZERO;
    };
    let (mut left, mut top) = (first.x, first.y);
    let (mut right, mut bottom) = (first.x, first.y);
    for p in rest {
      left = left.min(p.x);
      top = top.min(p.y);
      right = right.max(p.x);
      bottom = bottom.max(p.y);
    }
    Rect::new(left, top, right - left, bottom - top)
  }

  #[inline(always)]
  pub fn inflated(self, margin: f32) -> Rect {
    Self::new(
      self.x - margin,
      self.y - margin,
      self.w + margin * 2.0,
      self.h + margin * 2.0,
    )
  }

  #[inline(always)]
  pub fn moved_inside(self, bounds: Rect, delta: Point) -> Rect {
    Self::new(self.x + delta.x, self.y + delta.y, self.w, self.h)
      .clamped_inside(bounds)
  }

  #[inline(always)]
  pub fn clamped_inside(self, bounds: Rect) -> Rect {
    let w = self.w.min(bounds.w);
    let h = self.h.min(bounds.h);
    let x = clamp_span(self.x, w, bounds.x, bounds.right());
    let y = clamp_span(self.y, h, bounds.y, bounds.bottom());
    Self::new(x, y, w, h)
  }

  #[inline(always)]
  pub fn right(self) -> f32 {
    self.x + self.w
  }

  #[inline(always)]
  pub fn bottom(self) -> f32 {
    self.y + self.h
  }

  #[inline(always)]
  pub fn center(self) -> Point {
    Point::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
  }

  #[inline(always)]
  pub fn contains(self, p: Point) -> bool {
    self.x <= p.x
      && p.x <= self.right()
      && self.y <= p.y
      && p.y <= self.bottom()
  }

  #[inline(always)]
  pub fn is_empty(self) -> bool {
    self.w <= 0.0 || self.h <= 0.0
  }

  pub fn union(self, other: Rect) -> Rect {
    if other.is_empty() {
      return self;
    }
    if self.is_empty() {
      return other;
    }
    let x = self.x.min(other.x);
    let y = self.y.min(other.y);
    let right = self.right().max(other.right());
    let bottom = self.bottom().max(other.bottom());
    Rect::new(x, y, right - x, bottom - y)
  }

  #[inline(always)]
  pub fn overlaps(self, other: Rect) -> bool {
    !self.is_empty()
      && !other.is_empty()
      && self.x < other.right()
      && other.x < self.right()
      && self.y < other.bottom()
      && other.y < self.bottom()
  }

  #[inline(always)]
  pub fn contains_rect(self, other: Rect) -> bool {
    !other.is_empty()
      && !self.is_empty()
      && self.x <= other.x
      && other.right() <= self.right()
      && self.y <= other.y
      && other.bottom() <= self.bottom()
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
  TopLeft,
  Top,
  TopRight,
  Left,
  Right,
  BottomLeft,
  Bottom,
  BottomRight,
}

pub const HANDLES: [Handle; 8] = [
  Handle::TopLeft,
  Handle::Top,
  Handle::TopRight,
  Handle::Left,
  Handle::Right,
  Handle::BottomLeft,
  Handle::Bottom,
  Handle::BottomRight,
];

#[inline(always)]
pub fn handle_anchor(rect: Rect, handle: Handle) -> Point {
  let (x, y) = match handle {
    Handle::TopLeft => (rect.x, rect.y),
    Handle::Top => (rect.x + rect.w * 0.5, rect.y),
    Handle::TopRight => (rect.right(), rect.y),
    Handle::Left => (rect.x, rect.y + rect.h * 0.5),
    Handle::Right => (rect.right(), rect.y + rect.h * 0.5),
    Handle::BottomLeft => (rect.x, rect.bottom()),
    Handle::Bottom => (rect.x + rect.w * 0.5, rect.bottom()),
    Handle::BottomRight => (rect.right(), rect.bottom()),
  };
  Point::new(x, y)
}

pub const HANDLE_SLOP: f32 = 7.0;

#[inline]
pub fn hit_handle(rect: Rect, p: Point, slop: f32) -> Option<Handle> {
  let slop_sq = slop * slop;
  HANDLES
    .into_iter()
    .find(|&handle| handle_anchor(rect, handle).distance_squared(p) <= slop_sq)
}

#[inline]
pub fn resized(rect: Rect, handle: Handle, target: Point) -> Rect {
  let (mut left, mut top, mut right, mut bottom) =
    (rect.x, rect.y, rect.right(), rect.bottom());
  match handle {
    Handle::TopLeft | Handle::Left | Handle::BottomLeft => left = target.x,
    Handle::TopRight | Handle::Right | Handle::BottomRight => right = target.x,
    Handle::Top | Handle::Bottom => {}
  }
  match handle {
    Handle::TopLeft | Handle::Top | Handle::TopRight => top = target.y,
    Handle::BottomLeft | Handle::Bottom | Handle::BottomRight => {
      bottom = target.y
    }
    Handle::Left | Handle::Right => {}
  }
  Rect::spanning(Point::new(left, top), Point::new(right, bottom))
}

#[cfg(test)]
mod tests {
  use super::*;

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
}
