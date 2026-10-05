use core::ffi::c_void;
use std::{mem, ptr, slice};

use anyhow::{bail, Context, Result};
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

fn dib_info(width: u32, height: u32) -> BITMAPINFO {
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

pub struct Bitmap {
  screen: HDC,
  memory: HDC,
  bitmap: HGDIOBJ,
  previous: HGDIOBJ,
  pixels: *mut u8,
  width: u32,
  height: u32,
}

impl Bitmap {
  pub fn new(width: u32, height: u32) -> Result<Self> {
    // Partly built on purpose: `Drop` releases whatever has been made so far,
    // so every early return below still gives the handles back.
    let mut section = Self {
      screen: HDC::default(),
      memory: HDC::default(),
      bitmap: HGDIOBJ::default(),
      previous: HGDIOBJ::default(),
      pixels: ptr::null_mut(),
      width,
      height,
    };
    // SAFETY: each handle is stored in `section` before the next call can
    // fail, and the section stays mapped and untouched by anyone else until
    // `Drop`.
    unsafe {
      section.screen = GetDC(None);
      if section.screen.is_invalid() {
        bail!("no screen device context");
      }
      section.memory = CreateCompatibleDC(Some(section.screen));
      if section.memory.is_invalid() {
        bail!("CreateCompatibleDC failed");
      }
      let mut bits: *mut c_void = ptr::null_mut();
      let bitmap = CreateDIBSection(
        Some(section.memory),
        &dib_info(width, height),
        DIB_RGB_COLORS,
        &mut bits,
        None,
        0,
      )
      .context("CreateDIBSection failed")?;
      section.previous = SelectObject(section.memory, bitmap.into());
      section.bitmap = bitmap.into();
      if bits.is_null() {
        bail!("the DIB section came back without any pixels");
      }
      section.pixels = bits.cast();
    }
    Ok(section)
  }

  pub fn screen_dc(&self) -> HDC {
    self.screen
  }

  pub fn fits(&self, size: (u32, u32)) -> bool {
    (self.width, self.height) == size
  }

  pub fn memory_dc(&self) -> HDC {
    self.memory
  }

  pub fn pixels(&self) -> &[u8] {
    // SAFETY: `new` mapped exactly this many bytes and `Drop` is the only
    // thing that unmaps them, which cannot run while this borrow is alive.
    unsafe { slice::from_raw_parts(self.pixels, self.len()) }
  }

  pub fn pixels_mut(&mut self) -> &mut [u8] {
    // SAFETY: as `pixels`, and the exclusive borrow rules out a shared one.
    unsafe { slice::from_raw_parts_mut(self.pixels, self.len()) }
  }

  fn len(&self) -> usize {
    (self.width as usize) * (self.height as usize) * 4
  }

  pub fn capture_into(&self, desktop: &Desktop, dst: &mut [u8]) -> Result<()> {
    let Desktop { origin, size } = *desktop;
    debug_assert!(self.fits(size), "the section must fit this desktop");
    // SAFETY: both handles belong to `self`, which outlives the blit, and the
    // destination is the section mapped for exactly this width by height.
    unsafe {
      BitBlt(
        self.memory_dc(),
        0,
        0,
        size.0 as i32,
        size.1 as i32,
        Some(self.screen_dc()),
        origin.0,
        origin.1,
        SRCCOPY | CAPTUREBLT,
      )
      .context("BitBlt of the desktop failed")?;
    }
    swap_channels(self.pixels(), dst);
    Ok(())
  }
}

impl Drop for Bitmap {
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
