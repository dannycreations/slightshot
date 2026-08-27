use std::sync::OnceLock;

use tiny_skia::{
  Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap,
  PixmapPaint, Rect, Shader, Stroke, StrokeDash, Transform,
};

use crate::geom::{Point, Rect as GeoRect};

fn paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
  Paint {
    anti_alias: true,
    shader: Shader::SolidColor(Color::from_rgba8(r, g, b, a)),
    ..Paint::default()
  }
}

fn stroke(width: f32, dash: Option<f32>) -> Stroke {
  Stroke {
    width,
    line_cap: LineCap::Round,
    line_join: LineJoin::Round,
    dash: dash.and_then(|length| StrokeDash::new(vec![length, length], 0.0)),
    ..Stroke::default()
  }
}

pub fn polyline(
  pm: &mut Pixmap,
  pts: &[Point],
  rgb: [u8; 3],
  width: f32,
  alpha: u8,
) {
  let Some(path) = smooth_path(pts) else {
    return;
  };
  pm.stroke_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], alpha),
    &stroke(width, None),
    Transform::identity(),
    None,
  );
}

// Samples per span. A fast stroke is captured with few, far-apart points;
// subdividing each span keeps the stroked curve smooth instead of polygonal.
const SMOOTH_STEPS: usize = 16;

fn smooth_path(points: &[Point]) -> Option<Path> {
  if points.len() < 2 {
    return None;
  }
  let mut builder = PathBuilder::new();
  builder.move_to(points[0].x, points[0].y);
  for i in 0..points.len() - 1 {
    let prev = points[i.saturating_sub(1)];
    let curr = points[i];
    let next = points[i + 1];
    let after = points[(i + 2).min(points.len() - 1)];
    for step in 1..=SMOOTH_STEPS {
      let t = step as f32 / SMOOTH_STEPS as f32;
      builder.line_to(
        catmull_rom(prev.x, curr.x, next.x, after.x, t),
        catmull_rom(prev.y, curr.y, next.y, after.y, t),
      );
    }
  }
  builder.finish()
}

fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
  let t2 = t * t;
  let t3 = t2 * t;
  0.5
    * ((2.0 * p1)
      + (-p0 + p2) * t
      + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
      + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

fn open_builder(points: &[Point]) -> Option<PathBuilder> {
  let first = points.first()?;
  let mut builder = PathBuilder::new();
  builder.move_to(first.x, first.y);
  for p in &points[1..] {
    builder.line_to(p.x, p.y);
  }
  Some(builder)
}

fn closed_path(points: &[Point]) -> Option<Path> {
  let mut builder = open_builder(points)?;
  builder.close();
  builder.finish()
}

fn corners(rect: GeoRect) -> [Point; 5] {
  [
    Point::new(rect.x, rect.y),
    Point::new(rect.right(), rect.y),
    Point::new(rect.right(), rect.bottom()),
    Point::new(rect.x, rect.bottom()),
    Point::new(rect.x, rect.y),
  ]
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
  let (ux, uy) = (dx / length, dy / length);
  let spread = size * 0.45;
  let base = [
    head.x - ux * size - uy * spread,
    head.y - uy * size + ux * spread,
  ];
  let tip = [
    head.x - ux * size + uy * spread,
    head.y - uy * size - ux * spread,
  ];
  polygon_fill(
    pm,
    &[
      head,
      Point::new(base[0], base[1]),
      Point::new(tip[0], tip[1]),
    ],
    color,
    alpha,
  );
}

fn polygon_fill(pm: &mut Pixmap, points: &[Point], color: [u8; 3], alpha: u8) {
  let Some(path) = closed_path(points) else {
    return;
  };
  pm.fill_path(
    &path,
    &paint(color[0], color[1], color[2], alpha),
    FillRule::Winding,
    Transform::identity(),
    None,
  );
}

pub fn rect_stroke(
  pm: &mut Pixmap,
  rect: GeoRect,
  rgb: [u8; 3],
  width: f32,
  alpha: u8,
) {
  let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
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
    &stroke(width, None),
    Transform::identity(),
    None,
  );
}

