use std::{
  ffi::c_void,
  num::NonZeroU32,
  sync::Arc,
  thread,
  time::{Duration, Instant},
};

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
  keyboard::{Key, ModifiersState, NamedKey},
  platform::windows::WindowAttributesExtWindows,
  raw_window_handle::{HasWindowHandle, RawWindowHandle},
  window::{CursorIcon, Window, WindowId, WindowLevel},
};

use crate::{
  action::{self, Deliverable, Shot},
  annotate::{
    active_color, History, Shape, Tool, MAX_SIZE, MIN_SIZE, PALETTE, SIZE_STEP,
  },
  capture,
  geom::{hit_handle, resized, Handle, Point, Rect},
  hotkey::Trigger,
  pixel::swap_channels_to_words,
  render::{self, Chrome, Hotspot, Scene, HANDLE_SLOP},
  text::TextEngine,
};

const HINT_DURATION: Duration = Duration::from_millis(800);

pub enum Outcome {
  Close,
  Deliver {
    deliverable: Deliverable,
    shot: Shot,
  },
}

#[derive(Default)]
enum Mode {
  #[default]
  Idle,
  Rubber(Point),
  Draw(Shape, Point),
  Move(Point),
  Resize(Handle, Rect),
  Type(String, Point),
}

impl Mode {
  fn draft(&self) -> Option<&Shape> {
    match self {
      Mode::Draw(shape, _) => Some(shape),
      _ => None,
    }
  }

  fn shows_chrome(&self) -> bool {
    matches!(self, Mode::Idle | Mode::Draw(_, _) | Mode::Type(_, _))
  }
}

type Surface = SoftSurface<Arc<Window>, Arc<Window>>;

struct Session {
  window: Arc<Window>,
  canvas: Pixmap,
  backdrop: Pixmap,
  inked_backdrop: Pixmap,
  inked_canvas: Pixmap,
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
  chrome: Chrome,
  modifiers: ModifiersState,
  sizes: [f32; 7],
  hint: Option<String>,
  hint_until: Option<Instant>,
  hint_scheduled: bool,
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
      WindowEvent::ModifiersChanged(state) => session.modifiers = state.state(),
      WindowEvent::RedrawRequested => session.render(),
      WindowEvent::CursorMoved { position, .. } => session.mouse_move(position),
      WindowEvent::MouseInput {
        state: ElementState::Pressed,
        button: MouseButton::Left,
        ..
      } => {
        if let Some(outcome) = session.mouse_down() {
          self.finish(outcome);
        }
      }
      WindowEvent::MouseInput {
        state: ElementState::Released,
        button: MouseButton::Left,
        ..
      } => session.mouse_up(),
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
      } => {
        if ch.as_str().eq_ignore_ascii_case("c")
          && session.modifiers.control_key()
        {
          if let Some(outcome) = session.deliver(Deliverable::Copy) {
            self.finish(outcome);
          }
        } else if (ch.as_str() == "[" || ch.as_str() == "]")
          && session.tool.is_annotation()
          && !session.is_typing()
        {
          let delta = if ch.as_str() == "]" {
            SIZE_STEP
          } else {
            -SIZE_STEP
          };
          session.adjust_size(delta);
        } else {
          session.type_char(ch.as_str());
        }
      }
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
    self.session = None;
    let Outcome::Deliver { deliverable, shot } = outcome else {
      return;
    };
    thread::spawn(move || match action::execute(deliverable, &shot) {
      Ok(summary) => println!("slightshot: {summary}"),
      Err(error) => eprintln!("slightshot: {error:#}"),
    });
  }
}

impl Session {
  fn create(event_loop: &ActiveEventLoop) -> Result<Self> {
    let shot = capture::grab().context("screen capture failed")?;
    let engine = TextEngine::load()?;
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
    let mut surface = SoftSurface::new(&context, window.clone())
      .map_err(|error| anyhow!("no surface for the overlay: {error}"))?;
    surface
      .resize(
        NonZeroU32::new(canvas.width()).context("zero-width capture")?,
        NonZeroU32::new(canvas.height()).context("zero-height capture")?,
      )
      .map_err(|e| anyhow!("failed to resize the overlay surface: {e}"))?;

    let frame = Pixmap::new(canvas.width(), canvas.height())
      .expect("frame allocation failed");
    let inked_backdrop = backdrop.clone();
    let inked_canvas = canvas.clone();

    let mut sizes = [0.0f32; 7];
    for tool in Tool::all() {
      sizes[*tool as usize] = tool.default_size();
    }

    let mut session = Self {
      window,
      canvas,
      backdrop,
      inked_backdrop,
      inked_canvas,
      frame,
      surface,
      bounds,
      selection: None,
      mode: Mode::Idle,
      tool: Tool::Select,
      palette_index: 0,
      history: History::default(),
      engine,
      cursor: Point::default(),
      hover: None,
      chrome: Chrome::default(),
      modifiers: ModifiersState::default(),
      sizes,
      hint: None,
      hint_until: None,
      hint_scheduled: false,
    };
    session.window.set_visible(true);
    session.render();
    Ok(session)
  }

