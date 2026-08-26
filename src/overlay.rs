use std::{ffi::c_void, num::NonZeroU32, sync::Arc};

use anyhow::{anyhow, Context, Result};
use softbuffer::{Context as SoftContext, Surface as SoftSurface};
use tiny_skia::Pixmap;
use windows::{
  core::BOOL,
  Win32::{
    Foundation::{HWND, TRUE},
    Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED},
    UI::WindowsAndMessaging::{SetClassLongPtrW, GCLP_HBRBACKGROUND},
  },
};
use winit::{
  application::ApplicationHandler,
  dpi::{PhysicalPosition, PhysicalSize},
  event::{ElementState, KeyEvent, MouseButton, WindowEvent},
  event_loop::ActiveEventLoop,
  keyboard::{Key, NamedKey},
  platform::windows::WindowAttributesExtWindows,
  raw_window_handle::{HasWindowHandle, RawWindowHandle},
  window::{CursorIcon, Window, WindowId, WindowLevel},
};

use crate::{
  actions::{self, Deliverable, Shot},
  annotate::{History, Shape, Tool, LINE_WIDTH, PALETTE, PEN_WIDTH},
  capture,
  geom::{hit_handle, resized, Handle, Point, Rect},
  hotkey::Trigger,
  render::{self, Hotspot, Scene, HANDLE_SLOP},
  text::TextEngine,
};

pub struct Outcome {
  pub deliverable: Deliverable,
  pub shot: Shot,
}

impl Mode {
  fn draft(&self) -> Option<&Shape> {
    match self {
      Mode::Draw(shape, _) => Some(shape),
      _ => None,
    }
  }
}

#[derive(Default)]
enum Mode {
  #[default]
  Idle,
  Rubber(f32, f32),
  Draw(Shape, Point),
  Move(f32, f32),
  Resize(Handle, Rect),
  Type(String, Point),
}

type Surface = SoftSurface<Arc<Window>, Arc<Window>>;

struct Session {
  window: Arc<Window>,
  canvas: Pixmap,
  backdrop: Pixmap,
  frame: Pixmap,
  surface: Surface,
  bounds: Rect,
  selection: Option<Rect>,
  mode: Mode,
  tool: Tool,
  palette_index: usize,
  history: History,
  engine: TextEngine,
  cursor: Point,
  hover: Option<Hotspot>,
  pending: Option<Outcome>,
}

#[derive(Default)]
pub struct App {
  session: Option<Session>,
}

impl ApplicationHandler<Trigger> for App {
  fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

  fn window_event(
    &mut self,
    _event_loop: &ActiveEventLoop,
    _id: WindowId,
    event: WindowEvent,
  ) {
    let Some(session) = self.session.as_mut() else {
      return;
    };
    match event {
      WindowEvent::CloseRequested => self.session = None,
      WindowEvent::RedrawRequested => session.render(),
      WindowEvent::CursorMoved { position, .. } => session.mouse_move(position),
      WindowEvent::MouseInput {
        state: ElementState::Pressed,
        button: MouseButton::Left,
        ..
      } => session.mouse_down(),
      WindowEvent::MouseInput {
        state: ElementState::Released,
        button: MouseButton::Left,
        ..
      } => {
        if let Some(outcome) = session.mouse_up() {
          self.finish(outcome);
        }
      }
      WindowEvent::KeyboardInput {
        event:
          KeyEvent {
            state: ElementState::Pressed,
            logical_key: Key::Named(NamedKey::Escape),
            ..
          },
        ..
      } => self.session = None,
      WindowEvent::KeyboardInput {
        event:
          KeyEvent {
            state: ElementState::Pressed,
            logical_key: Key::Named(NamedKey::Enter),
            ..
          },
        ..
      } => {
        if let Some(outcome) = session.commit_label() {
          self.finish(outcome);
        }
      }
      WindowEvent::KeyboardInput {
        event:
          KeyEvent {
            state: ElementState::Pressed,
            logical_key: Key::Character(ch),
            ..
          },
        ..
      } => session.type_char(ch.as_str()),
      WindowEvent::KeyboardInput {
        event:
          KeyEvent {
            state: ElementState::Pressed,
            logical_key: Key::Named(NamedKey::Backspace),
            ..
          },
        ..
      } => session.backspace(),
      _ => {}
    }
  }

  fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Trigger) {
    match event {
      Trigger::Capture => {
        if self.session.is_none() {
          match Session::create(event_loop) {
            Ok(session) => self.session = Some(session),
            Err(error) => {
              eprintln!("slightshot: could not lock the screen for selection: {error:#}")
            }
          }
        }
      }
      Trigger::Quit => event_loop.exit(),
    }
  }
}