pub fn rect_fill(pm: &mut Pixmap, rect: GeoRect, rgb: [u8; 3], alpha: u8) {
  let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
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
  let Some(path) = closed_path(&corners(rect)) else {
    return;
  };
  let mut stroke = stroke(width, None);
  stroke.line_cap = LineCap::Butt;
  stroke.line_join = LineJoin::Miter;
  stroke.dash = StrokeDash::new(vec![3.0, 3.0], 0.0);
  pm.stroke_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], 255),
    &stroke,
    Transform::identity(),
    None,
  );
}

fn round_rect_path(r: Rect, radius: f32) -> Option<Path> {
  let rr = radius.min(r.width() / 2.0).min(r.height() / 2.0);
  let mut path = PathBuilder::new();
  path.move_to(r.x() + rr, r.y());
  path.line_to(r.right() - rr, r.y());
  path.quad_to(r.right() - rr, r.y(), r.right(), r.y() + rr);
  path.line_to(r.right(), r.bottom() - rr);
  path.quad_to(r.right(), r.bottom() - rr, r.right() - rr, r.bottom());
  path.line_to(r.x() + rr, r.bottom());
  path.quad_to(r.x() + rr, r.bottom(), r.x(), r.bottom() - rr);
  path.line_to(r.x(), r.y() + rr);
  path.quad_to(r.x(), r.y() + rr, r.x() + rr, r.y());
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
  let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
    return;
  };
  let Some(path) = round_rect_path(r, radius) else {
    return;
  };
  pm.fill_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], alpha),
    FillRule::Winding,
    Transform::identity(),
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
  let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
    return;
  };
  let Some(path) = round_rect_path(r, radius) else {
    return;
  };
  pm.stroke_path(
    &path,
    &paint(rgb[0], rgb[1], rgb[2], alpha),
    &stroke(width, None),
    Transform::identity(),
    None,
  );
}

#[derive(Clone, Copy, Debug)]
pub enum Icon {
  Pen,
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
    let sprite = sprite(self);
    let scale = box_size / sprite.width() as f32;
    let w = (sprite.width() as f32 * scale).round() as u32;
    let h = (sprite.height() as f32 * scale).round() as u32;
    let x = (center.x - box_size / 2.0).round() as i32;
    let y = (center.y - box_size / 2.0).round() as i32;

    let mut tinted =
      Pixmap::new(w, h).expect("allocating the icon pixmap failed");
    tinted.draw_pixmap(
      0,
      0,
      sprite.as_ref(),
      &PixmapPaint::default(),
      Transform::from_scale(scale, scale),
      None,
    );
    for pixel in tinted.data_mut().chunks_mut(4) {
      let a = pixel[3] as u32;
      pixel[0] = ((color[0] as u32 * a) / 255) as u8;
      pixel[1] = ((color[1] as u32 * a) / 255) as u8;
      pixel[2] = ((color[2] as u32 * a) / 255) as u8;
    }

    pm.draw_pixmap(
      x,
      y,
      tinted.as_ref(),
      &PixmapPaint::default(),
      Transform::identity(),
      None,
    );
  }
}

const ICONS: [Icon; 11] = [
  Icon::Pen,
  Icon::Marker,
  Icon::Arrow,
  Icon::Outline,
  Icon::Line,
  Icon::Letter,
  Icon::Undo,
  Icon::Upload,
  Icon::CopyImage,
  Icon::Save,
  Icon::Close,
];

fn sprite(icon: Icon) -> &'static Pixmap {
  static CACHE: OnceLock<Vec<Pixmap>> = OnceLock::new();
  let cache = CACHE.get_or_init(|| {
    ICONS
      .iter()
      .map(|&variant| load_sprite(sprite_bytes(variant)))
      .collect()
  });
  &cache[icon as usize]
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

  #[test]
  fn every_icon_paints_something() {
    for icon in ICONS {
      let mut pm = Pixmap::new(24, 24).expect("alloc");
      icon.paint(&mut pm, Point::new(12.0, 12.0), 18.0, [240, 240, 240]);
      assert!(
        pm.data().iter().any(|&c| c > 0),
        "icon {:?} left the canvas empty",
        icon
      );
    }
  }
}
