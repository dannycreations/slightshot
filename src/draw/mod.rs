mod icon;

#[cfg(test)]
mod mod_test;

use std::sync::OnceLock;

pub use icon::Icon;
use tiny_skia::{
  Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap,
  Rect as SkRect, Shader, Stroke, StrokeDash, Transform,
};

use crate::geom::{Point, Rect};

#[inline(always)]
fn skia_rect(rect: Rect) -> Option<SkRect> {
  SkRect::from_xywh(rect.x, rect.y, rect.w, rect.h)
}

#[inline(always)]
fn paint(rgb: [u8; 3], alpha: u8) -> Paint<'static> {
  Paint {
    anti_alias: true,
    shader: Shader::SolidColor(Color::from_rgba8(
      rgb[0], rgb[1], rgb[2], alpha,
    )),
    ..Paint::default()
  }
}

#[inline(always)]
fn stroke(width: f32) -> Stroke {
  Stroke {
    width,
    line_cap: LineCap::Round,
    line_join: LineJoin::Round,
    ..Stroke::default()
  }
}

#[inline]
fn rect_path(rect: Rect) -> Option<Path> {
  let r = skia_rect(rect)?;
  let mut path = PathBuilder::new();
  path.push_rect(r);
  path.finish()
}

pub fn polyline(
  pm: &mut Pixmap,
  pts: &[Point],
  rgb: [u8; 3],
  width: f32,
  alpha: u8,
) {
  let Some(path) = straight_path(pts) else {
    return;
  };
  pm.stroke_path(
    &path,
    &paint(rgb, alpha),
    &stroke(width),
    Transform::identity(),
    None,
  );
}

fn straight_path(points: &[Point]) -> Option<Path> {
  let (first, rest) = points.split_first()?;
  if rest.is_empty() {
    return None;
  }
  let mut builder = PathBuilder::new();
  builder.move_to(first.x, first.y);
  for p in rest {
    builder.line_to(p.x, p.y);
  }
  builder.finish()
}

pub fn arrow_head(
  pm: &mut Pixmap,
  tail: Point,
  head: Point,
  size: f32,
  color: [u8; 3],
  alpha: u8,
) {
  let (dx, dy) = (head.x - tail.x, head.y - tail.y);
  let length = dx.hypot(dy);
  if length <= f32::EPSILON {
    return;
  }
  let inv_len = 1.0 / length;
  let (ux, uy) = (dx * inv_len, dy * inv_len);
  let spread = size * 0.45;
  let ux_size = ux * size;
  let uy_size = uy * size;
  let ux_spread = ux * spread;
  let uy_spread = uy * spread;

  let base_x = head.x - ux_size - uy_spread;
  let base_y = head.y - uy_size + ux_spread;
  let tip_x = head.x - ux_size + uy_spread;
  let tip_y = head.y - uy_size - ux_spread;

  let mut builder = PathBuilder::new();
  builder.move_to(head.x, head.y);
  builder.line_to(base_x, base_y);
  builder.line_to(tip_x, tip_y);
  builder.close();
  if let Some(path) = builder.finish() {
    pm.fill_path(
      &path,
      &paint(color, alpha),
      FillRule::Winding,
      Transform::identity(),
      None,
    );
  }
}

pub fn rect_stroke(
  pm: &mut Pixmap,
  rect: Rect,
  rgb: [u8; 3],
  width: f32,
  alpha: u8,
) {
  let Some(path) = rect_path(rect) else {
    return;
  };
  pm.stroke_path(
    &path,
    &paint(rgb, alpha),
    &stroke(width),
    Transform::identity(),
    None,
  );
}

#[inline(always)]
pub fn rect_fill(pm: &mut Pixmap, rect: Rect, rgb: [u8; 3], alpha: u8) {
  let Some(r) = skia_rect(rect) else {
    return;
  };
  pm.fill_rect(r, &paint(rgb, alpha), Transform::identity(), None);
}

pub fn dashed_rect(pm: &mut Pixmap, rect: Rect, rgb: [u8; 3]) {
  let Some(path) = rect_path(rect) else {
    return;
  };
  static DASH_STROKE: OnceLock<Stroke> = OnceLock::new();
  let dash = DASH_STROKE.get_or_init(|| Stroke {
    line_cap: LineCap::Butt,
    line_join: LineJoin::Miter,
    dash: StrokeDash::new(vec![3.0, 3.0], 0.0),
    ..stroke(1.0)
  });
  pm.stroke_path(&path, &paint(rgb, 255), dash, Transform::identity(), None);
}

fn round_rect_path(rect: Rect, radius: f32) -> Option<Path> {
  let r = skia_rect(rect)?;
  let rr = radius.min(r.width() * 0.5).min(r.height() * 0.5);
  let mut path = PathBuilder::new();
  path.move_to(r.x() + rr, r.y());
  path.line_to(r.right() - rr, r.y());
  path.quad_to(r.right(), r.y(), r.right(), r.y() + rr);
  path.line_to(r.right(), r.bottom() - rr);
  path.quad_to(r.right(), r.bottom(), r.right() - rr, r.bottom());
  path.line_to(r.x() + rr, r.bottom());
  path.quad_to(r.x(), r.bottom(), r.x(), r.bottom() - rr);
  path.line_to(r.x(), r.y() + rr);
  path.quad_to(r.x(), r.y(), r.x() + rr, r.y());
  path.close();
  path.finish()
}

pub fn rounded_fill(
  pm: &mut Pixmap,
  rect: Rect,
  radius: f32,
  rgb: [u8; 3],
  alpha: u8,
) {
  let Some(path) = round_rect_path(rect, radius) else {
    return;
  };
  pm.fill_path(
    &path,
    &paint(rgb, alpha),
    FillRule::Winding,
    Transform::identity(),
    None,
  );
}

pub fn rounded_stroke(
  pm: &mut Pixmap,
  rect: Rect,
  radius: f32,
  rgb: [u8; 3],
  width: f32,
  alpha: u8,
) {
  let Some(path) = round_rect_path(rect, radius) else {
    return;
  };
  pm.stroke_path(
    &path,
    &paint(rgb, alpha),
    &stroke(width),
    Transform::identity(),
    None,
  );
}
