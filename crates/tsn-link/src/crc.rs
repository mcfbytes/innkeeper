/// Running CRC-16/CCITT-FALSE, computed with TSNEXEC's table-free byte update (`0000:16E0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Crc16(u16);

impl Crc16 {
    pub(crate) const fn new() -> Self {
        Crc16(0xFFFF)
    }

    pub(crate) fn over(bytes: &[u8]) -> Self {
        bytes
            .iter()
            .fold(Crc16::new(), |crc, &byte| crc.update(byte))
    }

    #[must_use]
    pub(crate) const fn update(self, byte: u8) -> Self {
        let mut x = (self.0 >> 8) ^ byte as u16;
        x ^= x >> 4;
        Crc16((self.0 << 8) ^ (x << 12) ^ (x << 5) ^ x)
    }

    pub(crate) const fn from_le_bytes(bytes: [u8; 2]) -> Self {
        Crc16(u16::from_le_bytes(bytes))
    }

    pub(crate) const fn to_le_bytes(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitwise_reference(bytes: &[u8]) -> u16 {
        let mut crc = 0xFFFFu16;
        for &byte in bytes {
            crc ^= u16::from(byte) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 {
                    (crc << 1) ^ 0x1021
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    #[test]
    fn check_value_matches_the_catalogue() {
        assert_eq!(Crc16::over(b"123456789"), Crc16(0x29B1));
    }

    #[test]
    fn table_free_update_equals_bitwise_crc() {
        let sample: Vec<u8> = (0..=255u8).chain((0..=255u8).rev()).collect();
        for end in 0..sample.len() {
            assert_eq!(
                Crc16::over(&sample[..end]).0,
                bitwise_reference(&sample[..end])
            );
        }
    }
}
