//! NTN Trivia on command 54 when the server word is 4. Shoppers Advantage uses 5.
//! Layouts follow vocab.729 (`proc_237`) and `PMSData` in `script.012`.

use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::MessageError;

/// The server word NTN writes before JOIN (`PMSData::init` sets `server` to 4).
pub const NTN_SERVER: u16 = 4;

const TEXT: u8 = 24;
const JOIN: u8 = 25;
const EXIT_SUB: u8 = 26;

const GAME: u8 = 16;
const JOINED: u8 = 17;
const JOIN_FAILED: u8 = 18;
const FORCED_EXIT: u8 = 20;
const SEND_FAILED: u8 = 24;
const JOIN_FAILED_OTHER: u8 = 25;
const LEAVE_FAILED: u8 = 26;

/// Inner command at absolute 13: one game message (`proc_237`).
const INNER_GAME: u8 = 23;
/// Inner command 42: the client leaves with "NTN Trivia has gone down".
const INNER_DOWN: u8 = 42;
/// The word a failure frame uses for that same dialog.
const GONE_DOWN: u16 = 42;

const LINE: usize = 24;
/// Type 14 steps records by 24 (`ldi 24` / `mul` at vocab.729 `0x0e52`).
const WINNER_STRIDE: usize = 24;
const PAD: usize = 7;

const TYPE_VERSION: u8 = 0;
const TYPE_LOGON: u8 = 1;
const TYPE_TEMPLATE: u8 = 2;
const TYPE_SETUP: u8 = 3;
const TYPE_QUESTION: u8 = 4;
const TYPE_ANSWERS: u8 = 5;
const TYPE_REGISTERED: u8 = 7;
const TYPE_CORRECT: u8 = 8;
const TYPE_SCORE: u8 = 10;
const TYPE_WINNERS: u8 = 14;
const TYPE_MINUTES: u8 = 22;
const TYPE_TICK: u8 = 23;
const TYPE_OVER: u8 = 24;
const TYPE_HANDLE: u8 = 39;

/// The five routing words every command-54 frame carries after the sub-command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriviaHeader {
    pub server: u16,
    pub number: u16,
    pub host: u16,
    pub remote: u16,
    /// NTNData's SID from the client; `toSID` on a host frame.
    pub sid: u16,
}

/// Fixed bytes of a sub-24 line: `b 0, b 3, w number, b 2`, then the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextFraming {
    pub zero: u8,
    pub three: u8,
    pub number: u16,
    pub two: u8,
}

/// Command 54 from the client when `server` is [`NTN_SERVER`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriviaRequest {
    /// Sub 24: a version line, a handle, or an answer (`"1"`..`"5"` plus CR LF).
    Text {
        header: TriviaHeader,
        framing: TextFraming,
        text: Vec<u8>,
    },
    /// Sub 25: JOIN. The extra words are the mailbox id; the host ignores them.
    Join {
        header: TriviaHeader,
        extra: Vec<u16>,
    },
    /// Sub 26: EXIT, the header alone.
    Exit(TriviaHeader),
}

/// whichCmd 18, 25 or 26: the word at byte 12, and 42 means the show is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CheckedFailure {
    Join = JOIN_FAILED,
    Other = JOIN_FAILED_OTHER,
    Leave = LEAVE_FAILED,
}

