//! Command 34 (`AccountMsg`): the Town Hall billing screens. Layouts are in docs/protocol/messages.md
//! section 3.3 and docs/server/services.md; the senders are `GetInfo` and `GetDetailedInfo`
//! (`hub/script.050`), and the logon handler `RoomZeroHandler::handleMsg` (`hub/script.011`).

use crate::assumptions::SUMMARY_UNREAD_BYTES_ASSUMED;
use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{AccountId, MessageError, Sid};

const RATES: u8 = 4;
const STATUS: u8 = 5;
const RATES_SEEN: u8 = 8;
const USAGE: u8 = 12;
const BILLING: u8 = 15;

/// Which long text a summary request wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SummaryKind {
    /// "Usage Summary".
    Usage = USAGE,
    /// "Billing Summary".
    Billing = BILLING,
}

impl SummaryKind {
    /// The sub-command, which the reply and a Nak echo.
    pub const fn sub(self) -> u8 {
        self as u8
    }

    fn from_sub(sub: u8) -> Option<Self> {
        match sub {
            USAGE => Some(SummaryKind::Usage),
            BILLING => Some(SummaryKind::Billing),
            _ => None,
        }
    }
}

/// Command 34 from the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountRequest {
    /// 34/4 `"bbw"`: the current rates. Sent at logon and by "Check My Current Rates".
    Rates { sid: Sid },
    /// 34/5 `"bbw"`: "Check My Account Status". The sid word is 0.
    Status { sid: Sid },
    /// 34/8 `"bbw"`: the client shows the rates text it was given and says so.
    RatesSeen { sid: Sid },
    /// 34/12 or 34/15 `"bbwwwbw"`: "Usage Summary" or "Billing Summary".
    Summary {
        kind: SummaryKind,
        sid: Sid,
        account: AccountId,
        /// The byte before the last word; the client sends 0.
        flag: u8,
        /// The last word; the client sends 100.
        limit: u16,
    },
}

impl AccountRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let sub = reader.byte("sub-command")?;
        let sid = Sid(reader.word("sid")?);
        match sub {
            RATES => Ok(AccountRequest::Rates { sid }),
            STATUS => Ok(AccountRequest::Status { sid }),
            RATES_SEEN => Ok(AccountRequest::RatesSeen { sid }),
            sub => {
                let kind = SummaryKind::from_sub(sub).ok_or(MessageError::UnsupportedSub {
                    command: Command::Account.byte(),
                    sub,
                })?;
                let low = reader.word("accountLow")?;
                let high = reader.word("accountHigh")?;
                Ok(AccountRequest::Summary {
                    kind,
                    sid,
                    account: AccountId::from_words(low, high),
                    flag: reader.byte("flag")?,
                    limit: reader.word("limit")?,
                })
            }
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Account.byte());
        match *self {
            AccountRequest::Rates { sid } => {
                writer.byte(RATES).word(sid.0);
            }
            AccountRequest::Status { sid } => {
                writer.byte(STATUS).word(sid.0);
            }
            AccountRequest::RatesSeen { sid } => {
                writer.byte(RATES_SEEN).word(sid.0);
            }
            AccountRequest::Summary {
                kind,
                sid,
                account,
                flag,
                limit,
            } => {
                let (low, high) = account.words();
                writer.byte(kind.sub()).word(sid.0).word(low).word(high);
                writer.byte(flag).word(limit);
            }
        }
    }

    /// The sub-command byte, for logs.
    pub fn sub(&self) -> u8 {
        match self {
            AccountRequest::Rates { .. } => RATES,
            AccountRequest::Status { .. } => STATUS,
            AccountRequest::RatesSeen { .. } => RATES_SEEN,
            AccountRequest::Summary { kind, .. } => kind.sub(),
        }
    }
}

/// Command 34 from the host: `b 34, b whichCmd, w toSID`, then the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountReply {
    /// Sub 5: a string at byte 4, shown in the "Account Status" box.
    Status { to: Sid, text: String },
    /// Sub 4: a string at byte 4. The client shows it in the "Current Rates" box when it differs
    /// from the copy it keeps.
    Rates { to: Sid, text: String },
    /// Sub 12 or 15: twelve unread bytes, `w length` at byte 16 and that many text bytes from 18,
    /// which the client shows as the "Account Summary" text.
    Summary {
        kind: SummaryKind,
        to: Sid,
        text: String,
    },
}

