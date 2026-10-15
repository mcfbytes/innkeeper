use crate::message::wire::{WireReader, WireWriter};
use crate::message::{ClientMessage, Command};
use crate::{MessageError, Money, Sid};

const SAVE_MONEY: u8 = 2;
const READ_MONEY: u8 = 3;
const SET_NAME: u8 = 4;

/// The two words between the object SID and a money amount: the message type and the variant, which
/// the client always sends as 1 and reads from bytes 4 and 6 of the reply (`class_7f::init`).
const MONEY_TYPE: u16 = 1;
const MONEY_VARIANT: u16 = 1;

/// Command 40: the sub-command picks the request, as in `messages.md` section 3.3.
pub(crate) fn parse(reader: &mut WireReader) -> Result<ClientMessage, MessageError> {
    match reader.byte("sub-command")? {
        SAVE_MONEY => Ok(ClientMessage::SaveMoney(SaveMoney::parse(reader)?)),
        READ_MONEY => Ok(ClientMessage::ReadMoney(ReadMoney::parse(reader)?)),
        SET_NAME => Ok(ClientMessage::SetPersona(SetPersona {
            name: reader.text("name")?,
        })),
        sub => Err(MessageError::UnsupportedSub {
            command: Command::UserInfo.byte(),
            sub,
        }),
    }
}

/// Command 40 sub 4 (`UserInfoMsg`): the persona name the player just chose, after logon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetPersona {
    pub name: String,
}

impl SetPersona {
    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::UserInfo.byte()).byte(SET_NAME);
        writer.text(&self.name);
    }
}

/// Command 40 sub 3: read the account's CasinoLand bankroll. `sid` is the object the reply names,
/// the requester's game object (`LL/script.000 ReadMoneyFile::doit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadMoney {
    pub sid: Sid,
}

impl ReadMoney {
    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        Ok(ReadMoney {
            sid: read_money_header(reader)?,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::UserInfo.byte()).byte(READ_MONEY);
        write_money_header(writer, self.sid);
    }
}

/// Command 40 sub 2: the client writes the account's CasinoLand bankroll (`MoneyLongInt::doit`).
/// The client then waits for an Ack or a Nak that names command 40 (`proc_14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveMoney {
    pub sid: Sid,
    pub amount: Money,
}

impl SaveMoney {
    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let sid = read_money_header(reader)?;
        let (low, high) = (reader.word("moneyLow")?, reader.word("moneyHigh")?);
        Ok(SaveMoney {
            sid,
            amount: Money::from_words(low, high),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::UserInfo.byte()).byte(SAVE_MONEY);
        write_money_header(writer, self.sid);
        write_amount(writer, self.amount);
    }
}

/// The host's answer to a read (command 40 sub 3): the bankroll. The bytes are the shape of a save,
/// with the object SID first; the client keeps its own copy when no reply arrives (`LL::handleMsg`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoneyReply {
    pub to: Sid,
    pub amount: Money,
}

impl MoneyReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        // Byte 1 is the `whichCmd` the reply answers; only a read carries an amount.
        match reader.byte("whichCmd")? {
            READ_MONEY => {}
            sub => {
                return Err(MessageError::UnsupportedSub {
                    command: Command::UserInfo.byte(),
                    sub,
                })
            }
        }
        let to = read_money_header(reader)?;
        let (low, high) = (reader.word("moneyLow")?, reader.word("moneyHigh")?);
        Ok(MoneyReply {
            to,
            amount: Money::from_words(low, high),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::UserInfo.byte()).byte(READ_MONEY);
        write_money_header(writer, self.to);
        write_amount(writer, self.amount);
    }
}

/// The object SID, then the type and variant the client sends as 1 and reads back from the reply.
fn read_money_header(reader: &mut WireReader) -> Result<Sid, MessageError> {
    let sid = Sid(reader.word("sid")?);
    for (expected, field) in [(MONEY_TYPE, "msgType"), (MONEY_VARIANT, "whichSub")] {
        let value = reader.word(field)?;
        if value != expected {
            return Err(MessageError::UnknownValue { field, value });
        }
    }
    Ok(sid)
}

fn write_money_header(writer: &mut WireWriter, sid: Sid) {
    writer.word(sid.0).word(MONEY_TYPE).word(MONEY_VARIANT);
}

fn write_amount(writer: &mut WireWriter, amount: Money) {
    let (low, high) = amount.words();
    writer.word(low).word(high);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientMessage;

    /// The read request as it was seen live: SID 0x0105, type 1, variant 1.
    const READ: [u8; 8] = [40, 3, 5, 1, 1, 0, 1, 0];
    /// The save request as it was seen live, carrying 247.
    const SAVE: [u8; 12] = [40, 2, 5, 1, 1, 0, 1, 0, 0xF7, 0, 0, 0];

    #[test]
    fn the_money_requests_decode_and_round_trip() {
        let read = ClientMessage::ReadMoney(ReadMoney { sid: Sid(0x0105) });
        let save = ClientMessage::SaveMoney(SaveMoney {
            sid: Sid(0x0105),
            amount: Money(247),
        });
        for (bytes, message) in [(&READ[..], read), (&SAVE[..], save)] {
            assert_eq!(ClientMessage::parse(bytes), Ok(message.clone()));
            assert_eq!(message.encode(), bytes);
        }
    }

    #[test]
    fn a_money_request_whose_type_words_are_not_1_is_rejected() {
        for (offset, field) in [(4, "msgType"), (6, "whichSub")] {
            let mut bytes = READ;
            bytes[offset] = 2;
            assert_eq!(
                ClientMessage::parse(&bytes),
                Err(MessageError::UnknownValue { field, value: 2 })
            );
        }
    }

    #[test]
    fn the_persona_name_decodes_and_the_other_subs_do_not() {
        assert_eq!(
            ClientMessage::parse(&[40, 9]),
            Err(MessageError::UnsupportedSub {
                command: 40,
                sub: 9
            })
        );
        assert_eq!(
            ClientMessage::parse(&[40, 4, b'x']),
            Err(MessageError::Truncated { field: "name" })
        );
    }
}
