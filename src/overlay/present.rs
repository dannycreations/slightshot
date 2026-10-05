use std::{num::NonZeroU32, sync::Arc};

use anyhow::{anyhow, Context, Result};
use softbuffer::{
  Context as SoftContext, Rect as Damage, Surface as SoftSurface,
};
use tiny_skia::Pixmap;
use winit::window::Window;

use crate::{geom::Rect, layer::Layered, pixel::swap_words_region};

pub(super) type Surface = SoftSurface<Arc<Window>, Arc<Window>>;

pub(super) enum Presenter {
  Solid(Surface),
  Live(Layered),
}

impl Presenter {
  pub(super) fn present(&mut self, frame: &Pixmap, area: Rect) -> Result<()> {
    if area.is_empty() {
      return Ok(());
    }
    match self {
      Presenter::Solid(surface) => {
        let Ok(mut buffer) = surface.buffer_mut() else {
          return Ok(());
        };
        if buffer.len() != (frame.width() * frame.height()) as usize {
          return Ok(());
        }
        swap_words_region(frame.data(), frame.width(), &mut buffer, area);
        let damage = [damage_of(area)];
        let _ = buffer.present_with_damage(&damage);
        Ok(())
      }
      // A layered window that cannot be composited stays on the screen as an
      // invisible sheet that still swallows the pointer, so this one is worth
      // reporting instead of dropping.
      Presenter::Live(layer) => layer.present(frame, area),
    }
  }
}

fn damage_of(area: Rect) -> Damage {
  // `MIN` is 1, so the fallback already floors a box that rounded down to
  // nothing. softbuffer rejects a rect with a zero side.
  Damage {
    // A float to integer cast saturates, so a negative origin and a NaN both
    // land on 0 here without a clamp.
    x: area.x.floor() as u32,
    y: area.y.floor() as u32,
    width: NonZeroU32::new(area.w.ceil() as u32).unwrap_or(NonZeroU32::MIN),
    height: NonZeroU32::new(area.h.ceil() as u32).unwrap_or(NonZeroU32::MIN),
  }
}

pub(super) fn solid_surface(
  window: &Arc<Window>,
  size: (u32, u32),
) -> Result<Surface> {
  let context = SoftContext::new(window.clone())
    .map_err(|error| anyhow!("no graphics context for the overlay: {error}"))?;
  let mut surface = SoftSurface::new(&context, window.clone())
    .map_err(|error| anyhow!("no surface for the overlay: {error}"))?;
  surface
    .resize(
      NonZeroU32::new(size.0).context("zero-width capture")?,
      NonZeroU32::new(size.1).context("zero-height capture")?,
    )
    .map_err(|e| anyhow!("failed to resize the overlay surface: {e}"))?;
  Ok(surface)
}
