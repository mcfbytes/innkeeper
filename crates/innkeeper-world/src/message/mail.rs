//! Commands 37 (`EMMsg`, mail) and 45 (`NewBoxHandler`, mailbox assignment), both directions.
//! The layouts are in docs/protocol/messages.md 3.3 and the behaviour in docs/server/mail.md.

use crate::assumptions::{
    ACK_UNREAD_BYTE_ASSUMED, LISTED_LETTER_STATUS_ASSUMED, REFUSED_MAILBOX_ASSUMED,
    SYSTEM_LIST_UNREAD_WORDS_ASSUMED,
};
use crate::host_time::HostTime;
use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{AccountId, LetterId, MailboxNumber, MessageError, Sid};

const BOXES_OF_ACCOUNT: u8 = 17;
const NEW_MAIL: u8 = 18;
const LIST: u8 = 19;
const SEND_TO_BOX: u8 = 20;
const SEND_CLAIMED: u8 = 21;
const SEND: u8 = 24;
const READ: u8 = 25;
const DELETE: u8 = 26;
const FORWARD: u8 = 27;
const FORM: u8 = 30;
const SERVICES: u8 = 31;
const SYSTEM_LIST: u8 = 32;
const ASSIGN_MAILBOX: u8 = 1;

/// The fixed text fields of an envelope are ten bytes; a shorter string is followed by heap bytes.
pub(crate) const FIELD_LEN: usize = 10;

/// The client keeps every 32-bit number as a low and a high word.
fn read_long(reader: &mut WireReader) -> Result<u32, MessageError> {
    let low = reader.word("low")?;
    let high = reader.word("high")?;
    Ok(u32::from(low) | (u32::from(high) << 16))
}

fn write_long(writer: &mut WireWriter, value: u32) {
    writer.word(value as u16).word((value >> 16) as u16);
}

fn read_box(reader: &mut WireReader) -> Result<MailboxNumber, MessageError> {
    read_long(reader).map(MailboxNumber)
}

fn write_box(writer: &mut WireWriter, mailbox: MailboxNumber) {
    write_long(writer, mailbox.0);
}

fn read_letter(reader: &mut WireReader) -> Result<LetterId, MessageError> {
    read_long(reader).map(LetterId)
}

fn write_letter(writer: &mut WireWriter, letter: LetterId) {
    write_long(writer, letter.0);
}

/// The six bytes of a host time, as `HostInfo` type 2 and the letter records carry them.
fn write_time(writer: &mut WireWriter, time: HostTime) {
    writer
        .byte(time.year_1900)
        .byte(time.month0)
        .byte(time.mday);
    writer.byte(time.hour).byte(time.minute).byte(time.second);
}

fn read_time(reader: &mut WireReader) -> Result<HostTime, MessageError> {
    Ok(HostTime {
        year_1900: reader.byte("year")?,
        month0: reader.byte("month")?,
        mday: reader.byte("day")?,
        hour: reader.byte("hour")?,
        minute: reader.byte("minute")?,
        second: reader.byte("second")?,
    })
}

/// A letter as the client composed it: every byte of a send message after the addressee's box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    /// Ten bytes the client fills from an empty string; nothing reads them.
    pub reserved: [u8; FIELD_LEN],
    /// The box the writer filled in as its own (`isMe` and `formatTo`).
    pub from: MailboxNumber,
    pub subject: [u8; FIELD_LEN],
    pub flags: u16,
    pub addressee: [u8; FIELD_LEN],
    pub sender: [u8; FIELD_LEN],
    /// The body as the client wrote it, with the NUL the client counts in.
    pub text: Vec<u8>,
}

