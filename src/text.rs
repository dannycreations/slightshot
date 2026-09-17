use std::{
  cell::RefCell,
  collections::HashMap,
  env, fs,
  path::Path,
  rc::Rc,
  sync::{Arc, OnceLock},
};

use anyhow::{anyhow, Result};
use fontdue::{Font, FontSettings, Metrics};
use tiny_skia::Pixmap;

const FONT_FILES: [&str; 4] =
  ["segoeui.ttf", "arial.ttf", "tahoma.ttf", "calibri.ttf"];
const ASCENT_RATIO: f32 = 0.8;

struct Glyph {
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
  font: Option<Arc<Font>>,
  cache: RefCell<HashMap<(char, u32), Rc<Glyph>>>,
}

impl TextEngine {
  pub fn load() -> Result<Self> {
    let font = get_system_font()
      .ok_or_else(|| anyhow!("no system font found under Windows\\Fonts"))?;
    Ok(Self {
      font: Some(font),
      cache: RefCell::new(HashMap::new()),
    })
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
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    for ch in text.chars() {
      let glyph = self.raster(ch, size);
      let m = &glyph.metrics;
      if m.width > 0 && m.height > 0 {
        let left = pen + m.xmin as f32;
        let top = baseline - (m.ymin + m.height as i32) as f32;
        Self::blend(
          pm.data_mut(),
          pw,
          ph,
          left.round() as i32,
          top.round() as i32,
          &glyph,
          rgb,
        );
      }
      pen += m.advance_width;
    }
  }

  fn raster(&self, ch: char, size: f32) -> Rc<Glyph> {
    let key = (ch, size.to_bits());
    if let Some(glyph) = self.cache.borrow().get(&key) {
      return Rc::clone(glyph);
    }
    let Some(font) = self.font.as_ref() else {
      return Rc::new(Glyph {
        metrics: Metrics::default(),
        coverage: Vec::new(),
      });
    };
    let (metrics, coverage) = font.rasterize(ch, size);
    let glyph = Rc::new(Glyph { metrics, coverage });
    self.cache.borrow_mut().insert(key, Rc::clone(&glyph));
    glyph
  }

  pub fn width(&self, text: &str, size: f32) -> f32 {
    let mut total = 0.0;
    for ch in text.chars() {
      let key = (ch, size.to_bits());
      if let Some(glyph) = self.cache.borrow().get(&key) {
        total += glyph.metrics.advance_width;
        continue;
      }
      if let Some(font) = &self.font {
        total += font.metrics(ch, size).advance_width;
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
      let py = (gy + row as i32) as usize;
      let px = (gx + col_start as i32) as usize;
      let cov_offset = row * gw as usize + col_start;
      let coverage = &glyph.coverage[cov_offset..cov_offset + cols];
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
          chunk[0] = ((chunk[0] as u32 * inv + r * a) / 255) as u8;
          chunk[1] = ((chunk[1] as u32 * inv + g * a) / 255) as u8;
          chunk[2] = ((chunk[2] as u32 * inv + b * a) / 255) as u8;
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