  fn render(&mut self) {
    if let Some(until) = self.hint_until {
      if Instant::now() >= until {
        self.hint = None;
        self.hint_until = None;
        self.hint_scheduled = false;
      }
    }
    render::build(
      &mut self.chrome,
      self.selection,
      self.bounds,
      self.tool,
      &self.history,
      self.mode.shows_chrome(),
    );
    let chrome = &self.chrome;
    let inked_backdrop = &self.inked_backdrop;
    let draft = self.mode.draft();
    let typing = Self::typing(&self.mode, self.size(Tool::Label));
    let text = &self.engine;

    let frame = &mut self.frame;
    let scene = Scene {
      inked_backdrop,
      inked_canvas: &self.inked_canvas,
      bounds: self.bounds,
      selection: self.selection,
      draft,
      typing,
      palette_index: self.palette_index,
      chrome,
      hotspot: self.hover,
      text,
      hint: self.hint.as_deref(),
    };

    render::paint(frame, &scene);
    self.present();
  }

  fn present(&mut self) {
    let Ok(mut buffer) = self.surface.buffer_mut() else {
      return;
    };
    if buffer.len() != (self.frame.width() * self.frame.height()) as usize {
      return;
    }
    swap_channels_to_words(self.frame.data(), &mut buffer);
    let _ = buffer.present();
  }

  fn typing(mode: &Mode, label_size: f32) -> Option<(Point, &str, f32)> {
    if let Mode::Type(buffer, anchor) = mode {
      Some((*anchor, buffer.as_str(), label_size))
    } else {
      None
    }
  }

  fn mouse_move(&mut self, position: PhysicalPosition<f64>) {
    let p = Point::new(position.x as f32, position.y as f32);
    self.cursor = p;
    let previous = self.hover;
    self.hover = self
      .selection
      .and_then(|_| render::hotspot_at(&self.chrome, p));
    if self.hover != previous {
      self.window.request_redraw();
    }
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
      Mode::Rubber(anchor) => {
        self.selection = Some(Rect::spanning(*anchor, p));
        self.window.request_redraw();
      }
      Mode::Draw(draft, anchor) => {
        extend_draft(*anchor, draft, p);
        self.window.request_redraw();
      }
      Mode::Move(last) => {
        let sel = self.selection.expect("move mode requires a selection");
        let moved =
          sel.moved_inside(self.bounds, Point::new(p.x - last.x, p.y - last.y));
        self.selection = Some(moved);
        *last = p;
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

  fn mouse_down(&mut self) -> Option<Outcome> {
    let p = self.cursor;
    if let Some(sel) = self.selection {
      if let Some(hotspot) = render::hotspot_at(&self.chrome, p) {
        return self.activate(hotspot);
      }
      if self.tool == Tool::Select {
        if let Some(handle) = hit_handle(sel, p, HANDLE_SLOP) {
          self.mode = Mode::Resize(handle, sel);
          return None;
        }
        if sel.contains(p) {
          self.mode = Mode::Move(p);
          return None;
        }
      }
    }
    self.mode = match self.tool {
      Tool::Select => {
        self.selection = None;
        Mode::Rubber(p)
      }
      Tool::Label => Mode::Type(String::new(), p),
      tool => Mode::Draw(self.new_shape(tool, p), p),
    };
    self.window.request_redraw();
    None
  }

  fn new_shape(&self, tool: Tool, p: Point) -> Shape {
    let color = active_color(self.palette_index);
    let width = self.size(tool);
    match tool {
      Tool::Pen => Shape::Stroke {
        points: vec![p],
        color,
        width,
        marker: false,
      },
      Tool::Marker => Shape::Stroke {
        points: vec![p],
        color,
        width,
        marker: true,
      },
      Tool::Line => Shape::Line {
        from: p,
        to: p,
        color,
        width,
        arrow: false,
      },
      Tool::Arrow => Shape::Line {
        from: p,
        to: p,
        color,
        width,
        arrow: true,
      },
      Tool::Box => Shape::Outline {
        rect: Rect::new(p.x, p.y, 0.0, 0.0),
        color,
        width,
      },
      Tool::Select | Tool::Label => {
        unreachable!("Select and Label are handled directly in mouse_down")
      }
    }
  }

  fn activate(&mut self, hotspot: Hotspot) -> Option<Outcome> {
    let command = match hotspot {
      Hotspot::Tool(i) => self.chrome.tools[i].command,
      Hotspot::Action(i) => self.chrome.actions[i].command,
    };
    match command {
      render::Command::Tool(tool) => {
        self.tool = if self.tool == tool {
          Tool::Select
        } else {
          tool
        };
        None
      }
      render::Command::NextColor => {
        self.palette_index = (self.palette_index + 1) % PALETTE.len();
        None
      }
      render::Command::Undo => {
        if self.history.undo() {
          self.rebuild_ink();
          self.window.request_redraw();
        }
        None
      }
      render::Command::Close => Some(Outcome::Close),
      render::Command::Deliver(deliverable) => self.deliver(deliverable),
    }
  }

  fn deliver(&self, deliverable: Deliverable) -> Option<Outcome> {
    let sel = render::deliverable_region(self.selection)?;
    let shot =
      render::flatten(&self.canvas, sel, self.history.shapes(), &self.engine);
    Some(Outcome::Deliver { deliverable, shot })
  }

  fn ink_shape(&mut self, shape: &Shape) {
    render::ink(
      &mut self.inked_backdrop,
      shape,
      Point::default(),
      &self.engine,
    );
    render::ink(
      &mut self.inked_canvas,
      shape,
      Point::default(),
      &self.engine,
    );
  }

  fn rebuild_ink(&mut self) {
    self.inked_backdrop = self.backdrop.clone();
    self.inked_canvas = self.canvas.clone();
    for shape in self.history.shapes() {
      render::ink(
        &mut self.inked_backdrop,
        shape,
        Point::default(),
        &self.engine,
      );
      render::ink(
        &mut self.inked_canvas,
        shape,
        Point::default(),
        &self.engine,
      );
    }
  }

  fn mouse_up(&mut self) {
    match std::mem::replace(&mut self.mode, Mode::Idle) {
      Mode::Rubber(_) => {
        self.selection = render::deliverable_region(self.selection);
      }
      Mode::Draw(draft, _) if draft.is_complete() => {
        self.ink_shape(&draft);
        self.history.push(draft);
      }
      Mode::Type(buffer, anchor) => {
        self.mode = Mode::Type(buffer, anchor);
      }
      _ => {}
    }
    self.window.request_redraw();
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
        color: active_color(self.palette_index),
        size: self.size(Tool::Label),
      };
      if text.is_complete() {
        self.ink_shape(&text);
        self.history.push(text);
      }
      self.mode = Mode::Idle;
      self.window.request_redraw();
    }
    None
  }

