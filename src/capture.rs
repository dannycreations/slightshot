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

#[derive(Clone, Copy, Debug)]
pub struct Desktop {
  pub origin: (i32, i32),
  pub size: (u32, u32),
}

pub fn desktop() -> Result<Desktop> {
  // SAFETY: GetSystemMetrics reads display settings through constants and
  // touches nothing this process owns, so there is no state to keep alive.
  unsafe {
    let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    if width <= 0 || height <= 0 {
      bail!("display reported no usable size ({width}x{height})");
    }
    Ok(Desktop {
      origin: (x, y),
      size: (width as u32, height as u32),
    })
  }
}

pub(crate) fn dib_info(width: u32, height: u32) -> BITMAPINFO {
  BITMAPINFO {
    bmiHeader: BITMAPINFOHEADER {
      biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
      biWidth: width as i32,
      biHeight: -(height as i32),
      biPlanes: 1,
      biBitCount: 32,
      biCompression: BI_RGB.0,
      biSizeImage: width * height * 4,
      ..BITMAPINFOHEADER::default()
    },
    ..BITMAPINFO::default()
  }
}

struct GdiCaptureGuard {
  screen_dc: HDC,
  mem_dc: HDC,
  bmp: HGDIOBJ,
  previous: HGDIOBJ,
}

impl Drop for GdiCaptureGuard {
  fn drop(&mut self) {
    // SAFETY: each handle was created by this thread (GetDC, CreateCompatibleDC,
    // CreateDIBSection) and is released exactly once; `bmp` is only deleted
    // after the memory DC is pointed back at `previous`.
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

pub fn grab() -> Result<Pixmap> {
  let Desktop { origin, size } = desktop()?;
  let (x, y) = origin;
  let (width, height) = (size.0 as i32, size.1 as i32);
  // SAFETY: every handle created below is owned by `guard`, whose `Drop` runs
  // on all exit paths including `?`. The bitmap stays mapped and untouched by
  // anyone else until then, and its pixels are copied out before this function
  // returns, which is also when the handles are released.
  unsafe {
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

    let info = dib_info(size.0, size.1);

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

    Ok(pixmap)
  }
}
