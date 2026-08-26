#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Point {
  pub x: f32,
  pub y: f32,
}

impl Point {
  pub fn new(x: f32, y: f32) -> Self {
    Self { x, y }
  }

  pub fn distance(self, other: Point) -> f32 {
    (self.x - other.x).hypot(self.y - other.y)
  }

  pub fn clamped_inside(self, bounds: Rect) -> Point {
    Point::new(
      self.x.clamp(bounds.x, bounds.right()),
      self.y.clamp(bounds.y, bounds.bottom()),
    )
  }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
  pub x: f32,
  pub y: f32,
  pub w: f32,
  pub h: f32,
}

impl Rect {
  pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
    Self { x, y, w, h }
  }

  pub fn spanning(a: Point, b: Point) -> Self {
    Self::new(
      a.x.min(b.x),
      a.y.min(b.y),
      (a.x - b.x).abs(),
      (a.y - b.y).abs(),
    )
  }

  pub fn translated(self, dx: f32, dy: f32) -> Rect {
    Self::new(self.x + dx, self.y + dy, self.w, self.h)
  }

  pub fn inflated(self, margin: f32) -> Rect {
    Self::new(
      self.x - margin,
      self.y - margin,
      self.w + margin * 2.0,
      self.h + margin * 2.0,
    )
  }

  pub fn moved_inside(self, bounds: Rect, delta: Point) -> Rect {
    Self::new(self.x + delta.x, self.y + delta.y, self.w, self.h)
      .clamped_inside(bounds)
  }

  pub fn clamped_inside(self, bounds: Rect) -> Rect {
    let w = self.w.min(bounds.w);
    let h = self.h.min(bounds.h);
    let x = self.x.clamp(bounds.x, bounds.right() - w);
    let y = self.y.clamp(bounds.y, bounds.bottom() - h);
    Self::new(x, y, w, h)
  }

  pub fn right(self) -> f32 {
    self.x + self.w
  }

  pub fn bottom(self) -> f32 {
    self.y + self.h
  }

  pub fn center(self) -> Point {
    Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
  }

  pub fn contains(self, p: Point) -> bool {
    self.x <= p.x
      && p.x <= self.right()
      && self.y <= p.y
      && p.y <= self.bottom()
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

pub fn handle_anchor(rect: Rect, handle: Handle) -> Point {
  let (x, y) = match handle {
    Handle::TopLeft => (rect.x, rect.y),
    Handle::Top => (rect.x + rect.w / 2.0, rect.y),
    Handle::TopRight => (rect.right(), rect.y),
    Handle::Left => (rect.x, rect.y + rect.h / 2.0),
    Handle::Right => (rect.right(), rect.y + rect.h / 2.0),
    Handle::BottomLeft => (rect.x, rect.bottom()),
    Handle::Bottom => (rect.x + rect.w / 2.0, rect.bottom()),
    Handle::BottomRight => (rect.right(), rect.bottom()),
  };
  Point::new(x, y)
}

pub fn hit_handle(rect: Rect, p: Point, slop: f32) -> Option<Handle> {
  HANDLES
    .iter()
    .copied()
    .find(|&h| handle_anchor(rect, h).distance(p) <= slop)
}

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
}
