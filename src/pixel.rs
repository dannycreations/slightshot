use std::slice;

use crate::geom::Rect;

#[cfg(test)]
#[path = "pixel_test.rs"]
mod pixel_test;

pub fn swap_channels(src: &[u8], dst: &mut [u8]) {
  reorder::<false>(src, dst)
}

pub fn swap_channels_region<const KEEP_ALPHA: bool>(
  src: &[u8],
  width: u32,
  dst: &mut [u8],
  area: Rect,
) {
  let stride = width as usize * 4;
  let wanted = (area.w as usize * 4).min(src.len()).min(dst.len()) & !3;
  if wanted == 0 {
    return;
  }
  for row in area.y as usize..(area.y as usize + area.h as usize) {
    let from = row * stride + area.x as usize * 4;
    let bytes = wanted
      .min(src.len().saturating_sub(from))
      .min(dst.len().saturating_sub(from));
    reorder::<KEEP_ALPHA>(
      &src[from..from + bytes],
      &mut dst[from..from + bytes],
    );
  }
}

fn reorder<const KEEP_ALPHA: bool>(src: &[u8], dst: &mut [u8]) {
  let total_bytes = src.len().min(dst.len()) & !3;
  for (out, chunk) in dst[..total_bytes]
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .zip(src[..total_bytes].as_chunks::<4>().0)
  {
    let px = u32::from_ne_bytes(*chunk);
    let alpha = if KEEP_ALPHA {
      px & 0xFF00_0000
    } else {
      0xFF00_0000
    };
    let swapped = ((px & 0x00FF_0000) >> 16)
      | ((px & 0x0000_00FF) << 16)
      | (px & 0x0000_FF00)
      | alpha;
    *out = swapped.to_ne_bytes();
  }
}

pub fn swap_words_region(src: &[u8], width: u32, dst: &mut [u32], area: Rect) {
  // SAFETY: a `u32` is 4 bytes with no padding and no invalid bit patterns, so
  // the word slice is valid as `dst.len() * 4` bytes and the swap only writes
  // whole pixels inside `area`, which the caller clamps to the frame.
  let dst_bytes = unsafe {
    slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut u8, dst.len() * 4)
  };
  swap_channels_region::<false>(src, width, dst_bytes, area);
}
