#[inline(always)]
pub fn swap_red_blue_opaque(word: u32) -> u32 {
  (word & 0x0000_ff00)
    | ((word & 0x00ff_0000) >> 16)
    | ((word & 0x0000_00ff) << 16)
    | 0xff00_0000
}

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn swap_channels_avx2(
  mut src_ptr: *const u8,
  mut dst_ptr: *mut u8,
  count: usize,
) {
  let mask = _mm256_setr_epi8(
    2, 1, 0, -128, 6, 5, 4, -128, 10, 9, 8, -128, 14, 13, 12, -128, 2, 1, 0,
    -128, 6, 5, 4, -128, 10, 9, 8, -128, 14, 13, 12, -128,
  );
  let alpha = _mm256_set1_epi32(0xff00_0000_u32 as i32);

  let chunks32 = count / 32;
  for _ in 0..chunks32 {
    let v = _mm256_loadu_si256(src_ptr as *const __m256i);
    let shuffled = _mm256_shuffle_epi8(v, mask);
    let result = _mm256_or_si256(shuffled, alpha);
    _mm256_storeu_si256(dst_ptr as *mut __m256i, result);
    src_ptr = src_ptr.add(32);
    dst_ptr = dst_ptr.add(32);
  }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn swap_channels_ssse3(
  mut src_ptr: *const u8,
  mut dst_ptr: *mut u8,
  count: usize,
) {
  let mask = _mm_setr_epi8(
    2, 1, 0, -128, 6, 5, 4, -128, 10, 9, 8, -128, 14, 13, 12, -128,
  );
  let alpha = _mm_set1_epi32(0xff00_0000_u32 as i32);

  let chunks16 = count / 16;
  for _ in 0..chunks16 {
    let v = _mm_loadu_si128(src_ptr as *const __m128i);
    let shuffled = _mm_shuffle_epi8(v, mask);
    let result = _mm_or_si128(shuffled, alpha);
    _mm_storeu_si128(dst_ptr as *mut __m128i, result);
    src_ptr = src_ptr.add(16);
    dst_ptr = dst_ptr.add(16);
  }
}

pub fn swap_channels(src: &[u8], dst: &mut [u8]) {
  let total_bytes = src.len().min(dst.len()) & !3;
  let mut processed = 0;

  #[cfg(target_arch = "x86_64")]
  {
    if is_x86_feature_detected!("avx2") {
      let simd_bytes = total_bytes & !31;
      if simd_bytes > 0 {
        unsafe {
          swap_channels_avx2(src.as_ptr(), dst.as_mut_ptr(), simd_bytes);
        }
        processed = simd_bytes;
      }
    } else if is_x86_feature_detected!("ssse3") {
      let simd_bytes = total_bytes & !15;
      if simd_bytes > 0 {
        unsafe {
          swap_channels_ssse3(src.as_ptr(), dst.as_mut_ptr(), simd_bytes);
        }
        processed = simd_bytes;
      }
    }
  }

  for (out, chunk) in dst[processed..total_bytes]
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .zip(src[processed..total_bytes].as_chunks::<4>().0)
  {
    out[0] = chunk[2];
    out[1] = chunk[1];
    out[2] = chunk[0];
    out[3] = 255;
  }
}

pub fn swap_channels_to_words(src: &[u8], dst: &mut [u32]) {
  let total_bytes = src.len().min(dst.len() * 4) & !3;
  let mut processed = 0;

  #[cfg(target_arch = "x86_64")]
  {
    if is_x86_feature_detected!("avx2") {
      let simd_bytes = total_bytes & !31;
      if simd_bytes > 0 {
        unsafe {
          swap_channels_avx2(
            src.as_ptr(),
            dst.as_mut_ptr() as *mut u8,
            simd_bytes,
          );
        }
        processed = simd_bytes;
      }
    } else if is_x86_feature_detected!("ssse3") {
      let simd_bytes = total_bytes & !15;
      if simd_bytes > 0 {
        unsafe {
          swap_channels_ssse3(
            src.as_ptr(),
            dst.as_mut_ptr() as *mut u8,
            simd_bytes,
          );
        }
        processed = simd_bytes;
      }
    }
  }

  let offset_words = processed / 4;
  let total_words = total_bytes / 4;
  for (word, chunk) in dst[offset_words..total_words]
    .iter_mut()
    .zip(src[processed..total_bytes].as_chunks::<4>().0)
  {
    *word = swap_red_blue_opaque(u32::from_le_bytes(*chunk));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn swap_is_its_own_inverse() {
    let word = u32::from_le_bytes([10, 20, 30, 0]);
    let swapped = swap_red_blue_opaque(word);
    assert_eq!(swapped.to_le_bytes(), [30, 20, 10, 255]);
    let back = swap_red_blue_opaque(swapped);
    assert_eq!(back.to_le_bytes(), [10, 20, 30, 255]);
  }

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
}
