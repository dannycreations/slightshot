use super::Buffer;

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
