use std::{
  collections::HashMap,
  hash::{BuildHasherDefault, Hasher},
};

#[derive(Default)]
pub struct FastHasher(u64);

const SEED: u64 = 0x517cc1b727220a95;

#[inline(always)]
fn mix(state: u64, word: u64) -> u64 {
  (state.rotate_left(5) ^ word).wrapping_mul(SEED)
}

impl Hasher for FastHasher {
  #[inline]
  fn write(&mut self, mut bytes: &[u8]) {
    while let Some((chunk, rest)) = bytes.split_first_chunk::<8>() {
      self.0 = mix(self.0, u64::from_ne_bytes(*chunk));
      bytes = rest;
    }
    if let Some((chunk, rest)) = bytes.split_first_chunk::<4>() {
      self.0 = mix(self.0, u32::from_ne_bytes(*chunk) as u64);
      bytes = rest;
    }
    for &b in bytes {
      self.0 = mix(self.0, b as u64);
    }
  }

  #[inline(always)]
  fn write_u8(&mut self, i: u8) {
    self.0 = mix(self.0, i as u64);
  }

  #[inline(always)]
  fn write_u32(&mut self, i: u32) {
    self.0 = mix(self.0, i as u64);
  }

  #[inline(always)]
  fn write_u64(&mut self, i: u64) {
    self.0 = mix(self.0, i);
  }

  #[inline(always)]
  fn write_usize(&mut self, i: usize) {
    self.0 = mix(self.0, i as u64);
  }

  #[inline(always)]
  fn finish(&self) -> u64 {
    self.0
  }
}

pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[cfg(test)]
mod tests {
  use std::hash::Hash;

  use super::*;

  fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = FastHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
  }

  #[test]
  fn equal_keys_hash_identically() {
    let a = (3usize, [10u8, 20, 30], 42u32);
    let b = (3usize, [10u8, 20, 30], 42u32);
    assert_eq!(hash_of(&a), hash_of(&b));
  }

  #[test]
  fn distinguishes_different_keys() {
    let a = (1usize, [0u8, 0, 0], 0u32);
    let b = (2usize, [0u8, 0, 0], 0u32);
    assert_ne!(hash_of(&a), hash_of(&b));
  }

  #[test]
  fn map_round_trips_values() {
    let mut map: FastMap<(char, u32), i32> = FastMap::default();
    map.insert(('a', 12), 1);
    map.insert(('b', 12), 2);
    assert_eq!(map.get(&('a', 12)), Some(&1));
    assert_eq!(map.get(&('b', 12)), Some(&2));
    assert_eq!(map.get(&('a', 13)), None);
  }
}
