#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;
use std::{slice, sync::OnceLock};

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn get_simd_level() -> u8 {
  static LEVEL: OnceLock<u8> = OnceLock::new();
  *LEVEL.get_or_init(|| {
    if is_x86_feature_detected!("avx2") {
      3
    } else if is_x86_feature_detected!("ssse3") {
      2
    } else {
      1
    }
  })
}

#[cfg(target_arch = "x86_64")]
const BGRA_TO_RGBA: [i8; 16] =
  [2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15];

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn reorder_avx2<const OPAQUE: bool>(
  mut src_ptr: *const u8,
  mut dst_ptr: *mut u8,
  count: usize,
) {
  // SAFETY: the mask is 16 bytes and is only read, so the load stays in
  // bounds and needs no alignment.
  let narrow = unsafe { _mm_loadu_si128(BGRA_TO_RGBA.as_ptr().cast()) };
  // Both halves shuffle within themselves, so one copy of the mask serves the
  // whole 32-byte register.
  let mask = _mm256_broadcastsi128_si256(narrow);
  let seal = if OPAQUE {
    _mm256_set1_epi32(0xff00_0000_u32 as i32)
  } else {
    _mm256_setzero_si256()
  };

  let chunks128 = count / 128;
  for _ in 0..chunks128 {
    let v0 = _mm256_loadu_si256(src_ptr as *const __m256i);
    let v1 = _mm256_loadu_si256(src_ptr.add(32) as *const __m256i);
    let v2 = _mm256_loadu_si256(src_ptr.add(64) as *const __m256i);
    let v3 = _mm256_loadu_si256(src_ptr.add(96) as *const __m256i);

    let r0 = _mm256_or_si256(_mm256_shuffle_epi8(v0, mask), seal);
    let r1 = _mm256_or_si256(_mm256_shuffle_epi8(v1, mask), seal);
    let r2 = _mm256_or_si256(_mm256_shuffle_epi8(v2, mask), seal);
    let r3 = _mm256_or_si256(_mm256_shuffle_epi8(v3, mask), seal);

    _mm256_storeu_si256(dst_ptr as *mut __m256i, r0);
    _mm256_storeu_si256(dst_ptr.add(32) as *mut __m256i, r1);
    _mm256_storeu_si256(dst_ptr.add(64) as *mut __m256i, r2);
    _mm256_storeu_si256(dst_ptr.add(96) as *mut __m256i, r3);

    src_ptr = src_ptr.add(128);
    dst_ptr = dst_ptr.add(128);
  }

  let remainder32 = (count % 128) / 32;
  for _ in 0..remainder32 {
    let v = _mm256_loadu_si256(src_ptr as *const __m256i);
    let r = _mm256_or_si256(_mm256_shuffle_epi8(v, mask), seal);
    _mm256_storeu_si256(dst_ptr as *mut __m256i, r);
    src_ptr = src_ptr.add(32);
    dst_ptr = dst_ptr.add(32);
  }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn reorder_ssse3<const OPAQUE: bool>(
  mut src_ptr: *const u8,
  mut dst_ptr: *mut u8,
  count: usize,
) {
  // SAFETY: the mask is 16 bytes and is only read, so the load stays in bounds
  // and needs no alignment.
  let mask = unsafe { _mm_loadu_si128(BGRA_TO_RGBA.as_ptr().cast()) };
  let seal = if OPAQUE {
    _mm_set1_epi32(0xff00_0000_u32 as i32)
  } else {
    _mm_setzero_si128()
  };

  let chunks64 = count / 64;
  for _ in 0..chunks64 {
    let v0 = _mm_loadu_si128(src_ptr as *const __m128i);
    let v1 = _mm_loadu_si128(src_ptr.add(16) as *const __m128i);
    let v2 = _mm_loadu_si128(src_ptr.add(32) as *const __m128i);
    let v3 = _mm_loadu_si128(src_ptr.add(48) as *const __m128i);

    let r0 = _mm_or_si128(_mm_shuffle_epi8(v0, mask), seal);
    let r1 = _mm_or_si128(_mm_shuffle_epi8(v1, mask), seal);
    let r2 = _mm_or_si128(_mm_shuffle_epi8(v2, mask), seal);
    let r3 = _mm_or_si128(_mm_shuffle_epi8(v3, mask), seal);

    _mm_storeu_si128(dst_ptr as *mut __m128i, r0);
    _mm_storeu_si128(dst_ptr.add(16) as *mut __m128i, r1);
    _mm_storeu_si128(dst_ptr.add(32) as *mut __m128i, r2);
    _mm_storeu_si128(dst_ptr.add(48) as *mut __m128i, r3);

    src_ptr = src_ptr.add(64);
    dst_ptr = dst_ptr.add(64);
  }

  let remainder16 = (count % 64) / 16;
  for _ in 0..remainder16 {
    let v = _mm_loadu_si128(src_ptr as *const __m128i);
    let r = _mm_or_si128(_mm_shuffle_epi8(v, mask), seal);
    _mm_storeu_si128(dst_ptr as *mut __m128i, r);
    src_ptr = src_ptr.add(16);
    dst_ptr = dst_ptr.add(16);
  }
}

pub fn swap_channels(src: &[u8], dst: &mut [u8]) {
  reorder::<true>(src, dst)
}

pub fn swap_channels_keeping_alpha(src: &[u8], dst: &mut [u8]) {
  reorder::<false>(src, dst)
}

fn reorder<const OPAQUE: bool>(src: &[u8], dst: &mut [u8]) {
  let total_bytes = src.len().min(dst.len()) & !3;
  let mut processed = 0;

  #[cfg(target_arch = "x86_64")]
  {
    match get_simd_level() {
      3 => {
        let simd_bytes = total_bytes & !31;
        if simd_bytes > 0 {
          // SAFETY: `get_simd_level` only returns 3 after confirming avx2 is
          // available, and `simd_bytes` is a whole number of 32-byte vectors
          // that fits inside both slices, so every load and store below stays
          // in bounds and the unaligned variants need no alignment.
          unsafe {
            reorder_avx2::<OPAQUE>(src.as_ptr(), dst.as_mut_ptr(), simd_bytes);
          }
          processed = simd_bytes;
        }
      }
      2 => {
        let simd_bytes = total_bytes & !15;
        if simd_bytes > 0 {
          // SAFETY: same argument as the avx2 branch, with ssse3 and 16-byte
          // vectors.
          unsafe {
            reorder_ssse3::<OPAQUE>(src.as_ptr(), dst.as_mut_ptr(), simd_bytes);
          }
          processed = simd_bytes;
        }
      }
      _ => {}
    }
  }

  let remaining_src = &src[processed..total_bytes];
  let remaining_dst = &mut dst[processed..total_bytes];
  for (out, chunk) in remaining_dst
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .zip(remaining_src.as_chunks::<4>().0)
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
  fn both_swap_modes_agree_across_every_path() {
    // A BGRA pixel becomes RGBA whichever path handles it: the avx2 loop, its
    // 32-byte remainder, the ssse3 loop, its 16-byte remainder, or the scalar
    // tail. Only `swap_channels` forces the alpha, so both modes are checked
    // over lengths that straddle every boundary.
    for len in [4usize, 16, 32, 48, 64, 160] {
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

    // Forty bytes reach the vector paths, which take the alpha from the shuffle
    // mask instead of the scalar mask, and every pixel has to agree.
    let wide: Vec<u8> = (0..40u8).collect();
    let mut out = [0u8; 40];
    swap_channels_keeping_alpha(&wide, &mut out);
    for (index, (got, px)) in out
      .as_chunks::<4>()
      .0
      .iter()
      .zip(wide.as_chunks::<4>().0)
      .enumerate()
    {
      let want = [px[2], px[1], px[0], px[3]];
      assert_eq!(*got, want, "pixel {index}");
    }
  }
}