/// One NTN game message. Payload offsets are from absolute byte 22.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NtnMessage {
    /// Type 0. No fields. The second one opens the handle dialog.
    Version,
    /// Type 1. No fields. Sets the logon flag.
    Logon,
    /// Type 2. Help byte 0 is what lets question types through.
    ScoreTemplate {
        mode: u8,
        cnf: u8,
        help: u8,
        game_id: u16,
        scope: u8,
    },
    /// Type 3. `seconds` is the clock; the client stores `seconds * 60` ticks.
    TickdownSetup {
        kind: u8,
        seconds: u16,
        points: u16,
        clues: u8,
    },
    /// Type 4. Four lines of 24 bytes, space padded.
    Question {
        number: u8,
        total: u8,
        lines: [[u8; LINE]; 4],
    },
    /// Type 5. `count` lines of 24. The client prints them 1-based.
    Answers { lines: Vec<[u8; LINE]> },
    /// Type 7. `answer` is 1-based (the client subtracts 1).
    AnswerRegistered {
        kind: u8,
        answer: u8,
        points: u16,
        points_again: u16,
    },
    /// Type 8. `answer` is 1-based.
    AnswerCorrect { answer: u8 },
    /// Type 10. Eight words: score, round score, round rank, total, total rank.
    Score(ScoreWords),
    /// Type 14. Up to the records that fit; the show sends the top three.
    RoundWinners { winners: Vec<RoundWinner> },
    /// Type 22. The client builds the "stay with us" line from this minute count.
    MinutesToNext { minutes: u8 },
    /// Type 23. Starts the clock from the type-3 values.
    StartTickdown { iteration: u8 },
    /// Type 24. Clears the round clock. The byte is unread beyond being present.
    GameOver { flag: u8 },
    /// Type 39. The whole payload becomes the handle.
    HandleRegistered { handle: Vec<u8> },
}

/// Type 10 as vocab.729 reads it. Each score is `(high, low)` into `class_9::set`.
/// `score` is this answer, `round` the round total, `total` the game total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoreWords {
    pub score_low: u16,
    pub score_high: u16,
    pub round_low: u16,
    pub round_high: u16,
    pub round_rank: u16,
    pub total_low: u16,
    pub total_high: u16,
    pub total_rank: u16,
}

/// One type-14 record, stride 24. The 10-byte field is the round column, then the
/// score words, then the 6-byte name. The last four bytes are padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoundWinner {
    pub label: [u8; 10],
    pub score_low: u16,
    pub score_high: u16,
    pub name: [u8; 6],
    pub pad: [u8; 4],
}

/// Command 54 from the host when `server` is [`NTN_SERVER`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriviaReply {
    /// whichCmd 16, inner 23. `pad` is absolute bytes 14..20; the client ignores it.
    Game {
        header: TriviaHeader,
        ignored: u8,
        pad: [u8; PAD],
        message: NtnMessage,
    },
    /// whichCmd 16, inner 42.
    ShowDown { header: TriviaHeader, ignored: u8 },
    /// whichCmd 17. The client stores the four routing words and sends the version line.
    Joined(TriviaHeader),
    /// whichCmd 20. Byte 12 is the `%u`. The next word, when present, is the 42 check.
    ForcedExit {
        header: TriviaHeader,
        reason: u8,
        follow: Option<u16>,
    },
    /// whichCmd 18, 25 or 26. The word at byte 12; 42 is the gone-down dialog.
    Checked {
        which: CheckedFailure,
        header: TriviaHeader,
        word: u16,
    },
    /// whichCmd 24. The client does not look for word 42.
    SendFailed { header: TriviaHeader, reason: u8 },
}

impl TriviaRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let sub = reader.byte("sub-command")?;
        let header = read_header(reader)?;
        let request = match sub {
            TEXT => TriviaRequest::Text {
                header,
                framing: read_framing(reader)?,
                text: reader.rest(),
            },
            JOIN => TriviaRequest::Join {
                header,
                extra: read_words(reader)?,
            },
            EXIT_SUB => TriviaRequest::Exit(header),
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
            TriviaRequest::Text {
                header,
                framing,
                text,
            } => {
                write_header(writer, TEXT, header);
                write_framing(writer, framing);
                writer.array(text);
            }
            TriviaRequest::Join { header, extra } => {
                write_header(writer, JOIN, header);
                extra.iter().for_each(|word| {
                    writer.word(*word);
                });
            }
            TriviaRequest::Exit(header) => write_header(writer, EXIT_SUB, header),
        }
    }
}