  fn size(&self, tool: Tool) -> f32 {
    self.sizes[tool as usize]
  }

  fn is_typing(&self) -> bool {
    matches!(self.mode, Mode::Type(..))
  }

  fn adjust_size(&mut self, delta: f32) {
    let tool = self.tool;
    if !tool.is_annotation() {
      return;
    }
    let next = (self.size(tool) + delta).clamp(MIN_SIZE, MAX_SIZE);
    self.sizes[tool as usize] = next;
    if let Mode::Draw(shape, _) = &mut self.mode {
      shape.set_width(next);
    }
    self.hint = Some(format_size(next));
    self.hint_until = Some(Instant::now() + HINT_DURATION);
    self.schedule_hint_clear();
    self.window.request_redraw();
  }

  fn schedule_hint_clear(&mut self) {
    if self.hint_scheduled {
      return;
    }
    self.hint_scheduled = true;
    let window = self.window.clone();
    let _ = thread::Builder::new()
      .name("slightshot-hint".to_string())
      .spawn(move || {
        thread::sleep(HINT_DURATION);
        window.request_redraw();
      });
  }
}

fn format_size(size: f32) -> String {
  if size.fract() == 0.0 {
    format!("{}", size as i32)
  } else {
    format!("{size:.1}")
  }
}

pub fn extend_draft(anchor: Point, draft: &mut Shape, p: Point) {
  match draft {
    Shape::Stroke { points, .. } => points.push(p),
    Shape::Line { to, .. } => *to = p,
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
    let mut free = Shape::Stroke {
      points: vec![Point::new(1.0, 1.0)],
      color: [0, 0, 0],
      width: 2.0,
      marker: false,
    };
    extend_draft(Point::new(0.0, 0.0), &mut free, Point::new(5.0, 5.0));
    assert_eq!(
      free,
      Shape::Stroke {
        points: vec![Point::new(1.0, 1.0), Point::new(5.0, 5.0)],
        color: [0, 0, 0],
        width: 2.0,
        marker: false,
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

  #[test]
  fn converts_straight_rgba_rows_to_argb_words() {
    let rgba = [10u8, 20, 30, 0, 40, 50, 60, 99];
    let mut out = [0u32; 2];
    swap_channels_to_words(&rgba, &mut out);
    assert_eq!(out[0], (0xFFu32 << 24) | (10 << 16) | (20 << 8) | 30);
    assert_eq!(out[1], (0xFFu32 << 24) | (40 << 16) | (50 << 8) | 60);
  }
}
