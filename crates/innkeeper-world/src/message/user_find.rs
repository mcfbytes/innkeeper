//! Command 49 (`UserFindMsg`): the phone book finder.
//!
//! Layouts are in docs/protocol/messages.md section 3.3. `FindPerson::handleMsg`
//! (`hub/script.126`) reads the reply from byte 4.

use crate::message::wire::{fitting_count, row_count, WireReader, WireWriter};
use crate::message::Command;
use crate::{AccountId, HostNumber, LandNumber, LandType, MessageError, Sid};

const BY_ACCOUNT: u8 = 1;
const BY_NAME: u8 = 2;

/// Command 49 from the client: who is online, by account or by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserFind {
    /// 49/1: `b 49, b 1, w 0, w accountLow, w accountHigh`.
    ByAccount(AccountId),
    /// 49/2: `b 49, b 2, w 0, w 0, s name`.
    ByName(String),
}

/// One player the reply names. The client matches the land triple to its directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundUser {
    pub host: HostNumber,
    pub land_type: LandType,
    pub land_number: LandNumber,
    /// The persona, which is what the client prints.
    pub name: String,
}

/// Command 49 from the host: `whichCmd` echoed, `toSID` 0, then the hits from byte 4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserFindReply {
    /// Sub 1 echoes the account words before the records. Records start at body offset 6.
    ByAccount {
        account: AccountId,
        found: Vec<FoundUser>,
    },
    /// Sub 2: records start right after the count, at body offset 2.
    ByName { found: Vec<FoundUser> },
}

impl UserFind {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let request = match reader.byte("sub-command")? {
            BY_ACCOUNT => {
                let _header = reader.word("toSID")?;
                let low = reader.word("accountLow")?;
                let high = reader.word("accountHigh")?;
                UserFind::ByAccount(AccountId::from_words(low, high))
            }
            BY_NAME => {
                let _header = reader.word("toSID")?;
                let _zero = reader.word("unused")?;
                UserFind::ByName(reader.text("name")?)
            }
            sub => {
                return Err(MessageError::UnsupportedSub {
                    command: Command::UserFind.byte(),
                    sub,
                })
            }
        };
        Ok(request)
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::UserFind.byte());
        match self {
            UserFind::ByAccount(account) => {
                let (low, high) = account.words();
                writer.byte(BY_ACCOUNT).word(0).word(low).word(high);
            }
            UserFind::ByName(name) => {
                writer.byte(BY_NAME).word(0).word(0).text(name);
            }
        }
    }
}

impl UserFindReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let which = reader.byte("whichCmd")?;
        let _to = reader.word("toSID")?;
        let count = reader.word("count")?;
        let reply = match which {
            BY_ACCOUNT => {
                let low = reader.word("accountLow")?;
                let high = reader.word("accountHigh")?;
                UserFindReply::ByAccount {
                    account: AccountId::from_words(low, high),
                    found: read_found(reader, count)?,
                }
            }
            BY_NAME => UserFindReply::ByName {
                found: read_found(reader, count)?,
            },
            sub => {
                return Err(MessageError::UnsupportedSub {
                    command: Command::UserFind.byte(),
                    sub,
                })
            }
        };
        Ok(reply)
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        let (which, account, found) = match self {
            UserFindReply::ByAccount { account, found } => (BY_ACCOUNT, Some(*account), found),
            UserFindReply::ByName { found } => (BY_NAME, None, found),
        };
        let found = fitting_count(found);
        writer.byte(Command::UserFind.byte()).byte(which);
        writer.word(Sid::GAME_OBJECT.0).word(row_count(found));
        if let Some(account) = account {
            let (low, high) = account.words();
            writer.word(low).word(high);
        }
        for user in found {
            writer
                .byte(user.host.0)
                .byte(user.land_type.0)
                .byte(user.land_number.0);
            writer.text(&user.name);
        }
    }
}

