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

  let chunks128 = count / 128;
  for _ in 0..chunks128 {
    let v0 = _mm256_loadu_si256(src_ptr as *const __m256i);
    let v1 = _mm256_loadu_si256(src_ptr.add(32) as *const __m256i);
    let v2 = _mm256_loadu_si256(src_ptr.add(64) as *const __m256i);
    let v3 = _mm256_loadu_si256(src_ptr.add(96) as *const __m256i);

    let r0 = _mm256_or_si256(_mm256_shuffle_epi8(v0, mask), alpha);
    let r1 = _mm256_or_si256(_mm256_shuffle_epi8(v1, mask), alpha);
    let r2 = _mm256_or_si256(_mm256_shuffle_epi8(v2, mask), alpha);
    let r3 = _mm256_or_si256(_mm256_shuffle_epi8(v3, mask), alpha);

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
    let r = _mm256_or_si256(_mm256_shuffle_epi8(v, mask), alpha);
    _mm256_storeu_si256(dst_ptr as *mut __m256i, r);
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

  let chunks64 = count / 64;
  for _ in 0..chunks64 {
    let v0 = _mm_loadu_si128(src_ptr as *const __m128i);
    let v1 = _mm_loadu_si128(src_ptr.add(16) as *const __m128i);
    let v2 = _mm_loadu_si128(src_ptr.add(32) as *const __m128i);
    let v3 = _mm_loadu_si128(src_ptr.add(48) as *const __m128i);

    let r0 = _mm_or_si128(_mm_shuffle_epi8(v0, mask), alpha);
    let r1 = _mm_or_si128(_mm_shuffle_epi8(v1, mask), alpha);
    let r2 = _mm_or_si128(_mm_shuffle_epi8(v2, mask), alpha);
    let r3 = _mm_or_si128(_mm_shuffle_epi8(v3, mask), alpha);

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
    let r = _mm_or_si128(_mm_shuffle_epi8(v, mask), alpha);
    _mm_storeu_si128(dst_ptr as *mut __m128i, r);
    src_ptr = src_ptr.add(16);
    dst_ptr = dst_ptr.add(16);
  }
}

pub fn swap_channels(src: &[u8], dst: &mut [u8]) {
  let total_bytes = src.len().min(dst.len()) & !3;
  let mut processed = 0;

  #[cfg(target_arch = "x86_64")]
  {
    match get_simd_level() {
      3 => {
        let simd_bytes = total_bytes & !31;
        if simd_bytes > 0 {
          unsafe {
            swap_channels_avx2(src.as_ptr(), dst.as_mut_ptr(), simd_bytes);
          }
          processed = simd_bytes;
        }
      }
      2 => {
        let simd_bytes = total_bytes & !15;
        if simd_bytes > 0 {
          unsafe {
            swap_channels_ssse3(src.as_ptr(), dst.as_mut_ptr(), simd_bytes);
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
    let swapped = ((px & 0x00FF_0000) >> 16)
      | ((px & 0x0000_00FF) << 16)
      | (px & 0x0000_FF00)
      | 0xFF00_0000;
    *out = swapped.to_ne_bytes();
  }
}

#[inline(always)]
pub fn swap_channels_to_words(src: &[u8], dst: &mut [u32]) {
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
}