impl TriviaReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let which = reader.byte("whichCmd")?;
        let header = read_header(reader)?;
        let reply = match which {
            GAME => parse_game(reader, header)?,
            JOINED => TriviaReply::Joined(header),
            FORCED_EXIT => parse_forced(reader, header)?,
            JOIN_FAILED => checked(CheckedFailure::Join, header, reader)?,
            JOIN_FAILED_OTHER => checked(CheckedFailure::Other, header, reader)?,
            LEAVE_FAILED => checked(CheckedFailure::Leave, header, reader)?,
            SEND_FAILED => TriviaReply::SendFailed {
                header,
                reason: reader.byte("reason")?,
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
            TriviaReply::Game {
                header,
                ignored,
                pad,
                message,
            } => {
                write_game_head(writer, header, *ignored, INNER_GAME, pad);
                message.write(writer);
            }
            TriviaReply::ShowDown { header, ignored } => {
                write_header(writer, GAME, header);
                writer.byte(*ignored).byte(INNER_DOWN);
            }
            TriviaReply::Joined(header) => write_header(writer, JOINED, header),
            TriviaReply::ForcedExit {
                header,
                reason,
                follow,
            } => {
                write_header(writer, FORCED_EXIT, header);
                writer.byte(*reason);
                if let Some(word) = follow {
                    writer.word(*word);
                }
            }
            TriviaReply::Checked {
                which,
                header,
                word,
            } => {
                write_header(writer, which.byte(), header);
                writer.word(*word);
            }
            TriviaReply::SendFailed { header, reason } => {
                write_header(writer, SEND_FAILED, header);
                writer.byte(*reason);
            }
        }
    }
}

impl NtnMessage {
    fn write(&self, writer: &mut WireWriter) {
        match self {
            NtnMessage::Version => writer.byte(TYPE_VERSION),
            NtnMessage::Logon => writer.byte(TYPE_LOGON),
            NtnMessage::ScoreTemplate {
                mode,
                cnf,
                help,
                game_id,
                scope,
            } => writer
                .byte(TYPE_TEMPLATE)
                .byte(*mode)
                .byte(*cnf)
                .byte(*help)
                .word(*game_id)
                .byte(*scope),
            NtnMessage::TickdownSetup {
                kind,
                seconds,
                points,
                clues,
            } => writer
                .byte(TYPE_SETUP)
                .byte(*kind)
                .word(*seconds)
                .word(*points)
                .byte(*clues),
            NtnMessage::Question {
                number,
                total,
                lines,
            } => {
                writer.byte(TYPE_QUESTION).byte(*number).byte(*total);
                lines.iter().for_each(|line| {
                    writer.array(line);
                });
                writer
            }
            NtnMessage::Answers { lines } => {
                let count = u8::try_from(lines.len()).unwrap_or(u8::MAX);
                writer.byte(TYPE_ANSWERS).byte(count);
                lines.iter().for_each(|line| {
                    writer.array(line);
                });
                writer
            }
            NtnMessage::AnswerRegistered {
                kind,
                answer,
                points,
                points_again,
            } => writer
                .byte(TYPE_REGISTERED)
                .byte(*kind)
                .byte(*answer)
                .word(*points)
                .word(*points_again),
            NtnMessage::AnswerCorrect { answer } => writer.byte(TYPE_CORRECT).byte(*answer),
            NtnMessage::Score(words) => {
                writer.byte(TYPE_SCORE);
                words.write(writer);
                writer
            }
            NtnMessage::RoundWinners { winners } => {
                let count = u8::try_from(winners.len()).unwrap_or(u8::MAX);
                writer.byte(TYPE_WINNERS).byte(count);
                winners.iter().for_each(|winner| winner.write(writer));
                writer
            }
            NtnMessage::MinutesToNext { minutes } => writer.byte(TYPE_MINUTES).byte(*minutes),
            NtnMessage::StartTickdown { iteration } => writer.byte(TYPE_TICK).byte(*iteration),
            NtnMessage::GameOver { flag } => writer.byte(TYPE_OVER).byte(*flag),
            NtnMessage::HandleRegistered { handle } => {
                writer.byte(TYPE_HANDLE).array(handle);
                writer
            }
        };
    }
}

