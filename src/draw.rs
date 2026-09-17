use std::{cell::RefCell, collections::HashMap, sync::OnceLock};

use tiny_skia::{
  Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap,
  PixmapPaint, Rect, Shader, Stroke, StrokeDash, Transform,
};

use crate::geom::{Point, Rect as GeoRect};

#[inline(always)]
fn skia_rect(rect: GeoRect) -> Option<Rect> {
  Rect::from_xywh(rect.x, rect.y, rect.w, rect.h)
}

#[inline(always)]
fn paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
  Paint {
    anti_alias: true,
    shader: Shader::SolidColor(Color::from_rgba8(r, g, b, a)),
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

pub fn polyline(
  pm: &mut Pixmap,
  pts: &[Point],
  rgb: [u8; 3],
  width: f32,
  alpha: u8,
  offset: Point,
) {
  let Some(path) = smooth_path(pts) else {
    return;
  };
  let transform = if offset.x == 0.0 && offset.y == 0.0 {
    Transform::identity()
  } else {
    Transform::from_translate(-offset.x, -offset.y)
  };
  pm.stroke_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], alpha),
    &stroke(width),
    transform,
    None,
  );
}

fn smooth_path(points: &[Point]) -> Option<Path> {
  if points.len() < 2 {
    return None;
  }

  let mut builder = PathBuilder::new();
  builder.move_to(points[0].x, points[0].y);
  if points.len() == 2 {
    builder.line_to(points[1].x, points[1].y);
    return builder.finish();
  }

  const ONE_SIXTH: f32 = 1.0 / 6.0;
  let len = points.len();
  for i in 0..len - 1 {
    let p0 = points[i.saturating_sub(1)];
    let p1 = points[i];
    let p2 = points[i + 1];
    let p3 = points[(i + 2).min(len - 1)];

    let c1x = p1.x + (p2.x - p0.x) * ONE_SIXTH;
    let c1y = p1.y + (p2.y - p0.y) * ONE_SIXTH;
    let c2x = p2.x - (p3.x - p1.x) * ONE_SIXTH;
    let c2y = p2.y - (p3.y - p1.y) * ONE_SIXTH;

    builder.cubic_to(c1x, c1y, c2x, c2y, p2.x, p2.y);
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
      &paint(color[0], color[1], color[2], alpha),
      FillRule::Winding,
      Transform::identity(),
      None,
    );
  }
}

pub fn rect_stroke(
  pm: &mut Pixmap,
  rect: GeoRect,
  rgb: [u8; 3],
  width: f32,
  alpha: u8,
) {
  let Some(r) = skia_rect(rect) else {
    return;
  };
  let mut path = PathBuilder::new();
  path.push_rect(r);
  let Some(path) = path.finish() else {
    return;
  };
  pm.stroke_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], alpha),
    &stroke(width),
    Transform::identity(),
    None,
  );
}

#[inline(always)]
pub fn rect_fill(pm: &mut Pixmap, rect: GeoRect, rgb: [u8; 3], alpha: u8) {
  let Some(r) = skia_rect(rect) else {
    return;
  };
  pm.fill_rect(
    r,
    &paint(rgb[0], rgb[1], rgb[2], alpha),
    Transform::identity(),
    None,
  );
}

pub fn dashed_rect(pm: &mut Pixmap, rect: GeoRect, rgb: [u8; 3], width: f32) {
  let Some(r) = skia_rect(rect) else {
    return;
  };
  let mut path = PathBuilder::new();
  path.push_rect(r);
  let Some(path) = path.finish() else {
    return;
  };
  static DASH_STROKE: OnceLock<Stroke> = OnceLock::new();
  let base_stroke = DASH_STROKE.get_or_init(|| {
    let mut s = stroke(1.0);
    s.line_cap = LineCap::Butt;
    s.line_join = LineJoin::Miter;
    s.dash = StrokeDash::new(vec![3.0, 3.0], 0.0);
    s
  });
  let stroke = if width == 1.0 {
    base_stroke.clone()
  } else {
    let mut s = base_stroke.clone();
    s.width = width;
    s
  };
  pm.stroke_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], 255),
    &stroke,
    Transform::identity(),
    None,
  );
}

fn round_rect_path(r: Rect, radius: f32) -> Option<Path> {
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
  rect: GeoRect,
  radius: f32,
  rgb: [u8; 3],
  alpha: u8,
) {
  let Some(r) = Rect::from_xywh(0.0, 0.0, rect.w, rect.h) else {
    return;
  };
  let Some(path) = round_rect_path(r, radius) else {
    return;
  };
  pm.fill_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], alpha),
    FillRule::Winding,
    Transform::from_translate(rect.x, rect.y),
    None,
  );
}

pub fn rounded_stroke(
  pm: &mut Pixmap,
  rect: GeoRect,
  radius: f32,
  rgb: [u8; 3],
  width: f32,
  alpha: u8,
) {
  let Some(r) = Rect::from_xywh(0.0, 0.0, rect.w, rect.h) else {
    return;
  };
  let Some(path) = round_rect_path(r, radius) else {
    return;
  };
  pm.stroke_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], alpha),
    &stroke(width),
    Transform::from_translate(rect.x, rect.y),
    None,
  );
}

#[derive(Clone, Copy, Debug)]
pub enum Icon {
  Pen = 0,
  Marker,
  Arrow,
  Outline,
  Line,
  Letter,
  Undo,
  Upload,
  CopyImage,
  Save,
  Close,
}

