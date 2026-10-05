mod chrome;
mod damage;
mod frame;
mod surface;

#[cfg(test)]
mod mod_test;

use chrome::draw_panels;
pub use chrome::{build, hotspot_at, Chrome, Command, Hotspot};
use damage::{bounding, inside, panels_area, picked_area, touches};
pub use damage::{shape_area, typed_area, Shown};
use frame::{
  badge_label, badge_rect, draw_badge, draw_handles, handle_boxes,
  outline_boxes, EDGE,
};
use surface::{copy_region, dim_region_into, ink, region_pixels};
pub use surface::{dimmed_into, flatten, ink_segment};
use tiny_skia::Pixmap;

use crate::{
  annotate::Shape,
  draw,
  geom::{Point, Rect},
  text::TextEngine,
};

const MIN_REGION: f32 = 6.0;
pub(super) const CARET_WIDTH: f32 = 1.5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backdrop {
  Frozen,
  Live,
}

impl Backdrop {
  pub fn picks_region(self) -> bool {
    matches!(self, Backdrop::Frozen)
  }
}

#[inline(always)]
pub fn deliverable_region(selection: Option<Rect>) -> Option<Rect> {
  selection.filter(|sel| sel.w >= MIN_REGION && sel.h >= MIN_REGION)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Run<'a> {
  pub at: Point,
  pub text: &'a str,
  pub size: f32,
  pub color: [u8; 3],
}

pub struct Scene<'a> {
  pub backdrop: &'a Pixmap,
  pub canvas: &'a Pixmap,
  pub bounds: Rect,
  pub selection: Option<Rect>,
  pub picked: Option<Rect>,
  pub kind: Backdrop,
  pub draft: Option<&'a Shape>,
  pub typing: Option<Run<'a>>,
  pub caret: bool,
  pub palette_index: usize,
  pub chrome: &'a Chrome,
  pub hotspot: Option<Hotspot>,
  pub text: &'a TextEngine,
  pub hint: Option<&'a str>,
}

pub fn repaint(
  canvas: &mut Pixmap,
  backdrop: &mut Pixmap,
  base: &Pixmap,
  shapes: &[Shape],
  area: Rect,
  engine: &TextEngine,
) -> Rect {
  let boxes: Vec<Rect> = shapes
    .iter()
    .map(|shape| shape_area(Some(shape), engine))
    .collect();
  let mut region = area;
  loop {
    let grown = shapes.iter().zip(&boxes).fold(
      region,
      |region, (shape, box_)| match box_.overlaps(region) && shape.blends() {
        true => region.union(*box_),
        false => region,
      },
    );
    if grown == region {
      break;
    }
    region = grown;
  }

  let (x, y, width, height) =
    region_pixels(region, canvas.width(), canvas.height());
  if width == 0 || height == 0 {
    return Rect::ZERO;
  }
  let region = Rect::new(x as f32, y as f32, width as f32, height as f32);
  copy_region(canvas, base, (x, y), (x, y), (width, height));
  for (shape, box_) in shapes.iter().zip(&boxes) {
    if box_.overlaps(region) {
      ink(canvas, shape, engine);
    }
  }
  dim_region_into(backdrop, canvas, region);
  region
}

pub fn paint(pm: &mut Pixmap, scene: &Scene, damage: Rect) {
  let engine = scene.text;
  let region = scene.selection.filter(|_| scene.kind.picks_region());
  let outline = region.map(outline_boxes).unwrap_or([Rect::ZERO; 4]);
  let handles = region.map(handle_boxes).unwrap_or([Rect::ZERO; 8]);
  let plate = region.map(|sel| badge_rect(sel, scene.bounds, engine));
  let badge = plate.map_or(Rect::ZERO, |plate| plate.inflated(EDGE));
  let draft = shape_area(scene.draft, engine);
  let typing = typed_area(scene.typing, engine);
  let picked = picked_area(scene.picked);
  let panels = panels_area(
    scene.chrome,
    scene.hotspot,
    scene.hint,
    scene.bounds,
    engine,
  );

  // Damage grows to whole pieces: each group is laid down in a single go,
  // so a box that has reached any part of one has to carry all of it. Carrying
  // one group can reach the next, so this settles rather than settling once:
  // anything the blit below clips is then either wholly outside the area or
  // wholly inside it, which is what the redraw guards below ask for.
  let mut area = damage;
  loop {
    let mut grown = area;
    if touches(&outline, area) {
      grown = grown.union(bounding(&outline));
    }
    if touches(&handles, area) {
      grown = grown.union(bounding(&handles));
    }
    for piece in [badge, draft, typing, picked, panels] {
      if grown.overlaps(piece) {
        grown = grown.union(piece);
      }
    }
    if grown == area {
      break;
    }
    area = grown;
  }

  let (x0, y0, width, height) = region_pixels(area, pm.width(), pm.height());
  if width == 0 || height == 0 {
    return;
  }
  let at = (x0, y0);
  copy_region(pm, scene.backdrop, at, at, (width, height));
  if let Some(sel) = scene.selection {
    let (sx, sy, sw, sh) = region_pixels(sel, pm.width(), pm.height());
    let left = x0.max(sx);
    let top = y0.max(sy);
    let right = (x0 + width).min(sx + sw);
    let bottom = (y0 + height).min(sy + sh);
    if right > left && bottom > top {
      let inner = (left, top);
      copy_region(pm, scene.canvas, inner, inner, (right - left, bottom - top));
    }
  }
  if let Some(sel) = region {
    if inside(&outline, area) {
      draw::dashed_rect(pm, sel, [255, 255, 255]);
    }
    if inside(&handles, area) {
      draw_handles(pm, sel);
    }
  }
  if let (Some(sel), Some(plate)) = (region, plate) {
    if area.contains_rect(badge) {
      draw_badge(pm, plate, &badge_label(sel), engine);
    }
  }
  if area.contains_rect(draft) {
    if let Some(shape) = scene.draft {
      ink(pm, shape, engine);
    }
  }
  if area.contains_rect(typing) {
    if let Some(run) = scene.typing {
      engine.draw(pm, run.text, run.at.x, run.at.y, run.size, run.color);
      // The caret is dark for the half of the blink it is not lit for, which
      // is the same as not painting it: the text underneath stays.
      if scene.caret {
        let x = run.at.x + engine.width(run.text, run.size);
        draw::polyline(
          pm,
          &[Point::new(x, run.at.y), Point::new(x, run.at.y + run.size)],
          [255, 255, 255],
          CARET_WIDTH,
          255,
        );
      }
    }
  }
  if area.contains_rect(picked) {
    if let Some(box_) = scene.picked {
      draw::dashed_rect(pm, box_, [255, 255, 255]);
      draw_handles(pm, box_);
    }
  }
  if area.contains_rect(panels) {
    draw_panels(pm, scene);
  }
}
