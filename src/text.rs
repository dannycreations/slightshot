use std::{env, fs, path::Path};

use anyhow::{anyhow, bail, Result};
use fontdue::{Font, FontSettings};
use tiny_skia::Pixmap;

const FONT_FILES: [&str; 4] =
  ["segoeui.ttf", "arial.ttf", "tahoma.ttf", "calibri.ttf"];
const ASCENT_RATIO: f32 = 0.8;

#[derive(Default)]
pub struct TextEngine {
  fonts: Vec<fontdue::Font>,
}

impl TextEngine {
  pub fn load() -> Result<Self> {
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
      let font = Font::from_bytes(bytes, settings).map_err(|reason| {
        anyhow!("{} is not a usable font: {reason}", path.display())
      })?;
      return Ok(Self { fonts: vec![font] });
    }
    bail!("no system font found under {}", fonts.display())
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
      let (m, cov) = self.fonts[0].rasterize(ch, size);
      if m.width > 0 && m.height > 0 {
        let left = pen + m.xmin as f32;
        let top = baseline - (m.ymin + m.height as i32) as f32;
        Self::blend(
          pm.data_mut(),
          pw,
          ph,
          left.round() as i32,
          top.round() as i32,
          &cov,
          m.width,
          m.height,
          rgb,
        );
      }
      pen += m.advance_width;
    }
  }

  pub fn width(&self, text: &str, size: f32) -> f32 {
    let mut total = 0.0;
    for ch in text.chars() {
      let (m, _) = self.fonts[0].rasterize(ch, size);
      total += m.advance_width;
    }
    total
  }

  #[allow(clippy::too_many_arguments)]
  fn blend(
    pm: &mut [u8],
    pw: i32,
    ph: i32,
    gx: i32,
    gy: i32,
    coverage: &[u8],
    gw: usize,
    gh: usize,
    rgb: [u8; 3],
  ) {
    let mut i = 0;
    for row in 0..gh as i32 {
      for col in 0..gw as i32 {
        let px = gx + col;
        let py = gy + row;
        if px >= 0 && px < pw && py >= 0 && py < ph {
          let a = coverage[i] as f32 / 255.0;
          let di = ((py * pw + px) * 4) as usize;
          for c in 0..3 {
            let base = pm[di + c] as f32;
            pm[di + c] =
              (base * (1.0 - a) + rgb[c] as f32 * a).min(255.0) as u8;
          }
          pm[di + 3] = (pm[di + 3] as f32 + 255.0 * a).min(255.0) as u8;
        }
        i += 1;
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn empty_string_has_zero_width_without_loading_a_font() {
    let engine = TextEngine { fonts: Vec::new() };
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
