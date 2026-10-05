use crate::MessageError;

/// Reads the field codes of the formatted send (`b`, `w`, `a[n]`, `s`) from a message body.
#[derive(Debug)]
pub(crate) struct WireReader<'a> {
    rest: &'a [u8],
}

impl<'a> WireReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        WireReader { rest: bytes }
    }

    pub(crate) fn byte(&mut self, field: &'static str) -> Result<u8, MessageError> {
        let [byte] = self.array(field)?;
        Ok(byte)
    }

    pub(crate) fn word(&mut self, field: &'static str) -> Result<u16, MessageError> {
        Ok(u16::from_le_bytes(self.array(field)?))
    }

    pub(crate) fn array<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], MessageError> {
        let (head, tail) = self
            .rest
            .split_first_chunk::<N>()
            .ok_or(MessageError::Truncated { field })?;
        self.rest = tail;
        Ok(*head)
    }

    /// A NUL-terminated ASCII string; the NUL is consumed and not returned.
    pub(crate) fn text(&mut self, field: &'static str) -> Result<String, MessageError> {
        let end = self
            .rest
            .iter()
            .position(|&byte| byte == 0)
            .ok_or(MessageError::Truncated { field })?;
        let (text, tail) = self.rest.split_at(end);
        self.rest = tail.get(1..).unwrap_or_default();
        ascii(text, field)
    }

    /// Everything not read yet, for a body the host relays without interpreting it.
    pub(crate) fn rest(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.rest).to_vec()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    pub(crate) fn finish(self) -> Result<(), MessageError> {
        match self.rest.len() {
            0 => Ok(()),
            left => Err(MessageError::TrailingBytes(left)),
        }
    }
}

fn ascii(bytes: &[u8], field: &'static str) -> Result<String, MessageError> {
    if bytes.is_ascii() {
        Ok(bytes.iter().copied().map(char::from).collect())
    } else {
        Err(MessageError::NotAscii { field })
    }
}

/// Row counts are words on the wire, so a longer list is cut to the rows the count can name.
pub(crate) fn fitting_count<T>(rows: &[T]) -> &[T] {
    rows.get(..usize::from(u16::MAX)).unwrap_or(rows)
}

pub(crate) fn row_count<T>(rows: &[T]) -> u16 {
    u16::try_from(rows.len()).unwrap_or(u16::MAX)
}

/// Writes the same field codes; the inverse of [`WireReader`].
#[derive(Debug, Default)]
pub(crate) struct WireWriter {
    bytes: Vec<u8>,
}

impl WireWriter {
    pub(crate) fn byte(&mut self, value: u8) -> &mut Self {
        self.bytes.push(value);
        self
    }

    pub(crate) fn word(&mut self, value: u16) -> &mut Self {
        self.bytes.extend(value.to_le_bytes());
        self
    }

    pub(crate) fn array(&mut self, values: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(values);
        self
    }

    /// Text ends at its first NUL, so a stray one cannot swallow the fields after it.
    pub(crate) fn text(&mut self, value: &str) -> &mut Self {
        self.bytes
            .extend(value.bytes().take_while(|&byte| byte != 0));
        self.bytes.push(0);
        self
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_read_back_what_was_written() {
        let mut writer = WireWriter::default();
        writer
            .byte(53)
            .word(0x86A1)
            .array(&[1, 2, 3])
            .text("guybrush");
        let bytes = writer.into_bytes();
        assert_eq!(bytes[1..3], [0xA1, 0x86]);
        let mut reader = WireReader::new(&bytes);
        assert_eq!(reader.byte("command"), Ok(53));
        assert_eq!(reader.word("word"), Ok(0x86A1));
        assert_eq!(reader.array("array"), Ok([1, 2, 3]));
        assert_eq!(reader.text("name").as_deref(), Ok("guybrush"));
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_list_longer_than_its_count_word_is_cut() {
        let rows = vec![0u8; usize::from(u16::MAX) + 1];
        assert_eq!(fitting_count(&rows).len(), usize::from(u16::MAX));
        assert_eq!(row_count(fitting_count(&rows)), u16::MAX);
    }

    #[test]
    fn rest_takes_the_unread_tail_once() {
        let mut reader = WireReader::new(&[1, 2, 3]);
        assert_eq!(reader.byte("head"), Ok(1));
        assert_eq!(reader.rest(), [2, 3]);
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn text_stops_at_an_embedded_nul() {
        let mut writer = WireWriter::default();
        writer.text("ab\0cd").byte(9);
        assert_eq!(writer.into_bytes(), [b'a', b'b', 0, 9]);
    }

    #[test]
    fn short_or_unterminated_input_is_an_error() {
        assert_eq!(
            WireReader::new(&[1]).word("toSID"),
            Err(MessageError::Truncated { field: "toSID" })
        );
        assert_eq!(
            WireReader::new(b"abc").text("name"),
            Err(MessageError::Truncated { field: "name" })
        );
        assert_eq!(
            WireReader::new(&[0xE9, 0]).text("name"),
            Err(MessageError::NotAscii { field: "name" })
        );
        assert_eq!(
            WireReader::new(&[1, 2]).finish(),
            Err(MessageError::TrailingBytes(2))
        );
    }
}
