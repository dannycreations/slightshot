use anyhow::{Context, Result};
use tiny_skia::Pixmap;

use crate::capture;

#[cfg(test)]
#[path = "screen_test.rs"]
mod screen_test;

/// A pixel buffer held between sessions.
///
/// The overlay opens and closes on every hotkey press, so these are parked
/// rather than freed: a desktop that has not changed shape must not fault in
/// every page again on the next press.
#[derive(Default)]
pub(super) struct Buffer(Option<Pixmap>);

impl Buffer {
  pub(super) fn fit(&mut self, size: (u32, u32)) -> Result<&mut Pixmap> {
    if !self
      .0
      .as_ref()
      .is_some_and(|pm| (pm.width(), pm.height()) == size)
    {
      self.0 = Some(
        Pixmap::new(size.0, size.1)
          .context("allocating a full-screen buffer failed")?,
      );
    }
    Ok(self.0.as_mut().expect("just ensured"))
  }

  pub(super) fn get(&self) -> &Pixmap {
    self
      .0
      .as_ref()
      .expect("a session sizes its buffers before it opens")
  }

  pub(super) fn at(&mut self) -> &mut Pixmap {
    self
      .0
      .as_mut()
      .expect("a session sizes its buffers before it opens")
  }
}

/// Every buffer a session draws into, kept together so they can be handed back
/// when it closes.
#[derive(Default)]
pub(super) struct Screen {
  pub(super) shot: Option<capture::Bitmap>,
  pub(super) canvas: Buffer,
  pub(super) backdrop: Buffer,
  pub(super) frame: Buffer,
}

/// The capture section for a desktop, re-made only when the desktop changes
/// shape.
pub(super) fn section(
  slot: &mut Option<capture::Bitmap>,
  size: (u32, u32),
) -> Result<&mut capture::Bitmap> {
  if !slot.as_ref().is_some_and(|s| s.fits(size)) {
    *slot = Some(capture::Bitmap::new(size.0, size.1)?);
  }
  Ok(slot.as_mut().expect("the desktop has a nonzero size"))
}
