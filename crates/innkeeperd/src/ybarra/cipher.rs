//! The cipher over a character record of the three Ybarra games (Shadow of Yserbius, Fates of
//! Twinion, Ruins of Cawdor). The games encrypt bytes 8 to 8 + `SEALED` of each 502-byte slot of
//! their character file before writing it and decrypt it after reading. Its tables and keys are
//! not in this source: they are read from the operator's own copy of a game (`extract`).
//! docs/server/ybarra.md section 2 describes the algorithm.

/// Bytes of a record that the cipher covers, starting at `SEALED_AT`.
pub(crate) const SEALED: usize = 0x1C4;
/// Where the covered bytes start in a record.
pub(crate) const SEALED_AT: usize = 8;
/// Bytes of the shuffle table kept: the largest index read is a key byte (at most 255) plus
/// `SEALED - 1`.
pub(crate) const SHUFFLE_BYTES: usize = 0x100 + SEALED - 1;
/// The bit-permutation table: 32 words of one set bit each.
pub(crate) const MASKS: usize = 32;
/// Key bytes.
pub(crate) const KEY_BYTES: usize = 8;

/// Everything one game's cipher needs.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Cipher {
    pub(crate) shuffle: [u8; SHUFFLE_BYTES],
    pub(crate) masks: [u32; MASKS],
    pub(crate) key: [u8; KEY_BYTES],
}

impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The tables are the operator's game data; a log line has no use for them.
        f.debug_struct("Cipher").finish_non_exhaustive()
    }
}

/// Why a set of tables cannot be a cipher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CipherError {
    /// A table had the wrong length.
    Length,
    /// The masks are not the 32 single bits each used once, so encryption would lose data.
    NotAPermutation,
}

impl Cipher {
    pub(crate) fn new(shuffle: &[u8], masks: &[u8], key: &[u8]) -> Result<Self, CipherError> {
        let mut shuffle: [u8; SHUFFLE_BYTES] =
            shuffle.try_into().map_err(|_| CipherError::Length)?;
        let key: [u8; KEY_BYTES] = key.try_into().map_err(|_| CipherError::Length)?;
        let [first, .., last] = key;
        // Only the bytes this key reaches are the table; past them is whatever the program keeps
        // there, which is neither needed nor kept.
        if let Some(unused) = shuffle.get_mut(usize::from(first.max(last)) + SEALED..) {
            unused.fill(0);
        }
        if masks.len() != MASKS * 4 {
            return Err(CipherError::Length);
        }
        let mut words = [0u32; MASKS];
        let mut seen = 0u32;
        for (word, bytes) in words.iter_mut().zip(masks.chunks_exact(4)) {
            *word = u32::from_le_bytes(bytes.try_into().map_err(|_| CipherError::Length)?);
            if word.count_ones() != 1 || seen & *word != 0 {
                return Err(CipherError::NotAPermutation);
            }
            seen |= *word;
        }
        Ok(Cipher {
            shuffle,
            masks: words,
            key,
        })
    }