impl App {
  fn finish(&mut self, outcome: Outcome) {
    match actions::execute(outcome.deliverable, &outcome.shot) {
      Ok(summary) => println!("slightshot: {summary}"),
      Err(error) => eprintln!("slightshot: {error:#}"),
    }
    self.session = None;
  }
}

impl Session {
  fn create(event_loop: &ActiveEventLoop) -> Result<Self> {
    let shot = capture::grab().context("screen capture failed")?;
    let origin = shot.origin;
    let canvas = shot.pixmap;
    let bounds = Rect::new(
      origin.0 as f32,
      origin.1 as f32,
      canvas.width() as f32,
      canvas.height() as f32,
    );
    let backdrop = render::dimmed_copy(&canvas);
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
    // SAFETY: `hwnd` comes from a winit window alive for the whole overlay
    // lifetime. `SetClassLongPtrW` only clears the class background brush so
    // Windows stops erasing the client area to white before our first frame
    // composites (the white flash seen on capture). `DwmSetWindowAttribute`
    // with `DWMWA_TRANSITIONS_FORCEDISABLED` disables the DWM open/close fade so
    // the dimmed overlay appears the instant the window is shown instead of
    // fading in over ~200ms. Both are documented Win32 calls that neither move
    // nor free the window or its class.
    if let Ok(handle) = window.window_handle() {
      if let RawWindowHandle::Win32(w) = handle.as_raw() {
        let hwnd = HWND(w.hwnd.get() as *mut c_void);
        unsafe {
          SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, 0);
          let disable: BOOL = TRUE;
          let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &disable as *const BOOL as *const c_void,
            std::mem::size_of::<BOOL>() as u32,
          );
        }
      }
    }
    let context = SoftContext::new(window.clone()).map_err(|error| {
      anyhow!("no graphics context for the overlay: {error}")
    })?;
    let surface = SoftSurface::new(&context, window.clone())
      .map_err(|error| anyhow!("no surface for the overlay: {error}"))?;
    let frame = Pixmap::new(canvas.width(), canvas.height())
      .expect("frame allocation failed");
    let mut session = Self {
      window,
      canvas,
      backdrop,
      frame,
      surface,
      bounds,
      selection: None,
      mode: Mode::Idle,
      tool: Tool::Select,
      palette_index: 0,
      history: History::default(),
      engine: TextEngine::default(),
      cursor: Point::default(),
      hover: None,
      pending: None,
    };
    session
      .surface
      .resize(
        NonZeroU32::new(session.canvas.width())
          .context("zero-width capture")?,
        NonZeroU32::new(session.canvas.height())
          .context("zero-height capture")?,
      )
      .map_err(|e| anyhow!("failed to resize the overlay surface: {e}"))?;
    session.window.set_visible(true);
    session.render();
    session.engine = TextEngine::load()?;
    session.window.request_redraw();
    Ok(session)
  }

  fn render(&mut self) {
    let chrome = render::build(
      self.selection,
      self.bounds,
      self.tool,
      &self.history,
      matches!(self.mode, Mode::Idle),
    );

    let canvas = &self.canvas;
    let backdrop = &self.backdrop;
    let shapes = self.history.shapes();
    let draft = self.mode.draft();
    let typing = Self::typing(&self.mode);
    let text = &self.engine;

    let frame = &mut self.frame;
    let scene = Scene {
      frame: canvas,
      backdrop,
      bounds: self.bounds,
      selection: self.selection,
      shapes,
      draft,
      typing,
      palette_index: self.palette_index,
      chrome: &chrome,
      hotspot: self.hover,
      text,
    };

    render::paint(frame, &scene);
    self.present();
  }

  fn present(&mut self) {
    let Ok(mut buffer) = self.surface.buffer_mut() else {
      return;
    };
    for (pixel, rgba) in
      buffer.iter_mut().zip(self.frame.data().as_chunks::<4>().0)
    {
      let [r, g, b, _] = *rgba;
      *pixel =
        (0xFFu32 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    }
    let _ = buffer.present();
  }

  fn typing(mode: &Mode) -> Option<(Point, &str)> {
    if let Mode::Type(buffer, anchor) = mode {
      Some((*anchor, buffer.as_str()))
    } else {
      None
    }
  }

  fn mouse_move(&mut self, position: PhysicalPosition<f64>) {
    let p = Point::new(position.x as f32, position.y as f32);
    self.cursor = p;
    self.hover = self.selection.and_then(|sel| {
      render::hotspot_at(
        &render::build(
          Some(sel),
          self.bounds,
          self.tool,
          &self.history,
          matches!(self.mode, Mode::Idle),
        ),
        p,
      )
    });
    match &mut self.mode {
      Mode::Idle => {
        if self.tool == Tool::Select {
          if let Some(sel) = self.selection {
            match hit_handle(sel, p, HANDLE_SLOP) {
              Some(_) => {
                self.window.set_cursor(resize_cursor(sel, p));
              }
              None if sel.contains(p) => {
                self.window.set_cursor(CursorIcon::Move);
              }
              None => {
                self.window.set_cursor(CursorIcon::default());
              }
            }
          }
        } else {
          self.window.set_cursor(CursorIcon::default());
        }
      }
      Mode::Rubber(x, y) => {
        self.selection = Some(Rect::spanning(Point::new(*x, *y), p));
        self.window.request_redraw();
      }
      Mode::Draw(draft, anchor) => {
        extend_draft(*anchor, draft, p);
        self.window.request_redraw();
      }
      Mode::Move(last_x, last_y) => {
        let sel = self.selection.expect("move mode requires a selection");
        let moved = sel
          .moved_inside(self.bounds, Point::new(p.x - *last_x, p.y - *last_y));
        let delta = Point::new(moved.x - sel.x, moved.y - sel.y);
        self.history.translate_all(delta.x, delta.y);
        self.selection = Some(moved);
        *last_x = p.x;
        *last_y = p.y;
        self.window.request_redraw();
      }
      Mode::Resize(handle, rect) => {
        let target = p.clamped_inside(self.bounds);
        self.selection = Some(resized(*rect, *handle, target));
        self.window.request_redraw();
      }
      Mode::Type(_, _) => {}
    }
  }

  fn mouse_down(&mut self) {
    let p = self.cursor;
    if let Some(sel) = self.selection {
      if let Some(hotspot) = render::hotspot_at(
        &render::build(
          self.selection,
          self.bounds,
          self.tool,
          &self.history,
          matches!(self.mode, Mode::Idle),
        ),
        p,
      ) {
        self.activate(hotspot);
        return;
      }
      if self.tool == Tool::Select {
        if let Some(handle) = hit_handle(sel, p, HANDLE_SLOP) {
          self.mode = Mode::Resize(handle, sel);
          return;
        }
        if sel.contains(p) {
          self.mode = Mode::Move(p.x, p.y);
          return;
        }
      }
    }
    match self.tool {
      Tool::Select => {
        self.selection = None;
        self.mode = Mode::Rubber(p.x, p.y);
        self.window.request_redraw();
      }
      Tool::Label => {
        self.mode = Mode::Type(String::new(), p);
        self.window.request_redraw();
      }
      Tool::Pen => {
        self.mode = Mode::Draw(
          Shape::Freehand {
            points: vec![p],
            color: PALETTE[self.palette_index % PALETTE.len()],
            width: PEN_WIDTH,
          },
          p,
        );
        self.window.request_redraw();
      }
      Tool::Marker => {
        self.mode = Mode::Draw(
          Shape::Marker {
            points: vec![p],
            color: PALETTE[self.palette_index % PALETTE.len()],
          },
          p,
        );
        self.window.request_redraw();
      }
      Tool::Line => {
        self.mode = Mode::Draw(
          Shape::Segment {
            from: p,
            to: p,
            color: PALETTE[self.palette_index % PALETTE.len()],
            width: LINE_WIDTH,
          },
          p,
        );
        self.window.request_redraw();
      }
      Tool::Arrow => {
        self.mode = Mode::Draw(
          Shape::Arrow {
            tail: p,
            head: p,
            color: PALETTE[self.palette_index % PALETTE.len()],
            width: LINE_WIDTH,
          },
          p,
        );
        self.window.request_redraw();
      }
      Tool::Box => {
        self.mode = Mode::Draw(
          Shape::Outline {
            rect: Rect::new(p.x, p.y, 0.0, 0.0),
            color: PALETTE[self.palette_index % PALETTE.len()],
            width: LINE_WIDTH,
          },
          p,
        );
        self.window.request_redraw();
      }
    }
  }

  fn activate(&mut self, hotspot: Hotspot) {
    let command = match hotspot {
      Hotspot::Tool(i) => self.button_command(i, true),
      Hotspot::Action(i) => self.button_command(i, false),
    };
    match command {
      render::Command::Tool(tool) => self.tool = tool,
      render::Command::NextColor => {
        self.palette_index = (self.palette_index + 1) % PALETTE.len()
      }
      render::Command::Undo => {
        self.history.undo();
        self.window.request_redraw();
      }
      render::Command::Deliver(deliverable) => {
        if let Some(sel) = render::deliverable_region(self.selection) {
          let shot = render::flatten(
            &self.canvas,
            sel,
            self.history.shapes(),
            &self.engine,
          );
          self.pending = Some(Outcome { deliverable, shot });
        }
      }
    }
  }

  fn button_command(&self, index: usize, tools: bool) -> render::Command {
    let chrome = render::build(
      self.selection,
      self.bounds,
      self.tool,
      &self.history,
      matches!(self.mode, Mode::Idle),
    );
    let button = if tools {
      &chrome.tools[index]
    } else {
      &chrome.actions[index]
    };
    button.command
  }

  fn mouse_up(&mut self) -> Option<Outcome> {
    match std::mem::replace(&mut self.mode, Mode::Idle) {
      Mode::Rubber(_, _) => {
        self.selection = render::deliverable_region(self.selection);
      }
      Mode::Draw(draft, _) if draft.is_complete() => {
        self.history.push(draft);
      }
      Mode::Draw(_, _) => {}
      Mode::Move(_, _) | Mode::Resize(_, _) => {}
      Mode::Type(buffer, anchor) => {
        self.mode = Mode::Type(buffer, anchor);
      }
      Mode::Idle => {}
    }
    self.window.request_redraw();
    self.pending.take()
  }

  fn type_char(&mut self, ch: &str) {
    if let Mode::Type(buffer, _) = &mut self.mode {
      buffer.push_str(ch);
      self.window.request_redraw();
    }
  }

  fn backspace(&mut self) {
    if let Mode::Type(buffer, _) = &mut self.mode {
      buffer.pop();
      self.window.request_redraw();
    }
  }

  fn commit_label(&mut self) -> Option<Outcome> {
    if let Mode::Type(buffer, anchor) = &mut self.mode {
      let text = Shape::Caption {
        at: *anchor,
        text: std::mem::take(buffer),
        color: PALETTE[self.palette_index % PALETTE.len()],
      };
      if text.is_complete() {
        self.history.push(text);
      }
      self.mode = Mode::Idle;
      self.window.request_redraw();
    }
    None
  }
}

