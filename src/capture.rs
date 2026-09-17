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

pub struct ScreenShot {
  pub pixmap: Pixmap,
  pub origin: (i32, i32),
}

fn bgra_to_rgba(src: &[u8], dst: &mut [u8]) {
  for (out, bgra) in dst
    .as_chunks_mut::<4>()
    .0
    .iter_mut()
    .zip(src.as_chunks::<4>().0)
  {
    let w = u32::from_le_bytes(*bgra);
    let rgba = (w & 0x0000_ff00)
      | ((w & 0x00ff_0000) >> 16)
      | ((w & 0x0000_00ff) << 16)
      | 0xff00_0000;
    *out = rgba.to_le_bytes();
  }
}

struct DcGuard {
  screen_dc: HDC,
  mem_dc: HDC,
}

impl Drop for DcGuard {
  fn drop(&mut self) {
    unsafe {
      let _ = DeleteDC(self.mem_dc);
      ReleaseDC(None, self.screen_dc);
    }
  }
}

struct BmpGuard {
  mem_dc: HDC,
  bmp: HGDIOBJ,
  previous: HGDIOBJ,
}

impl Drop for BmpGuard {
  fn drop(&mut self) {
    unsafe {
      SelectObject(self.mem_dc, self.previous);
      let _ = DeleteObject(self.bmp);
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
    let dc_guard = DcGuard { screen_dc, mem_dc };

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
    let previous = SelectObject(mem_dc, bmp.into());
    let bmp_guard = BmpGuard {
      mem_dc,
      bmp: bmp.into(),
      previous,
    };

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

    let raw = slice::from_raw_parts(bits as *const u8, pixels);
    let mut rgba = vec![0_u8; pixels];
    bgra_to_rgba(raw, &mut rgba);

    drop(bmp_guard);
    drop(dc_guard);

    let size = IntSize::from_wh(width as u32, height as u32)
      .context("zero-sized capture")?;
    let pixmap = Pixmap::from_vec(rgba, size)
      .context("captured buffer did not match the display")?;
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
  fn bgra_to_rgba_swaps_channels_and_opaques_alpha() {
    let src = [
      10, 20, 30, 0, // BGRA -> RGBA 30,20,10,255
      40, 50, 60, 99, // -> 60,50,40,255
    ];
    let mut dst = vec![0u8; 8];
    bgra_to_rgba(&src, &mut dst);
    assert_eq!(dst, [30, 20, 10, 255, 60, 50, 40, 255]);
  }
}
