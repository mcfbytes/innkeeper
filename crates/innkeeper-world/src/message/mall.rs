//! Shoppers Advantage, command 54. Layouts: docs/protocol/int14h-census/shopadv.md section 7.

use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::MessageError;

const TEXT: u8 = 24;
const JOIN: u8 = 25;
const EXIT: u8 = 26;
const STATUS: u8 = 27;
const OUTPUT: u8 = 16;
const JOINED: u8 = 17;
const CONTROL: u8 = 19;
const REFUSED: u8 = 20;

/// The five words Shop copies from request to reply. Word 2 is an object-table slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MallHeader {
    pub slot: u16,
    pub tokens: [u16; 3],
    pub route: u16,
}

/// Command 54 from the client (`shopadv.md` section 7.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MallRequest {
    /// `54/24`: one typed line, or a button command.
    Text { header: MallHeader, text: Vec<u8> },
    /// `54/25`: Shop is up and wants its token.
    Join(MallHeader),
    /// `54/26`: Shop is leaving.
    Exit(MallHeader),
    /// `54/27`: the INN mall gate, and Shop's status probe.
    Status(MallHeader),
}

/// Command 54 from the host (`shopadv.md` section 7.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MallReply {
    /// `54/16`: kind at byte 12, text at byte 13.
    Output {
        header: MallHeader,
        kind: u8,
        text: Vec<u8>,
    },
    /// `54/17`: the three token words, and nothing past the header.
    Joined(MallHeader),
    /// `54/19`: kind at byte 12. `0x1A` acknowledges an exit.
    Control { header: MallHeader, kind: u8 },
    /// `54/20`: kind at byte 12, two unread bytes, text at byte 15.
    Refused {
        header: MallHeader,
        kind: u8,
        unread: [u8; 2],
        text: Vec<u8>,
    },
}

impl MallRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let sub = reader.byte("sub-command")?;
        let header = read_header(reader)?;
        let request = match sub {
            TEXT => MallRequest::Text {
                header,
                text: reader.until_nul("text")?,
            },
            JOIN => MallRequest::Join(header),
            EXIT => MallRequest::Exit(header),
            STATUS => MallRequest::Status(header),
            sub => {
                return Err(MessageError::UnsupportedSub {
                    command: Command::Mall.byte(),
                    sub,
                })
            }
        };
        Ok(request)
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        match self {
            MallRequest::Text { header, text } => {
                write_header(writer, TEXT, header);
                write_text(writer, text);
            }
            MallRequest::Join(header) => write_header(writer, JOIN, header),
            MallRequest::Exit(header) => write_header(writer, EXIT, header),
            MallRequest::Status(header) => write_header(writer, STATUS, header),
        }
    }
}

impl MallReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let sub = reader.byte("sub-command")?;
        let header = read_header(reader)?;
        let reply = match sub {
            OUTPUT => MallReply::Output {
                header,
                kind: reader.byte("kind")?,
                text: reader.until_nul("text")?,
            },
            JOINED => MallReply::Joined(header),
            CONTROL => MallReply::Control {
                header,
                kind: reader.byte("kind")?,
            },
            REFUSED => MallReply::Refused {
                header,
                kind: reader.byte("kind")?,
                unread: reader.array("unread")?,
                text: reader.until_nul("text")?,
            },
            sub => {
                return Err(MessageError::UnsupportedSub {
                    command: Command::Mall.byte(),
                    sub,
                })
            }
        };
        Ok(reply)
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        match self {
            MallReply::Output { header, kind, text } => {
                write_header(writer, OUTPUT, header);
                writer.byte(*kind);
                write_text(writer, text);
            }
            MallReply::Joined(header) => write_header(writer, JOINED, header),
            MallReply::Control { header, kind } => {
                write_header(writer, CONTROL, header);
                writer.byte(*kind);
            }
            MallReply::Refused {
                header,
                kind,
                unread,
                text,
            } => {
                write_header(writer, REFUSED, header);
                writer.byte(*kind).array(unread);
                write_text(writer, text);
            }
        }
    }
}

fn read_header(reader: &mut WireReader) -> Result<MallHeader, MessageError> {
    Ok(MallHeader {
        slot: reader.word("slot")?,
        tokens: [
            reader.word("token")?,
            reader.word("token")?,
            reader.word("token")?,
        ],
        route: reader.word("route")?,
    })
}

fn write_header(writer: &mut WireWriter, sub: u8, header: &MallHeader) {
    writer
        .byte(Command::Mall.byte())
        .byte(sub)
        .word(header.slot);
    for token in header.tokens {
        writer.word(token);
    }
    writer.word(header.route);
}

fn write_text(writer: &mut WireWriter, text: &[u8]) {
    let text = text.iter().copied().take_while(|&byte| byte != 0);
    writer.array(&text.collect::<Vec<_>>()).byte(0);
}
