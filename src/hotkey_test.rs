use super::{Key, Trigger, KEYS};

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
