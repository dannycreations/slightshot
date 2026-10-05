use super::{
  chrome::{tooltip_rect, Chrome, Hotspot},
  frame::{badge_rect, handle_boxes, outline_boxes, EDGE},
  Run, Scene, CARET_WIDTH,
};
use crate::{
  annotate::{stroke_bounds, Shape},
  geom::{Point, Rect},
  text::TextEngine,
};

#[cfg(test)]
#[path = "damage_test.rs"]
mod damage_test;

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
      chrome: *scene.chrome,
      hotspot: scene.hotspot,
      hint: scene.hint.map(str::to_string),
      palette: scene.palette_index,
      draft: scene.draft.cloned(),
      typing: scene
        .typing
        .map(|run| (run.at, run.text.to_owned(), run.size, run.color)),
    }
  }

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

  fn typed(&self) -> Option<Run<'_>> {
    self.typing.as_ref().map(|(at, text, size, color)| Run {
      at: *at,
      text,
      size: *size,
      color: *color,
    })
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
  for button in chrome.buttons() {
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
    Some(Shape::Stroke { points, width, .. }) => {
      stroke_bounds(Rect::around(points), *width)
    }
    Some(Shape::Line {
      from, to, width, ..
    }) => stroke_bounds(Rect::spanning(*from, *to), *width),
    Some(Shape::Outline { rect, width, .. }) => stroke_bounds(*rect, *width),
    Some(Shape::Text { at, text, size, .. }) => engine.inked(text, *at, *size),
    None => Rect::ZERO,
  }
}

pub fn typed_area(typing: Option<Run<'_>>, engine: &TextEngine) -> Rect {
  let Some(run) = typing else {
    return Rect::ZERO;
  };
  let x = run.at.x + engine.width(run.text, run.size);
  // The caret claims its box as a stroke does, the cap above the anchor
  // included. A box that stopped at the anchor would leave every cap behind
  // as the caret steps right.
  let caret = stroke_bounds(
    Rect::spanning(Point::new(x, run.at.y), Point::new(x, run.at.y + run.size)),
    CARET_WIDTH,
  );
  engine.inked(run.text, run.at, run.size).union(caret)
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
