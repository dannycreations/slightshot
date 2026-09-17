#[inline]
pub fn swap_red_blue_opaque(word: u32) -> u32 {
  (word & 0x0000_ff00)
    | ((word & 0x00ff_0000) >> 16)
    | ((word & 0x0000_00ff) << 16)
    | 0xff00_0000
}

pub fn swap_channels(src: &[u8], dst: &mut [u8]) {
  for (out, chunk) in dst
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .zip(src.as_chunks::<4>().0)
  {
    *out = swap_red_blue_opaque(u32::from_le_bytes(*chunk)).to_le_bytes();
  }
}

pub fn swap_channels_to_words(src: &[u8], dst: &mut [u32]) {
  for (word, chunk) in dst.iter_mut().zip(src.as_chunks::<4>().0) {
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
