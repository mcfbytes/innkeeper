use crate::{BoundedBytes, BoundedText, Int14hError};

/// Field encoder for envelope bodies; the field formats are in int14h-transport.md.
#[derive(Debug, Default)]
pub(crate) struct WireWriter {
    bytes: Vec<u8>,
}

impl WireWriter {
    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes.extend(value.to_le_bytes());
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub(crate) fn text<const MAX: usize>(&mut self, text: &BoundedText<MAX>) {
        self.u8(text.as_str().len() as u8);
        self.bytes.extend(text.as_str().as_bytes());
    }

    pub(crate) fn bytes<const MAX: usize>(&mut self, bytes: &BoundedBytes<MAX>) {
        self.u16(bytes.as_bytes().len() as u16);
        self.bytes.extend(bytes.as_bytes());
    }

    pub(crate) fn option<T>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Self, &T)) {
        self.bool(value.is_some());
        if let Some(value) = value {
            write(self, value);
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// Field decoder over one envelope body.
#[derive(Debug)]
pub(crate) struct WireReader<'a> {
    rest: &'a [u8],
}

impl<'a> WireReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        WireReader { rest: bytes }
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Int14hError> {
        let (&first, rest) = self.rest.split_first().ok_or(Int14hError::Truncated)?;
        self.rest = rest;
        Ok(first)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, Int14hError> {
        Ok(u16::from_le_bytes([self.u8()?, self.u8()?]))
    }

    pub(crate) fn bool(&mut self) -> Result<bool, Int14hError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(Int14hError::InvalidBool(other)),
        }
    }

    pub(crate) fn text<const MAX: usize>(&mut self) -> Result<BoundedText<MAX>, Int14hError> {
        let len = usize::from(self.u8()?);
        let raw = self.take(len)?;
        let text = std::str::from_utf8(raw).map_err(|_| Int14hError::NotAscii { field: "text" })?;
        BoundedText::try_new(text)
    }

    pub(crate) fn bytes<const MAX: usize>(&mut self) -> Result<BoundedBytes<MAX>, Int14hError> {
        let len = usize::from(self.u16()?);
        BoundedBytes::try_new(self.take(len)?)
    }

    pub(crate) fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, Int14hError>,
    ) -> Result<Option<T>, Int14hError> {
        if self.bool()? {
            read(self).map(Some)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn finish(self) -> Result<(), Int14hError> {
        match self.rest.len() {
            0 => Ok(()),
            left => Err(Int14hError::TrailingBytes(left)),
        }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], Int14hError> {
        let (taken, rest) = self
            .rest
            .split_at_checked(len)
            .ok_or(Int14hError::Truncated)?;
        self.rest = rest;
        Ok(taken)
    }
}
