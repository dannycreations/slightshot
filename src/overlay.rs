use std::{
  ffi::c_void,
  mem,
  num::NonZeroU32,
  sync::Arc,
  thread,
  time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use softbuffer::{
  Context as SoftContext, Rect as Damage, Surface as SoftSurface,
};
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
  pixel::swap_words_region,
  render::{self, Backdrop, Chrome, Hotspot, Scene, Shown},
  text::TextEngine,
};

const HINT_DURATION: Duration = Duration::from_millis(800);
const CARET_BLINK: Duration = Duration::from_millis(530);
const MAX_STRETCH: f32 = 8.0;
const GRAB_SLOP: f32 = 4.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_SLOP: f32 = 4.0;

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
  MoveRegion(Point),
  MoveText {
    index: usize,
    lifted: Option<Shape>,
    last: Point,
  },
  ResizeRegion(Handle, Rect),
  ResizeText {
    index: usize,
    lifted: Option<Shape>,
    handle: Handle,
    origin: Rect,
  },
  Type(Typing),
}

struct Typing {
  buffer: String,
  at: Point,
  size: f32,
  color: [u8; 3],
  editing: Option<usize>,
  caret: Caret,
}

impl Typing {
  fn new(at: Point, size: f32, color: [u8; 3]) -> Self {
    Self {
      buffer: String::new(),
      at,
      size,
      color,
      editing: None,
      caret: Caret::new(Instant::now()),
    }
  }
}

struct Caret {
  lit: bool,
  due: Instant,
}

impl Caret {
  fn new(now: Instant) -> Self {
    Self {
      lit: true,
      due: now + CARET_BLINK,
    }
  }

  fn flip(&mut self, now: Instant) -> bool {
    if now < self.due {
      return false;
    }
    self.lit = !self.lit;
    self.due = now + CARET_BLINK;
    true
  }

  fn restart(&mut self, now: Instant) {
    self.lit = true;
    self.due = now + CARET_BLINK;
  }
}

impl Mode {
  #[inline(always)]
  fn draft(&self) -> Option<&Shape> {
    match self {
      Mode::Draw(shape, _) if !shape.is_stroke() => Some(shape),
      Mode::MoveText { lifted, .. } | Mode::ResizeText { lifted, .. } => {
        lifted.as_ref()
      }
      _ => None,
    }
  }

  #[inline(always)]
  fn dragging_text(&self) -> bool {
    matches!(self, Mode::MoveText { .. } | Mode::ResizeText { .. })
  }

  #[inline(always)]
  fn typing(&self) -> Option<(Point, &str, f32, [u8; 3])> {
    match self {
      Mode::Type(typing) => {
        Some((typing.at, typing.buffer.as_str(), typing.size, typing.color))
      }
      _ => None,
    }
  }

  #[inline(always)]
  fn caret(&self) -> Option<&Caret> {
    match self {
      Mode::Type(typing) => Some(&typing.caret),
      _ => None,
    }
  }

  #[inline(always)]
  fn caret_due(&self) -> Option<Instant> {
    Some(self.caret()?.due)
  }

