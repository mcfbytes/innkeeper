use crate::assumptions::{BOARD_UNREAD_BYTES_ASSUMED, PRIVILEGED_UNREAD_BYTES_ASSUMED};
use crate::message::wire::{fitting_count, row_count, WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

const POST: u8 = 0;
const MESSAGE_REPLY: u8 = 1;
const DIRECTORY: u8 = 13;
const INDEX: u8 = 14;
/// The post builder always sends this many bytes; the rest after the NUL is whatever was in the buffer.
const POST_LENGTH: usize = 0x135;
/// The index request is 27 bytes: 8 zero bytes after the header, then 11 the client never fills.
const INDEX_PADDING: usize = 19;
/// A directory record is 17 bytes; the client reads the first 9.
const LISTING_UNREAD: usize = 8;
/// Bytes between the header and the index words; the client reads the words from byte 0x12.
const INDEX_UNREAD: usize = 10;
/// Bytes between the position flags and the post text, which the client reads from byte 0x12.
const MESSAGE_UNREAD: usize = 6;

/// The two words of a board as the directory handed them out; the client echoes them at bytes 2
/// and 6 of every request. innkeeperd puts the location and the section number there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoardRef {
    pub location: u16,
    pub section: u16,
}

/// Which message a read asks for, relative to the number it sends (docs/protocol/yserbius.md 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ReadStep {
    This = 1,
    Next = 2,
    Previous = 3,
    First = 4,
    Last = 5,
}

const READ_STEPS: [ReadStep; 5] = [
    ReadStep::This,
    ReadStep::Next,
    ReadStep::Previous,
    ReadStep::First,
    ReadStep::Last,
];

/// Command 27 from the RPG clients (`rpgbbs.cpp`): `b 27, b sub, w @2, w fromSID @4, w @6, body @8`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BbsRequest {
    /// Sub 13: the boards of one location of the game (the tavern, the Hall of Heroes).
    Directory { from: Sid, location: u16 },
    /// Sub 14: the message numbers of one board.
    Index { from: Sid, board: BoardRef },
    /// Subs 1 to 5: one message, relative to `message`.
    Read {
        from: Sid,
        board: BoardRef,
        step: ReadStep,
        message: u16,
    },
    /// Sub 0: a new message, `author|text` as the client wrote it; the host stores it unread.
    Post {
        from: Sid,
        board: BoardRef,
        body: Vec<u8>,
    },
}

/// One row of the directory reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardListing {
    /// Bit 0 of the record's flag byte: without it the client does not read the board.
    pub readable: bool,
    /// Message count; 0 makes the client say the section is empty.
    pub total: u16,
    pub board: BoardRef,
    /// How many words of the index the client reads (at most 120).
    pub indexed: u16,
    /// At most 21 characters, `_` for a space.
    pub name: String,
}

/// Where a message stands in its board: the client greys out Next and Previous by these flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MessagePosition {
    pub last: bool,
    pub first: bool,
}

const LAST_FLAG: u16 = 0x4000;
const FIRST_FLAG: u16 = 0x8000;

/// Command 27 from the host, keyed by the sub-command the client dispatches on at byte 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BbsReply {
    Directory {
        to: Sid,
        location: u16,
        boards: Vec<BoardListing>,
    },
    Index {
        to: Sid,
        board: BoardRef,
        messages: Vec<u16>,
    },
    /// Sub 1, whatever step was asked: the message, its number and its position.
    Message {
        to: Sid,
        board: BoardRef,
        number: u16,
        position: MessagePosition,
        /// A privileged reader (user flags 2 or 4) reads the text 4 bytes later.
        privileged: bool,
        body: Vec<u8>,
    },
}

impl TryFrom<u8> for ReadStep {
    type Error = MessageError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        READ_STEPS
            .into_iter()
            .find(|step| *step as u8 == byte)
            .ok_or(MessageError::UnsupportedSub {
                command: Command::Bbs.byte(),
                sub: byte,
            })
    }
}

