use std::{
  ffi::c_void,
  mem,
  num::NonZeroU32,
  sync::Arc,
  thread,
  time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use softbuffer::{Context as SoftContext, Surface as SoftSurface};
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
  geom::{hit_handle, resized, Handle, Point, Rect, HANDLE_SLOP},
  hotkey::Trigger,
  layer::Layered,
  pixel::swap_channels_to_words,
  render::{self, Backdrop, Chrome, Hotspot, Scene},
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
  fn typing(&self) -> Option<(Point, &str)> {
    match self {
      Mode::Type(buffer, at) => Some((*at, buffer.as_str())),
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

#[derive(Default)]
struct Buffer(Option<Pixmap>);

impl Buffer {
  fn fit(&mut self, size: (u32, u32)) -> Result<&mut Pixmap> {
    if !self
      .0
      .as_ref()
      .is_some_and(|pm| (pm.width(), pm.height()) == size)
    {
      self.0 = Some(
        Pixmap::new(size.0, size.1)
          .context("allocating a full-screen buffer failed")?,
      );
    }
    Ok(self.0.as_mut().expect("just ensured"))
  }

  fn at(&mut self) -> &mut Pixmap {
    self
      .0
      .as_mut()
      .expect("a session sizes its buffers before it opens")
  }
}

fn section(
  slot: &mut Option<capture::Bitmap>,
  size: (u32, u32),
) -> Result<&mut capture::Bitmap> {
  if !slot.as_ref().is_some_and(|s| s.fits(size)) {
    *slot = Some(capture::Bitmap::new(size.0, size.1)?);
  }
  Ok(slot.as_mut().expect("the desktop has a nonzero size"))
}

#[derive(Default)]
struct Screen {
  shot: Option<capture::Bitmap>,
  canvas: Buffer,
  backdrop: Buffer,
  frame: Buffer,
}

enum Presenter {
  Solid(Surface),
  Live(Layered),
}

impl Presenter {
  fn present(&mut self, frame: &Pixmap) -> Result<()> {
    match self {
      Presenter::Solid(surface) => {
        let Ok(mut buffer) = surface.buffer_mut() else {
          return Ok(());
        };
        if buffer.len() != (frame.width() * frame.height()) as usize {
          return Ok(());
        }
        swap_channels_to_words(frame.data(), &mut buffer);
        let _ = buffer.present();
        Ok(())
      }
      // A layered window that cannot be composited stays on the screen as an
      // invisible sheet that still swallows the pointer, so this one is worth
      // reporting instead of dropping.
      Presenter::Live(layer) => layer.present(frame),
    }
  }
}

struct Session {
  window: Arc<Window>,
  backdrop_kind: Backdrop,
  base: Option<Pixmap>,
  buffers: Screen,
  presenter: Presenter,
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
  hint: Option<(String, Instant)>,
  current_cursor: CursorIcon,
}

#[derive(Default)]
pub struct App {
  session: Option<Session>,
  screen: Option<Screen>,
}

impl ApplicationHandler<Trigger> for App {
  fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

  fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
    let Some(session) = self.session.as_mut() else {
      return;
    };
    if session.expire_hint() {
      session.window.request_redraw();
    } else if let Some((_, until)) = session.hint {
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
    // The events that end a session have to be answered before the session is
    // borrowed, because answering one means putting that borrow down.
    match &event {
      WindowEvent::CloseRequested => return self.close(),
      WindowEvent::KeyboardInput { event, .. }
        if event.state == ElementState::Pressed
          && event.logical_key == Key::Named(NamedKey::Escape) =>
      {
        return self.close()
      }
      _ => {}
    }
    let Some(session) = self.session.as_mut() else {
      return;
    };
    match event {
      WindowEvent::ModifiersChanged(state) => session.modifiers = state.state(),
      WindowEvent::RedrawRequested => {
        let outcome = session.render();
        if let Err(error) = outcome {
          // The overlay is still a full-screen window even when nothing shows
          // through it, so a dead presenter has to give the desktop back.
          eprintln!("slightshot: the overlay stopped presenting: {error:#}");
          self.close();
        }
      }
      WindowEvent::CursorMoved { position, .. } => session.mouse_move(position),
      WindowEvent::MouseInput {
        state,
        button: MouseButton::Left,
        ..
      } => match state {
        ElementState::Pressed => {
          if let Some(outcome) = session.mouse_down() {
            self.finish(outcome);
          }
        }
        ElementState::Released => session.mouse_up(),
      },
      WindowEvent::KeyboardInput { event, .. } => {
        if event.state != ElementState::Pressed {
          return;
        }
        match event.logical_key {
          Key::Named(NamedKey::Enter) => session.commit_label(),
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
      Trigger::Capture => self.open(event_loop, Backdrop::Frozen),
      Trigger::Live => self.open(event_loop, Backdrop::Live),
      Trigger::Quit => event_loop.exit(),
    }
  }
}

impl App {
  fn open(&mut self, event_loop: &ActiveEventLoop, backdrop: Backdrop) {
    if self.session.is_some() {
      return;
    }
    // A press that fails to open should not leave the buffers locked up here.
    let screen = self.screen.take().unwrap_or_default();
    match Session::create(event_loop, backdrop, screen) {
      Ok(session) => self.session = Some(session),
      Err(error) => {
        eprintln!("slightshot: could not open the overlay: {error:#}")
      }
    }
  }

  fn close(&mut self) {
    if let Some(session) = self.session.take() {
      self.screen = Some(session.buffers);
    }
  }

  fn finish(&mut self, outcome: Outcome) {
    self.close();
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
  fn create(
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
      mode: Mode::Idle,
      tool,
      palette_index: 0,
      history: History::default(),
      engine,
      cursor: Point::default(),
      raw_path: Vec::new(),
      hover: None,
      chrome: Chrome::new(backdrop),
      modifiers: ModifiersState::default(),
      sizes: TOOLS.map(Tool::default_size),
      hint: None,
      current_cursor: CursorIcon::default(),
    };
    session.window.set_visible(true);
    session.render()?;
    Ok(session)
  }

  #[inline]
  fn update_cursor(&mut self, icon: CursorIcon) {
    if self.current_cursor != icon {
      self.current_cursor = icon;
      self.window.set_cursor(icon);
    }
  }

  fn expire_hint(&mut self) -> bool {
    match self.hint {
      Some((_, until)) if Instant::now() >= until => {
        self.hint = None;
        true
      }
      _ => false,
    }
  }

  fn render(&mut self) -> Result<()> {
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
    let label_size = self.size(Tool::Label);
    let draft = self.mode.draft();
    let typing = self
      .mode
      .typing()
      .map(|(at, buffer)| (at, buffer, label_size));
    let hint = self.hint.as_ref().map(|(text, _)| text.as_str());
    let Screen {
      canvas,
      backdrop,
      frame,
      ..
    } = &mut self.buffers;
    let canvas = canvas.at();
    let backdrop = backdrop.at();
    render::paint(
      frame.at(),
      &Scene {
        backdrop,
        canvas,
        bounds: self.bounds,
        selection: self.selection,
        kind: self.backdrop_kind,
        draft,
        typing,
        palette_index: self.palette_index,
        chrome: &self.chrome,
        hotspot: self.hover,
        text: &self.engine,
        hint,
      },
    );
    self.presenter.present(frame.at())
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
          (Tool::Select, Some(sel)) => match grab_at(sel, p) {
            Some(Grab::Resize(handle)) => resize_cursor(handle),
            Some(Grab::Move) => CursorIcon::Move,
            None => CursorIcon::default(),
          },
          _ => CursorIcon::default(),
        };
        self.update_cursor(icon);
      }
      Mode::Rubber(anchor) => {
        let drawn = Rect::spanning(*anchor, p);
        self.set_selection(drawn);
      }
      Mode::Draw(draft, anchor) => {
        step = extend_draft(*anchor, draft, p);
      }
      Mode::Move(last) => {
        let sel = self.selection.expect("move mode requires a selection");
        let delta = Point::new(p.x - last.x, p.y - last.y);
        let moved = sel.moved_inside(self.bounds, delta);
        *last = p;
        self.set_selection(moved);
      }
      Mode::Resize(handle, rect) => {
        let target = p.clamped_inside(self.bounds);
        let dragged = resized(*rect, *handle, target);
        self.set_selection(dragged);
      }
      Mode::Type(_, _) => {}
    }
    if let Some(step) = step {
      match step {
        Step::Repaint => self.window.request_redraw(),
        Step::Segment(segment) => {
          self.ink_segment(segment);
          self.window.request_redraw();
        }
      }
    }
  }

  fn set_selection(&mut self, sel: Rect) {
    if self.selection != Some(sel) {
      self.selection = Some(sel);
      self.window.request_redraw();
    }
  }

  fn mouse_down(&mut self) -> Option<Outcome> {
    let p = self.cursor;
    if let Some(sel) = self.selection {
      if let Some(hotspot) = render::hotspot_at(&self.chrome, p) {
        return self.activate(hotspot);
      }
      if self.tool == Tool::Select {
        if let Some(grab) = grab_at(sel, p) {
          self.mode = match grab {
            Grab::Resize(handle) => Mode::Resize(handle, sel),
            Grab::Move => Mode::Move(p),
          };
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
        // Clicking the tool in hand puts it down and brings the region back,
        // which only means something where the user has a region to pick.
        self.tool = if self.tool == tool && self.backdrop_kind.picks_region() {
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
          self.replay();
          self.window.request_redraw();
        }
        None
      }
      render::Command::Close => Some(Outcome::Close),
      render::Command::Deliver(deliverable) => self.deliver(deliverable),
    }
  }

  fn deliver(&self, deliverable: Deliverable) -> Option<Outcome> {
    if !self.backdrop_kind.picks_region() {
      return None;
    }
    let sel = render::deliverable_region(self.selection)?;
    let canvas = self
      .buffers
      .canvas
      .0
      .as_ref()
      .expect("a session has a canvas");
    Some(Outcome::Deliver {
      deliverable,
      shot: render::flatten(canvas, sel),
    })
  }

  fn snapshot(&mut self) {
    if self.base.is_some() {
      return;
    }
    self.base = Some(self.buffers.canvas.at().clone());
  }

  fn ink_shape(&mut self, shape: &Shape) {
    self.snapshot();
    let Screen {
      canvas, backdrop, ..
    } = &mut self.buffers;
    let canvas = canvas.at();
    render::ink(canvas, shape, &self.engine);
    render::dimmed_into(backdrop.at(), canvas);
  }

  fn ink_segment(&mut self, segment: Segment) {
    self.snapshot();
    let Screen {
      canvas, backdrop, ..
    } = &mut self.buffers;
    render::ink_segment(canvas.at(), backdrop.at(), segment);
  }

  fn replay(&mut self) {
    let Some(base) = &self.base else {
      // Nothing was ever inked, so the canvas is still the capture and undo has
      // nothing to put back.
      return;
    };
    let Screen {
      canvas, backdrop, ..
    } = &mut self.buffers;
    let canvas = canvas.at();
    canvas.data_mut().copy_from_slice(base.data());
    for shape in self.history.shapes() {
      render::ink(canvas, shape, &self.engine);
    }
    render::dimmed_into(backdrop.at(), canvas);
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
    if self.mode.typing().is_none() && matches!(ch, "[" | "]") {
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

  fn commit_label(&mut self) {
    let Mode::Type(buffer, anchor) = &mut self.mode else {
      return;
    };
    let label = Shape::Caption {
      at: *anchor,
      text: mem::take(buffer),
      color: active_color(self.palette_index),
      size: self.size(Tool::Label),
    };
    if label.is_complete() {
      self.ink_shape(&label);
      self.history.push(label);
    }
    self.mode = Mode::Idle;
    self.window.request_redraw();
  }

  #[inline(always)]
  fn size(&self, tool: Tool) -> f32 {
    self.sizes[tool as usize]
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
    self.hint = Some((format_size(next), Instant::now() + HINT_DURATION));
    self.window.request_redraw();
  }
}

fn opening(bounds: Rect, backdrop: Backdrop) -> (Option<Rect>, Tool) {
  match backdrop {
    Backdrop::Frozen => (None, Tool::Select),
    Backdrop::Live => (Some(bounds), Tool::Pen),
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

fn solid_surface(window: &Arc<Window>, size: (u32, u32)) -> Result<Surface> {
  let context = SoftContext::new(window.clone())
    .map_err(|error| anyhow!("no graphics context for the overlay: {error}"))?;
  let mut surface = SoftSurface::new(&context, window.clone())
    .map_err(|error| anyhow!("no surface for the overlay: {error}"))?;
  surface
    .resize(
      NonZeroU32::new(size.0).context("zero-width capture")?,
      NonZeroU32::new(size.1).context("zero-height capture")?,
    )
    .map_err(|e| anyhow!("failed to resize the overlay surface: {e}"))?;
  Ok(surface)
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

#[derive(Clone, Copy, PartialEq, Debug)]
enum Grab {
  Resize(Handle),
  Move,
}

fn grab_at(sel: Rect, p: Point) -> Option<Grab> {
  if let Some(handle) = hit_handle(sel, p, HANDLE_SLOP) {
    return Some(Grab::Resize(handle));
  }
  sel.contains(p).then_some(Grab::Move)
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
  fn a_buffer_is_reallocated_only_when_the_desktop_changes_shape() {
    let mut buffer = Buffer::default();
    let at = buffer.fit((800, 600)).expect("a first fit");
    assert_eq!((at.width(), at.height()), (800, 600));
    let first = at.data_mut().as_mut_ptr();

    // The whole point of parking these between sessions is that the memory
    // survives, so a second press does not fault in every page again.
    let again = buffer.fit((800, 600)).expect("a repeat fit");
    assert_eq!(
      again.data_mut().as_mut_ptr(),
      first,
      "an unchanged desktop must keep the same allocation"
    );

    let wider = buffer.fit((1920, 1080)).expect("a resized fit");
    assert_eq!((wider.width(), wider.height()), (1920, 1080));
    assert_ne!(
      wider.data_mut().as_mut_ptr(),
      first,
      "a new shape needs new memory, not a stale buffer"
    );
  }

  #[test]
  fn a_live_overlay_opens_covering_the_screen_with_the_pen_out() {
    let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
    // The whole screen is the area and the pen is already in hand, so the very
    // first click after the hotkey draws instead of dragging out a region or
    // hunting for the pen in the toolbar.
    assert_eq!(opening(bounds, Backdrop::Live), (Some(bounds), Tool::Pen));
    assert_eq!(opening(bounds, Backdrop::Frozen), (None, Tool::Select));
  }

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
  fn a_handle_wins_over_the_interior_it_overlaps() {
    // The cursor under a point and the press that follows it both ask
    // `grab_at`, so a point that sits inside the region but within reach of a
    // handle has to come back as the same answer for both.
    let sel = Rect::new(100.0, 100.0, 100.0, 100.0);
    assert_eq!(grab_at(sel, Point::new(150.0, 150.0)), Some(Grab::Move));
    assert_eq!(
      grab_at(sel, Point::new(100.0, 150.0)),
      Some(Grab::Resize(Handle::Left)),
      "a handle grabs the interior it overlaps"
    );
    assert_eq!(grab_at(sel, Point::new(90.0, 150.0)), None);
  }

  #[test]
  fn every_handle_is_hit_at_its_anchor() {
    let sel = Rect::new(10.0, 10.0, 100.0, 100.0);
    for &h in &HANDLES {
      let anchor = handle_anchor(sel, h);
      assert_eq!(hit_handle(sel, anchor, HANDLE_SLOP), Some(h));
    }
  }
}
