use std::{
  borrow::Cow,
  env, fs,
  io::Cursor,
  path::{Path, PathBuf},
  thread,
  time::SystemTime,
};

use anyhow::{Context, Result};
use arboard::{Clipboard, ImageData};
use png::{BitDepth, ColorType, Encoder};

use crate::upload;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Deliverable {
  Upload,
  Copy,
  Save,
  Close,
}

pub struct Shot {
  pub width: u32,
  pub height: u32,
  pub rgba: Vec<u8>,
}

impl Shot {
  pub fn empty() -> Self {
    Self {
      width: 0,
      height: 0,
      rgba: Vec::new(),
    }
  }
}

pub fn execute(deliverable: Deliverable, shot: &Shot) -> Result<String> {
  match deliverable {
    Deliverable::Upload => {
      let png = png_bytes(shot)?;
      let kib = png.len() / 1024;
      // Off-thread so the watcher stays live while the upload runs.
      thread::spawn(move || deliver_upload(&png));
      Ok(format!("uploading {kib} KiB"))
    }
    Deliverable::Copy => {
      copy_to_clipboard(shot)?;
      Ok(format!(
        "copied {}x{} to the clipboard",
        shot.width, shot.height
      ))
    }
    Deliverable::Save => {
      let dir = pictures_dir();
      fs::create_dir_all(&dir)
        .context("creating the Pictures folder failed")?;
      let path = dir.join(stamp());
      encode_png(shot, &path)?;
      Ok(format!("saved {}", path.display()))
    }
    Deliverable::Close => Ok(String::new()),
  }
}

fn deliver_upload(png: &[u8]) {
  let link = match upload::upload(png) {
    Ok(link) => link,
    Err(error) => return eprintln!("slightshot: upload failed: {error:#}"),
  };
  match copy_text(&link) {
    Ok(()) => println!("slightshot: uploaded {link}; link copied"),
    Err(error) => {
      eprintln!("slightshot: uploaded {link}; copying it failed: {error:#}")
    }
  }
}

fn pictures_dir() -> PathBuf {
  env::var_os("USERPROFILE")
    .map(|profile| PathBuf::from(profile).join("Pictures"))
    .unwrap_or_else(env::temp_dir)
}

fn stamp() -> String {
  let seconds = SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)
    .map(|elapsed| elapsed.as_secs())
    .unwrap_or_default();
  format!("slightshot_{seconds}.png")
}

fn png_bytes(shot: &Shot) -> Result<Vec<u8>> {
  let mut cursor = Cursor::new(Vec::new());
  {
    let mut encoder = Encoder::new(&mut cursor, shot.width, shot.height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&shot.rgba)?;
    writer.finish()?;
  }
  Ok(cursor.into_inner())
}

fn encode_png(shot: &Shot, path: &Path) -> Result<()> {
  fs::write(path, png_bytes(shot)?)
    .with_context(|| format!("cannot create {}", path.display()))
}

fn copy_to_clipboard(shot: &Shot) -> Result<()> {
  let mut clipboard =
    Clipboard::new().context("opening the clipboard failed")?;
  clipboard
    .set_image(ImageData {
      width: shot.width as usize,
      height: shot.height as usize,
      bytes: Cow::Borrowed(shot.rgba.as_slice()),
    })
    .context("writing image data to the clipboard failed")
}

fn copy_text(text: &str) -> Result<()> {
  Clipboard::new()
    .and_then(|mut clipboard| clipboard.set_text(text.to_owned()))
    .context("writing text to the clipboard failed")
}

#[cfg(test)]
mod tests {
  use super::*;

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
}