impl ScoreWords {
    fn write(&self, writer: &mut WireWriter) {
        writer.word(self.score_low).word(self.score_high);
        writer.word(self.round_low).word(self.round_high);
        writer.word(self.round_rank);
        writer.word(self.total_low).word(self.total_high);
        writer.word(self.total_rank);
    }

    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        Ok(ScoreWords {
            score_low: reader.word("scoreLow")?,
            score_high: reader.word("scoreHigh")?,
            round_low: reader.word("roundLow")?,
            round_high: reader.word("roundHigh")?,
            round_rank: reader.word("roundRank")?,
            total_low: reader.word("totalLow")?,
            total_high: reader.word("totalHigh")?,
            total_rank: reader.word("totalRank")?,
        })
    }
}

impl RoundWinner {
    fn write(&self, writer: &mut WireWriter) {
        writer.array(&self.label);
        writer.word(self.score_low).word(self.score_high);
        writer.array(&self.name).array(&self.pad);
    }

    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        Ok(RoundWinner {
            label: reader.array("label")?,
            score_low: reader.word("scoreLow")?,
            score_high: reader.word("scoreHigh")?,
            name: reader.array("name")?,
            pad: reader.array("pad")?,
        })
    }
}

fn parse_game(reader: &mut WireReader, header: TriviaHeader) -> Result<TriviaReply, MessageError> {
    let ignored = reader.byte("ignored")?;
    let inner = reader.byte("inner")?;
    match inner {
        INNER_DOWN => Ok(TriviaReply::ShowDown { header, ignored }),
        INNER_GAME => {
            let pad = reader.array("pad")?;
            let message = parse_message(reader)?;
            Ok(TriviaReply::Game {
                header,
                ignored,
                pad,
                message,
            })
        }
        other => Err(MessageError::UnknownValue {
            field: "inner",
            value: u16::from(other),
        }),
    }
}

fn parse_message(reader: &mut WireReader) -> Result<NtnMessage, MessageError> {
    let kind = reader.byte("type")?;
    let message = match kind {
        TYPE_VERSION => NtnMessage::Version,
        TYPE_LOGON => NtnMessage::Logon,
        TYPE_TEMPLATE => NtnMessage::ScoreTemplate {
            mode: reader.byte("mode")?,
            cnf: reader.byte("cnf")?,
            help: reader.byte("help")?,
            game_id: reader.word("gameID")?,
            scope: reader.byte("scope")?,
        },
        TYPE_SETUP => NtnMessage::TickdownSetup {
            kind: reader.byte("kind")?,
            seconds: reader.word("secs")?,
            points: reader.word("points")?,
            clues: reader.byte("clueCount")?,
        },
        TYPE_QUESTION => NtnMessage::Question {
            number: reader.byte("number")?,
            total: reader.byte("total")?,
            lines: read_four(reader)?,
        },
        TYPE_ANSWERS => NtnMessage::Answers {
            lines: read_counted_lines(reader)?,
        },
        TYPE_REGISTERED => NtnMessage::AnswerRegistered {
            kind: reader.byte("kind")?,
            answer: reader.byte("answer")?,
            points: reader.word("points")?,
            points_again: reader.word("points2")?,
        },
        TYPE_CORRECT => NtnMessage::AnswerCorrect {
            answer: reader.byte("answer")?,
        },
        TYPE_SCORE => NtnMessage::Score(ScoreWords::parse(reader)?),
        TYPE_WINNERS => NtnMessage::RoundWinners {
            winners: read_winners(reader)?,
        },
        TYPE_MINUTES => NtnMessage::MinutesToNext {
            minutes: reader.byte("minutes")?,
        },
        TYPE_TICK => NtnMessage::StartTickdown {
            iteration: reader.byte("iteration")?,
        },
        TYPE_OVER => NtnMessage::GameOver {
            flag: reader.byte("flag")?,
        },
        TYPE_HANDLE => NtnMessage::HandleRegistered {
            handle: reader.rest(),
        },
        other => {
            return Err(MessageError::UnknownValue {
                field: "ntn type",
                value: u16::from(other),
            })
        }
    };
    Ok(message)
}

fn parse_forced(
    reader: &mut WireReader,
    header: TriviaHeader,
) -> Result<TriviaReply, MessageError> {
    let reason = reader.byte("reason")?;
    let follow = match reader.is_empty() {
        true => None,
        false => Some(reader.word("follow")?),
    };
    Ok(TriviaReply::ForcedExit {
        header,
        reason,
        follow,
    })
}

