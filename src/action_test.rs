use std::{env, fs, io::Cursor};

use super::{encode_png, png_bytes, stamp, Shot};

#[test]
fn empty_shot_has_no_pixels() {
  let shot = Shot::empty();
  assert_eq!((shot.width, shot.height), (0, 0));
  assert!(shot.rgba.is_empty());
}

#[test]
fn stamp_names_a_png_in_the_slightshot_prefix() {
  let name = stamp();
  assert!(name.starts_with("slightshot_"));
  assert!(name.ends_with(".png"));
}

#[test]
fn png_round_trips_through_decode() {
  let shot = Shot {
    width: 2,
    height: 1,
    rgba: vec![10, 20, 30, 255, 40, 50, 60, 255],
  };
  let bytes = png_bytes(&shot).expect("encode");
  let decoder = png::Decoder::new(Cursor::new(bytes));
  let mut reader = decoder.read_info().expect("decode header");
  let info = reader.info();
  let (w, h) = (info.width, info.height);
  assert_eq!((w, h), (2, 1));
  let mut buf = vec![
    0;
    reader
      .output_buffer_size()
      .expect("the decoded PNG fits in memory")
  ];
  let _ = reader.next_frame(&mut buf).expect("decode frame");
  let expected = (w as usize) * (h as usize) * 4;
  assert_eq!(&buf[..expected], shot.rgba.as_slice());
}

#[test]
fn encode_png_writes_readable_bytes() {
  let shot = Shot {
    width: 1,
    height: 1,
    rgba: vec![1, 2, 3, 255],
  };
  let dir = env::temp_dir().join("slightshot_test_tmp");
  let _ = fs::create_dir_all(&dir);
  let path = dir.join(stamp());
  encode_png(&shot, &path).expect("write png");
  let bytes = fs::read(&path).expect("read png");
  assert!(png::Decoder::new(Cursor::new(bytes)).read_info().is_ok());
}
