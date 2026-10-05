use std::fmt;

use crate::LinkError;

/// A length byte of 0xFF announces a 16-bit little-endian length (`0000:2604`).
const LONG_LENGTH_MARKER: u8 = 0xFF;
/// Largest body the length prefix can describe.
pub(crate) const MAX_MESSAGE_LEN: usize = u16::MAX as usize;

/// One application message: an opaque body whose first byte is the opcode.
#[derive(Clone, PartialEq, Eq)]
pub struct Message(Vec<u8>);

impl Message {
    pub fn try_new(body: Vec<u8>) -> Result<Self, LinkError> {
        if body.len() > MAX_MESSAGE_LEN {
            return Err(LinkError::MessageTooLarge { len: body.len() });
        }
        Ok(Message(body))
    }

    pub fn body(&self) -> &[u8] {
        &self.0
    }

    pub fn opcode(&self) -> Option<u8> {
        self.0.first().copied()
    }

    /// The prefix that precedes the body on the message stream.
    pub(crate) fn length_prefix(&self) -> Vec<u8> {
        match u8::try_from(self.0.len()) {
            Ok(short) if short != LONG_LENGTH_MARKER => vec![short],
            _ => {
                let [low, high] = (self.0.len() as u16).to_le_bytes();
                vec![LONG_LENGTH_MARKER, low, high]
            }
        }
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Message({self})")
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "len={}", self.0.len())?;
        for byte in &self.0 {
            write!(f, " {byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
enum UnpackState {
    #[default]
    Length,
    LongLow,
    LongHigh {
        low: u8,
    },
    Body {
        remaining: usize,
        body: Vec<u8>,
    },
}

/// Splits the in-order DATA payload stream into messages (`0000:2872`); bodies may straddle frames.
#[derive(Debug, Default)]
pub(crate) struct MessageParser {
    state: UnpackState,
}

impl MessageParser {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<Message> {
        let mut complete = Vec::new();
        for &byte in bytes {
            self.state = match std::mem::take(&mut self.state) {
                UnpackState::Length if byte == LONG_LENGTH_MARKER => UnpackState::LongLow,
                UnpackState::Length => Self::start_body(usize::from(byte), &mut complete),
                UnpackState::LongLow => UnpackState::LongHigh { low: byte },
                UnpackState::LongHigh { low } => {
                    let len = usize::from(u16::from_le_bytes([low, byte]));
                    Self::start_body(len, &mut complete)
                }
                UnpackState::Body {
                    remaining,
                    mut body,
                } => {
                    body.push(byte);
                    Self::continue_body(remaining - 1, body, &mut complete)
                }
            };
        }
        complete
    }

    fn start_body(len: usize, complete: &mut Vec<Message>) -> UnpackState {
        Self::continue_body(len, Vec::with_capacity(len), complete)
    }

    fn continue_body(remaining: usize, body: Vec<u8>, complete: &mut Vec<Message>) -> UnpackState {
        if remaining == 0 {
            complete.push(Message(body));
            UnpackState::Length
        } else {
            UnpackState::Body { remaining, body }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(body: &[u8]) -> Message {
        Message::try_new(body.to_vec()).unwrap()
    }

    #[test]
    fn short_and_long_prefixes() {
        assert_eq!(message(&[0x22; 3]).length_prefix(), vec![3]);
        assert_eq!(message(&[0; 0xFE]).length_prefix(), vec![0xFE]);
        assert_eq!(message(&[0; 0xFF]).length_prefix(), vec![0xFF, 0xFF, 0x00]);
        assert_eq!(message(&[0; 300]).length_prefix(), vec![0xFF, 0x2C, 0x01]);
    }

    #[test]
    fn oversized_body_is_rejected() {
        let len = MAX_MESSAGE_LEN + 1;
        assert_eq!(
            Message::try_new(vec![0; len]),
            Err(LinkError::MessageTooLarge { len })
        );
    }

    #[test]
    fn messages_survive_arbitrary_chunking() {
        let originals = [
            message(&[0x22, 0x04, 0x10]),
            message(&[]),
            message(&[0x41; 300]),
        ];
        let mut stream = Vec::new();
        for original in &originals {
            stream.extend(original.length_prefix());
            stream.extend(original.body());
        }
        for chunk_size in [1, 2, 7, stream.len()] {
            let mut parser = MessageParser::new();
            let parsed: Vec<Message> = stream
                .chunks(chunk_size)
                .flat_map(|c| parser.push(c))
                .collect();
            assert_eq!(parsed, originals);
        }
    }
}
