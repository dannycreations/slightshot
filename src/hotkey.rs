use std::{sync::mpsc, thread};

use anyhow::{bail, Context, Result};
use windows::Win32::UI::{
  Input::KeyboardAndMouse::{
    RegisterHotKey, MOD_NOREPEAT, VIRTUAL_KEY, VK_NUMPAD8, VK_NUMPAD9,
  },
  WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY},
};
use winit::event_loop::EventLoopProxy;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trigger {
  Capture,
  Live,
  Quit,
}

struct Key {
  id: i32,
  vkey: VIRTUAL_KEY,
  name: &'static str,
  trigger: Trigger,
}

const KEYS: [Key; 2] = [
  Key {
    id: 1,
    vkey: VK_NUMPAD8,
    name: "Numpad 8",
    trigger: Trigger::Capture,
  },
  Key {
    id: 2,
    vkey: VK_NUMPAD9,
    name: "Numpad 9",
    trigger: Trigger::Live,
  },
];

impl Key {
  fn trigger_for(id: usize) -> Option<Trigger> {
    KEYS
      .iter()
      .find(|key| key.id as usize == id)
      .map(|key| key.trigger)
  }
}

pub fn spawn(proxy: EventLoopProxy<Trigger>) -> Result<()> {
  let (report, reports) = mpsc::sync_channel::<Result<()>>(1);
  thread::Builder::new()
    .name("slightshot-hotkey".to_string())
    .spawn(move || match register() {
      Ok(()) => {
        let _ = report.send(Ok(()));
        watch(proxy);
      }
      Err(error) => {
        let _ = report.send(Err(error));
      }
    })
    .context("spawning the hotkey watcher failed")?;
  reports.recv().context("the hotkey watcher stopped early")?
}

fn register() -> Result<()> {
  for key in KEYS {
    // SAFETY: a null window handle makes the system post WM_HOTKEY to this
    // thread's own queue, which the watcher below drains; the id and key are
    // constants owned by this thread for the process lifetime.
    if let Err(error) = unsafe {
      RegisterHotKey(None, key.id, MOD_NOREPEAT, u32::from(key.vkey.0))
    } {
      bail!(
        "{} could not be registered as a global hotkey ({error}). \
         Another program may already own that key, or another slightshot \
         instance is running. Close or reconfigure that owner, then start \
         slightshot again.",
        key.name
      );
    }
  }
  Ok(())
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
    if message.message == WM_HOTKEY {
      if let Some(trigger) = Key::trigger_for(message.wParam.0) {
        if proxy.send_event(trigger).is_err() {
          return;
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn numpad_8_snapshots_and_numpad_9_draws_live() {
    let found = KEYS
      .iter()
      .map(|key| (key.name, key.trigger))
      .collect::<Vec<_>>();
    assert_eq!(
      found,
      vec![("Numpad 8", Trigger::Capture), ("Numpad 9", Trigger::Live),]
    );
    assert_eq!(Key::trigger_for(2), Some(Trigger::Live));
    assert_eq!(Key::trigger_for(99), None);
  }
}
