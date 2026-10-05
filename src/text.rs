use std::{
  cell::RefCell,
  collections::{hash_map::Entry, HashMap},
  env, fs,
  path::Path,
  sync::OnceLock,
  thread,
};

use anyhow::{anyhow, Result};
use fontdue::{Font, FontSettings, Metrics};
use tiny_skia::Pixmap;

use crate::geom::{Point, Rect};

#[cfg(test)]
#[path = "text_test.rs"]
mod text_test;

const FONT_FILES: [&str; 4] =
  ["segoeui.ttf", "arial.ttf", "tahoma.ttf", "calibri.ttf"];
const ASCENT_RATIO: f32 = 0.8;

#[derive(Clone)]
struct GlyphInfo {
  metrics: Metrics,
  offset: u32,
  len: u32,
}

static SYSTEM_FONT: OnceLock<Option<Font>> = OnceLock::new();

fn system_font() -> Option<&'static Font> {
  SYSTEM_FONT
    .get_or_init(|| {
      let windir =
        env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
      let fonts = Path::new(&windir).join("Fonts");
      for name in FONT_FILES {
        let path = fonts.join(name);
        let Ok(bytes) = fs::read(&path) else {
          continue;
        };
        let settings = FontSettings {
          collection_index: 0,
          scale: 40.0,
          load_substitutions: false,
        };
        if let Ok(font) = Font::from_bytes(bytes, settings) {
          return Some(font);
        }
      }
      None
    })
    .as_ref()
}

pub struct TextEngine {
  font: &'static Font,
  glyphs: RefCell<HashMap<(char, u32), GlyphInfo>>,
  atlas: RefCell<Vec<u8>>,
}

pub fn warm() {
  thread::spawn(|| {
    system_font();
  });
}

impl TextEngine {
  pub fn load() -> Result<Self> {
    let font = system_font()
      .ok_or_else(|| anyhow!("no system font found under Windows\\Fonts"))?;
    Ok(Self {
      font,
      glyphs: RefCell::new(HashMap::with_capacity(64)),
      atlas: RefCell::new(Vec::with_capacity(64 * 64)),
    })
  }

  fn glyph_info(&self, ch: char, size: f32) -> GlyphInfo {
    match self.glyphs.borrow_mut().entry((ch, size.to_bits())) {
      Entry::Occupied(entry) => entry.get().clone(),
      Entry::Vacant(entry) => {
        let (metrics, coverage) = self.font.rasterize(ch, size);
        let mut atlas = self.atlas.borrow_mut();
        let offset = atlas.len() as u32;
        atlas.extend_from_slice(&coverage);
        let info = GlyphInfo {
          metrics,
          offset,
          len: coverage.len() as u32,
        };
        entry.insert(info.clone());
        info
      }
    }
  }

  pub fn draw(
    &self,
    pm: &mut Pixmap,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    rgb: [u8; 3],
  ) {
    let baseline = y + size * ASCENT_RATIO;
    let mut pen = x;

    for ch in text.chars() {
      let info = self.glyph_info(ch, size);
      let m = &info.metrics;
      let left = pen + m.xmin as f32;
      let top = baseline - (m.ymin + m.height as i32) as f32;
      let atlas = self.atlas.borrow();
      let coverage =
        &atlas[info.offset as usize..(info.offset + info.len) as usize];
      Self::blend(
        pm,
        left.round() as i32,
        top.round() as i32,
        m,
        coverage,
        rgb,
      );
      pen += m.advance_width;
    }
  }

  pub fn width(&self, text: &str, size: f32) -> f32 {
    text
      .chars()
      .map(|ch| self.glyph_info(ch, size).metrics.advance_width)
      .sum()
  }

  pub fn bounds(&self, text: &str, at: Point, size: f32) -> Rect {
    let width = self.width(text, size);
    if width <= 0.0 {
      return Rect::ZERO;
    }
    Rect::new(at.x, at.y, width, size)
  }

  pub fn inked(&self, text: &str, at: Point, size: f32) -> Rect {
    let baseline = at.y + size * ASCENT_RATIO;
    let mut pen = at.x;
    let mut area = Rect::ZERO;
    for ch in text.chars() {
      let info = self.glyph_info(ch, size);
      let metrics = &info.metrics;
      area = area.union(Rect::new(
        pen + metrics.xmin as f32,
        baseline - (metrics.ymin + metrics.height as i32) as f32,
        metrics.width as f32,
        metrics.height as f32,
      ));
      pen += metrics.advance_width;
    }
    // A run that inks nothing, be it no characters at all or nothing but
    // spaces, reports no box: inflating the empty rect would leave one
    // sitting at the origin for a click to land on.
    if area.is_empty() {
      return Rect::ZERO;
    }
    area.inflated(1.0)
  }

  fn blend(
    pm: &mut Pixmap,
    gx: i32,
    gy: i32,
    metrics: &Metrics,
    coverage: &[u8],
    rgb: [u8; 3],
  ) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let gw = metrics.width as i32;
    let gh = metrics.height as i32;
    if gw <= 0
      || gh <= 0
      || gx >= pw
      || gy >= ph
      || gx + gw <= 0
      || gy + gh <= 0
    {
      return;
    }

    let col_start = (-gx).max(0) as usize;
    let cols = gw.min(pw - gx) as usize - col_start;
    let row_start = (-gy).max(0) as usize;
    let row_end = (gh.min(ph - gy)) as usize;
    let (r, g, b) = (rgb[0] as u32, rgb[1] as u32, rgb[2] as u32);
    let pm = pm.data_mut();

    for row in row_start..row_end {
      let cov_offset = row * gw as usize + col_start;
      let coverage = &coverage[cov_offset..cov_offset + cols];

      let py = (gy + row as i32) as usize;
      let px = (gx + col_start as i32) as usize;
      let di = (py * pw as usize + px) * 4;
      let dest = &mut pm[di..di + cols * 4];

      for (alpha_cov, chunk) in coverage.iter().zip(dest.as_chunks_mut::<4>().0)
      {
        let a = *alpha_cov as u32;
        if a == 0 {
          continue;
        }
        if a == 255 {
          chunk[0] = rgb[0];
          chunk[1] = rgb[1];
          chunk[2] = rgb[2];
          chunk[3] = 255;
        } else {
          let inv = 255 - a;
          let r_val = chunk[0] as u32 * inv + r * a;
          let g_val = chunk[1] as u32 * inv + g * a;
          let b_val = chunk[2] as u32 * inv + b * a;
          chunk[0] = ((r_val * 32897) >> 23) as u8;
          chunk[1] = ((g_val * 32897) >> 23) as u8;
          chunk[2] = ((b_val * 32897) >> 23) as u8;
          chunk[3] = (chunk[3] as u32 + a).min(255) as u8;
        }
      }
    }
  }
}