impl Envelope {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        Ok(Envelope {
            reserved: reader.array("reserved")?,
            from: read_box(reader)?,
            subject: reader.array("subject")?,
            flags: reader.word("flags")?,
            addressee: reader.array("addressee")?,
            sender: reader.array("sender")?,
            text: reader.rest(),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.array(&self.reserved);
        write_box(writer, self.from);
        writer.array(&self.subject).word(self.flags);
        writer.array(&self.addressee).array(&self.sender);
        writer.array(&self.text);
    }
}

/// What the host keeps of a delivered letter: when it arrived, then the envelope unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredLetter {
    pub(crate) received: HostTime,
    pub(crate) envelope: Envelope,
}

impl StoredLetter {
    pub(crate) fn to_body(&self) -> Vec<u8> {
        let mut writer = WireWriter::default();
        write_time(&mut writer, self.received);
        self.envelope.write(&mut writer);
        writer.into_bytes()
    }

    pub(crate) fn from_body(body: &[u8]) -> Result<Self, MessageError> {
        let mut reader = WireReader::new(body);
        let received = read_time(&mut reader)?;
        let envelope = Envelope::parse(&mut reader)?;
        Ok(StoredLetter { received, envelope })
    }
}

/// A letter on its way: the box it goes to, the object that gets the answer and the envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingLetter {
    pub sid: Sid,
    pub to: MailboxNumber,
    pub envelope: Envelope,
}

/// A request about one letter of one box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LetterRef {
    pub sid: Sid,
    pub mailbox: MailboxNumber,
    pub letter: LetterId,
}

/// Command 37 from the client: `b 37, b sub, w sid`, then the sub-command's own fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailRequest {
    /// 17: which boxes belong to this account; the Mail Room asks with the account number.
    Boxes {
        sid: Sid,
        account: AccountId,
    },
    /// 18: does the box hold mail.
    NewMail {
        sid: Sid,
        mailbox: MailboxNumber,
    },
    List {
        sid: Sid,
        mailbox: MailboxNumber,
    },
    /// 20, 21 or 24: put a letter in the addressee's box.
    Send {
        sub: u8,
        letter: OutgoingLetter,
    },
    /// 30: a form for a service box, such as `AcctUpdate`.
    Form(OutgoingLetter),
    Read(LetterRef),
    Delete(LetterRef),
    /// 27: copy a letter of this box into another box.
    Forward {
        letter: LetterRef,
        to: MailboxNumber,
        addressee: [u8; FIELD_LEN],
        /// The client's note for the new reader, kept as it came.
        note: Vec<u8>,
    },
    Services {
        sid: Sid,
        mailbox: MailboxNumber,
    },
    SystemList {
        sid: Sid,
    },
}

