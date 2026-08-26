mod actions;
mod annotate;
mod capture;
mod draw;
mod geom;
mod hotkey;
mod overlay;
mod render;
mod text;
mod upload;

use std::process::ExitCode;

use anyhow::Result;
use windows::Win32::UI::HiDpi::{
  SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use winit::event_loop::EventLoop;

use crate::overlay::App;

fn main() -> ExitCode {
  if let Err(error) = run() {
    eprintln!("slightshot: {error:#}");
    return ExitCode::FAILURE;
  }
  ExitCode::SUCCESS
}

fn run() -> Result<()> {
  // SAFETY: must run before any window is created; failure is harmless.
  unsafe {
    let _ =
      SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
  }
  println!("slightshot: starting; Numpad 8 will snapshot the screen for selection. Ctrl+C here quits.");
  let event_loop = EventLoop::<hotkey::Trigger>::with_user_event().build()?;
  let proxy = event_loop.create_proxy();
  hotkey::spawn(proxy.clone())?;
  println!("slightshot: watching Numpad 8.");
  ctrlc::set_handler(move || {
    let _ = proxy.send_event(hotkey::Trigger::Quit);
  })?;
  event_loop.run_app(&mut App::default())?;
  println!("slightshot: stopped watching Numpad 8.");
  Ok(())
}