impl BbsRequest {
    /// The sub-command byte, which a Nak names.
    pub(crate) fn sub(&self) -> u8 {
        match self {
            BbsRequest::Directory { .. } => DIRECTORY,
            BbsRequest::Index { .. } => INDEX,
            BbsRequest::Read { step, .. } => *step as u8,
            BbsRequest::Post { .. } => POST,
        }
    }

    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let sub = reader.byte("sub-command")?;
        let first = reader.word("board")?;
        let from = Sid(reader.word("fromSID")?);
        let second = reader.word("boardId")?;
        let board = BoardRef {
            location: first,
            section: second,
        };
        let request = match sub {
            DIRECTORY => BbsRequest::Directory {
                from,
                location: second,
            },
            INDEX => {
                reader.rest();
                BbsRequest::Index { from, board }
            }
            POST => {
                let body = reader.until_nul("post")?;
                reader.rest();
                BbsRequest::Post { from, board, body }
            }
            step => BbsRequest::Read {
                from,
                board,
                step: ReadStep::try_from(step)?,
                message: reader.word("message")?,
            },
        };
        Ok(request)
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Bbs.byte());
        match self {
            BbsRequest::Directory { from, location } => {
                writer.byte(DIRECTORY).word(0).word(from.0).word(*location);
            }
            BbsRequest::Index { from, board } => {
                write_header(writer, INDEX, *board, *from);
                writer.array(&[0; INDEX_PADDING]);
            }
            BbsRequest::Read {
                from,
                board,
                step,
                message,
            } => {
                write_header(writer, *step as u8, *board, *from);
                writer.word(*message);
            }
            BbsRequest::Post { from, board, body } => {
                write_header(writer, POST, *board, *from);
                let body: Vec<u8> = body.iter().copied().take_while(|&b| b != 0).collect();
                let padding = POST_LENGTH.saturating_sub(body.len() + 8);
                writer.array(&body).array(&vec![0; padding.max(1)]);
            }
        }
    }
}

impl BbsReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        match reader.byte("sub-command")? {
            DIRECTORY => parse_directory(reader),
            INDEX => {
                let (board, to) = read_header(reader)?;
                reader.array::<INDEX_UNREAD>("unread")?;
                let mut messages = Vec::new();
                while !reader.is_empty() {
                    messages.push(reader.word("message")?);
                }
                Ok(BbsReply::Index {
                    to,
                    board,
                    messages,
                })
            }
            MESSAGE_REPLY => parse_message(reader),
            sub => Err(MessageError::UnsupportedSub {
                command: Command::Bbs.byte(),
                sub,
            }),
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Bbs.byte());
        match self {
            BbsReply::Directory {
                to,
                location,
                boards,
            } => {
                let boards = fitting_count(boards);
                writer.byte(DIRECTORY).word(row_count(boards));
                writer.word(to.0).word(*location);
                boards.iter().for_each(|board| board.write_record(writer));
                boards.iter().for_each(|board| {
                    writer.text(&board.name);
                });
            }
            BbsReply::Index {
                to,
                board,
                messages,
            } => {
                write_header(writer, INDEX, *board, *to);
                writer.array(&[0; INDEX_UNREAD]);
                messages.iter().for_each(|&number| {
                    writer.word(number);
                });
            }
            BbsReply::Message {
                to,
                board,
                number,
                position,
                privileged,
                body,
            } => {
                write_header(writer, MESSAGE_REPLY, *board, *to);
                writer.word(*number).word(position.flags());
                writer.array(&[0; MESSAGE_UNREAD]);
                if *privileged {
                    writer.array(&PRIVILEGED_UNREAD_BYTES_ASSUMED);
                }
                let body: Vec<u8> = body.iter().copied().take_while(|&b| b != 0).collect();
                writer.array(&body).byte(0);
            }
        }
    }
}

impl BoardListing {
    fn write_record(&self, writer: &mut WireWriter) {
        writer.byte(u8::from(self.readable)).word(self.total);
        writer.word(self.board.location).word(self.board.section);
        writer.word(self.indexed).array(&BOARD_UNREAD_BYTES_ASSUMED);
    }
}

impl MessagePosition {
    fn flags(self) -> u16 {
        let last = if self.last { LAST_FLAG } else { 0 };
        let first = if self.first { FIRST_FLAG } else { 0 };
        last | first
    }

    fn from_flags(flags: u16) -> Self {
        MessagePosition {
            last: flags & LAST_FLAG != 0,
            first: flags & FIRST_FLAG != 0,
        }
    }
}

fn write_header(writer: &mut WireWriter, sub: u8, board: BoardRef, sid: Sid) {
    writer.byte(sub).word(board.location).word(sid.0);
    writer.word(board.section);
}

fn read_header(reader: &mut WireReader) -> Result<(BoardRef, Sid), MessageError> {
    let location = reader.word("board")?;
    let sid = Sid(reader.word("toSID")?);
    let section = reader.word("boardId")?;
    Ok((BoardRef { location, section }, sid))
}

