//! Randomness from the operating system, shared by `Str::random` and `Lottery`.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// Fills `buf` from the OS random source.
///
/// If the OS refuses (essentially never), falls back to the standard library's randomly seeded
/// hasher, which is unpredictable but not meant for secrets.
pub(crate) fn fill(buf: &mut [u8]) {
    if getrandom::fill(buf).is_ok() {
        return;
    }
    for chunk in buf.chunks_mut(8) {
        let word = RandomState::new().build_hasher().finish().to_le_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
}

pub(crate) fn u64() -> u64 {
    let mut bytes = [0_u8; 8];
    fill(&mut bytes);
    u64::from_le_bytes(bytes)
}

/// A uniform number in `0..bound` (`bound` of zero gives zero), without modulo bias.
pub(crate) fn below(bound: u64) -> u64 {
    if bound <= 1 {
        return 0;
    }
    let zone = u64::MAX - u64::MAX % bound;
    loop {
        let value = u64();
        if value < zone {
            return value % bound;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_stays_in_range() {
        assert_eq!(below(0), 0);
        assert_eq!(below(1), 0);
        assert!((0..200).all(|_| below(7) < 7));
    }

    #[test]
    fn fill_writes_every_byte_position() {
        let mut a = [0_u8; 33];
        let mut b = [0_u8; 33];
        fill(&mut a);
        fill(&mut b);
        assert_ne!(a, b);
    }
}