impl MailRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let sub = reader.byte("sub-command")?;
        let sid = Sid(reader.word("sid")?);
        match sub {
            BOXES_OF_ACCOUNT => Ok(MailRequest::Boxes {
                sid,
                account: AccountId(read_long(reader)?),
            }),
            NEW_MAIL => Ok(MailRequest::NewMail {
                sid,
                mailbox: read_box(reader)?,
            }),
            LIST => Ok(MailRequest::List {
                sid,
                mailbox: read_box(reader)?,
            }),
            SEND_TO_BOX | SEND_CLAIMED | SEND => Ok(MailRequest::Send {
                sub,
                letter: OutgoingLetter::parse(reader, sid)?,
            }),
            FORM => Ok(MailRequest::Form(OutgoingLetter::parse(reader, sid)?)),
            READ => Ok(MailRequest::Read(LetterRef::parse(reader, sid)?)),
            DELETE => Ok(MailRequest::Delete(LetterRef::parse(reader, sid)?)),
            FORWARD => Ok(MailRequest::Forward {
                letter: LetterRef::parse(reader, sid)?,
                to: read_box(reader)?,
                addressee: reader.array("addressee")?,
                note: reader.rest(),
            }),
            SERVICES => Ok(MailRequest::Services {
                sid,
                mailbox: read_box(reader)?,
            }),
            SYSTEM_LIST => {
                reader.word("unused")?;
                reader.word("unused")?;
                Ok(MailRequest::SystemList { sid })
            }
            sub => Err(MessageError::UnsupportedSub {
                command: Command::Mail.byte(),
                sub,
            }),
        }
    }

    /// The Nak that refuses this request, shaped as the client matches it; a form has no answer.
    pub fn refused(&self, fault: MailFault) -> Option<MailReply> {
        let refusal = |sub, to, mailbox| MailReply::Refused {
            sub,
            to,
            fault,
            mailbox,
        };
        match self {
            MailRequest::Boxes { sid, account } => {
                Some(refusal(BOXES_OF_ACCOUNT, *sid, MailboxNumber(account.0)))
            }
            MailRequest::NewMail { sid, mailbox } => Some(refusal(NEW_MAIL, *sid, *mailbox)),
            MailRequest::List { sid, mailbox } => Some(refusal(LIST, *sid, *mailbox)),
            MailRequest::Send { sub, letter } => Some(refusal(*sub, letter.sid, letter.to)),
            MailRequest::Form(_) => None,
            MailRequest::Read(letter) => Some(refusal(READ, letter.sid, letter.mailbox)),
            MailRequest::Delete(letter) => Some(refusal(DELETE, letter.sid, letter.mailbox)),
            MailRequest::Forward { letter, to, .. } => Some(refusal(FORWARD, letter.sid, *to)),
            MailRequest::Services { sid, mailbox } => Some(refusal(SERVICES, *sid, *mailbox)),
            MailRequest::SystemList { sid } => {
                Some(refusal(SYSTEM_LIST, *sid, REFUSED_MAILBOX_ASSUMED))
            }
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Mail.byte());
        match self {
            MailRequest::Boxes { sid, account } => {
                writer.byte(BOXES_OF_ACCOUNT).word(sid.0);
                write_long(writer, account.0);
            }
            MailRequest::NewMail { sid, mailbox } => {
                writer.byte(NEW_MAIL).word(sid.0);
                write_box(writer, *mailbox);
            }
            MailRequest::List { sid, mailbox } => {
                writer.byte(LIST).word(sid.0);
                write_box(writer, *mailbox);
            }
            MailRequest::Send { sub, letter } => letter.write(writer, *sub),
            MailRequest::Form(letter) => letter.write(writer, FORM),
            MailRequest::Read(letter) => letter.write(writer, READ),
            MailRequest::Delete(letter) => letter.write(writer, DELETE),
            MailRequest::Forward {
                letter,
                to,
                addressee,
                note,
            } => {
                letter.write(writer, FORWARD);
                write_box(writer, *to);
                writer.array(addressee).array(note);
            }
            MailRequest::Services { sid, mailbox } => {
                writer.byte(SERVICES).word(sid.0);
                write_box(writer, *mailbox);
            }
            MailRequest::SystemList { sid } => {
                writer.byte(SYSTEM_LIST).word(sid.0).word(0).word(0);
            }
        }
    }
}

impl OutgoingLetter {
    fn parse(reader: &mut WireReader, sid: Sid) -> Result<Self, MessageError> {
        Ok(OutgoingLetter {
            sid,
            to: read_box(reader)?,
            envelope: Envelope::parse(reader)?,
        })
    }

    fn write(&self, writer: &mut WireWriter, sub: u8) {
        writer.byte(sub).word(self.sid.0);
        write_box(writer, self.to);
        self.envelope.write(writer);
    }
}

impl LetterRef {
    fn parse(reader: &mut WireReader, sid: Sid) -> Result<Self, MessageError> {
        let mailbox = read_box(reader)?;
        let letter = read_letter(reader)?;
        Ok(LetterRef {
            sid,
            mailbox,
            letter,
        })
    }

    fn write(&self, writer: &mut WireWriter, sub: u8) {
        writer.byte(sub).word(self.sid.0);
        write_box(writer, self.mailbox);
        write_letter(writer, self.letter);
    }
}

