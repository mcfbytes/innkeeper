use std::collections::VecDeque;

use crate::frame::{escaped_len, escaped_size, MAX_ESCAPED_PAYLOAD};
use crate::Message;

/// Packs messages into DATA payloads like TSNEXEC (`0000:2604`): a length prefix never straddles
/// frames, a body may split at any byte, and every frame stays within the escaped-size limit.
#[derive(Debug, Default)]
pub(crate) struct FramePacker {
    open: Vec<u8>,
    open_escaped: usize,
    closed: VecDeque<Vec<u8>>,
}

impl FramePacker {
    pub(crate) fn push_message(&mut self, message: &Message) {
        let prefix = message.length_prefix();
        if self.open_escaped + escaped_size(&prefix) > MAX_ESCAPED_PAYLOAD {
            self.close();
        }
        prefix.iter().for_each(|&byte| self.append(byte));
        for &byte in message.body() {
            if self.open_escaped + escaped_len(byte) > MAX_ESCAPED_PAYLOAD {
                self.close();
            }
            self.append(byte);
        }
    }

    /// Closes a partly filled frame so it can be sent now, as the Flush export does.
    pub(crate) fn flush(&mut self) {
        if !self.open.is_empty() {
            self.close();
        }
    }

    pub(crate) fn has_closed_frame(&self) -> bool {
        !self.closed.is_empty()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.open.is_empty() && self.closed.is_empty()
    }

    pub(crate) fn pop_closed(&mut self) -> Option<Vec<u8>> {
        self.closed.pop_front()
    }

    fn append(&mut self, byte: u8) {
        self.open.push(byte);
        self.open_escaped += escaped_len(byte);
    }

    fn close(&mut self) {
        self.closed.push_back(std::mem::take(&mut self.open));
        self.open_escaped = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageParser;

    fn drain(packer: &mut FramePacker) -> Vec<Vec<u8>> {
        packer.flush();
        std::iter::from_fn(|| packer.pop_closed()).collect()
    }

    #[test]
    fn small_messages_share_one_frame() {
        let mut packer = FramePacker::default();
        packer.push_message(&Message::try_new(vec![0x22, 0x04]).unwrap());
        packer.push_message(&Message::try_new(vec![0x24]).unwrap());
        assert_eq!(drain(&mut packer), vec![vec![2, 0x22, 0x04, 1, 0x24]]);
    }

    #[test]
    fn large_messages_split_within_the_limit_and_reassemble() {
        let messages = [
            Message::try_new(vec![0x81; 400]).unwrap(),
            Message::try_new((0..=255).collect()).unwrap(),
        ];
        let mut packer = FramePacker::default();
        messages.iter().for_each(|m| packer.push_message(m));
        let frames = drain(&mut packer);
        assert!(frames
            .iter()
            .all(|f| escaped_size(f) <= MAX_ESCAPED_PAYLOAD));
        let mut parser = MessageParser::new();
        let parsed: Vec<Message> = frames.iter().flat_map(|f| parser.push(f)).collect();
        assert_eq!(parsed, messages);
    }

    #[test]
    fn prefix_moves_to_a_new_frame_rather_than_splitting() {
        let mut packer = FramePacker::default();
        packer.push_message(&Message::try_new(vec![0x41; MAX_ESCAPED_PAYLOAD - 3]).unwrap());
        packer.push_message(&Message::try_new(vec![0x42; 300]).unwrap());
        let frames = drain(&mut packer);
        assert_eq!(frames[0].len(), MAX_ESCAPED_PAYLOAD - 2);
        assert_eq!(&frames[1][..3], &[0xFF, 0x2C, 0x01]);
    }
}
