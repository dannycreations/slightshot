mod input;
mod mode;
mod present;
mod resize;
mod screen;
mod session;

use std::thread;

use screen::Screen;
use session::Session;
use winit::{
  application::ApplicationHandler,
  event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent},
  event_loop::{ActiveEventLoop, ControlFlow},
  keyboard::{Key, NamedKey},
  window::WindowId,
};

use crate::{
  action::{self, Deliverable, Shot},
  hotkey::Trigger,
  render::Backdrop,
};

pub(super) enum Outcome {
  Close,
  Deliver {
    deliverable: Deliverable,
    shot: Shot,
  },
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