/// Why the host refuses a mail request; the client shows the text of the code (script.145).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MailFault {
    /// "because the host had some kind of disk error".
    HostDisk = 2,
    /// "because it does not exist": the addressee's box.
    NoSuchBox = 6,
    /// "because the box number was not between 1 and 4294901759".
    BoxOutOfRange = 9,
    /// "because you do not have rights to do that".
    NotYourBox = 12,
    /// "because that letter does not exist".
    NoSuchLetter = 14,
    /// "because the post office is closed".
    PostOfficeClosed = 21,
}

impl TryFrom<u8> for MailFault {
    type Error = MessageError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        [
            MailFault::HostDisk,
            MailFault::NoSuchBox,
            MailFault::BoxOutOfRange,
            MailFault::NotYourBox,
            MailFault::NoSuchLetter,
            MailFault::PostOfficeClosed,
        ]
        .into_iter()
        .find(|fault| *fault as u8 == byte)
        .ok_or(MessageError::UnknownValue {
            field: "mail fault",
            value: u16::from(byte),
        })
    }
}

/// One row of a box listing, the 47 bytes the client reads in `ackMsgStr` (script.145).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedLetter {
    pub id: LetterId,
    pub from: MailboxNumber,
    pub subject: [u8; FIELD_LEN],
    pub sender: [u8; FIELD_LEN],
    pub flags: u16,
    pub received: HostTime,
    pub addressee: [u8; FIELD_LEN],
}

impl ListedLetter {
    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("status")?;
        let id = read_letter(reader)?;
        let from = read_box(reader)?;
        let subject = reader.array("subject")?;
        let sender = reader.array("sender")?;
        let flags = reader.word("flags")?;
        let received = read_time(reader)?;
        Ok(ListedLetter {
            id,
            from,
            subject,
            sender,
            flags,
            received,
            addressee: reader.array("addressee")?,
        })
    }

    fn write(&self, writer: &mut WireWriter) {
        writer.byte(LISTED_LETTER_STATUS_ASSUMED);
        write_letter(writer, self.id);
        write_box(writer, self.from);
        writer.array(&self.subject).array(&self.sender);
        writer.word(self.flags);
        write_time(writer, self.received);
        writer.array(&self.addressee);
    }
}

/// Command 37 from the host, and the Ack and Nak for it, which carry `whichCmd` 37.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailReply {
    /// 17: the boxes of the account, from byte 8 as pairs of words.
    Boxes {
        to: Sid,
        account: AccountId,
        boxes: Vec<MailboxNumber>,
    },
    /// 18: bit 0 of `status` lights the mail icon, bit 1 marks the box.
    NewMail {
        to: Sid,
        mailbox: MailboxNumber,
        status: u16,
    },
    Listing {
        to: Sid,
        mailbox: MailboxNumber,
        letters: Vec<ListedLetter>,
    },
    Letter {
        to: Sid,
        mailbox: MailboxNumber,
        id: LetterId,
        text: Vec<u8>,
    },
    /// 32: the system list, always without records.
    EmptySystemList { to: Sid },
    /// Ack 37/20, 37/21 or 37/24: the letter reached `mailbox`.
    Delivered {
        sub: u8,
        to: Sid,
        mailbox: MailboxNumber,
    },
    /// Ack 37/27: the client says "Letter successfully forwarded."
    Forwarded { to: Sid, mailbox: MailboxNumber },
    /// Nak 37: request `sub` failed for `mailbox`.
    Refused {
        sub: u8,
        to: Sid,
        fault: MailFault,
        mailbox: MailboxNumber,
    },
}

