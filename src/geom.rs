#[cfg(test)]
#[path = "geom_test.rs"]
mod geom_test;

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
