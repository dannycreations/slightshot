use super::{swap_channels, swap_channels_region, swap_words_region};
use crate::geom::Rect;

#[test]
fn swap_channels_matches_word_level_swap() {
  let src = [10u8, 20, 30, 0, 40, 50, 60, 99];
  let mut bytes = [0u8; 8];
  swap_channels(&src, &mut bytes);
  assert_eq!(bytes, [30, 20, 10, 255, 60, 50, 40, 255]);

  let mut words = [0u32; 2];
  swap_words_region(&src, 2, &mut words, Rect::new(0.0, 0.0, 2.0, 1.0));
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
    swap_channels_region::<true>(
      &src,
      1,
      &mut kept,
      Rect::new(0.0, 0.0, len as f32, 1.0),
    );
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
  swap_channels_region::<true>(
    &src,
    2,
    &mut bytes,
    Rect::new(0.0, 0.0, 2.0, 1.0),
  );
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

#[test]
fn a_region_swap_leaves_the_rest_of_the_frame_alone() {
  // Two rows of two pixels, so a whole-frame swap and a one-row swap can be
  // compared directly.
  let src: Vec<u8> = (0..16u8).collect();
  let mut full = vec![0u8; 16];
  swap_channels_region::<true>(
    &src,
    2,
    &mut full,
    Rect::new(0.0, 0.0, 2.0, 2.0),
  );

  // A partial frame: what the last present left behind, then one new row.
  let mut kept = vec![99u8; 16];
  swap_channels_region::<true>(
    &src,
    2,
    &mut kept,
    Rect::new(0.0, 1.0, 2.0, 1.0),
  );
  assert_eq!(&kept[0..8], &[99; 8], "the row above the swap is untouched");
  assert_eq!(&kept[8..16], &full[8..16], "the swapped row matches");
}

#[test]
fn a_region_swap_can_start_mid_row() {
  let src: Vec<u8> = (0..16u8).collect();
  let mut whole = vec![0u8; 16];
  swap_channels_region::<true>(
    &src,
    4,
    &mut whole,
    Rect::new(0.0, 0.0, 4.0, 1.0),
  );

  let mut cut = vec![7u8; 16];
  swap_channels_region::<true>(
    &src,
    4,
    &mut cut,
    Rect::new(2.0, 0.0, 2.0, 1.0),
  );
  assert_eq!(&cut[0..8], &[7; 8], "columns left of the area are skipped");
  assert_eq!(
    &cut[8..16],
    &whole[8..16],
    "the columns inside it are swapped"
  );
}