fn parse_directory(reader: &mut WireReader) -> Result<BbsReply, MessageError> {
    let count = reader.word("count")?;
    let to = Sid(reader.word("toSID")?);
    let location = reader.word("location")?;
    let mut records = Vec::new();
    for _ in 0..count {
        let readable = reader.byte("flags")? & 1 != 0;
        let total = reader.word("total")?;
        let board = BoardRef {
            location: reader.word("board")?,
            section: reader.word("boardId")?,
        };
        let indexed = reader.word("indexed")?;
        reader.array::<LISTING_UNREAD>("unread")?;
        records.push((readable, total, board, indexed));
    }
    let mut boards = Vec::new();
    for (readable, total, board, indexed) in records {
        boards.push(BoardListing {
            readable,
            total,
            board,
            indexed,
            name: reader.text("name")?,
        });
    }
    Ok(BbsReply::Directory {
        to,
        location,
        boards,
    })
}

fn parse_message(reader: &mut WireReader) -> Result<BbsReply, MessageError> {
    let (board, to) = read_header(reader)?;
    let number = reader.word("message")?;
    let position = MessagePosition::from_flags(reader.word("position")?);
    reader.array::<MESSAGE_UNREAD>("unread")?;
    let rest = reader.rest();
    // A post never holds a NUL, so four of them before the text mean the privileged layout.
    let privileged = rest.len() > PRIVILEGED_UNREAD_BYTES_ASSUMED.len()
        && rest.starts_with(&PRIVILEGED_UNREAD_BYTES_ASSUMED);
    let skip = if privileged {
        PRIVILEGED_UNREAD_BYTES_ASSUMED.len()
    } else {
        0
    };
    let mut text = WireReader::new(rest.get(skip..).unwrap_or_default());
    let body = text.until_nul("post")?;
    text.finish()?;
    Ok(BbsReply::Message {
        to,
        board,
        number,
        position,
        privileged,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMessage, HostMessage};

    const TAVERN: BoardRef = BoardRef {
        location: 0x59,
        section: 1,
    };

    #[test]
    fn a_post_keeps_the_text_and_drops_the_buffer_after_its_nul() {
        let mut bytes = vec![27, 0, 0x59, 0, 2, 1, 1, 0];
        bytes.extend(b"Sir Ian|hello\0");
        bytes.resize(POST_LENGTH, 0xCC);
        let parsed = ClientMessage::parse(&bytes);
        assert_eq!(
            parsed,
            Ok(ClientMessage::Bbs(BbsRequest::Post {
                from: Sid(0x0102),
                board: TAVERN,
                body: b"Sir Ian|hello".to_vec(),
            }))
        );
        let encoded = parsed.map(|message| message.encode()).unwrap_or_default();
        assert_eq!(encoded.len(), POST_LENGTH);
    }

    #[test]
    fn the_index_request_ignores_the_bytes_the_client_never_fills() {
        let mut bytes = vec![27, 14, 0x59, 0, 2, 1, 1, 0];
        bytes.resize(27, 0xCC);
        assert_eq!(
            ClientMessage::parse(&bytes),
            Ok(ClientMessage::Bbs(BbsRequest::Index {
                from: Sid(0x0102),
                board: TAVERN,
            }))
        );
    }

    #[test]
    fn steps_past_five_are_unknown_sub_commands() {
        assert_eq!(
            ClientMessage::parse(&[27, 6, 0x59, 0, 2, 1, 1, 0, 1, 0]),
            Err(MessageError::UnsupportedSub {
                command: 27,
                sub: 6
            })
        );
    }

    #[test]
    fn a_privileged_reader_gets_the_text_four_bytes_later() {
        let message = |privileged| BbsReply::Message {
            to: Sid(0x0102),
            board: TAVERN,
            number: 1,
            position: MessagePosition {
                last: true,
                first: true,
            },
            privileged,
            body: b"|".to_vec(),
        };
        let plain = HostMessage::Bbs(message(false)).encode();
        let wide = HostMessage::Bbs(message(true)).encode();
        assert_eq!(plain.get(0x12), Some(&b'|'));
        assert_eq!(wide.get(0x16), Some(&b'|'));
        assert_eq!(plain.get(10..12), Some(&[0x00, 0xC0][..]));
        assert_eq!(
            HostMessage::parse(&wide),
            Ok(HostMessage::Bbs(message(true)))
        );
    }
}
