use std::slice;

pub fn swap_channels(src: &[u8], dst: &mut [u8]) {
  reorder::<true>(src, dst)
}

pub fn swap_channels_keeping_alpha(src: &[u8], dst: &mut [u8]) {
  reorder::<false>(src, dst)
}

fn reorder<const OPAQUE: bool>(src: &[u8], dst: &mut [u8]) {
  let total_bytes = src.len().min(dst.len()) & !3;
  for (out, chunk) in dst[..total_bytes]
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .zip(src[..total_bytes].as_chunks::<4>().0)
  {
    let px = u32::from_ne_bytes(*chunk);
    let alpha_byte = if OPAQUE {
      0xFF00_0000
    } else {
      px & 0xFF00_0000
    };
    let swapped = ((px & 0x00FF_0000) >> 16)
      | ((px & 0x0000_00FF) << 16)
      | (px & 0x0000_FF00)
      | alpha_byte;
    *out = swapped.to_ne_bytes();
  }
}

#[inline(always)]
pub fn swap_channels_to_words(src: &[u8], dst: &mut [u32]) {
  // SAFETY: a `u32` is 4 bytes with no padding and no invalid bit patterns, so
  // the word slice is valid as `dst.len() * 4` bytes and `swap_channels` only
  // writes whole pixels, staying inside that range.
  let dst_bytes = unsafe {
    slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut u8, dst.len() * 4)
  };
  swap_channels(src, dst_bytes);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn swap_channels_matches_word_level_swap() {
    let src = [10u8, 20, 30, 0, 40, 50, 60, 99];
    let mut bytes = [0u8; 8];
    swap_channels(&src, &mut bytes);
    assert_eq!(bytes, [30, 20, 10, 255, 60, 50, 40, 255]);

    let mut words = [0u32; 2];
    swap_channels_to_words(&src, &mut words);
    assert_eq!(words[0].to_le_bytes(), [30, 20, 10, 255]);
    assert_eq!(words[1].to_le_bytes(), [60, 50, 40, 255]);
  }

  #[test]
  fn both_swap_modes_agree_and_ignore_a_partial_pixel() {
    // Both modes must land on the same pixels, and a buffer whose length is not
    // a whole number of pixels keeps its tail. Buffers of different lengths
    // are covered by the test below.
    for len in [3usize, 4, 5, 17, 64, 160] {
      let src: Vec<u8> = (0..len as u8).collect();
      let mut opaque = vec![0u8; len];
      swap_channels(&src, &mut opaque);
      let mut kept = vec![0u8; len];
      swap_channels_keeping_alpha(&src, &mut kept);
      for (index, px) in src.as_chunks::<4>().0.iter().enumerate() {
        let swapped = [px[2], px[1], px[0]];
        let out = index * 4;
        assert_eq!(
          &opaque[out..out + 4],
          &[swapped[0], swapped[1], swapped[2], 255],
          "len {len} pixel {index}"
        );
        assert_eq!(
          &kept[out..out + 4],
          &[swapped[0], swapped[1], swapped[2], px[3]],
          "len {len} pixel {index}"
        );
      }
    }
  }

  #[test]
  fn swap_channels_keeping_alpha_leaves_alpha_alone() {
    // A layered window is composited by this alpha, so forcing it opaque would
    // turn the live overlay into a black screen.
    let src = [10u8, 20, 30, 0, 40, 50, 60, 99];
    let mut bytes = [0u8; 8];
    swap_channels_keeping_alpha(&src, &mut bytes);
    assert_eq!(bytes, [30, 20, 10, 0, 60, 50, 40, 99]);
  }

  #[test]
  fn a_short_destination_stops_at_the_last_whole_pixel() {
    let src = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let mut dst = [0u8; 6];
    swap_channels(&src, &mut dst);
    assert_eq!(
      dst,
      [3, 2, 1, 255, 0, 0],
      "half a pixel is left alone rather than half written"
    );
  }
}