impl CheckedFailure {
    fn byte(self) -> u8 {
        self as u8
    }
}

fn checked(
    which: CheckedFailure,
    header: TriviaHeader,
    reader: &mut WireReader,
) -> Result<TriviaReply, MessageError> {
    Ok(TriviaReply::Checked {
        which,
        header,
        word: reader.word("word")?,
    })
}

fn read_header(reader: &mut WireReader) -> Result<TriviaHeader, MessageError> {
    Ok(TriviaHeader {
        server: reader.word("server")?,
        number: reader.word("number")?,
        host: reader.word("host")?,
        remote: reader.word("remote")?,
        sid: reader.word("sid")?,
    })
}

fn write_header(writer: &mut WireWriter, sub: u8, header: &TriviaHeader) {
    writer.byte(Command::Mall.byte()).byte(sub);
    writer.word(header.server).word(header.number);
    writer
        .word(header.host)
        .word(header.remote)
        .word(header.sid);
}

fn write_game_head(
    writer: &mut WireWriter,
    header: &TriviaHeader,
    ignored: u8,
    inner: u8,
    pad: &[u8; PAD],
) {
    write_header(writer, GAME, header);
    writer.byte(ignored).byte(inner).array(pad);
}

fn read_framing(reader: &mut WireReader) -> Result<TextFraming, MessageError> {
    Ok(TextFraming {
        zero: reader.byte("zero")?,
        three: reader.byte("three")?,
        number: reader.word("number")?,
        two: reader.byte("two")?,
    })
}

fn write_framing(writer: &mut WireWriter, framing: &TextFraming) {
    writer.byte(framing.zero).byte(framing.three);
    writer.word(framing.number).byte(framing.two);
}

fn read_words(reader: &mut WireReader) -> Result<Vec<u16>, MessageError> {
    let mut words = Vec::new();
    while !reader.is_empty() {
        words.push(reader.word("extra")?);
    }
    Ok(words)
}

fn read_four(reader: &mut WireReader) -> Result<[[u8; LINE]; 4], MessageError> {
    Ok([
        reader.array("line")?,
        reader.array("line")?,
        reader.array("line")?,
        reader.array("line")?,
    ])
}

fn read_counted_lines(reader: &mut WireReader) -> Result<Vec<[u8; LINE]>, MessageError> {
    let count = usize::from(reader.byte("count")?);
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        lines.push(reader.array("line")?);
    }
    Ok(lines)
}

fn read_winners(reader: &mut WireReader) -> Result<Vec<RoundWinner>, MessageError> {
    let count = usize::from(reader.byte("count")?);
    let mut winners = Vec::with_capacity(count);
    for _ in 0..count {
        winners.push(RoundWinner::parse(reader)?);
    }
    Ok(winners)
}