impl AccountReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let which = reader.byte("whichCmd")?;
        let to = Sid(reader.word("toSID")?);
        match which {
            STATUS => Ok(AccountReply::Status {
                to,
                text: reader.text("text")?,
            }),
            RATES => Ok(AccountReply::Rates {
                to,
                text: reader.text("text")?,
            }),
            which => {
                let kind = SummaryKind::from_sub(which).ok_or(MessageError::UnsupportedSub {
                    command: Command::Account.byte(),
                    sub: which,
                })?;
                let _unread: [u8; 12] = reader.array("unread")?;
                let length = usize::from(reader.word("length")?);
                let bytes = reader.rest();
                let text = bytes
                    .get(..length)
                    .ok_or(MessageError::Truncated { field: "summary" })?;
                match bytes.len() - length {
                    0 => {}
                    extra => return Err(MessageError::TrailingBytes(extra)),
                }
                if !text.is_ascii() {
                    return Err(MessageError::NotAscii { field: "summary" });
                }
                Ok(AccountReply::Summary {
                    kind,
                    to,
                    text: text.iter().copied().map(char::from).collect(),
                })
            }
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Account.byte());
        match self {
            AccountReply::Status { to, text } => {
                writer.byte(STATUS).word(to.0).text(text);
            }
            AccountReply::Rates { to, text } => {
                writer.byte(RATES).word(to.0).text(text);
            }
            AccountReply::Summary { kind, to, text } => {
                let bytes = text.as_bytes();
                let text = bytes.get(..usize::from(u16::MAX)).unwrap_or(bytes);
                writer.byte(kind.sub()).word(to.0);
                writer.array(&SUMMARY_UNREAD_BYTES_ASSUMED);
                writer.word(u16::try_from(text.len()).unwrap_or(u16::MAX));
                writer.array(text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMessage, HostMessage};

    /// 34/4 from the game object (sid `0x0100`).
    const RATES_REQUEST: &[u8] = &[0x22, 0x04, 0x00, 0x01];
    /// 34/5 with the sid word 0.
    const STATUS_REQUEST: &[u8] = &[0x22, 0x05, 0x00, 0x00];
    /// 34/8, the client's note that it has shown the rates.
    const RATES_SEEN_REQUEST: &[u8] = &[0x22, 0x08, 0x00, 0x01];
    /// 34/12 for account 100001 (`0x0001_86A1`): low word, high word, flag 0, limit 100.
    const USAGE_REQUEST: &[u8] = &[
        0x22, 0x0C, 0x00, 0x01, 0xA1, 0x86, 0x01, 0x00, 0x00, 0x64, 0x00,
    ];
    /// 34/15 for the same account.
    const BILLING_REQUEST: &[u8] = &[
        0x22, 0x0F, 0x00, 0x01, 0xA1, 0x86, 0x01, 0x00, 0x00, 0x64, 0x00,
    ];

    #[test]
    fn every_request_round_trips_byte_for_byte() {
        let sid = Sid(0x0100);
        let account = AccountId(100_001);
        let requests = [
            (RATES_REQUEST, AccountRequest::Rates { sid }),
            (STATUS_REQUEST, AccountRequest::Status { sid: Sid(0) }),
            (RATES_SEEN_REQUEST, AccountRequest::RatesSeen { sid }),
            (
                USAGE_REQUEST,
                AccountRequest::Summary {
                    kind: SummaryKind::Usage,
                    sid,
                    account,
                    flag: 0,
                    limit: 100,
                },
            ),
            (
                BILLING_REQUEST,
                AccountRequest::Summary {
                    kind: SummaryKind::Billing,
                    sid,
                    account,
                    flag: 0,
                    limit: 100,
                },
            ),
        ];
        for (bytes, request) in requests {
            let message = ClientMessage::Account(request);
            assert_eq!(ClientMessage::parse(bytes), Ok(message.clone()));
            assert_eq!(message.encode(), bytes);
            assert_eq!(request.sub(), bytes[1]);
        }
    }

    #[test]
    fn replies_round_trip_byte_for_byte() {
        let replies: [(&[u8], AccountReply); 3] = [
            (
                &[0x22, 0x05, 0x00, 0x00, b'O', b'K', 0],
                AccountReply::Status {
                    to: Sid(0),
                    text: "OK".into(),
                },
            ),
            (
                &[0x22, 0x04, 0x00, 0x01, b'F', b'r', b'e', b'e', 0],
                AccountReply::Rates {
                    to: Sid(0x0100),
                    text: "Free".into(),
                },
            ),
            (
                &[
                    0x22, 0x0C, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x05, 0x00, b'N',
                    b'o', b'n', b'e', b'\n',
                ],
                AccountReply::Summary {
                    kind: SummaryKind::Usage,
                    to: Sid(0x0100),
                    text: "None\n".into(),
                },
            ),
        ];
        for (bytes, reply) in replies {
            let message = HostMessage::Account(reply);
            assert_eq!(HostMessage::parse(bytes), Ok(message.clone()));
            assert_eq!(message.encode(), bytes);
        }
    }

    #[test]
    fn a_summary_whose_length_disagrees_with_its_text_is_rejected() {
        let mut short = vec![0x22, 0x0F, 0x00, 0x01];
        short.extend([0; 12]);
        short.extend([0x09, 0x00, b'x']);
        assert_eq!(
            HostMessage::parse(&short),
            Err(MessageError::Truncated { field: "summary" })
        );
        let mut long = vec![0x22, 0x0F, 0x00, 0x01];
        long.extend([0; 12]);
        long.extend([0x01, 0x00, b'x', b'y']);
        assert_eq!(
            HostMessage::parse(&long),
            Err(MessageError::TrailingBytes(1))
        );
    }

    #[test]
    fn an_unknown_sub_or_a_short_request_is_rejected() {
        assert_eq!(
            ClientMessage::parse(&[0x22, 9, 0, 1]),
            Err(MessageError::UnsupportedSub {
                command: 34,
                sub: 9
            })
        );
        assert_eq!(
            ClientMessage::parse(&[0x22, 12, 0, 1, 0xA1]),
            Err(MessageError::Truncated {
                field: "accountLow"
            })
        );
        assert_eq!(
            ClientMessage::parse(&[0x22, 4, 0]),
            Err(MessageError::Truncated { field: "sid" })
        );
    }
}