  #[inline(always)]
  fn shows_chrome(&self) -> bool {
    matches!(self, Mode::Idle | Mode::Draw(_, _) | Mode::Type(_))
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

  fn get(&self) -> &Pixmap {
    self
      .0
      .as_ref()
      .expect("a session sizes its buffers before it opens")
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
  fn present(&mut self, frame: &Pixmap, area: Rect) -> Result<()> {
    if area.is_empty() {
      return Ok(());
    }
    match self {
      Presenter::Solid(surface) => {
        let Ok(mut buffer) = surface.buffer_mut() else {
          return Ok(());
        };
        if buffer.len() != (frame.width() * frame.height()) as usize {
          return Ok(());
        }
        swap_words_region(frame.data(), frame.width(), &mut buffer, area);
        let damage = [damage_of(area)];
        let _ = buffer.present_with_damage(&damage);
        Ok(())
      }
      // A layered window that cannot be composited stays on the screen as an
      // invisible sheet that still swallows the pointer, so this one is worth
      // reporting instead of dropping.
      Presenter::Live(layer) => layer.present(frame, area),
    }
  }
}

fn damage_of(area: Rect) -> Damage {
  // `MIN` is 1, so the fallback already floors a box that rounded down to
  // nothing. softbuffer rejects a rect with a zero side.
  Damage {
    // A float to integer cast saturates, so a negative origin and a NaN both
    // land on 0 here without a clamp.
    x: area.x.floor() as u32,
    y: area.y.floor() as u32,
    width: NonZeroU32::new(area.w.ceil() as u32).unwrap_or(NonZeroU32::MIN),
    height: NonZeroU32::new(area.h.ceil() as u32).unwrap_or(NonZeroU32::MIN),
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
  picked: Option<usize>,
  mode: Mode,
  tool: Tool,
  palette_index: usize,
  history: History,
  engine: TextEngine,
  cursor: Point,
  press: Option<(Instant, Point)>,
  raw_path: Vec<Point>,
  hover: Option<Hotspot>,
  chrome: Chrome,
  modifiers: ModifiersState,
  sizes: [f32; TOOLS.len()],
  hint: Option<(String, Instant)>,
  current_cursor: CursorIcon,
  painted: Rect,
  shown: Option<Shown>,
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
    session.blink();
    if session.expire_hint() {
      session.window.request_redraw();
    }
    // A hint about to expire and a caret about to blink both want the loop,
    // and whichever is due first is the one that has to wake it.
    let due = [session.hint_due(), session.mode.caret_due()]
      .into_iter()
      .flatten()
      .min();
    event_loop
      .set_control_flow(due.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
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
          Key::Named(NamedKey::Enter) => session.press_enter(),
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
      picked: None,
      mode: Mode::Idle,
      tool,
      palette_index: 0,
      history: History::default(),
      engine,
      cursor: Point::default(),
      press: None,
      raw_path: Vec::new(),
      hover: None,
      chrome: Chrome::new(backdrop),
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

  fn hint_due(&self) -> Option<Instant> {
    self.hint.as_ref().map(|(_, until)| *until)
  }

  fn blink(&mut self) {
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

  fn repaint_typing(&mut self) {
    let area = render::typed_area(self.mode.typing(), &self.engine);
    if !area.is_empty() {
      self.repainted(area);
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
    let draft = self.mode.draft();
    let typing = self.mode.typing();
    // The caret is the session's own clock, so whether it is painted is the
    // mode's answer and not something the picture can be asked for.
    let caret = self.mode.caret().is_some_and(|caret| caret.lit);
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

  fn repainted(&mut self, area: Rect) {
    self.painted = self.painted.union(area);
    self.window.request_redraw();
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
    // A drag of a run of text is dispatched on its own, because it has to take
    // the run off the canvas and the borrow of the mode in hand would not allow
    // it to.
    if self.mode.dragging_text() {
      if self.drag_text(p) {
        self.window.request_redraw();
      }
      return;
    }
    let mut step = None;
    match &mut self.mode {
      Mode::Idle => {
        let icon = self.cursor_at(p);
        self.update_cursor(icon);
      }
      Mode::Rubber(anchor) => {
        let drawn = Rect::spanning(*anchor, p);
        self.set_selection(drawn);
      }
      Mode::Draw(draft, anchor) => {
        step = extend_draft(*anchor, draft, p);
      }
      Mode::MoveRegion(last) => {
        let sel = self
          .selection
          .expect("a region is held to move it, and there is one");
        let delta = Point::new(p.x - last.x, p.y - last.y);
        *last = p;
        self.set_selection(sel.moved_inside(self.bounds, delta));
      }
      Mode::ResizeRegion(handle, rect) => {
        let target = p.clamped_inside(self.bounds);
        let dragged = resized(*rect, *handle, target);
        self.set_selection(dragged);
      }
      // A run being dragged was dispatched above, and typing follows no
      // path of its own.
      Mode::MoveText { .. } | Mode::ResizeText { .. } | Mode::Type(_) => {}
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

  fn drag_text(&mut self, p: Point) -> bool {
    let mut mode = mem::replace(&mut self.mode, Mode::Idle);
    let moved = match &mut mode {
      Mode::MoveText {
        index,
        lifted,
        last,
      } => {
        let delta = Point::new(p.x - last.x, p.y - last.y);
        *last = p;
        if lifted.is_none() {
          *lifted = self.lift_text(*index);
        }
        match lifted.as_mut() {
          Some(Shape::Text { at, text, size, .. }) => {
            // The whole box has to stay on screen, or the run cannot be
            // pressed again to bring it back.
            let box_ = self
              .engine
              .bounds(text, Point::new(at.x + delta.x, at.y + delta.y), *size)
              .clamped_inside(self.bounds);
            *at = Point::new(box_.x, box_.y);
            true
          }
          _ => false,
        }
      }
      Mode::ResizeText {
        index,
        lifted,
        handle,
        origin,
      } => {
        let target = p.clamped_inside(self.bounds);
        if lifted.is_none() {
          *lifted = self.lift_text(*index);
        }
        match lifted.as_mut() {
          Some(Shape::Text { at, text, size, .. }) => {
            let (next_at, next_size) =
              resize_text(&self.engine, text, *origin, *handle, target);
            let moved = *at != next_at || *size != next_size;
            *at = next_at;
            *size = next_size;
            moved
          }
          _ => false,
        }
      }
      _ => false,
    };
    self.mode = mode;
    moved
  }

  fn set_selection(&mut self, sel: Rect) {
    if self.selection != Some(sel) {
      self.selection = Some(sel);
      self.window.request_redraw();
    }
  }

  fn mouse_down(&mut self) -> Option<Outcome> {
    let p = self.cursor;
    // Whatever is half typed lands first, so no click can drop text the user
    // can still see being typed.
    self.commit_typing();
    let double = self.double_clicked(p);
    if let Some(hotspot) = render::hotspot_at(&self.chrome, p) {
      return self.activate(hotspot);
    }
    // A run of text is an object: a press on it picks it up, whatever tool is
    // in hand, so it can be moved and scaled long after it was written.
    if let Some((index, box_)) = self.text_at(p) {
      let handle = self.handle_on(index, box_, p);
      self.picked = Some(index);
      // The second press of a pair asks the run for its text back, so it opens
      // the run for rewriting instead of picking it up all over again.
      if double {
        self.edit_picked();
        self.window.request_redraw();
        return None;
      }
      self.mode = match handle {
        Some(handle) => Mode::ResizeText {
          index,
          lifted: None,
          handle,
          origin: box_,
        },
        None => Mode::MoveText {
          index,
          lifted: None,
          last: p,
        },
      };
      self.window.request_redraw();
      return None;
    }
    // A press that reaches no run at all is the only way to put one down.
    self.picked = None;
    if self.tool == Tool::Select {
      if let Some(region) = self.region_at(p) {
        self.mode = region;
        self.window.request_redraw();
        return None;
      }
    }
    self.mode = match self.tool {
      // A new region starts only where there is one to pick. A live backdrop is
      // the whole screen already, so the select tool has nothing to start
      // there, and a rubber band would shrink the live canvas to it.
      Tool::Select if self.backdrop_kind.picks_region() => {
        self.selection = None;
        Mode::Rubber(p)
      }
      Tool::Select => Mode::Idle,
      Tool::Label => Mode::Type(Typing::new(
        p,
        self.size(Tool::Label),
        active_color(self.palette_index),
      )),
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

  fn double_clicked(&mut self, p: Point) -> bool {
    let now = Instant::now();
    let second = self.press.is_some_and(|(when, at)| {
      is_double_click(now.duration_since(when), at, p)
    });
    self.press = Some((now, p));
    second
  }

  fn region_at(&self, p: Point) -> Option<Mode> {
    // A live backdrop is the whole screen, which is not a thing to move, so
    // there the select tool is only ever a way of picking up what is on it.
    let sel = self
      .selection
      .filter(|_| self.backdrop_kind.picks_region())?;
    match hit_handle(sel, p, HANDLE_SLOP) {
      Some(handle) => Some(Mode::ResizeRegion(handle, sel)),
      None if sel.contains(p) => Some(Mode::MoveRegion(p)),
      None => None,
    }
  }

  fn text_at(&self, p: Point) -> Option<(usize, Rect)> {
    self
      .history
      .shapes()
      .iter()
      .enumerate()
      .rev()
      .find_map(|(index, shape)| {
        let Shape::Text { at, text, size, .. } = shape else {
          return None;
        };
        let box_ = self.engine.bounds(text, *at, *size);
        (!box_.is_empty() && box_.inflated(GRAB_SLOP).contains(p))
          .then_some((index, box_))
      })
  }

  fn picked_box(&self) -> Option<Rect> {
    if let Some(Shape::Text { at, text, size, .. }) = self.mode.draft() {
      return self.text_box(*at, text, *size);
    }
    let Shape::Text { at, text, size, .. } =
      self.history.shape(self.picked?)?
    else {
      return None;
    };
    self.text_box(*at, text, *size)
  }

  fn text_box(&self, at: Point, text: &str, size: f32) -> Option<Rect> {
    let box_ = self.engine.bounds(text, at, size);
    (!box_.is_empty()).then_some(box_)
  }

  fn cursor_at(&self, p: Point) -> CursorIcon {
    if let Some((index, box_)) = self.text_at(p) {
      return match self.handle_on(index, box_, p) {
        Some(handle) => resize_cursor(handle),
        None => CursorIcon::Move,
      };
    }
    if self.tool != Tool::Select {
      return CursorIcon::default();
    }
    match self.region_at(p) {
      Some(Mode::ResizeRegion(handle, _)) => resize_cursor(handle),
      Some(Mode::MoveRegion(_)) => CursorIcon::Move,
      _ => CursorIcon::default(),
    }
  }

  fn handle_on(&self, index: usize, box_: Rect, p: Point) -> Option<Handle> {
    (self.picked == Some(index))
      .then(|| hit_handle(box_, p, HANDLE_SLOP))
      .flatten()
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
          // Undo renumbers everything after the shape it dropped, so a pick
          // that was pointing at that shape has to let go rather than land on
          // whatever took its place.
          if self
            .picked
            .is_some_and(|pick| pick >= self.history.shapes().len())
          {
            self.picked = None;
          }
          self.rebuild(self.bounds);
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
    let canvas = self.buffers.canvas.get();
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

  fn rebuild(&mut self, area: Rect) {
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

  fn ink_segment(&mut self, segment: Segment) {
    self.snapshot();
    let area = segment.bounds();
    let Screen {
      canvas, backdrop, ..
    } = &mut self.buffers;
    render::ink_segment(canvas.at(), backdrop.at(), segment);
    self.repainted(area);
  }

  fn lift_text(&mut self, index: usize) -> Option<Shape> {
    let Shape::Text {
      at,
      text,
      color,
      size,
    } = self.history.shape(index)?
    else {
      return None;
    };
    let shape = Shape::Text {
      at: *at,
      text: text.clone(),
      color: *color,
      size: *size,
    };
    // The damage has to be the ink and not the line the run sits on: a
    // descender reaches below the line and a hook can reach left of it, and
    // whatever is left of a letterform stays on the canvas for the rest of
    // the session.
    let area = render::shape_area(Some(&shape), &self.engine);
    if let Some(Shape::Text { text, .. }) = self.history.shape_mut(index) {
      text.clear();
    }
    self.rebuild(area);
    Some(shape)
  }

  fn land_text(&mut self, index: usize, shape: Shape) {
    let area = render::shape_area(Some(&shape), &self.engine);
    if let Some(slot) = self.history.shape_mut(index) {
      *slot = shape;
    }
    self.rebuild(area);
  }

  fn mouse_up(&mut self) {
    match mem::replace(&mut self.mode, Mode::Idle) {
      Mode::Rubber(_) => {
        self.selection = render::deliverable_region(self.selection);
      }
      Mode::Draw(draft, _) if draft.is_complete() => {
        let area = render::shape_area(Some(&draft), &self.engine);
        let stroke = draft.is_stroke();
        self.history.push(draft);
        // A stroke is already on the ink, segment by segment. Rebuilding it
        // here would composite the whole log over itself a second time.
        if !stroke {
          self.rebuild(area);
        }
      }
      // A run that never left the canvas is already where it belongs.
      Mode::MoveText {
        index,
        lifted: Some(shape),
        ..
      }
      | Mode::ResizeText {
        index,
        lifted: Some(shape),
        ..
      } => self.land_text(index, shape),
      // Typing hands its buffer and anchor straight back: placing a run of
      // text is a click that opens it, so that one mode outlives the release.
      Mode::Type(typing) => self.mode = Mode::Type(typing),
      _ => {}
    }
    self.window.request_redraw();
  }

  fn character(&mut self, ch: &str) -> Option<Outcome> {
    if ch.eq_ignore_ascii_case("c") && self.modifiers.control_key() {
      // A run the pointer is in the middle of moving, or the one the caret is
      // sitting in, is not on the canvas, so it has to land before the image is
      // handed over.
      if self.mode.dragging_text() {
        self.mouse_up();
      }
      self.commit_typing();
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
    if let Mode::Type(typing) = &mut self.mode {
      typing.buffer.push_str(ch);
      typing.caret.restart(Instant::now());
    }
    self.repaint_typing();
  }

  fn backspace(&mut self) {
    if let Mode::Type(typing) = &mut self.mode {
      typing.buffer.pop();
      typing.caret.restart(Instant::now());
    }
    self.repaint_typing();
  }

  fn press_enter(&mut self) {
    if self.mode.typing().is_some() {
      self.commit_typing();
    } else if matches!(self.mode, Mode::Idle) && self.tool == Tool::Select {
      self.edit_picked();
    }
    self.window.request_redraw();
  }

  fn commit_typing(&mut self) {
    let Mode::Type(typing) = mem::replace(&mut self.mode, Mode::Idle) else {
      return;
    };
    let label = Shape::Text {
      at: typing.at,
      text: typing.buffer,
      color: typing.color,
      size: typing.size,
    };
    let Some(index) = typing.editing else {
      if label.is_complete() {
        let area = render::shape_area(Some(&label), &self.engine);
        self.history.push(label);
        // A run that has just been written is picked straight away, so its
        // box is there to press on without reaching for the select tool.
        self.picked = Some(self.history.shapes().len() - 1);
        self.rebuild(area);
      }
      return;
    };
    if !label.is_complete() {
      // A run emptied out while it was being rewritten leaves nothing to
      // draw, and nothing to put back if the session is undone to here.
      self.history.remove(index);
      self.picked = None;
      return;
    }
    self.land_text(index, label);
  }

  fn edit_picked(&mut self) {
    let Some(index) = self.picked else {
      return;
    };
    let Some(Shape::Text {
      at,
      text,
      color,
      size,
    }) = self.lift_text(index)
    else {
      return;
    };
    self.mode = Mode::Type(Typing {
      buffer: text,
      at,
      size,
      color,
      editing: Some(index),
      caret: Caret::new(Instant::now()),
    });
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
    // A stroke keeps the width it was started with; the rest take the new one
    // mid-drag.
    if let Mode::Draw(
      Shape::Line { width, .. } | Shape::Outline { width, .. },
      _,
    ) = &mut self.mode
    {
      *width = next;
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
    Shape::Line { to, .. } => {
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
    Shape::Text { .. } => {
      unreachable!("a run of text is typed, never dragged as a draft")
    }
  }
}

fn fixed_corner(
  origin: Rect,
  handle: Handle,
  width: f32,
  height: f32,
) -> Point {
  let (x, y) = match handle {
    Handle::TopLeft => (origin.right() - width, origin.bottom() - height),
    Handle::Top | Handle::TopRight => (origin.x, origin.bottom() - height),
    Handle::Left | Handle::BottomLeft => (origin.right() - width, origin.y),
    Handle::Right | Handle::Bottom | Handle::BottomRight => {
      (origin.x, origin.y)
    }
  };
  Point::new(x, y)
}

fn resize_text(
  engine: &TextEngine,
  text: &str,
  origin: Rect,
  handle: Handle,
  target: Point,
) -> (Point, f32) {
  if origin.is_empty() {
    return (Point::new(origin.x, origin.y), origin.h);
  }
  let width = match handle {
    Handle::TopLeft | Handle::Left | Handle::BottomLeft => {
      origin.right() - target.x
    }
    Handle::TopRight | Handle::Right | Handle::BottomRight => {
      target.x - origin.x
    }
    Handle::Top | Handle::Bottom => origin.w,
  };
  let height = match handle {
    Handle::TopLeft | Handle::Top | Handle::TopRight => {
      origin.bottom() - target.y
    }
    Handle::BottomLeft | Handle::Bottom | Handle::BottomRight => {
      target.y - origin.y
    }
    Handle::Left | Handle::Right => origin.h,
  };
  let (across, down) = (width / origin.w, height / origin.h);
  let factor = match handle {
    Handle::Left | Handle::Right => across,
    Handle::Top | Handle::Bottom => down,
    _ if (across - 1.0).abs() >= (down - 1.0).abs() => across,
    _ => down,
  };
  let size = (origin.h * factor).clamp(MIN_SIZE, MAX_SIZE);
  let anchor = fixed_corner(origin, handle, engine.width(text, size), size);
  (anchor, size)
}

fn is_double_click(elapsed: Duration, from: Point, to: Point) -> bool {
  elapsed <= DOUBLE_CLICK
    && from.distance_squared(to) <= DOUBLE_CLICK_SLOP * DOUBLE_CLICK_SLOP
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
    // The cursor under a point and the press that follows it both resolve what
    // is under the pointer, so a point inside the region but within reach of a
    // handle has to come back as the same answer for both.
    let sel = Rect::new(100.0, 100.0, 100.0, 100.0);
    assert_eq!(hit_handle(sel, Point::new(150.0, 150.0), HANDLE_SLOP), None);
    assert_eq!(
      hit_handle(sel, Point::new(100.0, 150.0), HANDLE_SLOP),
      Some(Handle::Left),
      "a handle grabs the interior it overlaps"
    );
    assert!(
      !sel.contains(Point::new(90.0, 150.0)),
      "and nothing is held outside the region"
    );
  }

  #[test]
  fn a_run_of_text_takes_the_size_the_handle_drag_reports() {
    let Ok(engine) = TextEngine::load() else {
      return;
    };
    let (at, size, text) = (Point::new(40.0, 60.0), 20.0, "Hg");
    let origin = engine.bounds(text, at, size);

    // A handle taken back to where the drag began has to leave the run exactly
    // where it found it, so a press on a handle without a drag does nothing.
    for &handle in &HANDLES {
      assert_eq!(
        resize_text(
          &engine,
          text,
          origin,
          handle,
          handle_anchor(origin, handle)
        ),
        (at, size),
        "the {handle:?} handle has to leave the run where it found it"
      );
    }

    // A corner leaves the one opposite it alone, and takes the scale from the
    // way it was pulled further: as wide again is no change, as tall again is
    // double.
    assert_eq!(
      resize_text(
        &engine,
        text,
        origin,
        Handle::BottomRight,
        Point::new(origin.right(), origin.bottom() + origin.h),
      ),
      (at, 40.0)
    );

    // Every report of a drag is measured against the box the drag started on,
    // so a corner pulled further and further keeps growing by the step the
    // pointer took rather than chasing the box it just produced.
    let mut walked = size;
    for step in 1..=4 {
      let target = Point::new(
        origin.right() + origin.w * step as f32,
        origin.bottom() + origin.h * step as f32,
      );
      let (_, next) =
        resize_text(&engine, text, origin, Handle::BottomRight, target);
      assert_eq!(
        next,
        (size * (1.0 + step as f32)).min(MAX_SIZE),
        "step {step} of a corner drag has to be the scale the pointer asked \
         for"
      );
      assert!(next >= walked, "and a drag that keeps going keeps growing");
      walked = next;
    }

    // A side handle reads its own axis, ignores the other one, and grows the
    // run out of the anchor it already had.
    let across = Point::new(origin.right() + origin.w, origin.y - 100.0);
    let resized = resize_text(&engine, text, origin, Handle::Right, across);
    assert_eq!(resized.0, at);
    assert!(resized.1 > size);

    // A handle dragged past its anchor bottoms out rather than turning it
    // inside out.
    assert_eq!(
      resize_text(
        &engine,
        text,
        origin,
        Handle::BottomRight,
        handle_anchor(origin, Handle::TopLeft),
      )
      .1,
      MIN_SIZE
    );
  }

  #[test]
  fn two_presses_make_a_pair_only_on_the_same_spot_inside_the_window() {
    let at = Point::new(120.0, 80.0);
    // The rule is the whole of a double click: the same place, inside the
    // window. Anything else is a first press, so a run is only opened for
    // rewriting by a gesture the user meant as one.
    assert!(is_double_click(
      Duration::from_millis(80),
      at,
      Point::new(122.0, 81.0)
    ));
    assert!(
      is_double_click(DOUBLE_CLICK, at, at),
      "the window is as long as it says"
    );
    assert!(
      !is_double_click(DOUBLE_CLICK + Duration::from_millis(1), at, at),
      "a press too late is a first press"
    );
    assert!(
      !is_double_click(Duration::from_millis(80), at, Point::new(160.0, 80.0)),
      "and so is one on the run next door"
    );
  }

  #[test]
  fn the_caret_holds_each_state_for_a_blink_and_a_keystroke_brings_it_back() {
    let start = Instant::now();
    let mut caret = Caret::new(start);
    assert!(caret.lit, "a caret comes up lit");

    // Nothing changes the caret until its time is up, so a frame redrawn for
    // any other reason cannot leave it in the wrong state.
    assert!(!caret.flip(start));
    assert!(!caret.flip(start + CARET_BLINK / 2));
    assert!(caret.lit);

    assert!(caret.flip(start + CARET_BLINK), "then it goes dark");
    assert!(!caret.lit);
    assert!(
      !caret.flip(start + CARET_BLINK + CARET_BLINK / 2),
      "and stays dark for a whole blink"
    );
    assert!(caret.flip(start + 3 * CARET_BLINK), "until it comes back");
    assert!(caret.lit);

    // A keystroke starts the blink again from lit, so the caret is never dark
    // while there is still typing going on.
    caret.restart(start + 3 * CARET_BLINK);
    assert!(caret.lit);
    assert!(
      !caret.flip(start + 3 * CARET_BLINK + Duration::from_millis(1)),
      "and holds steady for a blink after the last keystroke"
    );
    assert!(caret.flip(start + 4 * CARET_BLINK), "then blinks as before");
    assert!(!caret.lit);
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
