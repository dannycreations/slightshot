use std::{ffi::c_void, mem, sync::Arc, time::Instant};

use anyhow::{bail, Context, Result};
use tiny_skia::{Color, Pixmap};
use windows::{
  core::BOOL,
  Win32::{
    Foundation::{HWND, TRUE},
    Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED},
    UI::WindowsAndMessaging::{SetClassLongPtrW, GCLP_HBRBACKGROUND},
  },
};
use winit::{
  dpi::{PhysicalPosition, PhysicalSize},
  event_loop::ActiveEventLoop,
  keyboard::ModifiersState,
  platform::windows::WindowAttributesExtWindows,
  raw_window_handle::{HasWindowHandle, RawWindowHandle},
  window::{CursorIcon, Window, WindowLevel},
};

use super::{
  mode::Mode,
  present::{solid_surface, Presenter},
  screen::{section, Screen},
};
use crate::{
  annotate::{History, Segment, Shape, Tool, TOOLS},
  capture,
  geom::{Point, Rect},
  layer::Layered,
  render::{self, Backdrop, Chrome, Hotspot, Scene, Shown},
  text::TextEngine,
};

#[cfg(test)]
#[path = "session_test.rs"]
mod session_test;

pub(super) struct Session {
  pub(super) window: Arc<Window>,
  pub(super) backdrop_kind: Backdrop,
  pub(super) base: Option<Pixmap>,
  pub(super) buffers: Screen,
  pub(super) presenter: Presenter,
  pub(super) bounds: Rect,
  pub(super) selection: Option<Rect>,
  pub(super) picked: Option<usize>,
  pub(super) mode: Mode,
  pub(super) lifted: Option<Shape>,
  pub(super) tool: Tool,
  pub(super) palette_index: usize,
  pub(super) history: History,
  pub(super) engine: TextEngine,
  pub(super) cursor: Point,
  pub(super) press: Option<(Instant, Point)>,
  pub(super) raw_path: Vec<Point>,
  pub(super) hover: Option<Hotspot>,
  pub(super) chrome: Chrome,
  pub(super) modifiers: ModifiersState,
  pub(super) sizes: [f32; TOOLS.len()],
  pub(super) hint: Option<(String, Instant)>,
  pub(super) current_cursor: CursorIcon,
  pub(super) painted: Rect,
  pub(super) shown: Option<Shown>,
}

fn opening(bounds: Rect, backdrop: Backdrop) -> (Option<Rect>, Tool) {
  match backdrop {
    Backdrop::Frozen => (None, Tool::Select),
    Backdrop::Live => (Some(bounds), Tool::Pen),
  }
}