fn read_found(reader: &mut WireReader, count: u16) -> Result<Vec<FoundUser>, MessageError> {
    let mut found = Vec::new();
    for _ in 0..count {
        let [host, land_type, land_number] = reader.array("location")?;
        found.push(FoundUser {
            host: HostNumber(host),
            land_type: LandType(land_type),
            land_number: LandNumber(land_number),
            name: reader.text("name")?,
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMessage, HostMessage};

    /// 49/1 for account 100001 (`0x0001_86A1`).
    const FIND_ACCOUNT: &[u8] = &[0x31, 0x01, 0x00, 0x00, 0xA1, 0x86, 0x01, 0x00];
    /// 49/2 for "Guybrush".
    const FIND_NAME: &[u8] = &[
        0x31, 0x02, 0x00, 0x00, 0x00, 0x00, b'G', b'u', b'y', b'b', b'r', b'u', b's', b'h', 0,
    ];
    /// Sub 1, count 0, the account echoed.
    const ACCOUNT_MISS: &[u8] = &[0x31, 0x01, 0x00, 0x00, 0x00, 0x00, 0xA1, 0x86, 0x01, 0x00];
    /// Sub 1, Guybrush on host 7 in land 1/1.
    const ACCOUNT_HIT: &[u8] = &[
        0x31, 0x01, 0x00, 0x00, 0x01, 0x00, 0xA1, 0x86, 0x01, 0x00, 0x07, 0x01, 0x01, b'G', b'u',
        b'y', b'b', b'r', b'u', b's', b'h', 0,
    ];
    /// Sub 2, count 0.
    const NAME_MISS: &[u8] = &[0x31, 0x02, 0x00, 0x00, 0x00, 0x00];
    /// Sub 2: Guybrush in the Clubhouse, then Elaine on the map.
    const NAME_HITS: &[u8] = &[
        0x31, 0x02, 0x00, 0x00, 0x02, 0x00, 0x07, 0x01, 0x01, b'G', b'u', b'y', b'b', b'r', b'u',
        b's', b'h', 0, 0x07, 0x00, 0x00, b'E', b'l', b'a', b'i', b'n', b'e', 0,
    ];

    fn guybrush_home() -> FoundUser {
        FoundUser {
            host: HostNumber(7),
            land_type: LandType(1),
            land_number: LandNumber(1),
            name: "Guybrush".into(),
        }
    }

    fn elaine_on_the_map() -> FoundUser {
        FoundUser {
            host: HostNumber(7),
            land_type: LandType(0),
            land_number: LandNumber(0),
            name: "Elaine".into(),
        }
    }

    #[test]
    fn both_requests_and_replies_round_trip_byte_for_byte() {
        let requests = [
            (
                FIND_ACCOUNT,
                ClientMessage::UserFind(UserFind::ByAccount(AccountId(100_001))),
            ),
            (
                FIND_NAME,
                ClientMessage::UserFind(UserFind::ByName("Guybrush".into())),
            ),
        ];
        for (bytes, message) in requests {
            assert_eq!(ClientMessage::parse(bytes), Ok(message.clone()));
            assert_eq!(message.encode(), bytes);
        }

        let replies = [
            (
                ACCOUNT_MISS,
                HostMessage::UserFind(UserFindReply::ByAccount {
                    account: AccountId(100_001),
                    found: Vec::new(),
                }),
            ),
            (
                ACCOUNT_HIT,
                HostMessage::UserFind(UserFindReply::ByAccount {
                    account: AccountId(100_001),
                    found: vec![guybrush_home()],
                }),
            ),
            (
                NAME_MISS,
                HostMessage::UserFind(UserFindReply::ByName { found: Vec::new() }),
            ),
            (
                NAME_HITS,
                HostMessage::UserFind(UserFindReply::ByName {
                    found: vec![guybrush_home(), elaine_on_the_map()],
                }),
            ),
        ];
        for (bytes, message) in replies {
            assert_eq!(HostMessage::parse(bytes), Ok(message.clone()));
            assert_eq!(message.encode(), bytes);
        }
    }

    #[test]
    fn an_unknown_sub_or_a_short_name_is_rejected() {
        assert_eq!(
            ClientMessage::parse(&[0x31, 9]),
            Err(MessageError::UnsupportedSub {
                command: 49,
                sub: 9
            })
        );
        assert_eq!(
            HostMessage::parse(&[0x31, 9, 0, 0, 0, 0]),
            Err(MessageError::UnsupportedSub {
                command: 49,
                sub: 9
            })
        );
        assert_eq!(
            ClientMessage::parse(&[0x31, 2, 0, 0, 0, 0, b'G']),
            Err(MessageError::Truncated { field: "name" })
        );
        let mut extra = FIND_ACCOUNT.to_vec();
        extra.push(0);
        assert_eq!(
            ClientMessage::parse(&extra),
            Err(MessageError::TrailingBytes(1))
        );
    }
}