impl MailReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let which_cmd = reader.byte("whichCmd")?;
        let to = Sid(reader.word("toSID")?);
        match which_cmd {
            BOXES_OF_ACCOUNT => {
                let account = AccountId(read_long(reader)?);
                let mut boxes = Vec::new();
                while !reader.is_empty() {
                    boxes.push(read_box(reader)?);
                }
                Ok(MailReply::Boxes { to, account, boxes })
            }
            NEW_MAIL => Ok(MailReply::NewMail {
                to,
                mailbox: read_box(reader)?,
                status: reader.word("status")?,
            }),
            LIST => {
                let mailbox = read_box(reader)?;
                let mut letters = Vec::new();
                while !reader.is_empty() {
                    letters.push(ListedLetter::parse(reader)?);
                }
                Ok(MailReply::Listing {
                    to,
                    mailbox,
                    letters,
                })
            }
            READ => Ok(MailReply::Letter {
                to,
                mailbox: read_box(reader)?,
                id: read_letter(reader)?,
                text: reader.rest(),
            }),
            SYSTEM_LIST => {
                read_box(reader)?;
                Ok(MailReply::EmptySystemList { to })
            }
            sub => Err(MessageError::UnsupportedSub {
                command: Command::Mail.byte(),
                sub,
            }),
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        match self {
            MailReply::Boxes { to, account, boxes } => {
                let header = writer.byte(Command::Mail.byte());
                header.byte(BOXES_OF_ACCOUNT).word(to.0);
                write_long(writer, account.0);
                boxes.iter().for_each(|mailbox| write_box(writer, *mailbox));
            }
            MailReply::NewMail {
                to,
                mailbox,
                status,
            } => {
                writer.byte(Command::Mail.byte()).byte(NEW_MAIL).word(to.0);
                write_box(writer, *mailbox);
                writer.word(*status);
            }
            MailReply::Listing {
                to,
                mailbox,
                letters,
            } => {
                writer.byte(Command::Mail.byte()).byte(LIST).word(to.0);
                write_box(writer, *mailbox);
                letters.iter().for_each(|letter| letter.write(writer));
            }
            MailReply::Letter {
                to,
                mailbox,
                id,
                text,
            } => {
                writer.byte(Command::Mail.byte()).byte(READ).word(to.0);
                write_box(writer, *mailbox);
                write_letter(writer, *id);
                writer.array(text);
            }
            MailReply::EmptySystemList { to } => {
                writer
                    .byte(Command::Mail.byte())
                    .byte(SYSTEM_LIST)
                    .word(to.0);
                let [low, high] = SYSTEM_LIST_UNREAD_WORDS_ASSUMED;
                writer.word(low).word(high);
            }
            MailReply::Delivered { sub, to, mailbox } => {
                writer.byte(Command::Ack.byte()).byte(0).word(to.0);
                writer.byte(Command::Mail.byte()).byte(*sub);
                writer.byte(ACK_UNREAD_BYTE_ASSUMED);
                write_box(writer, *mailbox);
            }
            MailReply::Forwarded { to, mailbox } => {
                writer.byte(Command::Ack.byte()).byte(0).word(to.0);
                writer.byte(Command::Mail.byte()).byte(FORWARD);
                writer.byte(ACK_UNREAD_BYTE_ASSUMED);
                write_box(writer, *mailbox);
            }
            MailReply::Refused {
                sub,
                to,
                fault,
                mailbox,
            } => {
                writer.byte(Command::Nak.byte()).byte(0).word(to.0);
                writer
                    .byte(Command::Mail.byte())
                    .byte(*sub)
                    .byte(*fault as u8);
                write_box(writer, *mailbox);
            }
        }
    }

    /// The Ack for command 37, after its `whichCmd` and `whichSub`.
    pub(crate) fn parse_ack_after(
        reader: &mut WireReader,
        to: Sid,
        sub: u8,
    ) -> Result<Self, MessageError> {
        reader.byte("unused")?;
        let mailbox = read_box(reader)?;
        match sub {
            FORWARD => Ok(MailReply::Forwarded { to, mailbox }),
            sub => Ok(MailReply::Delivered { sub, to, mailbox }),
        }
    }

    /// The Nak for command 37, after its `whichCmd` and `whichSub`.
    pub(crate) fn parse_nak_after(
        reader: &mut WireReader,
        to: Sid,
        sub: u8,
    ) -> Result<Self, MessageError> {
        let fault = MailFault::try_from(reader.byte("fault")?)?;
        let mailbox = read_box(reader)?;
        Ok(MailReply::Refused {
            sub,
            to,
            fault,
            mailbox,
        })
    }
}

