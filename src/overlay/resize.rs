use winit::window::CursorIcon;

use crate::{
  annotate::{MAX_SIZE, MIN_SIZE},
  geom::{Handle, Point, Rect},
  text::TextEngine,
};

#[cfg(test)]
#[path = "resize_test.rs"]
mod resize_test;

pub(super) fn resize_text(
  engine: &TextEngine,
  text: &str,
  origin: Rect,
  handle: Handle,
  target: Point,
) -> (Point, f32) {
  if origin.is_empty() {
    return (Point::new(origin.x, origin.y), origin.h);
  }
  let width = match handle {
    Handle::TopLeft | Handle::Left | Handle::BottomLeft => {
      origin.right() - target.x
    }
    Handle::TopRight | Handle::Right | Handle::BottomRight => {
      target.x - origin.x
    }
    Handle::Top | Handle::Bottom => origin.w,
  };
  let height = match handle {
    Handle::TopLeft | Handle::Top | Handle::TopRight => {
      origin.bottom() - target.y
    }
    Handle::BottomLeft | Handle::Bottom | Handle::BottomRight => {
      target.y - origin.y
    }
    Handle::Left | Handle::Right => origin.h,
  };
  let (across, down) = (width / origin.w, height / origin.h);
  let factor = match handle {
    Handle::Left | Handle::Right => across,
    Handle::Top | Handle::Bottom => down,
    _ if (across - 1.0).abs() >= (down - 1.0).abs() => across,
    _ => down,
  };
  let size = (origin.h * factor).clamp(MIN_SIZE, MAX_SIZE);
  let anchor = fixed_corner(origin, handle, engine.width(text, size), size);
  (anchor, size)
}

fn fixed_corner(
  origin: Rect,
  handle: Handle,
  width: f32,
  height: f32,
) -> Point {
  let (x, y) = match handle {
    Handle::TopLeft => (origin.right() - width, origin.bottom() - height),
    Handle::Top | Handle::TopRight => (origin.x, origin.bottom() - height),
    Handle::Left | Handle::BottomLeft => (origin.right() - width, origin.y),
    Handle::Right | Handle::Bottom | Handle::BottomRight => {
      (origin.x, origin.y)
    }
  };
  Point::new(x, y)
}

pub(super) fn resize_cursor(handle: Handle) -> CursorIcon {
  match handle {
    Handle::TopLeft | Handle::BottomRight => CursorIcon::NwseResize,
    Handle::BottomLeft | Handle::TopRight => CursorIcon::NeswResize,
    Handle::Top | Handle::Bottom => CursorIcon::NsResize,
    Handle::Left | Handle::Right => CursorIcon::EwResize,
  }
}
