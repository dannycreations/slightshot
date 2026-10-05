use super::{
  chrome::{tooltip_rect, Chrome, Hotspot},
  frame::{badge_rect, handle_boxes, outline_boxes, EDGE},
  Scene, CARET_WIDTH,
};
use crate::{
  annotate::Shape,
  geom::{Point, Rect},
  text::TextEngine,
};

#[cfg(test)]
#[path = "damage_test.rs"]
mod damage_test;

/// Everything the last frame put on screen, held so that the next frame can be
/// told only the box that changed. Without it every repaint would have to
/// rebuild the whole picture, because a shape the user just moved still sits on
/// the canvas where it used to be.
#[derive(Debug, PartialEq)]
pub struct Shown {
  selection: Option<Rect>,
  picked: Option<Rect>,
  chrome: Chrome,
  hotspot: Option<Hotspot>,
  hint: Option<String>,
  palette: usize,
  draft: Option<Shape>,
  typing: Option<(Point, String, f32, [u8; 3])>,
}

impl Shown {
  pub fn capture(scene: &Scene) -> Self {
    Self {
      selection: scene.selection,
      picked: scene.picked,
      chrome: scene.chrome.clone(),
      hotspot: scene.hotspot,
      hint: scene.hint.map(str::to_string),
      palette: scene.palette_index,
      draft: scene.draft.cloned(),
      typing: scene
        .typing
        .map(|(at, buffer, size, color)| (at, buffer.to_string(), size, color)),
    }
  }

  /// The box to repaint so that `next` replaces `self` on screen.
  pub fn settled_from(
    &self,
    next: &Shown,
    bounds: Rect,
    engine: &TextEngine,
  ) -> Rect {
    let mut area = Rect::ZERO;
    if self.selection != next.selection {
      area = area
        .union(region_area(self.selection, bounds, engine))
        .union(region_area(next.selection, bounds, engine));
    }
    if self.picked != next.picked {
      area = area
        .union(picked_area(self.picked))
        .union(picked_area(next.picked));
    }
    // A new colour repaints the panels as well, because the swatch on the
    // colour button is the one thing on them that takes it.
    let recolored = self.palette != next.palette;
    if self.chrome != next.chrome
      || self.hotspot != next.hotspot
      || self.hint != next.hint
      || recolored
    {
      area = area
        .union(self.panels_area(bounds, engine))
        .union(next.panels_area(bounds, engine));
    }
    if self.draft != next.draft {
      area = area
        .union(shape_area(self.draft.as_ref(), engine))
        .union(shape_area(next.draft.as_ref(), engine));
    }
    // The text being typed is the other thing that takes the active colour.
    if self.typing != next.typing || recolored {
      area = area
        .union(typed_area(self.typed(), engine))
        .union(typed_area(next.typed(), engine));
    }
    area
  }

  fn panels_area(&self, bounds: Rect, engine: &TextEngine) -> Rect {
    panels_area(
      &self.chrome,
      self.hotspot,
      self.hint.as_deref(),
      bounds,
      engine,
    )
  }

  fn typed(&self) -> Option<(Point, &str, f32, [u8; 3])> {
    self
      .typing
      .as_ref()
      .map(|(at, buffer, size, color)| (*at, buffer.as_str(), *size, *color))
  }
}

fn region_area(sel: Option<Rect>, bounds: Rect, engine: &TextEngine) -> Rect {
  match sel {
    Some(sel) => sel
      .union(framed_area(sel))
      .union(badge_rect(sel, bounds, engine).inflated(EDGE)),
    None => Rect::ZERO,
  }
}

pub(super) fn framed_area(sel: Rect) -> Rect {
  bounding(&outline_boxes(sel)).union(bounding(&handle_boxes(sel)))
}

pub(super) fn picked_area(sel: Option<Rect>) -> Rect {
  sel.map(framed_area).unwrap_or(Rect::ZERO)
}

pub(super) fn panels_area(
  chrome: &Chrome,
  hotspot: Option<Hotspot>,
  hint: Option<&str>,
  bounds: Rect,
  engine: &TextEngine,
) -> Rect {
  let mut area = Rect::ZERO;
  for button in chrome.tools.iter().chain(&chrome.actions) {
    // A button that was never laid out sits on `Rect::ZERO`, which is how the
    // chrome says "no button here". Inflating that sentinel would turn it
    // into a live box at the origin, and the union would then stretch every
    // repaint back to the top left of the screen.
    if button.shown() {
      area = area.union(button.area.inflated(EDGE));
    }
  }
  for (at, label, side) in chrome.tooltips(hotspot, hint) {
    area = area.union(tooltip_rect(at, label, bounds, engine, side));
  }
  area
}

pub fn shape_area(shape: Option<&Shape>, engine: &TextEngine) -> Rect {
  match shape {
    // Half the width past each end for the round caps, plus the
    // antialiased pixel outside them.
    Some(Shape::Stroke { points, width, .. }) => {
      Rect::around(points).inflated(width * 0.5 + 1.0)
    }
    Some(Shape::Line {
      from, to, width, ..
    }) => Rect::spanning(*from, *to).inflated(width * 0.5 + 1.0),
    Some(Shape::Outline { rect, width, .. }) => {
      rect.inflated(width * 0.5 + 1.0)
    }
    Some(Shape::Text { at, text, size, .. }) => engine.inked(text, *at, *size),
    None => Rect::ZERO,
  }
}

pub fn typed_area(
  typing: Option<(Point, &str, f32, [u8; 3])>,
  engine: &TextEngine,
) -> Rect {
  match typing {
    Some((at, buffer, size, _)) => {
      let x = at.x + engine.width(buffer, size);
      // The caret is a round-capped stroke, so like every other stroke it
      // paints half its width past each end, the one above the anchor
      // included, plus the antialiased pixel outside that. A box that stops at
      // the anchor would leave every cap behind as the caret steps right.
      let caret =
        Rect::spanning(Point::new(x, at.y), Point::new(x, at.y + size))
          .inflated(CARET_WIDTH * 0.5 + 1.0);
      engine.inked(buffer, at, size).union(caret)
    }
    None => Rect::ZERO,
  }
}

pub(super) fn touches(piece: &[Rect], area: Rect) -> bool {
  piece.iter().any(|part| part.overlaps(area))
}

pub(super) fn bounding(piece: &[Rect]) -> Rect {
  piece
    .iter()
    .fold(Rect::ZERO, |area, part| area.union(*part))
}

pub(super) fn inside(piece: &[Rect], area: Rect) -> bool {
  piece.iter().all(|part| area.contains_rect(*part))
}