pub fn extend_draft(anchor: Point, draft: &mut Shape, p: Point) {
  match draft {
    Shape::Freehand { points, .. } | Shape::Marker { points, .. } => {
      points.push(p)
    }
    Shape::Segment { to, .. } | Shape::Arrow { head: to, .. } => *to = p,
    Shape::Outline { rect, .. } => *rect = Rect::spanning(anchor, p),
    Shape::Caption { at, .. } => *at = p,
  }
}

pub fn resize_cursor(selection: Rect, p: Point) -> CursorIcon {
  match hit_handle(selection, p, HANDLE_SLOP) {
    Some(Handle::TopLeft) | Some(Handle::BottomRight) => CursorIcon::NwseResize,
    Some(Handle::BottomLeft) | Some(Handle::TopRight) => CursorIcon::NeswResize,
    Some(Handle::Top) | Some(Handle::Bottom) => CursorIcon::NsResize,
    Some(Handle::Left) | Some(Handle::Right) => CursorIcon::EwResize,
    None => CursorIcon::default(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geom::{handle_anchor, hit_handle, Rect as GeoRect, HANDLES};

  #[test]
  fn extend_draft_appends_to_paths_and_resizes_boxes() {
    let mut free = Shape::Freehand {
      points: vec![Point::new(1.0, 1.0)],
      color: [0, 0, 0],
      width: 2.0,
    };
    extend_draft(Point::new(0.0, 0.0), &mut free, Point::new(5.0, 5.0));
    assert_eq!(
      free,
      Shape::Freehand {
        points: vec![Point::new(1.0, 1.0), Point::new(5.0, 5.0)],
        color: [0, 0, 0],
        width: 2.0,
      }
    );

    let mut boxed = Shape::Outline {
      rect: GeoRect::new(10.0, 20.0, 0.0, 0.0),
      color: [0, 0, 0],
      width: 2.0,
    };
    extend_draft(Point::new(10.0, 20.0), &mut boxed, Point::new(40.0, 60.0));
    assert_eq!(
      boxed,
      Shape::Outline {
        rect: GeoRect::new(10.0, 20.0, 30.0, 40.0),
        color: [0, 0, 0],
        width: 2.0,
      }
    );
  }

  #[test]
  fn resize_cursor_picks_the_right_handle_cursor() {
    let sel = GeoRect::new(10.0, 10.0, 100.0, 100.0);
    let corner = handle_anchor(sel, Handle::TopLeft);
    assert_eq!(resize_cursor(sel, corner), CursorIcon::NwseResize);
    let edge = handle_anchor(sel, Handle::Top);
    assert_eq!(resize_cursor(sel, edge), CursorIcon::NsResize);
    assert_eq!(
      resize_cursor(sel, Point::new(500.0, 500.0)),
      CursorIcon::default()
    );
  }

  #[test]
  fn every_handle_is_hit_at_its_anchor() {
    let sel = GeoRect::new(10.0, 10.0, 100.0, 100.0);
    for &h in &HANDLES {
      let anchor = handle_anchor(sel, h);
      assert_eq!(hit_handle(sel, anchor, HANDLE_SLOP), Some(h));
    }
  }
}
