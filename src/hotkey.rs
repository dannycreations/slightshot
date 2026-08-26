use std::{sync::mpsc, thread};

use anyhow::{anyhow, Context, Result};
use windows::Win32::UI::{
  Input::KeyboardAndMouse::{RegisterHotKey, MOD_NOREPEAT, VK_NUMPAD8},
  WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY},
};
use winit::event_loop::EventLoopProxy;

const HOTKEY_ID: i32 = 1;

#[derive(Clone, Copy, Debug)]
pub enum Trigger {
  Capture,
  Quit,
}

pub fn spawn(proxy: EventLoopProxy<Trigger>) -> Result<()> {
  let (ready, registered) = mpsc::sync_channel::<Result<()>>(1);
  thread::Builder::new()
    .name("slightshot-hotkey".to_string())
    .spawn(move || {
      let outcome = register();
      let registered = outcome.is_ok();
      let _ = ready.send(outcome);
      if registered {
        watch(proxy);
      }
    })
    .context("spawning the hotkey watcher failed")?;
  registered
    .recv()
    .context("the hotkey watcher stopped early")?
}

fn register() -> Result<()> {
  // SAFETY: a null window handle makes the system post WM_HOTKEY to this
  // thread's own queue, which the watcher below drains; the id and key are
  // constants owned by this thread for the process lifetime.
  unsafe {
    RegisterHotKey(None, HOTKEY_ID, MOD_NOREPEAT, u32::from(VK_NUMPAD8.0))
  }
  .map_err(|error| {
    anyhow!(
      "Numpad 8 could not be registered as a global hotkey ({error}). \
       Another program may already own that key, or another slightshot \
       instance is running. Close or reconfigure that owner, then start \
       slightshot again."
    )
  })
}

fn watch(proxy: EventLoopProxy<Trigger>) {
  let mut message = MSG::default();
  loop {
    // SAFETY: `message` is a valid MSG for GetMessageW to fill in. A false
    // return means WM_QUIT or failure; we never post WM_QUIT.
    let received = unsafe { GetMessageW(&mut message, None, 0, 0) };
    if !received.as_bool() {
      return;
    }
    if message.message == WM_HOTKEY
      && message.wParam.0 == HOTKEY_ID as usize
      && proxy.send_event(Trigger::Capture).is_err()
    {
      return;
    }
  }
}
