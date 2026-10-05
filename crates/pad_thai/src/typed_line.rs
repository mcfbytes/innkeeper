pub(crate) const SEVEN_BITS: u8 = 0x7F;
pub(crate) const CR: u8 = b'\r';
const LF: u8 = b'\n';
const BACKSPACE: u8 = 0x08;

/// A line typed at a terminal: CR completes it, backspace edits it, LF and overflow are dropped.
#[derive(Debug)]
pub(crate) struct TypedLine {
    bytes: Vec<u8>,
    max_len: usize,
}

impl TypedLine {
    pub(crate) fn new(max_len: usize) -> Self {
        TypedLine {
            bytes: Vec::new(),
            max_len,
        }
    }

    /// Applies one typed byte (bit 7 ignored); returns the finished line when a CR completes it.
    pub(crate) fn push(&mut self, byte: u8) -> Option<String> {
        match byte & SEVEN_BITS {
            CR => return Some(String::from_utf8_lossy(&std::mem::take(&mut self.bytes)).into()),
            LF => {}
            BACKSPACE => {
                self.bytes.pop();
            }
            ascii if self.bytes.len() < self.max_len => self.bytes.push(ascii),
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_all(line: &mut TypedLine, text: &[u8]) -> Vec<String> {
        text.iter().filter_map(|&byte| line.push(byte)).collect()
    }

    #[test]
    fn return_completes_the_line_and_backspace_edits_it() {
        let mut line = TypedLine::new(8);
        assert_eq!(type_all(&mut line, b"ATX\x08Z\n\rD\r"), ["ATZ", "D"]);
    }

    #[test]
    fn bit_seven_is_ignored_and_overflow_is_dropped() {
        let mut line = TypedLine::new(3);
        assert_eq!(type_all(&mut line, b"\xC1BCDE\r"), ["ABC"]);
    }
}