    /// The masks as stored: 32 little-endian words.
    pub(crate) fn mask_bytes(&self) -> Vec<u8> {
        self.masks.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    /// Each covered byte's position with the table byte a shuffle pass starting at `start` pairs
    /// it with.
    fn pass(&self, start: u8) -> Vec<(usize, u8)> {
        let table = self.shuffle.iter().copied().skip(usize::from(start));
        (0..SEALED).zip(table).collect()
    }

    /// Encrypts the covered bytes of a record in place.
    pub(crate) fn seal(&self, data: &mut [u8; SEALED]) {
        let [first, .., last] = self.key;
        for (at, mixer) in self.pass(first) {
            let to = usize::from(mixer);
            data.swap(at, to);
            if let Some(byte) = data.get_mut(to) {
                *byte ^= mixer;
            }
        }
        self.permute(data, true);
        for (at, mixer) in self.pass(last) {
            data.swap(at, usize::from(mixer));
        }
    }

    /// Decrypts the covered bytes of a record in place; the exact inverse of `seal`.
    pub(crate) fn open(&self, data: &mut [u8; SEALED]) {
        let [first, .., last] = self.key;
        for (at, mixer) in self.pass(last).into_iter().rev() {
            data.swap(at, usize::from(mixer));
        }
        self.permute(data, false);
        for (at, mixer) in self.pass(first).into_iter().rev() {
            let to = usize::from(mixer);
            if let Some(byte) = data.get_mut(to) {
                *byte ^= mixer;
            }
            data.swap(at, to);
        }
    }

    /// The middle pass: each 4-byte block's bits are moved by the masks, which rotate from block
    /// to block by key byte 5 and by a running sum of the encrypted bytes (key byte 6 at first).
    fn permute(&self, data: &mut [u8; SEALED], sealing: bool) {
        let [_, _, _, _, _, turn, mut chain, _] = self.key;
        let mut masks = self.masks;
        for block in data.chunks_exact_mut(4) {
            for _ in 0..=turn {
                if let Some(part) = masks.get_mut(0..=24) {
                    part.rotate_left(1);
                }
            }
            for _ in 0..=chain {
                if let Some(part) = masks.get_mut(18..=28) {
                    part.rotate_left(1);
                }
            }
            let Ok(word): Result<[u8; 4], _> = (&*block).try_into() else {
                continue;
            };
            let out = if sealing {
                let plain = u32::from_le_bytes(word);
                let sealed = masks
                    .iter()
                    .fold(0u32, |sealed, mask| (sealed << 1) | u32::from(plain & mask != 0));
                sealed.to_be_bytes()
            } else {
                let sealed = u32::from_be_bytes(word);
                let plain = masks
                    .iter()
                    .zip((0..32).rev())
                    .filter(|(_, bit)| sealed & (1 << bit) != 0)
                    .fold(0u32, |plain, (mask, _)| plain | mask);
                plain.to_le_bytes()
            };
            block.copy_from_slice(&out);
            chain = byte_sum(if sealing { &out } else { &word });
        }
    }
}

fn byte_sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A made-up cipher with the right shape: a scrambled shuffle table and a mask permutation.
    pub(crate) fn sample() -> Cipher {
        let mut shuffle = [0u8; SHUFFLE_BYTES];
        let mut state = 0x2545_f491_u32;
        for byte in &mut shuffle {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state.to_le_bytes()[1];
        }
        let mut masks = Vec::new();
        for bit in 0..MASKS {
            masks.extend_from_slice(&(1u32 << ((bit * 7 + 3) % 32)).to_le_bytes());
        }
        Cipher::new(&shuffle, &masks, &[0x21, 0x07, 0x99, 0x10, 0x3c, 0x2a, 0x05, 0xf0]).unwrap()
    }

    #[test]
    fn open_undoes_seal() {
        let cipher = sample();
        let mut data = [0u8; SEALED];
        for (at, byte) in data.iter_mut().enumerate() {
            *byte = (at * 31 % 251) as u8;
        }
        let plain = data;
        cipher.seal(&mut data);
        assert_ne!(data, plain);
        cipher.open(&mut data);
        assert_eq!(data, plain);
    }

    #[test]
    fn a_changed_byte_spreads_into_later_blocks() {
        let cipher = sample();
        let mut a = [0u8; SEALED];
        let mut b = a;
        b[SEALED - 1] = 1;
        cipher.seal(&mut a);
        cipher.seal(&mut b);
        let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert!(differing > 4, "{differing}");
    }

    #[test]
    fn masks_must_be_a_permutation() {
        let shuffle = [0u8; SHUFFLE_BYTES];
        let twice: Vec<u8> = (0..MASKS).flat_map(|_| 1u32.to_le_bytes()).collect();
        assert_eq!(
            Cipher::new(&shuffle, &twice, &[0; KEY_BYTES]),
            Err(CipherError::NotAPermutation)
        );
        assert_eq!(
            Cipher::new(&shuffle[..10], &twice, &[0; KEY_BYTES]),
            Err(CipherError::Length)
        );
    }
}
