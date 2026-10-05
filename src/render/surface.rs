use tiny_skia::Pixmap;

use crate::{
  action::Shot,
  annotate::{stroke_alpha, Segment, Shape},
  draw,
  geom::{Point, Rect},
  text::TextEngine,
};

#[cfg(test)]
#[path = "surface_test.rs"]
mod surface_test;

const DIM_ALPHA: u8 = 105;

const DIM_LUT: [u8; 256] = {
  let keep = 255 - DIM_ALPHA as u32;
  let mut lut = [0u8; 256];
  let mut i = 0;
  while i < 256 {
    lut[i] = ((i as u32 * keep + 127) / 255) as u8;
    i += 1;
  }
  lut
};

pub fn dimmed_into(dst: &mut Pixmap, src: &Pixmap) {
  let width = src.width().min(dst.width());
  let height = src.height().min(dst.height());
  let whole = Rect::new(0.0, 0.0, width as f32, height as f32);
  dim_region_into(dst, src, whole);
}

pub(super) fn dim_region_into(dst: &mut Pixmap, src: &Pixmap, rect: Rect) {
  let (x0, y0, width, height) = region_pixels(
    rect,
    src.width().min(dst.width()),
    src.height().min(dst.height()),
  );
  if width == 0 || height == 0 {
    return;
  }
  let row_bytes = width as usize * 4;
  let (src_stride, dst_stride) =
    (src.width() as usize * 4, dst.width() as usize * 4);
  let (src_data, dst_data) = (src.data(), dst.data_mut());
  for row in 0..height as usize {
    let y = y0 as usize + row;
    let from = y * src_stride + x0 as usize * 4;
    let to = y * dst_stride + x0 as usize * 4;
    let (src_pixels, dst_pixels) = (
      &src_data[from..from + row_bytes],
      &mut dst_data[to..to + row_bytes],
    );
    for (src_px, dst_px) in src_pixels
      .as_chunks::<4>()
      .0
      .iter()
      .zip(dst_pixels.as_chunks_mut::<4>().0)
    {
      dim_pixel(dst_px, src_px);
    }
  }
}

#[inline(always)]
fn dim_pixel(dst: &mut [u8], src: &[u8]) {
  dst[0] = DIM_LUT[src[0] as usize];
  dst[1] = DIM_LUT[src[1] as usize];
  dst[2] = DIM_LUT[src[2] as usize];
  dst[3] = src[3];
}

pub fn ink_segment(
  canvas: &mut Pixmap,
  backdrop: &mut Pixmap,
  segment: Segment,
) {
  draw::polyline(
    canvas,
    &[segment.from, segment.to],
    segment.color,
    segment.width,
    segment.alpha,
  );
  dim_region_into(backdrop, canvas, segment.bounds());
}

pub(super) fn copy_region(
  dest: &mut Pixmap,
  source: &Pixmap,
  from: (u32, u32),
  to: (u32, u32),
  size: (u32, u32),
) {
  let (from_x, from_y) = from;
  let (to_x, to_y) = to;
  let (width, height) = size;
  let row_bytes = width as usize * 4;
  let src_stride = source.width() as usize * 4;
  let dst_stride = dest.width() as usize * 4;
  let (src, dst) = (source.data(), dest.data_mut());
  let src_row = from_y as usize * src_stride + from_x as usize * 4;
  let dst_row = to_y as usize * dst_stride + to_x as usize * 4;

  // Whole rows in both buffers abut, so every row of the box follows the
  // last one and the box is one contiguous run. The column offsets are
  // already baked into `src_row` and `dst_row`.
  if to_x == 0 && row_bytes == src_stride && row_bytes == dst_stride {
    let total = row_bytes * height as usize;
    dst[dst_row..dst_row + total]
      .copy_from_slice(&src[src_row..src_row + total]);
    return;
  }

  for row in 0..height as usize {
    let s = src_row + row * src_stride;
    let d = dst_row + row * dst_stride;
    dst[d..d + row_bytes].copy_from_slice(&src[s..s + row_bytes]);
  }
}

#[inline(always)]
pub(super) fn region_pixels(
  sel: Rect,
  px_w: u32,
  px_h: u32,
) -> (u32, u32, u32, u32) {
  let x0 = (sel.x.floor() as u32).min(px_w);
  let y0 = (sel.y.floor() as u32).min(px_h);
  let width = ((sel.right().ceil() as u32).saturating_sub(x0)).min(px_w - x0);
  let height = ((sel.bottom().ceil() as u32).saturating_sub(y0)).min(px_h - y0);
  (x0, y0, width, height)
}

pub fn flatten(frame: &Pixmap, sel: Rect) -> Shot {
  let (x0, y0, width, height) =
    region_pixels(sel, frame.width(), frame.height());
  if width == 0 || height == 0 {
    return Shot::empty();
  }
  let Some(mut layer) = Pixmap::new(width, height) else {
    return Shot::empty();
  };
  copy_region(&mut layer, frame, (x0, y0), (0, 0), (width, height));
  Shot {
    width,
    height,
    rgba: layer.take(),
  }
}

pub(super) fn ink(pm: &mut Pixmap, shape: &Shape, engine: &TextEngine) {
  match shape {
    Shape::Stroke {
      points,
      color,
      width,
      marker,
    } => {
      draw::polyline(pm, points, *color, *width, stroke_alpha(*marker));
    }
    Shape::Line {
      from,
      to,
      color,
      width,
      arrow,
    } => {
      // An arrow stops its shaft where the head begins, so the two meet
      // without the head overlapping the line. A plain line runs the full
      // distance, which is that same shaft with nothing pulled back.
      let size = if *arrow { (*width * 3.5).max(6.0) } else { 0.0 };
      let shaft_end = arrow_base(*from, *to, size);
      draw::polyline(pm, &[*from, shaft_end], *color, *width, 255);
      if *arrow {
        draw::arrow_head(pm, *from, *to, size, *color, 255);
      }
    }
    Shape::Outline { rect, color, width } => {
      draw::rect_stroke(pm, *rect, *color, *width, 255);
    }
    Shape::Text {
      at,
      text,
      color,
      size,
    } => {
      engine.draw(pm, text, at.x, at.y, *size, *color);
    }
  }
}

fn arrow_base(from: Point, to: Point, head_size: f32) -> Point {
  let (dx, dy) = (to.x - from.x, to.y - from.y);
  let len = dx.hypot(dy);
  if len <= 0.0 {
    return to;
  }
  let back = head_size.min(len) / len;
  Point::new(to.x - dx * back, to.y - dy * back)
}