/// Command 45 from the client: `b 45, b 1, w sid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MailboxRequest {
    pub sid: Sid,
}

impl MailboxRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        match reader.byte("sub-command")? {
            ASSIGN_MAILBOX => Ok(MailboxRequest {
                sid: Sid(reader.word("sid")?),
            }),
            sub => Err(MessageError::UnsupportedSub {
                command: Command::NewBox.byte(),
                sub,
            }),
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::NewBox.byte()).byte(ASSIGN_MAILBOX);
        writer.word(self.sid.0);
    }
}

/// Command 45 from the host: `b 45, b 1, w toSID, b status, w low, w high`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxAnswer {
    Assigned {
        to: Sid,
        mailbox: MailboxNumber,
    },
    /// Status 2 to 6; the client shows an error and, for 4, tries again at the next logon.
    Refused {
        to: Sid,
        status: u8,
    },
}

const ASSIGNED_STATUS: u8 = 1;
const FIRST_REFUSAL_STATUS: u8 = 2;
const LAST_REFUSAL_STATUS: u8 = 6;

impl MailboxAnswer {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let which_cmd = reader.byte("whichCmd")?;
        let to = Sid(reader.word("toSID")?);
        let status = reader.byte("status")?;
        let mailbox = read_box(reader)?;
        match (which_cmd, status) {
            (ASSIGN_MAILBOX, ASSIGNED_STATUS) => Ok(MailboxAnswer::Assigned { to, mailbox }),
            (ASSIGN_MAILBOX, FIRST_REFUSAL_STATUS..=LAST_REFUSAL_STATUS) => {
                Ok(MailboxAnswer::Refused { to, status })
            }
            _ => Err(MessageError::UnknownValue {
                field: "status",
                value: u16::from(status),
            }),
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::NewBox.byte()).byte(ASSIGN_MAILBOX);
        match self {
            MailboxAnswer::Assigned { to, mailbox } => {
                writer.word(to.0).byte(ASSIGNED_STATUS);
                write_box(writer, *mailbox);
            }
            MailboxAnswer::Refused { to, status } => {
                writer.word(to.0).byte(*status);
                write_box(writer, REFUSED_MAILBOX_ASSUMED);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mailbox_number_is_its_low_word_then_its_high_word() {
        let mut writer = WireWriter::default();
        write_box(&mut writer, MailboxNumber(0x0002_0001));
        assert_eq!(writer.into_bytes(), [1, 0, 2, 0]);
    }

    #[test]
    fn a_stored_letter_reads_back_what_was_stored() {
        let stored = StoredLetter {
            received: HostTime {
                year_1900: 94,
                month0: 1,
                mday: 2,
                hour: 3,
                minute: 4,
                second: 5,
            },
            envelope: Envelope {
                reserved: [0; FIELD_LEN],
                from: MailboxNumber(7),
                subject: *b"hello\0\0\0\0\0",
                flags: 0,
                addressee: *b"Bob\0\0\0\0\0\0\0",
                sender: *b"Alice\0\0\0\0\0",
                text: b"hi\0".to_vec(),
            },
        };
        assert_eq!(StoredLetter::from_body(&stored.to_body()), Ok(stored));
    }

    #[test]
    fn a_short_sysop_form_of_sub_20_is_not_an_envelope() {
        let bytes = [20, 0, 1, 1, 0, 0, 0, 0x30, 0x75];
        let mut reader = WireReader::new(&bytes);
        assert!(matches!(
            MailRequest::parse(&mut reader),
            Err(MessageError::Truncated { .. })
        ));
    }

    #[test]
    fn only_the_faults_the_host_uses_are_known() {
        assert_eq!(MailFault::try_from(6), Ok(MailFault::NoSuchBox));
        assert!(MailFault::try_from(3).is_err());
    }
}
