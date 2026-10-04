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

use crate::{capture::Bitmap, pixel::swap_channels_keeping_alpha};

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

  pub fn present(&mut self, frame: &Pixmap) -> Result<()> {
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
    swap_channels_keeping_alpha(frame.data(), self.surface.pixels_mut());
    // SAFETY: the section has just been filled with a frame of exactly this
    // size, and the handles and positions passed here belong to this struct and
    // to the window it outlives.
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
    // SAFETY: both calls only read and write this window's own extended style,
    // and the window outlives this struct.
    unsafe {
      if GetWindowLongPtrW(self.window, GWL_EXSTYLE) & layered == 0 {
        let style = GetWindowLongPtrW(self.window, GWL_EXSTYLE);
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