impl Session {
  pub(super) fn create(
    event_loop: &ActiveEventLoop,
    backdrop: Backdrop,
    mut buffers: Screen,
  ) -> Result<Self> {
    let desktop = capture::desktop().context("reading the display failed")?;
    let engine = TextEngine::load()?;
    let origin = desktop.origin;
    let size = desktop.size;
    let Screen {
      shot,
      canvas,
      backdrop: dim,
      frame,
    } = &mut buffers;

    // Capture and dim first: everything after this is window plumbing, and
    // doing the pixel work up front means the window is only ever created once
    // there is something real to show in it.
    let canvas = canvas.fit(size)?;
    match backdrop {
      Backdrop::Frozen => section(shot, size)?
        .capture_into(&desktop, canvas.data_mut())
        .context("screen capture failed")?,
      Backdrop::Live => canvas.fill(Color::from_rgba8(0, 0, 0, 1)),
    }
    render::dimmed_into(dim.fit(size)?, canvas);

    let bounds = Rect::new(
      origin.0 as f32,
      origin.1 as f32,
      size.0 as f32,
      size.1 as f32,
    );
    let attributes = Window::default_attributes()
      .with_title("slightshot")
      .with_window_level(WindowLevel::AlwaysOnTop)
      .with_skip_taskbar(true)
      .with_decorations(false)
      .with_resizable(false)
      .with_visible(false)
      .with_inner_size(PhysicalSize::new(bounds.w as f64, bounds.h as f64))
      .with_position(PhysicalPosition::new(bounds.x as f64, bounds.y as f64));
    let window = Arc::new(
      event_loop
        .create_window(attributes)
        .context("creating the overlay window failed")?,
    );
    let Some(handle) = window_handle(&window) else {
      bail!("the overlay window is not a Win32 window");
    };
    quiet_window(handle);
    let presenter = match backdrop {
      Backdrop::Frozen => Presenter::Solid(solid_surface(&window, size)?),
      Backdrop::Live => Presenter::Live(Layered::new(handle, size.0, size.1)?),
    };
    // Reserving the scratch frame here keeps the allocation off the first paint,
    // so a press only ever pays for one it has not made before.
    frame.fit(size)?;

    let (selection, tool) = opening(bounds, backdrop);
    let mut session = Self {
      window,
      backdrop_kind: backdrop,
      base: None,
      buffers,
      presenter,
      bounds,
      selection,
      picked: None,
      mode: Mode::Idle,
      lifted: None,
      tool,
      palette_index: 0,
      history: History::default(),
      engine,
      cursor: Point::default(),
      press: None,
      raw_path: Vec::new(),
      hover: None,
      chrome: Chrome::new(),
      modifiers: ModifiersState::default(),
      sizes: TOOLS.map(Tool::default_size),
      hint: None,
      current_cursor: CursorIcon::default(),
      painted: bounds,
      shown: None,
    };
    session.window.set_visible(true);
    session.render()?;
    Ok(session)
  }

  #[inline]
  pub(super) fn update_cursor(&mut self, icon: CursorIcon) {
    if self.current_cursor != icon {
      self.current_cursor = icon;
      self.window.set_cursor(icon);
    }
  }

  pub(super) fn expire_hint(&mut self) -> bool {
    match self.hint {
      Some((_, until)) if Instant::now() >= until => {
        self.hint = None;
        true
      }
      _ => false,
    }
  }

  pub(super) fn hint_due(&self) -> Option<Instant> {
    self.hint.as_ref().map(|(_, until)| *until)
  }

  pub(super) fn blink(&mut self) {
    let flipped = match &mut self.mode {
      Mode::Type(typing) => typing.caret.flip(Instant::now()),
      _ => false,
    };
    if flipped {
      // The caret is a stroke on the backdrop like any other, so the box the
      // run takes up is all that has to be painted again.
      self.repaint_typing();
    }
  }

  pub(super) fn repaint_typing(&mut self) {
    let area = render::typed_area(self.mode.typing(), &self.engine);
    if !area.is_empty() {
      self.repainted(area);
    }
  }

  pub(super) fn render(&mut self) -> Result<()> {
    self.expire_hint();
    render::build(
      &mut self.chrome,
      self.selection,
      self.bounds,
      self.tool,
      &self.history,
      self.mode.shows_chrome(),
      self.backdrop_kind,
    );
    let draft = self.mode.draft(self.lifted.as_ref());
    let typing = self.mode.typing();
    // The caret is the session's own clock, so whether it is painted is the
    // mode's answer and not something the picture can be asked for.
    let caret = self.mode.caret().is_some_and(|caret| caret.lit());
    // A run that has been picked keeps its frame until a press reaches no run
    // at all, whatever tool is in hand: it is an object, not a mode.
    let picked = self.picked_box();
    let hint = self.hint.as_ref().map(|(text, _)| text.as_str());
    let Screen {
      canvas,
      backdrop,
      frame,
      ..
    } = &mut self.buffers;
    let canvas = canvas.at();
    let backdrop = backdrop.at();
    let frame = frame.at();
    let scene = Scene {
      backdrop,
      canvas,
      bounds: self.bounds,
      selection: self.selection,
      picked,
      kind: self.backdrop_kind,
      draft,
      typing,
      caret,
      palette_index: self.palette_index,
      chrome: &self.chrome,
      hotspot: self.hover,
      text: &self.engine,
      hint,
    };
    // A frame only has to be redrawn where the picture or the chrome above it
    // moved since the last one, and the rest of the buffer still holds what
    // that frame drew. The box is in frame pixels, which is also what the
    // presenter writes from.
    let shown = Shown::capture(&scene);
    let moved = match &self.shown {
      Some(previous) => {
        previous.settled_from(&shown, self.bounds, &self.engine)
      }
      None => Rect::ZERO,
    };
    let pixels =
      Rect::new(0.0, 0.0, frame.width() as f32, frame.height() as f32);
    let area = self.painted.union(moved).clamped_inside(pixels);
    self.shown = Some(shown);
    self.painted = Rect::ZERO;
    if area.is_empty() {
      // Nothing of ours moved, but the system can still be asking because the
      // window was covered and uncovered. The frame holds the whole picture, so
      // hand it over again rather than leave what was on top showing through.
      return self.presenter.present(frame, pixels);
    }
    render::paint(frame, &scene, area);
    self.presenter.present(frame, area)
  }

