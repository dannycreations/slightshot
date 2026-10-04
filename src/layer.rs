use std::{ffi::c_void, mem, ptr, slice};

use anyhow::{bail, Context, Result};
use tiny_skia::Pixmap;
use windows::Win32::{
  Foundation::{COLORREF, HWND, POINT, RECT, SIZE},
  Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC,
    ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HDC, HGDIOBJ,
  },
  UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, UpdateLayeredWindow,
    GWL_EXSTYLE, ULW_ALPHA, WS_EX_LAYERED,
  },
};

use crate::pixel::swap_channels_keeping_alpha;

pub struct Layered {
  window: HWND,
  screen: HDC,
  memory: HDC,
  bitmap: HGDIOBJ,
  previous: HGDIOBJ,
  bits: *mut u8,
  pixels: usize,
  at: POINT,
  size: SIZE,
}

impl Layered {
  pub fn new(window: HWND, width: u32, height: u32) -> Result<Self> {
    let pixels = (width as usize) * (height as usize) * 4;
    // Partly built on purpose: `Drop` releases whatever was made so far, so
    // every early return below still gives the handles back.
    let mut layer = Self {
      window,
      screen: HDC::default(),
      memory: HDC::default(),
      bitmap: HGDIOBJ::default(),
      previous: HGDIOBJ::default(),
      bits: ptr::null_mut(),
      pixels,
      at: POINT::default(),
      size: SIZE {
        cx: width as i32,
        cy: height as i32,
      },
    };

    // SAFETY: every handle created below is owned by `layer`, whose `Drop`
    // runs on all exit paths including `?`. The bitmap stays mapped and
    // untouched by anyone else until then.
    unsafe {
      layer.screen = GetDC(None);
      if layer.screen.is_invalid() {
        bail!("no screen device context for the live overlay");
      }
      layer.memory = CreateCompatibleDC(Some(layer.screen));
      if layer.memory.is_invalid() {
        bail!("CreateCompatibleDC failed for the live overlay");
      }

      let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
          biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
          biWidth: width as i32,
          biHeight: -(height as i32), // negative: rows top-down
          biPlanes: 1,
          biBitCount: 32,
          biCompression: BI_RGB.0,
          biSizeImage: pixels as u32,
          ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
      };
      let mut bits: *mut c_void = ptr::null_mut();
      let bitmap = CreateDIBSection(
        Some(layer.memory),
        &info,
        DIB_RGB_COLORS,
        &mut bits,
        None,
        0,
      )
      .context("CreateDIBSection failed for the live overlay")?;
      layer.previous = SelectObject(layer.memory, bitmap.into());
      layer.bitmap = bitmap.into();
      if bits.is_null() {
        bail!("the live overlay surface came back without any pixels");
      }
      layer.bits = bits.cast();

      let mut rect = RECT::default();
      GetWindowRect(window, &mut rect)
        .context("the live overlay has no window position")?;
      layer.at = POINT {
        x: rect.left,
        y: rect.top,
      };
    }
    Ok(layer)
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
    // SAFETY: `bits` covers `pixels` bytes of the section created above, the
    // frame has just been checked to be the same size, and the pointers below
    // are either null or values this struct owns and keeps alive across the
    // call.
    unsafe {
      let surface = slice::from_raw_parts_mut(self.bits, self.pixels);
      swap_channels_keeping_alpha(frame.data(), surface);
      let source = POINT::default();
      UpdateLayeredWindow(
        self.window,
        Some(self.screen),
        Some(&self.at),
        Some(&self.size),
        Some(self.memory),
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

impl Drop for Layered {
  fn drop(&mut self) {
    // SAFETY: each handle was created by `new` on the thread that owns this
    // struct and is released exactly once, including when `new` returns early.
    // `bitmap` is only deleted after the memory DC has been pointed back at
    // `previous`.
    unsafe {
      if !self.bitmap.is_invalid() {
        SelectObject(self.memory, self.previous);
        let _ = DeleteObject(self.bitmap);
      }
      if !self.memory.is_invalid() {
        let _ = DeleteDC(self.memory);
      }
      if !self.screen.is_invalid() {
        ReleaseDC(None, self.screen);
      }
    }
  }
}
