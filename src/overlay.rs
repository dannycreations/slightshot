use std::{
  ffi::c_void,
  mem,
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
  event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent},
  event_loop::{ActiveEventLoop, ControlFlow},
  keyboard::{Key, ModifiersState, NamedKey},
  platform::windows::WindowAttributesExtWindows,
  raw_window_handle::{HasWindowHandle, RawWindowHandle},
  window::{CursorIcon, Window, WindowId, WindowLevel},
};

use crate::{
  action::{self, Deliverable, Shot},
  annotate::{
    active_color, stroke_alpha, History, Segment, Shape, Tool, MAX_SIZE,
    MIN_SIZE, PALETTE, SIZE_STEP, TOOLS,
  },
  capture,
  geom::{hit_handle, resized, Handle, Point, Rect},
  hotkey::Trigger,
  pixel::swap_channels_to_words,
  render::{self, Chrome, Hotspot, Scene, HANDLE_SLOP},
  text::TextEngine,
};

const HINT_DURATION: Duration = Duration::from_millis(800);

const MAX_STRETCH: f32 = 8.0;

enum Outcome {
  Close,
  Deliver {
    deliverable: Deliverable,
    shot: Shot,
  },
}

enum Mode {
  Idle,
  Rubber(Point),
  Draw(Shape, Point),
  Move(Point),
  Resize(Handle, Rect),
  Type(String, Point),
}

impl Mode {
  #[inline(always)]
  fn draft(&self) -> Option<&Shape> {
    match self {
      Mode::Draw(shape, _) if !shape.is_stroke() => Some(shape),
      _ => None,
    }
  }

  #[inline(always)]
  fn typing(&self, label_size: f32) -> Option<(Point, &str, f32)> {
    match self {
      Mode::Type(buffer, at) => Some((*at, buffer.as_str(), label_size)),
      _ => None,
    }
  }

  #[inline(always)]
  fn shows_chrome(&self) -> bool {
    matches!(self, Mode::Idle | Mode::Draw(_, _) | Mode::Type(_, _))
  }

  #[inline(always)]
  fn stroking(&self) -> bool {
    matches!(self, Mode::Draw(shape, _) if shape.is_stroke())
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
  Repaint,
  Segment(Segment),
}

type Surface = SoftSurface<Arc<Window>, Arc<Window>>;

struct Ink {
  canvas: Pixmap,
  backdrop: Pixmap,
}

impl Ink {
  fn fresh(canvas: &Pixmap) -> Self {
    // The backdrop starts dimmed over the whole capture, so a stroke that is
    // still being drawn only has to dim the box it just added.
    let mut backdrop = Pixmap::new(canvas.width(), canvas.height())
      .expect("backdrop allocation failed");
    render::dimmed_into(&mut backdrop, canvas);
    Self {
      canvas: canvas.clone(),
      backdrop,
    }
  }

  fn stamp(&mut self, engine: &TextEngine, shape: &Shape) {
    render::ink(&mut self.canvas, shape, engine);
    render::dimmed_into(&mut self.backdrop, &self.canvas);
  }
}

struct Session {
  window: Arc<Window>,
  canvas: Pixmap,
  backdrop: Pixmap,
  ink: Option<Ink>,
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
  raw_path: Vec<Point>,
  hover: Option<Hotspot>,
  chrome: Chrome,
  modifiers: ModifiersState,
  sizes: [f32; TOOLS.len()],
  hint: Option<String>,
  hint_until: Option<Instant>,
  current_cursor: CursorIcon,
}

#[derive(Default)]
pub struct App {
  session: Option<Session>,
}

impl ApplicationHandler<Trigger> for App {
  fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

  fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
    let Some(session) = self.session.as_mut() else {
      return;
    };
    if session.expire_hint() {
      session.window.request_redraw();
    } else if let Some(until) = session.hint_until {
      event_loop.set_control_flow(ControlFlow::WaitUntil(until));
    } else {
      event_loop.set_control_flow(ControlFlow::Wait);
    }
  }

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
      WindowEvent::KeyboardInput { event, .. } => {
        if event.state != ElementState::Pressed {
          return;
        }
        match event.logical_key {
          Key::Named(NamedKey::Escape) => self.session = None,
          Key::Named(NamedKey::Enter) => {
            if let Some(outcome) = session.commit_label() {
              self.finish(outcome);
            }
          }
          Key::Named(NamedKey::Backspace) => session.backspace(),
          Key::Character(ch) => {
            if let Some(outcome) = session.character(ch.as_str()) {
              self.finish(outcome);
            }
          }
          _ => {}
        }
      }
      _ => {}
    }
  }

  fn device_event(
    &mut self,
    _event_loop: &ActiveEventLoop,
    _device_id: DeviceId,
    event: DeviceEvent,
  ) {
    if let DeviceEvent::MouseMotion { delta } = event {
      if let Some(session) = self.session.as_mut() {
        session.pointer_motion(delta);
      }
    }
  }

  fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Trigger) {
    match event {
      Trigger::Capture => {
        if self.session.is_none() {
          match Session::create(event_loop) {
            Ok(session) => self.session = Some(session),
            Err(error) => {
              eprintln!(
                "slightshot: could not lock the screen for selection: {error:#}"
              )
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
    quiet_window(&window);
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

    let mut session = Self {
      window,
      canvas,
      backdrop,
      ink: None,
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
      raw_path: Vec::new(),
      hover: None,
      chrome: Chrome::default(),
      modifiers: ModifiersState::default(),
      sizes: TOOLS.map(Tool::default_size),
      hint: None,
      hint_until: None,
      current_cursor: CursorIcon::default(),
    };
    session.window.set_visible(true);
    session.render();
    Ok(session)
  }

  #[inline]
  fn update_cursor(&mut self, icon: CursorIcon) {
    if self.current_cursor != icon {
      self.current_cursor = icon;
      self.window.set_cursor(icon);
    }
  }

  /// Clears the hint once its timer elapses. Returns true if it just cleared.
  fn expire_hint(&mut self) -> bool {
    match self.hint_until {
      Some(until) if Instant::now() >= until => {
        self.hint = None;
        self.hint_until = None;
        true
      }
      _ => false,
    }
  }

  fn render(&mut self) {
    self.expire_hint();
    render::build(
      &mut self.chrome,
      self.selection,
      self.bounds,
      self.tool,
      &self.history,
      self.mode.shows_chrome(),
    );
    let chrome = &self.chrome;
    let (inked_backdrop, inked_canvas) = match &self.ink {
      Some(ink) => (&ink.backdrop, &ink.canvas),
      None => (&self.backdrop, &self.canvas),
    };
    let draft = self.mode.draft();
    let typing = self.mode.typing(self.size(Tool::Label));
    let text = &self.engine;

    let frame = &mut self.frame;
    let scene = Scene {
      inked_backdrop,
      inked_canvas,
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

  fn mouse_move(&mut self, position: PhysicalPosition<f64>) {
    let p = Point::new(position.x as f32, position.y as f32);
    if self.mode.stroking() {
      self.place_reports(p);
    }
    self.pointer_at(p);
  }

  fn pointer_motion(&mut self, delta: (f64, f64)) {
    if !self.mode.stroking() {
      return;
    }
    let from = self.raw_path.last().copied().unwrap_or(self.cursor);
    self
      .raw_path
      .push(Point::new(from.x + delta.0 as f32, from.y + delta.1 as f32));
  }

  fn place_reports(&mut self, to: Point) {
    let from = self.cursor;
    let mut reports = mem::take(&mut self.raw_path);
    if place_burst(&mut reports, from, to) {
      for point in reports {
        self.pointer_at(point);
      }
    }
  }

  fn pointer_at(&mut self, p: Point) {
    if self.cursor == p {
      return;
    }
    self.cursor = p;
    let previous = self.hover;
    self.hover = if self.mode.shows_chrome() && self.selection.is_some() {
      render::hotspot_at(&self.chrome, p)
    } else {
      None
    };
    if self.hover != previous {
      self.window.request_redraw();
    }
    let mut step = None;
    match &mut self.mode {
      Mode::Idle => {
        let icon = match (self.tool, self.selection) {
          (Tool::Select, Some(sel)) => match hit_handle(sel, p, HANDLE_SLOP) {
            Some(handle) => resize_cursor(handle),
            None if sel.contains(p) => CursorIcon::Move,
            None => CursorIcon::default(),
          },
          _ => CursorIcon::default(),
        };
        self.update_cursor(icon);
      }
      Mode::Rubber(anchor) => {
        let new_sel = Rect::spanning(*anchor, p);
        if self.selection != Some(new_sel) {
          self.selection = Some(new_sel);
          self.window.request_redraw();
        }
      }
      Mode::Draw(draft, anchor) => {
        step = extend_draft(*anchor, draft, p);
      }
      Mode::Move(last) => {
        let sel = self.selection.expect("move mode requires a selection");
        let moved =
          sel.moved_inside(self.bounds, Point::new(p.x - last.x, p.y - last.y));
        if self.selection != Some(moved) {
          self.selection = Some(moved);
          self.window.request_redraw();
        }
        *last = p;
      }
      Mode::Resize(handle, rect) => {
        let target = p.clamped_inside(self.bounds);
        let resized_sel = resized(*rect, *handle, target);
        if self.selection != Some(resized_sel) {
          self.selection = Some(resized_sel);
          self.window.request_redraw();
        }
      }
      Mode::Type(_, _) => {}
    }
    match step {
      None => {}
      Some(Step::Repaint) => self.window.request_redraw(),
      Some(Step::Segment(segment)) => {
        self.ink_segment(segment);
        self.window.request_redraw();
      }
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
    if self.mode.stroking() {
      // A stroke that ended before the system reported a position left
      // reports behind. The next one starts from the cursor, not from them.
      self.raw_path.clear();
    }
    self.window.request_redraw();
    None
  }

  fn new_shape(&self, tool: Tool, p: Point) -> Shape {
    let color = active_color(self.palette_index);
    let width = self.size(tool);
    match tool {
      Tool::Pen | Tool::Marker => Shape::Stroke {
        points: vec![p],
        color,
        width,
        marker: tool == Tool::Marker,
      },
      Tool::Line | Tool::Arrow => Shape::Line {
        from: p,
        to: p,
        color,
        width,
        arrow: tool == Tool::Arrow,
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
    let source = self.ink.as_ref().map_or(&self.canvas, |ink| &ink.canvas);
    Some(Outcome::Deliver {
      deliverable,
      shot: render::flatten(source, sel),
    })
  }

  fn ink_shape(&mut self, shape: &Shape) {
    let Session {
      canvas,
      engine,
      ink,
      ..
    } = self;
    let ink = ink.get_or_insert_with(|| Ink::fresh(canvas));
    ink.stamp(engine, shape);
  }

  fn ink_segment(&mut self, segment: Segment) {
    let Session { canvas, ink, .. } = self;
    let ink = ink.get_or_insert_with(|| Ink::fresh(canvas));
    render::ink_segment(&mut ink.canvas, &mut ink.backdrop, segment);
  }

  fn rebuild_ink(&mut self) {
    let Session {
      canvas,
      engine,
      history,
      ink,
      ..
    } = self;
    if history.shapes().is_empty() {
      // Nothing survives to replay, so release both buffers rather than hold
      // two full-screen copies for an empty history.
      *ink = None;
      return;
    }
    let ink = ink.get_or_insert_with(|| Ink::fresh(canvas));
    ink.canvas.data_mut().copy_from_slice(canvas.data());
    for shape in history.shapes() {
      render::ink(&mut ink.canvas, shape, engine);
    }
    render::dimmed_into(&mut ink.backdrop, &ink.canvas);
  }

  fn mouse_up(&mut self) {
    // `Type` hands its buffer and anchor straight back: clicking while typing
    // must keep the text, so that one mode survives the release.
    match mem::replace(&mut self.mode, Mode::Idle) {
      Mode::Rubber(_) => {
        self.selection = render::deliverable_region(self.selection);
      }
      Mode::Draw(draft, _) if draft.is_complete() => {
        // A stroke is already on the ink, segment by segment. Inking it again
        // here would composite the whole log over itself a second time.
        if !draft.is_stroke() {
          self.ink_shape(&draft);
        }
        self.history.push(draft);
      }
      Mode::Type(buffer, anchor) => {
        self.mode = Mode::Type(buffer, anchor);
      }
      _ => {}
    }
    self.window.request_redraw();
  }

  fn character(&mut self, ch: &str) -> Option<Outcome> {
    if ch.eq_ignore_ascii_case("c") && self.modifiers.control_key() {
      return self.deliver(Deliverable::Copy);
    }
    if !self.is_typing() && matches!(ch, "[" | "]") {
      let step = if ch == "]" { SIZE_STEP } else { -SIZE_STEP };
      self.adjust_size(step);
      return None;
    }
    self.type_char(ch);
    None
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
        text: mem::take(buffer),
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

  #[inline(always)]
  fn size(&self, tool: Tool) -> f32 {
    self.sizes[tool as usize]
  }

  fn is_typing(&self) -> bool {
    matches!(self.mode, Mode::Type(_, _))
  }

  fn adjust_size(&mut self, delta: f32) {
    if !self.tool.is_annotation() {
      return;
    }
    let tool = self.tool;
    let next = (self.size(tool) + delta).clamp(MIN_SIZE, MAX_SIZE);
    self.sizes[tool as usize] = next;
    if let Mode::Draw(shape, _) = &mut self.mode {
      shape.set_width(next);
    }
    self.hint = Some(format_size(next));
    self.hint_until = Some(Instant::now() + HINT_DURATION);
    self.window.request_redraw();
  }
}

fn quiet_window(window: &Window) {
  let Ok(handle) = window.window_handle() else {
    return;
  };
  let RawWindowHandle::Win32(win32) = handle.as_raw() else {
    return;
  };
  let hwnd = HWND(win32.hwnd.get() as *mut c_void);
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

fn format_size(size: f32) -> String {
  if size.fract() == 0.0 {
    format!("{}", size as i32)
  } else {
    format!("{size:.1}")
  }
}

fn stretch(reported: f32, moved: f32) -> Option<f32> {
  let factor = moved / reported;
  (factor.is_finite() && factor.abs() <= MAX_STRETCH).then_some(factor)
}

fn place_burst(reports: &mut [Point], from: Point, to: Point) -> bool {
  let Some(last) = reports.last().copied() else {
    return false;
  };
  let (Some(kx), Some(ky)) = (
    stretch(last.x - from.x, to.x - from.x),
    stretch(last.y - from.y, to.y - from.y),
  ) else {
    return false;
  };
  for point in reports.iter_mut() {
    point.x = from.x + (point.x - from.x) * kx;
    point.y = from.y + (point.y - from.y) * ky;
  }
  true
}

fn extend_draft(anchor: Point, draft: &mut Shape, p: Point) -> Option<Step> {
  match draft {
    Shape::Stroke {
      points,
      color,
      width,
      marker,
    } => {
      let segment = Segment {
        from: points.last().copied().unwrap_or(p),
        to: p,
        color: *color,
        width: *width,
        alpha: stroke_alpha(*marker),
      };
      points.push(p);
      Some(Step::Segment(segment))
    }
    Shape::Line { to, .. } | Shape::Caption { at: to, .. } => {
      if *to == p {
        return None;
      }
      *to = p;
      Some(Step::Repaint)
    }
    Shape::Outline { rect, .. } => {
      let new_rect = Rect::spanning(anchor, p);
      if *rect == new_rect {
        return None;
      }
      *rect = new_rect;
      Some(Step::Repaint)
    }
  }
}

fn resize_cursor(handle: Handle) -> CursorIcon {
  match handle {
    Handle::TopLeft | Handle::BottomRight => CursorIcon::NwseResize,
    Handle::BottomLeft | Handle::TopRight => CursorIcon::NeswResize,
    Handle::Top | Handle::Bottom => CursorIcon::NsResize,
    Handle::Left | Handle::Right => CursorIcon::EwResize,
  }
}

#[cfg(test)]
mod tests {
  use Handle::*;

  use super::*;
  use crate::geom::{handle_anchor, HANDLES};

  #[test]
  fn extend_draft_reports_the_segment_a_stroke_gained() {
    let mut stroke = Shape::Stroke {
      points: vec![Point::new(1.0, 1.0)],
      color: [7, 8, 9],
      width: 3.0,
      marker: true,
    };
    assert_eq!(
      extend_draft(Point::new(1.0, 1.0), &mut stroke, Point::new(5.0, 5.0)),
      Some(Step::Segment(Segment {
        from: Point::new(1.0, 1.0),
        to: Point::new(5.0, 5.0),
        color: [7, 8, 9],
        width: 3.0,
        alpha: stroke_alpha(true),
      }))
    );
    assert_eq!(
      stroke,
      Shape::Stroke {
        points: vec![Point::new(1.0, 1.0), Point::new(5.0, 5.0)],
        color: [7, 8, 9],
        width: 3.0,
        marker: true,
      }
    );
  }

  #[test]
  fn extend_draft_logs_moves_smaller_than_a_pixel() {
    // Sampling used to drop anything under a pixel. The log is raw now, so a
    // stroke taken slowly still follows the cursor.
    let mut stroke = Shape::Stroke {
      points: vec![Point::new(1.0, 1.0)],
      color: [0, 0, 0],
      width: 2.0,
      marker: false,
    };
    let nudge = Point::new(1.2, 1.0);
    assert!(matches!(
      extend_draft(Point::new(1.0, 1.0), &mut stroke, nudge),
      Some(Step::Segment(_))
    ));
    let Shape::Stroke { points, .. } = &stroke else {
      panic!("the draft is a stroke");
    };
    assert_eq!(points, &[Point::new(1.0, 1.0), nudge]);
  }

  #[test]
  fn extend_draft_reports_no_step_when_the_cursor_does_not_move() {
    let mut line = Shape::Line {
      from: Point::new(0.0, 0.0),
      to: Point::new(4.0, 4.0),
      color: [0, 0, 0],
      width: 2.0,
      arrow: false,
    };
    assert_eq!(
      extend_draft(Point::new(0.0, 0.0), &mut line, Point::new(4.0, 4.0)),
      None
    );

    let mut boxed = Shape::Outline {
      rect: Rect::new(10.0, 20.0, 0.0, 0.0),
      color: [0, 0, 0],
      width: 2.0,
    };
    assert_eq!(
      extend_draft(Point::new(10.0, 20.0), &mut boxed, Point::new(40.0, 60.0)),
      Some(Step::Repaint)
    );
    assert_eq!(
      extend_draft(Point::new(10.0, 20.0), &mut boxed, Point::new(40.0, 60.0)),
      None,
      "a box that has not moved should not ask for another frame"
    );
  }

  #[test]
  fn a_burst_of_reports_is_pinned_to_both_cursor_positions() {
    // The reports know the shape of the path, not where the cursor ended up:
    // Windows scales the position it reports for pointer speed, so a fast
    // stroke arrives short and the ink would trail the cursor. Anchoring only
    // one end leaves a gap, so both ends have to land on the cursor.
    let from = Point::new(100.0, 100.0);
    let mut reports =
      vec![from, Point::new(104.0, 101.0), Point::new(108.0, 104.0)];
    assert!(place_burst(&mut reports, from, Point::new(120.0, 120.0)));
    assert_eq!(reports[0], from, "the burst starts on the old cursor");
    assert_eq!(
      reports[2],
      Point::new(120.0, 120.0),
      "and ends on the new one, or the ink drifts off the cursor"
    );
    assert_eq!(
      reports[1],
      Point::new(110.0, 105.0),
      "the turn between the ends is the hand's, and has to survive"
    );
  }

  #[test]
  fn a_burst_that_says_nothing_about_the_gap_is_not_drawn() {
    let from = Point::new(100.0, 100.0);
    // The reports went nowhere while the cursor crossed the screen. The
    // straight line is the honest answer, and drawing the reports where they
    // landed would put ink off the cursor.
    let mut still = vec![from, from, from];
    assert!(!place_burst(&mut still, from, Point::new(400.0, 100.0)));

    // A drag straight along one axis is rejected too, because there is no
    // direction to stretch and the straight line already is its path. A rule
    // that looks like a defect here is the whole difference between a level
    // line and a wiggle drawn through it.
    let mut level = vec![from, Point::new(110.0, 100.0)];
    assert!(!place_burst(&mut level, from, Point::new(110.0, 100.0)));

    // A drag that already matches the cursor keeps the ink where it is.
    let mut agreed =
      vec![from, Point::new(105.0, 105.0), Point::new(110.0, 110.0)];
    let expected = agreed.clone();
    assert!(place_burst(&mut agreed, from, Point::new(110.0, 110.0),));
    assert_eq!(agreed, expected);
  }

  #[test]
  fn stretch_rejects_a_gap_the_reports_do_not_account_for() {
    // A mouse held still reports no distance at all, and a report the app
    // never received leaves the survivors far shorter than the cursor moved.
    // Either way the straight line between two cursor positions is the honest
    // answer, and a rejected factor must not reach the path maths as a NaN.
    assert_eq!(stretch(0.0, 0.0), None);
    assert_eq!(stretch(0.0, 40.0), None);
    assert_eq!(stretch(2.0, 100.0), None, "a factor of 50 is a lost report");
    assert_eq!(stretch(20.0, 30.0), Some(1.5), "pointer speed, or reports");
    assert_eq!(
      stretch(30.0, 20.0),
      Some(20.0 / 30.0),
      "a path that doubled back"
    );
  }

  #[test]
  fn only_a_stroke_takes_the_pointer_from_the_raw_feed() {
    // Windows merges the mouse positions it cannot deliver in time, so a
    // stroke has to be built from the raw reports instead. A tool that
    // settles on a position rather than following a path reads fine from
    // the merged feed, and collecting reports for it as well would drag it
    // around for motion the user never made.
    let stroke = || {
      Mode::Draw(
        Shape::Stroke {
          points: vec![Point::new(0.0, 0.0)],
          color: [0, 0, 0],
          width: 2.0,
          marker: false,
        },
        Point::new(0.0, 0.0),
      )
    };
    assert!(stroke().stroking(), "a stroke follows the raw reports");
    assert!(!Mode::Idle.stroking());
    assert!(!Mode::Draw(
      Shape::Line {
        from: Point::new(0.0, 0.0),
        to: Point::new(4.0, 4.0),
        color: [0, 0, 0],
        width: 2.0,
        arrow: false,
      },
      Point::new(0.0, 0.0)
    )
    .stroking());
    assert!(!Mode::Rubber(Point::new(0.0, 0.0)).stroking());
  }

  #[test]
  fn resize_cursor_maps_each_handle_to_its_icon() {
    let cases = [
      (TopLeft, CursorIcon::NwseResize),
      (BottomRight, CursorIcon::NwseResize),
      (BottomLeft, CursorIcon::NeswResize),
      (TopRight, CursorIcon::NeswResize),
      (Top, CursorIcon::NsResize),
      (Bottom, CursorIcon::NsResize),
      (Left, CursorIcon::EwResize),
      (Right, CursorIcon::EwResize),
    ];
    for (handle, expected) in cases {
      assert_eq!(resize_cursor(handle), expected);
    }
  }

  #[test]
  fn every_handle_is_hit_at_its_anchor() {
    let sel = Rect::new(10.0, 10.0, 100.0, 100.0);
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