  pub(super) fn repainted(&mut self, area: Rect) {
    self.painted = self.painted.union(area);
    self.window.request_redraw();
  }

  pub(super) fn snapshot(&mut self) {
    if self.base.is_some() {
      return;
    }
    self.base = Some(self.buffers.canvas.at().clone());
  }

  pub(super) fn rebuild(&mut self, area: Rect) {
    self.snapshot();
    let Some(base) = &self.base else {
      // Nothing was ever inked, so the canvas is still the capture and there is
      // nothing to put back.
      return;
    };
    let Screen {
      canvas, backdrop, ..
    } = &mut self.buffers;
    let laid = render::repaint(
      canvas.at(),
      backdrop.at(),
      base,
      self.history.shapes(),
      area,
      &self.engine,
    );
    self.repainted(laid);
  }

  pub(super) fn ink_segment(&mut self, segment: Segment) {
    self.snapshot();
    let area = segment.bounds();
    let Screen {
      canvas, backdrop, ..
    } = &mut self.buffers;
    render::ink_segment(canvas.at(), backdrop.at(), segment);
    self.repainted(area);
  }

  pub(super) fn lift_text(&mut self, index: usize) -> Option<Shape> {
    let slot = self.history.shape_mut(index)?;
    let Shape::Text {
      at,
      text,
      color,
      size,
    } = slot
    else {
      return None;
    };
    // The buffer moves out of the slot whole rather than being copied and
    // then cleared, which is the same empty run left behind.
    let run = Shape::Text {
      at: *at,
      text: mem::take(text),
      color: *color,
      size: *size,
    };
    // The damage has to be the ink and not the line the run sits on: a
    // descender reaches below the line and a hook can reach left of it, and
    // whatever is left of a letterform stays on the canvas for the rest of
    // the session.
    let area = render::shape_area(Some(&run), &self.engine);
    self.rebuild(area);
    Some(run)
  }

  pub(super) fn land_text(&mut self, index: usize, shape: Shape) {
    let area = render::shape_area(Some(&shape), &self.engine);
    if let Some(slot) = self.history.shape_mut(index) {
      *slot = shape;
    }
    self.rebuild(area);
  }
}

fn window_handle(window: &Window) -> Option<HWND> {
  let handle = window.window_handle().ok()?;
  let RawWindowHandle::Win32(win32) = handle.as_raw() else {
    return None;
  };
  Some(HWND(win32.hwnd.get() as *mut c_void))
}

fn quiet_window(hwnd: HWND) {
  // SAFETY: `hwnd` is the handle of the window just created, which outlives
  // this call. Both writes target only that window: the class brush is set to
  // null, and the DWM attribute is a plain value write.
  unsafe {
    SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, 0);
    let disable: BOOL = TRUE;
    let _ = DwmSetWindowAttribute(
      hwnd,
      DWMWA_TRANSITIONS_FORCEDISABLED,
      &disable as *const BOOL as *const c_void,
      mem::size_of::<BOOL>() as u32,
    );
  }
}
