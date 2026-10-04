use anyhow::{bail, Context, Result};
use tiny_skia::Pixmap;
use windows::Win32::{
  Foundation::{COLORREF, HWND, POINT, RECT, SIZE},
  Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION},
  UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, UpdateLayeredWindow,
    GWL_EXSTYLE, ULW_ALPHA, WS_EX_LAYERED,
  },
};

use crate::{capture::Bitmap, geom::Rect, pixel::swap_channels_region};

pub struct Layered {
  window: HWND,
  surface: Bitmap,
  at: POINT,
  size: SIZE,
}

impl Layered {
  pub fn new(window: HWND, width: u32, height: u32) -> Result<Self> {
    let surface =
      Bitmap::new(width, height).context("opening the live overlay failed")?;
    // SAFETY: `window` is the handle of the window just created, which outlives
    // this call, and the call only reads its position.
    let at = unsafe {
      let mut rect = RECT::default();
      GetWindowRect(window, &mut rect)
        .context("the live overlay has no window position")?;
      POINT {
        x: rect.left,
        y: rect.top,
      }
    };
    Ok(Self {
      window,
      surface,
      at,
      size: SIZE {
        cx: width as i32,
        cy: height as i32,
      },
    })
  }

  pub fn present(&mut self, frame: &Pixmap, area: Rect) -> Result<()> {
    if frame.width() != self.size.cx as u32
      || frame.height() != self.size.cy as u32
    {
      bail!(
        "the frame is {}x{} but the live overlay is {}x{}",
        frame.width(),
        frame.height(),
        self.size.cx,
        self.size.cy
      );
    }
    if area.is_empty() {
      return Ok(());
    }
    self.ensure_layered()?;
    // tiny-skia keeps premultiplied RGBA, which is what a 32-bit layered
    // window wants once red and blue are the right way round.
    let blend = BLENDFUNCTION {
      BlendOp: AC_SRC_OVER as u8,
      BlendFlags: 0,
      SourceConstantAlpha: 255,
      AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let source = POINT::default();
    let area = area.clamped_inside(Rect::new(
      0.0,
      0.0,
      frame.width() as f32,
      frame.height() as f32,
    ));
    swap_channels_region::<true>(
      frame.data(),
      frame.width(),
      self.surface.pixels_mut(),
      area,
    );
    // SAFETY: the section holds a frame of exactly this size, with `area` just
    // written over it, and the handles and positions passed here belong to this
    // struct and to the window it outlives.
    unsafe {
      UpdateLayeredWindow(
        self.window,
        Some(self.surface.screen_dc()),
        Some(&self.at),
        Some(&self.size),
        Some(self.surface.memory_dc()),
        Some(&source),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
      )?;
    }
    Ok(())
  }

  fn ensure_layered(&self) -> Result<()> {
    let layered = WS_EX_LAYERED.0 as isize;
    // SAFETY: every call only reads or writes this window's own extended
    // style, and the window outlives this struct.
    unsafe {
      let style = GetWindowLongPtrW(self.window, GWL_EXSTYLE);
      if style & layered == 0 {
        SetWindowLongPtrW(self.window, GWL_EXSTYLE, style | layered);
        if GetWindowLongPtrW(self.window, GWL_EXSTYLE) & layered == 0 {
          bail!(
            "the overlay window refused the layered style live drawing needs"
          );
        }
      }
    }
    Ok(())
  }
}
