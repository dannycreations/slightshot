use std::{
  collections::HashMap,
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
  actions::{self, Deliverable, Shot},
  annotate::{
    active_color, History, Shape, Tool, MAX_SIZE, MIN_SIZE, PALETTE, SIZE_STEP,
  },
  capture,
  geom::{hit_handle, resized, Handle, Point, Rect},
  hotkey::Trigger,
  render::{self, Chrome, Hotspot, Scene, HANDLE_SLOP},
  text::TextEngine,
};

const HINT_DURATION: Duration = Duration::from_millis(800);

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

  fn shows_chrome(&self) -> bool {
    matches!(self, Mode::Idle | Mode::Draw(_, _) | Mode::Type(_, _))
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
  revision: usize,
  history: History,
  engine: TextEngine,
  cursor: Point,
  hover: Option<Hotspot>,
  chrome: Chrome,
  dirty: Option<Rect>,
  scratch: Option<Pixmap>,
  committed: Option<render::CommittedLayer>,
  pending: Option<Outcome>,
  modifiers: ModifiersState,
  sizes: HashMap<Tool, f32>,
  hint: Option<String>,
  hint_until: Option<Instant>,
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
    thread::spawn(move || {
      match actions::execute(outcome.deliverable, &outcome.shot) {
        Ok(summary) => println!("slightshot: {summary}"),
        Err(error) => eprintln!("slightshot: {error:#}"),
      }
    });
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
      revision: 0,
      history: History::default(),
      engine: TextEngine::default(),
      cursor: Point::default(),
      hover: None,
      chrome: Chrome {
        tools: Vec::new(),
        actions: Vec::new(),
      },
      dirty: None,
      scratch: None,
      committed: None,
      pending: None,
      modifiers: ModifiersState::default(),
      sizes: {
        let mut sizes = HashMap::new();
        for tool in Tool::all() {
          sizes.insert(*tool, tool.default_size());
        }
        sizes
      },
      hint: None,
      hint_until: None,
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
    if let Some(until) = self.hint_until {
      if Instant::now() >= until {
        self.hint = None;
        self.hint_until = None;
      }
    }
    self.chrome = render::build(
      self.selection,
      self.bounds,
      self.tool,
      &self.history,
      self.mode.shows_chrome(),
    );
    let chrome = &self.chrome;
    let canvas = &self.canvas;
    let backdrop = &self.backdrop;
    let shapes = self.history.shapes();
    let draft = self.mode.draft();
    let typing = Self::typing(&self.mode, self.size(Tool::Label));
    let text = &self.engine;

    let prev = self.dirty;
    let typing_rect = typing.map(|(at, buffer, size)| {
      Rect::new(at.x, at.y, text.width(buffer, size), size)
    });
    let curr = match (
      render::dirty_rect(
        self.selection,
        chrome,
        self.bounds,
        text,
        self.hover,
        self.hint.as_deref(),
      ),
      typing_rect,
    ) {
      (base, None) => base,
      (None, Some(c)) => Some(c),
      (Some(b), Some(c)) => Some(b.union(c)),
    };

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
      revision: self.revision,
      chrome,
      hotspot: self.hover,
      text,
      hint: self.hint.as_deref(),
    };

    let restore = match (prev, curr) {
      (None, _) => None,
      (Some(p), Some(c)) => Some(p.union(c)),
      (Some(p), None) => Some(p),
    };

    render::paint(
      frame,
      &scene,
      restore,
      &mut self.committed,
      &mut self.scratch,
    );
    self.dirty = Some(curr.unwrap_or_default());
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
        self.revision += 1;
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
      if let Some(hotspot) = render::hotspot_at(&self.chrome, p) {
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
            color: active_color(self.palette_index),
            width: self.size(Tool::Pen),
          },
          p,
        );
        self.window.request_redraw();
      }
      Tool::Marker => {
        self.mode = Mode::Draw(
          Shape::Marker {
            points: vec![p],
            color: active_color(self.palette_index),
            width: self.size(Tool::Marker),
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
            color: active_color(self.palette_index),
            width: self.size(Tool::Line),
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
            color: active_color(self.palette_index),
            width: self.size(Tool::Arrow),
          },
          p,
        );
        self.window.request_redraw();
      }
      Tool::Box => {
        self.mode = Mode::Draw(
          Shape::Outline {
            rect: Rect::new(p.x, p.y, 0.0, 0.0),
            color: active_color(self.palette_index),
            width: self.size(Tool::Box),
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
      render::Command::Tool(tool) => {
        self.tool = if self.tool == tool {
          Tool::Select
        } else {
          tool
        };
      }
      render::Command::NextColor => {
        self.palette_index = (self.palette_index + 1) % PALETTE.len()
      }
      render::Command::Undo => {
        if self.history.undo() {
          self.revision += 1;
        }
        self.window.request_redraw();
      }
      render::Command::Deliver(deliverable) => {
        if let Some(outcome) = self.deliver(deliverable) {
          self.pending = Some(outcome);
        }
      }
    }
  }

  fn button_command(&self, index: usize, tools: bool) -> render::Command {
    let button = if tools {
      &self.chrome.tools[index]
    } else {
      &self.chrome.actions[index]
    };
    button.command
  }

  fn deliver(&self, deliverable: Deliverable) -> Option<Outcome> {
    render::deliverable_region(self.selection).map(|sel| {
      let shot =
        render::flatten(&self.canvas, sel, self.history.shapes(), &self.engine);
      Outcome { deliverable, shot }
    })
  }

  fn mouse_up(&mut self) -> Option<Outcome> {
    match std::mem::replace(&mut self.mode, Mode::Idle) {
      Mode::Rubber(_, _) => {
        self.selection = render::deliverable_region(self.selection);
      }
      Mode::Draw(draft, _) if draft.is_complete() => {
        self.history.push(draft);
        self.revision += 1;
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
        color: active_color(self.palette_index),
        size: self.size(Tool::Label),
      };
      if text.is_complete() {
        self.history.push(text);
        self.revision += 1;
      }
      self.mode = Mode::Idle;
      self.window.request_redraw();
    }
    None
  }

  fn size(&self, tool: Tool) -> f32 {
    self.sizes.get(&tool).copied().unwrap_or_default()
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
    self.sizes.insert(tool, next);
    if let Mode::Draw(
      Shape::Marker { width, .. }
      | Shape::Freehand { width, .. }
      | Shape::Segment { width, .. }
      | Shape::Arrow { width, .. }
      | Shape::Outline { width, .. },
      _,
    ) = &mut self.mode
    {
      *width = next;
    }
    self.hint = Some(format_size(next));
    self.hint_until = Some(Instant::now() + HINT_DURATION);
    self.schedule_hint_clear();
    self.window.request_redraw();
  }

  fn schedule_hint_clear(&self) {
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
