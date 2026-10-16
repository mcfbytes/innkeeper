//! Command 38 (`HintMsg`): the Hint Board room. The sender is `HintRoom` and `hintScript`
//! (`hub/script.135`); the layouts and what the room waits on are in docs/server/hub-services.md.

use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// How many bytes follow the sid in a request: the choices made so far at each menu level.
pub const HINT_ARGS: usize = 4;

/// Nak reason 5: the menu or the whole board is unavailable (`hintScript::handleMsg`).
pub const HINT_UNAVAILABLE: u8 = 5;
/// Nak reason 6: there are no more hints on the topic.
pub const HINT_NO_MORE: u8 = 6;

const MAX_STEP: u8 = 3;

/// Command 38 from the client: `b 38, b step, w sid, b ×4`.
///
/// Step 0 asks for the first menu (the four bytes are 0). Step 1 carries the series picked (byte
/// 0), step 2 adds the game (byte 1), and step 3 asks for one hint: series, game, area and the
/// hint's number, counted from 1. Menu choices are the 1-based position of the line picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HintRequest {
    /// The Hint Board room's own object; every reply is addressed to it.
    pub sid: Sid,
    pub step: u8,
    pub args: [u8; HINT_ARGS],
}

/// What a text reply is: the room keeps menus by level and hints by number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HintKind {
    /// The series menu, answering step 0.
    Series = 0,
    /// The games of one series, answering step 1; `which` is the series.
    Games = 1,
    /// The areas of one game, answering step 2; `which` is the game.
    Areas = 2,
    /// A hint's question, answering step 3; `which` is the hint number.
    Question = 3,
    /// The same hint's answer, sent right after the question.
    Answer = 4,
}

impl TryFrom<u8> for HintKind {
    type Error = MessageError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        [
            HintKind::Series,
            HintKind::Games,
            HintKind::Areas,
            HintKind::Question,
            HintKind::Answer,
        ]
        .into_iter()
        .find(|kind| *kind as u8 == byte)
        .ok_or(MessageError::UnsupportedSub {
            command: Command::Hint.byte(),
            sub: byte,
        })
    }
}

/// Command 38 from the host: `b 38, b kind, w toSID, b which, text, NUL`. A refusal is an ordinary
/// Nak naming command 38 with [`HINT_UNAVAILABLE`] or [`HINT_NO_MORE`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintReply {
    pub to: Sid,
    pub kind: HintKind,
    pub which: u8,
    /// Menus are lines separated by `\n`; the client wraps the rest.
    pub text: String,
}

impl HintRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let step = reader.byte("sub-command")?;
        if step > MAX_STEP {
            return Err(MessageError::UnsupportedSub {
                command: Command::Hint.byte(),
                sub: step,
            });
        }
        Ok(HintRequest {
            sid: Sid(reader.word("sid")?),
            step,
            args: reader.array("args")?,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Hint.byte()).byte(self.step);
        writer.word(self.sid.0).array(&self.args);
    }
}

impl HintReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let kind = HintKind::try_from(reader.byte("whichCmd")?)?;
        Ok(HintReply {
            kind,
            to: Sid(reader.word("toSID")?),
            which: reader.byte("which")?,
            text: reader.text("text")?,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Hint.byte()).byte(self.kind as u8);
        writer.word(self.to.0).byte(self.which).text(&self.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_the_step_the_sid_and_four_choices() {
        let bytes = [38, 3, 0x34, 0x12, 2, 1, 3, 7];
        let mut reader = WireReader::new(&bytes[1..]);
        let request = HintRequest::parse(&mut reader).unwrap();
        assert_eq!(request.sid, Sid(0x1234));
        assert_eq!((request.step, request.args), (3, [2, 1, 3, 7]));
        let mut writer = WireWriter::default();
        request.write(&mut writer);
        assert_eq!(writer.into_bytes(), bytes);
    }

    #[test]
    fn a_step_past_the_hint_is_refused() {
        let mut reader = WireReader::new(&[4, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            HintRequest::parse(&mut reader),
            Err(MessageError::UnsupportedSub {
                command: 38,
                sub: 4
            })
        );
    }

    #[test]
    fn a_reply_is_the_kind_the_sid_the_which_byte_and_the_text() {
        let reply = HintReply {
            to: Sid(0x0102),
            kind: HintKind::Games,
            which: 2,
            text: "Ab\nC".into(),
        };
        let mut writer = WireWriter::default();
        reply.write(&mut writer);
        let bytes = writer.into_bytes();
        assert_eq!(bytes, [38, 1, 2, 1, 2, b'A', b'b', 10, b'C', 0]);
        let mut reader = WireReader::new(&bytes[1..]);
        assert_eq!(HintReply::parse(&mut reader), Ok(reply));
        assert_eq!(reader.finish(), Ok(()));
    }
}
