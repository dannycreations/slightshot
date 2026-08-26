use core::ffi::c_void;
use std::{mem, ptr, slice};

use anyhow::{bail, Context, Result};
use tiny_skia::{IntSize, Pixmap};
use windows::Win32::{
  Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
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
    *out = [bgra[2], bgra[1], bgra[0], 255];
  }
}

pub fn grab() -> Result<ScreenShot> {
  // SAFETY: the DIB section owns the only raw pointer involved; its bits
  // are read while `bmp` is alive and every GDI handle is released on all
  // paths before returning. The calls themselves are thread-safe here.
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
    let blitted = BitBlt(
      mem_dc,
      0,
      0,
      width,
      height,
      Some(screen_dc),
      x,
      y,
      SRCCOPY | CAPTUREBLT,
    );

    let converted = blitted.context("BitBlt of the desktop failed").map(|_| {
      let raw = slice::from_raw_parts(bits as *const u8, pixels);
      let mut rgba = vec![0_u8; pixels];
      bgra_to_rgba(raw, &mut rgba);
      rgba
    });

    SelectObject(mem_dc, previous);
    let _ = DeleteObject(bmp.into());
    let _ = DeleteDC(mem_dc);
    ReleaseDC(None, screen_dc);

    let rgba = converted?;
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
