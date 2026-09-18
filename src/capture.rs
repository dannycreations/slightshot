use core::ffi::c_void;
use std::{mem, ptr, slice};

use anyhow::{bail, Context, Result};
use tiny_skia::{IntSize, Pixmap};
use windows::Win32::{
  Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CAPTUREBLT, DIB_RGB_COLORS, HDC, HGDIOBJ, SRCCOPY,
  },
  UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
  },
};

use crate::pixel::swap_channels;

pub struct ScreenShot {
  pub pixmap: Pixmap,
  pub origin: (i32, i32),
}

struct GdiCaptureGuard {
  screen_dc: HDC,
  mem_dc: HDC,
  bmp: HGDIOBJ,
  previous: HGDIOBJ,
}

impl Drop for GdiCaptureGuard {
  fn drop(&mut self) {
    unsafe {
      if !self.bmp.is_invalid() {
        SelectObject(self.mem_dc, self.previous);
        let _ = DeleteObject(self.bmp);
      }
      if !self.mem_dc.is_invalid() {
        let _ = DeleteDC(self.mem_dc);
      }
      if !self.screen_dc.is_invalid() {
        ReleaseDC(None, self.screen_dc);
      }
    }
  }
}

pub fn grab() -> Result<ScreenShot> {
  unsafe {
    let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    if width <= 0 || height <= 0 {
      bail!("display reported no usable size ({width}x{height})");
    }
    let pixels = (width * height * 4) as usize;

    let screen_dc = GetDC(None);
    let mem_dc = CreateCompatibleDC(Some(screen_dc));
    if mem_dc.is_invalid() {
      ReleaseDC(None, screen_dc);
      bail!("CreateCompatibleDC failed");
    }

    let mut guard = GdiCaptureGuard {
      screen_dc,
      mem_dc,
      bmp: HGDIOBJ::default(),
      previous: HGDIOBJ::default(),
    };

    let info = BITMAPINFO {
      bmiHeader: BITMAPINFOHEADER {
        biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height, // negative: rows top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: pixels as u32,
        ..BITMAPINFOHEADER::default()
      },
      ..BITMAPINFO::default()
    };

    let mut bits: *mut c_void = ptr::null_mut();
    let bmp =
      CreateDIBSection(Some(mem_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        .context("CreateDIBSection failed")?;

    guard.previous = SelectObject(mem_dc, bmp.into());
    guard.bmp = bmp.into();

    BitBlt(
      mem_dc,
      0,
      0,
      width,
      height,
      Some(screen_dc),
      x,
      y,
      SRCCOPY | CAPTUREBLT,
    )
    .context("BitBlt of the desktop failed")?;

    let size = IntSize::from_wh(width as u32, height as u32)
      .context("invalid capture dimensions")?;

    let mut data: Vec<u8> = Vec::with_capacity(pixels);
    // SAFETY: `u8` has no invalid bit patterns, and the `swap_channels` call
    // immediately below unconditionally overwrites every element in `0..pixels`,
    // so the vector is fully initialized before anything reads from it.
    #[allow(clippy::uninit_vec)]
    data.set_len(pixels);

    let raw = slice::from_raw_parts(bits as *const u8, pixels);
    swap_channels(raw, &mut data);

    let pixmap = Pixmap::from_vec(data, size)
      .context("zero-sized capture or allocation failed")?;

    drop(guard);

    Ok(ScreenShot {
      pixmap,
      origin: (x, y),
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn swaps_channels_and_opaques_alpha() {
    let src = [
      10, 20, 30, 0, // BGRA -> RGBA 30,20,10,255
      40, 50, 60, 99, // -> 60,50,40,255
    ];
    let mut dst = vec![0u8; 8];
    swap_channels(&src, &mut dst);
    assert_eq!(dst, [30, 20, 10, 255, 60, 50, 40, 255]);
  }
}