impl Icon {
  pub fn paint(
    self,
    pm: &mut Pixmap,
    center: Point,
    box_size: f32,
    color: [u8; 3],
  ) {
    let x = (center.x - box_size * 0.5).round() as i32;
    let y = (center.y - box_size * 0.5).round() as i32;
    render_tinted_sprite(self, color, box_size, pm, x, y);
  }
}

type TintCache = HashMap<(usize, [u8; 3], u32), Pixmap>;

thread_local! {
  static TINT_CACHE: RefCell<TintCache> = RefCell::new(HashMap::new());
}

fn create_tinted_sprite(icon: Icon, color: [u8; 3], box_size: f32) -> Pixmap {
  let source = sprite(icon);
  let scale = box_size / source.width() as f32;
  let w = (source.width() as f32 * scale).round() as u32;
  let h = (source.height() as f32 * scale).round() as u32;
  let mut tinted =
    Pixmap::new(w, h).expect("allocating the icon pixmap failed");
  tinted.draw_pixmap(
    0,
    0,
    source.as_ref(),
    &PixmapPaint::default(),
    Transform::from_scale(scale, scale),
    None,
  );
  let (cr, cg, cb) = (color[0] as u32, color[1] as u32, color[2] as u32);
  for pixel in tinted.data_mut().as_chunks_mut::<4>().0 {
    let a = pixel[3] as u32;
    pixel[0] = (((cr * a + 128) * 257) >> 16) as u8;
    pixel[1] = (((cg * a + 128) * 257) >> 16) as u8;
    pixel[2] = (((cb * a + 128) * 257) >> 16) as u8;
  }
  tinted
}

fn render_tinted_sprite(
  icon: Icon,
  color: [u8; 3],
  box_size: f32,
  pm: &mut Pixmap,
  x: i32,
  y: i32,
) {
  TINT_CACHE.with(|cache| {
    let mut map = cache.borrow_mut();
    let key = (icon as usize, color, box_size.to_bits());
    let tinted = map
      .entry(key)
      .or_insert_with(|| create_tinted_sprite(icon, color, box_size));

    pm.draw_pixmap(
      x,
      y,
      tinted.as_ref(),
      &PixmapPaint::default(),
      Transform::identity(),
      None,
    );
  });
}

static SPRITE_CACHE: [OnceLock<Pixmap>; 11] = [const { OnceLock::new() }; 11];

fn sprite(icon: Icon) -> &'static Pixmap {
  SPRITE_CACHE[icon as usize].get_or_init(|| load_sprite(sprite_bytes(icon)))
}

fn sprite_bytes(icon: Icon) -> &'static [u8] {
  match icon {
    Icon::Pen => include_bytes!("icons/pencil.png"),
    Icon::Marker => include_bytes!("icons/highlighter.png"),
    Icon::Arrow => include_bytes!("icons/arrow-up-right.png"),
    Icon::Outline => include_bytes!("icons/square.png"),
    Icon::Line => include_bytes!("icons/minus.png"),
    Icon::Letter => include_bytes!("icons/type.png"),
    Icon::Undo => include_bytes!("icons/undo-2.png"),
    Icon::Upload => include_bytes!("icons/upload.png"),
    Icon::CopyImage => include_bytes!("icons/copy.png"),
    Icon::Save => include_bytes!("icons/save.png"),
    Icon::Close => include_bytes!("icons/close.png"),
  }
}

fn load_sprite(bytes: &'static [u8]) -> Pixmap {
  Pixmap::decode_png(bytes).expect("decoding the embedded icon PNG failed")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geom::Rect as GeoRect;

  #[test]
  fn polyline_changes_pixels_on_the_canvas() {
    let mut pm = Pixmap::new(20, 20).expect("alloc");
    polyline(
      &mut pm,
      &[Point::new(2.0, 2.0), Point::new(18.0, 18.0)],
      [255, 0, 0],
      2.0,
      255,
      Point::new(0.0, 0.0),
    );
    let painted = pm
      .data()
      .as_chunks::<4>()
      .0
      .iter()
      .any(|p| p[0] == 255 && p[3] == 255);
    assert!(painted, "expected at least one red, opaque pixel");
  }

  #[test]
  fn rounded_fill_paints_an_opaque_interior() {
    let mut pm = Pixmap::new(20, 20).expect("alloc");
    rounded_fill(
      &mut pm,
      GeoRect::new(4.0, 4.0, 12.0, 12.0),
      3.0,
      [0, 128, 255],
      255,
    );
    let center = &pm.data()[(10 * 20 + 10) as usize * 4..];
    assert_eq!(center[0], 0);
    assert_eq!(center[1], 128);
    assert_eq!(center[2], 255);
    assert_eq!(center[3], 255);
  }

  #[test]
  fn icon_renders_at_large_coordinates() {
    let w = 1920u32;
    let h = 1080u32;
    let mut pm = Pixmap::new(w, h).expect("alloc");
    let bx = 1485.0;
    let by = 285.0;
    rounded_fill(
      &mut pm,
      GeoRect::new(bx, by, 30.0, 30.0),
      5.0,
      [12, 12, 12],
      175,
    );
    Icon::Upload.paint(
      &mut pm,
      Point::new(bx + 15.0, by + 15.0),
      18.0,
      [240, 240, 240],
    );
    let mut lit = 0usize;
    for y in (by as usize)..(by as usize + 30) {
      for x in (bx as usize)..(bx as usize + 30) {
        if pm.data()[(y * w as usize + x) * 4] > 100 {
          lit += 1;
        }
      }
    }
    assert!(lit > 20, "icon missing at large coords: lit={lit}");
  }
}