/// A type-14 record is this many payload bytes. The fields above add up to it.
const _: [u8; WINNER_STRIDE] = [0; 10 + 2 + 2 + 6 + 4];
/// Word 42 at the failure check is "NTN Trivia has gone down".
const _: u16 = GONE_DOWN;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MallHeader, MallReply, MallRequest};
    use crate::{ClientMessage, HostMessage};

    fn header() -> TriviaHeader {
        TriviaHeader {
            server: NTN_SERVER,
            number: 7,
            host: 1,
            remote: 2,
            sid: 0x1234,
        }
    }

    fn join_header() -> TriviaHeader {
        TriviaHeader {
            server: NTN_SERVER,
            number: 0,
            host: 0,
            remote: 0,
            sid: 0x0102,
        }
    }

    fn assert_request(request: TriviaRequest, bytes: &[u8]) {
        let encoded = ClientMessage::Trivia(request.clone()).encode();
        assert_eq!(encoded, bytes);
        assert_eq!(
            ClientMessage::parse(bytes),
            Ok(ClientMessage::Trivia(request))
        );
    }

    fn assert_reply(reply: TriviaReply, bytes: &[u8]) {
        let encoded = HostMessage::Trivia(reply.clone()).encode();
        assert_eq!(encoded, bytes);
        assert_eq!(HostMessage::parse(bytes), Ok(HostMessage::Trivia(reply)));
    }

    fn line(text: &[u8]) -> [u8; LINE] {
        let mut out = [b' '; LINE];
        let n = text.len().min(LINE);
        if let (Some(slot), Some(src)) = (out.get_mut(..n), text.get(..n)) {
            slot.copy_from_slice(src);
        }
        out
    }

    #[test]
    fn join_is_the_header_plus_the_two_mailbox_words() {
        let request = TriviaRequest::Join {
            header: join_header(),
            extra: vec![0x1111, 0x2222],
        };
        assert_request(
            request,
            &[
                54, 25, 4, 0, 0, 0, 0, 0, 0, 0, 0x02, 0x01, 0x11, 0x11, 0x22, 0x22,
            ],
        );
    }

    #[test]
    fn exit_is_the_twelve_byte_header() {
        assert_request(
            TriviaRequest::Exit(header()),
            &[54, 26, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12],
        );
    }

    #[test]
    fn an_answer_line_is_the_fixed_framing_then_the_text() {
        let request = TriviaRequest::Text {
            header: header(),
            framing: TextFraming {
                zero: 0,
                three: 3,
                number: 7,
                two: 2,
            },
            text: b"1\r\n".to_vec(),
        };
        assert_request(
            request,
            &[
                54, 24, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12, 0, 3, 7, 0, 2, b'1', 0x0d, 0x0a,
            ],
        );
    }

    #[test]
    fn the_version_line_uses_the_same_text_frame() {
        let request = TriviaRequest::Text {
            header: header(),
            framing: TextFraming {
                zero: 0,
                three: 3,
                number: 7,
                two: 2,
            },
            text: b"TRIVIA%V01.200\r\n".to_vec(),
        };
        let bytes = ClientMessage::Trivia(request.clone()).encode();
        assert!(bytes.ends_with(b"TRIVIA%V01.200\r\n"));
        assert_eq!(
            ClientMessage::parse(&bytes),
            Ok(ClientMessage::Trivia(request))
        );
    }

    #[test]
    fn joined_is_which_cmd_17_and_the_four_routing_words() {
        assert_reply(
            TriviaReply::Joined(header()),
            &[54, 17, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12],
        );
    }

    #[test]
    fn a_game_packet_puts_the_type_at_absolute_21() {
        let reply = TriviaReply::Game {
            header: header(),
            ignored: 0,
            pad: [0; PAD],
            message: NtnMessage::AnswerCorrect { answer: 3 },
        };
        let bytes = HostMessage::Trivia(reply.clone()).encode();
        assert_eq!(bytes.get(13).copied(), Some(INNER_GAME));
        assert_eq!(bytes.get(21).copied(), Some(TYPE_CORRECT));
        assert_eq!(bytes.get(22).copied(), Some(3));
        assert_eq!(HostMessage::parse(&bytes), Ok(HostMessage::Trivia(reply)));
    }

    #[test]
    fn type_10_words_follow_the_score_round_and_total_order() {
        let words = ScoreWords {
            score_low: 1000,
            score_high: 0,
            round_low: 750,
            round_high: 0,
            round_rank: 1,
            total_low: 750,
            total_high: 0,
            total_rank: 2,
        };
        let reply = TriviaReply::Game {
            header: header(),
            ignored: 0,
            pad: [0; PAD],
            message: NtnMessage::Score(words),
        };
        let bytes = HostMessage::Trivia(reply.clone()).encode();
        assert_eq!(bytes.get(21).copied(), Some(TYPE_SCORE));
        assert_eq!(
            bytes.get(22..),
            Some(&[0xe8, 0x03, 0, 0, 0xee, 0x02, 0, 0, 1, 0, 0xee, 0x02, 0, 0, 2, 0][..])
        );
        assert_eq!(HostMessage::parse(&bytes), Ok(HostMessage::Trivia(reply)));
    }

    #[test]
    fn the_show_messages_round_trip() {
        let question = NtnMessage::Question {
            number: 1,
            total: 15,
            lines: [
                line(b"Who founded Sierra?"),
                line(b""),
                line(b""),
                line(b""),
            ],
        };
        let answers = NtnMessage::Answers {
            lines: vec![line(b"Ken Williams"), line(b"Al Lowe")],
        };
        let winners = NtnMessage::RoundWinners {
            winners: vec![RoundWinner {
                label: *b"1         ",
                score_low: 500,
                score_high: 0,
                name: *b"GUY   ",
                pad: [0; 4],
            }],
        };
        let messages = [
            NtnMessage::Version,
            NtnMessage::Logon,
            NtnMessage::ScoreTemplate {
                mode: 0,
                cnf: 0,
                help: 0,
                game_id: 0,
                scope: 0,
            },
            NtnMessage::TickdownSetup {
                kind: 0,
                seconds: 20,
                points: 1000,
                clues: 0,
            },
            question,
            answers,
            NtnMessage::AnswerRegistered {
                kind: 0,
                answer: 1,
                points: 1000,
                points_again: 1000,
            },
            NtnMessage::MinutesToNext { minutes: 1 },
            NtnMessage::StartTickdown { iteration: 0 },
            NtnMessage::GameOver { flag: 0 },
            NtnMessage::HandleRegistered {
                handle: b"GUY".to_vec(),
            },
            winners,
        ];
        for message in messages {
            let reply = TriviaReply::Game {
                header: header(),
                ignored: 0,
                pad: [0; PAD],
                message,
            };
            let bytes = HostMessage::Trivia(reply.clone()).encode();
            assert!(bytes.get(21).is_some());
            assert_eq!(HostMessage::parse(&bytes), Ok(HostMessage::Trivia(reply)));
        }
    }

    #[test]
    fn inner_42_and_the_failure_words_round_trip() {
        assert_reply(
            TriviaReply::ShowDown {
                header: header(),
                ignored: 0,
            },
            &[54, 16, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12, 0, INNER_DOWN],
        );
        assert_reply(
            TriviaReply::ForcedExit {
                header: header(),
                reason: 1,
                follow: Some(GONE_DOWN),
            },
            &[54, 20, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12, 1, 42, 0],
        );
        assert_reply(
            TriviaReply::Checked {
                which: CheckedFailure::Join,
                header: header(),
                word: GONE_DOWN,
            },
            &[54, 18, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12, 42, 0],
        );
        assert_reply(
            TriviaReply::Checked {
                which: CheckedFailure::Other,
                header: header(),
                word: 0,
            },
            &[54, 25, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12, 0, 0],
        );
        assert_reply(
            TriviaReply::Checked {
                which: CheckedFailure::Leave,
                header: header(),
                word: 7,
            },
            &[54, 26, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12, 7, 0],
        );
        assert_reply(
            TriviaReply::SendFailed {
                header: header(),
                reason: 3,
            },
            &[54, 24, 4, 0, 7, 0, 1, 0, 2, 0, 0x34, 0x12, 3],
        );
    }

    #[test]
    fn a_mall_frame_is_still_the_mall() {
        let header = MallHeader {
            slot: 5,
            tokens: [0xFFFF, 0xEEEE, 0xDDDD],
            route: 0,
        };
        let request = MallRequest::Status(header);
        let bytes = ClientMessage::Mall(request.clone()).encode();
        assert_eq!(
            ClientMessage::parse(&bytes),
            Ok(ClientMessage::Mall(request))
        );
        let reply = MallReply::Joined(header);
        let bytes = HostMessage::Mall(reply.clone()).encode();
        assert_eq!(HostMessage::parse(&bytes), Ok(HostMessage::Mall(reply)));
    }

    #[test]
    fn an_unknown_type_and_a_short_frame_are_rejected() {
        let mut bytes = HostMessage::Trivia(TriviaReply::Game {
            header: header(),
            ignored: 0,
            pad: [0; PAD],
            message: NtnMessage::Version,
        })
        .encode();
        if let Some(slot) = bytes.get_mut(21) {
            *slot = 9;
        }
        assert!(matches!(
            HostMessage::parse(&bytes),
            Err(MessageError::UnknownValue {
                field: "ntn type",
                ..
            })
        ));
        assert!(ClientMessage::parse(&[54, 24, 4, 0]).is_err());
    }
}
