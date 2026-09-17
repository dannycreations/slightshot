use std::{
  cell::RefCell,
  collections::hash_map::{Entry, HashMap},
  env, fs,
  path::Path,
  sync::{Arc, OnceLock},
};

use anyhow::{anyhow, Result};
use fontdue::{Font, FontSettings, Metrics};
use tiny_skia::Pixmap;

const FONT_FILES: [&str; 4] =
  ["segoeui.ttf", "arial.ttf", "tahoma.ttf", "calibri.ttf"];
const ASCENT_RATIO: f32 = 0.8;

pub struct Glyph {
  metrics: Metrics,
  coverage: Vec<u8>,
}

static SYSTEM_FONT: OnceLock<Option<Arc<Font>>> = OnceLock::new();

fn get_system_font() -> Option<Arc<Font>> {
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
          return Some(Arc::new(font));
        }
      }
      None
    })
    .clone()
}

#[derive(Default)]
pub struct TextEngine {
  pub font: Option<Arc<Font>>,
  pub cache: RefCell<HashMap<(char, u32), Glyph>>,
}

impl TextEngine {
  pub fn load() -> Result<Self> {
    let font = get_system_font()
      .ok_or_else(|| anyhow!("no system font found under Windows\\Fonts"))?;
    Ok(Self {
      font: Some(font),
      cache: RefCell::new(HashMap::with_capacity(64)),
    })
  }

  fn glyph_for<'a>(
    cache: &'a mut HashMap<(char, u32), Glyph>,
    font: Option<&Font>,
    ch: char,
    size: f32,
    size_bits: u32,
  ) -> Option<&'a Glyph> {
    match cache.entry((ch, size_bits)) {
      Entry::Occupied(entry) => Some(entry.into_mut()),
      Entry::Vacant(entry) => {
        let font = font?;
        let (metrics, coverage) = font.rasterize(ch, size);
        Some(entry.insert(Glyph { metrics, coverage }))
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
    let size_bits = size.to_bits();
    let baseline = y + size * ASCENT_RATIO;
    let mut pen = x;
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let pm_data = pm.data_mut();

    let mut cache = self.cache.borrow_mut();
    for ch in text.chars() {
      let Some(glyph) =
        Self::glyph_for(&mut cache, self.font.as_deref(), ch, size, size_bits)
      else {
        continue;
      };

      let m = &glyph.metrics;
      if m.width > 0 && m.height > 0 {
        let left = pen + m.xmin as f32;
        let top = baseline - (m.ymin + m.height as i32) as f32;
        Self::blend(
          pm_data,
          pw,
          ph,
          left.round() as i32,
          top.round() as i32,
          glyph,
          rgb,
        );
      }
      pen += m.advance_width;
    }
  }

  pub fn width(&self, text: &str, size: f32) -> f32 {
    let size_bits = size.to_bits();
    let mut total = 0.0;
    let mut cache = self.cache.borrow_mut();
    for ch in text.chars() {
      if let Some(glyph) =
        Self::glyph_for(&mut cache, self.font.as_deref(), ch, size, size_bits)
      {
        total += glyph.metrics.advance_width;
      }
    }
    total
  }

  fn blend(
    pm: &mut [u8],
    pw: i32,
    ph: i32,
    gx: i32,
    gy: i32,
    glyph: &Glyph,
    rgb: [u8; 3],
  ) {
    let gw = glyph.metrics.width as i32;
    let gh = glyph.metrics.height as i32;
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
    let col_end = (gw.min(pw - gx)) as usize;
    let row_start = (-gy).max(0) as usize;
    let row_end = (gh.min(ph - gy)) as usize;
    let cols = col_end - col_start;
    if cols == 0 {
      return;
    }

    let (r, g, b) = (rgb[0] as u32, rgb[1] as u32, rgb[2] as u32);

    for row in row_start..row_end {
      let cov_offset = row * gw as usize + col_start;
      let coverage = &glyph.coverage[cov_offset..cov_offset + cols];

      if coverage.iter().all(|&c| c == 0) {
        continue;
      }

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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn empty_string_has_zero_width_without_loading_a_font() {
    let engine = TextEngine {
      font: None,
      cache: RefCell::new(HashMap::new()),
    };
    assert_eq!(engine.width("", 20.0), 0.0);
  }

  #[test]
  fn draw_blends_a_glyph_without_panicking() {
    let Ok(engine) = TextEngine::load() else {
      return;
    };
    let mut pm = Pixmap::new(60, 30).unwrap();
    pm.data_mut().iter_mut().for_each(|p| *p = 0);
    engine.draw(&mut pm, "Hi", 4.0, 22.0, 20.0, [255, 255, 255]);
    let lit = pm.data().as_chunks::<4>().0.iter().any(|p| p[3] > 0);
    assert!(lit, "expected at least one lit pixel after drawing text");
  }

  #[test]
  fn width_scales_with_font_size() {
    let Ok(engine) = TextEngine::load() else {
      return;
    };
    let small = engine.width("MM", 10.0);
    let large = engine.width("MM", 40.0);
    assert!(large > small, "larger text should be wider");
  }
}
