use std::{cell::RefCell, collections::HashMap, sync::OnceLock};

use tiny_skia::{Pixmap, PixmapPaint, Transform};

use crate::geom::Point;

#[cfg(test)]
#[path = "icon_test.rs"]
mod icon_test;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(usize)]
pub enum Icon {
  Pen = 0,
  Marker,
  Arrow,
  Outline,
  Line,
  Letter,
  Undo,
  Upload,
  CopyImage,
  Save,
  Close,
}

impl Icon {
  const COUNT: usize = 11;

  pub fn paint(
    self,
    pm: &mut Pixmap,
    center: Point,
    box_size: f32,
    color: [u8; 3],
  ) {
    let x = (center.x - box_size * 0.5).round() as i32;
    let y = (center.y - box_size * 0.5).round() as i32;
    render_tinted_sprite(self, pm, color, box_size, x, y);
  }
}

type TintCache = HashMap<(usize, [u8; 3], u32), Pixmap>;

thread_local! {
  static TINT_CACHE: RefCell<TintCache> =
    RefCell::new(HashMap::with_capacity(64));
}

fn create_tinted_sprite(icon: Icon, color: [u8; 3], box_size: f32) -> Pixmap {
  let source = sprite(icon);
  let scale = box_size / source.width() as f32;
  let w = (source.width() as f32 * scale).round() as u32;
  let h = (source.height() as f32 * scale).round() as u32;
  let mut tinted =
    Pixmap::new(w, h).expect("allocating the icon pixmap failed");
  tinted.draw_pixmap(
    0,
    0,
    source.as_ref(),
    &PixmapPaint::default(),
    Transform::from_scale(scale, scale),
    None,
  );
  let (cr, cg, cb) = (color[0] as u32, color[1] as u32, color[2] as u32);
  for pixel in tinted.data_mut().as_chunks_mut::<4>().0 {
    let a = pixel[3] as u32;
    pixel[0] = (((cr * a + 128) * 257) >> 16) as u8;
    pixel[1] = (((cg * a + 128) * 257) >> 16) as u8;
    pixel[2] = (((cb * a + 128) * 257) >> 16) as u8;
  }
  tinted
}

fn render_tinted_sprite(
  icon: Icon,
  pm: &mut Pixmap,
  color: [u8; 3],
  box_size: f32,
  x: i32,
  y: i32,
) {
  TINT_CACHE.with(|cache| {
    let mut map = cache.borrow_mut();
    let key = (icon as usize, color, box_size.to_bits());
    let tinted = map
      .entry(key)
      .or_insert_with(|| create_tinted_sprite(icon, color, box_size));

    pm.draw_pixmap(
      x,
      y,
      tinted.as_ref(),
      &PixmapPaint::default(),
      Transform::identity(),
      None,
    );
  });
}

static SPRITE_CACHE: [OnceLock<Pixmap>; Icon::COUNT] =
  [const { OnceLock::new() }; Icon::COUNT];

fn sprite(icon: Icon) -> &'static Pixmap {
  SPRITE_CACHE[icon as usize].get_or_init(|| {
    Pixmap::decode_png(sprite_bytes(icon))
      .expect("decoding the embedded icon PNG failed")
  })
}

fn sprite_bytes(icon: Icon) -> &'static [u8] {
  match icon {
    Icon::Pen => include_bytes!("../icons/pencil.png"),
    Icon::Marker => include_bytes!("../icons/highlighter.png"),
    Icon::Arrow => include_bytes!("../icons/arrow-up-right.png"),
    Icon::Outline => include_bytes!("../icons/square.png"),
    Icon::Line => include_bytes!("../icons/minus.png"),
    Icon::Letter => include_bytes!("../icons/type.png"),
    Icon::Undo => include_bytes!("../icons/undo-2.png"),
    Icon::Upload => include_bytes!("../icons/upload.png"),
    Icon::CopyImage => include_bytes!("../icons/copy.png"),
    Icon::Save => include_bytes!("../icons/save.png"),
    Icon::Close => include_bytes!("../icons/close.png"),
  }
}
