//! The Arena games this host runs, sent as GAMEINFO.CFG when a client's stamp differs.
//! Column meanings and the client's read rules: docs/server/games.md.

use crate::assumptions::GAME_INFO_STAMP_ASSUMED;
use crate::{HostMessage, Stamp};

/// One row of the file `script.144` reads. The name uses `_` for a space the slider shows later.
#[derive(Clone, Copy)]
struct ArenaGame {
    land: u8,
    position: i16,
    name: &'static str,
    minimum: i16,
    maximum: i16,
    queue: i16,
    invite: i16,
    score: i16,
    chain: i16,
    add_players: i16,
}

/// Red Baron, 3-D Golf and Football, relative positions 0, 1 and 2 on land 5.
const ARENA_GAMES: [ArenaGame; 3] = [
    ArenaGame {
        land: 5,
        position: 0,
        name: "Red_Baron",
        minimum: 2,
        maximum: 4,
        queue: 0,
        invite: 910,
        score: 910,
        chain: 101,
        add_players: 0,
    },
    ArenaGame {
        land: 5,
        position: 1,
        name: "3-D_Golf",
        minimum: 1,
        maximum: 4,
        queue: 0,
        invite: 911,
        score: -911,
        chain: 102,
        add_players: 0,
    },
    ArenaGame {
        land: 5,
        position: 2,
        name: "Football",
        minimum: 2,
        maximum: 2,
        queue: 0,
        invite: 912,
        score: -912,
        chain: 104,
        add_players: 0,
    },
];

const HEADER: [&str; 2] = [
    "; Games this host runs in the Arena.",
    "; land position name minimum maximum queue invite score chain add",
];

/// `script.144` reads into `String(80)`, so a longer line is split before the newline.
const MAX_LINE: usize = 79;
/// A token is read into `String(40)`.
const MAX_TOKEN: usize = 39;
/// One application message (link-layer.md). The client also accepts this size.
const MAX_MESSAGE: usize = u16::MAX as usize;
/// A type-8 reply longer than this is written to `.\GAMEINFO.CFG`.
const FILE_REPLACES_AT: usize = 10;

/// HostInfo 36/8. A stamp other than [`GAME_INFO_STAMP_ASSUMED`] gets the file; a match is six bytes.
pub(crate) fn reply(client_stamp: Stamp) -> HostMessage {
    let stamp = GAME_INFO_STAMP_ASSUMED;
    let file = if client_stamp == stamp {
        Vec::new()
    } else {
        file_bytes()
    };
    HostMessage::GameInfoFile { stamp, file }
}

fn file_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    for line in HEADER {
        push_line(&mut out, line);
    }
    for game in ARENA_GAMES {
        push_line(&mut out, &game.line());
    }
    let message = 6 + out.len();
    assert!(message > FILE_REPLACES_AT && message <= MAX_MESSAGE);
    out
}

fn push_line(out: &mut Vec<u8>, line: &str) {
    assert!(
        !line.is_empty()
            && line.len() <= MAX_LINE
            && !line.bytes().any(|b| b == b'\n' || b == b'\r'),
        "GAMEINFO line does not fit the client's read"
    );
    out.extend(line.as_bytes());
    out.extend(b"\r\n");
}

impl ArenaGame {
    fn line(self) -> String {
        assert!(!self.name.is_empty() && self.name.len() <= MAX_TOKEN);
        assert!(!self.name.contains([' ', '\t']));
        format!(
            "{} {} {} {} {} {} {} {} {} {}",
            self.land,
            self.position,
            self.name,
            self.minimum,
            self.maximum,
            self.queue,
            self.invite,
            self.score,
            self.chain,
            self.add_players,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! game {
        (
            $land:expr, $pos:expr, $name:expr, $min:expr, $max:expr, $queue:expr, $inv:expr,
            $score:expr, $chain:expr, $add:expr
        ) => {
            ParsedGame {
                land: $land,
                position: $pos,
                name: $name.to_owned(),
                minimum: $min,
                maximum: $max,
                queue: $queue,
                invite: $inv,
                score: $score,
                chain: $chain,
                add_players: $add,
            }
        };
    }

    /// The bytes `reply` must keep producing. A change to the table or the header fails this.
    const GAME_INFO_TEXT: &str = "\
; Games this host runs in the Arena.\r\n\
; land position name minimum maximum queue invite score chain add\r\n\
5 0 Red_Baron 2 4 0 910 910 101 0\r\n\
5 1 3-D_Golf 1 4 0 911 -911 102 0\r\n\
5 2 Football 2 2 0 912 -912 104 0\r\n";

    #[test]
    fn stamp_zero_is_the_pinned_file_and_the_host_stamp_is_six_bytes() {
        let fresh = reply(Stamp(0));
        let encoded = fresh.encode();
        assert!(encoded.len() > FILE_REPLACES_AT && encoded.len() <= MAX_MESSAGE);
        assert_eq!(&encoded[..6], &[0x24, 0x08, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(&encoded[6..], GAME_INFO_TEXT.as_bytes());
        assert_eq!(HostMessage::parse(&encoded), Ok(fresh));

        let current = reply(GAME_INFO_STAMP_ASSUMED);
        assert_eq!(current.encode(), [0x24, 0x08, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(
            HostMessage::parse(&current.encode()),
            Ok(HostMessage::GameInfoFile {
                stamp: GAME_INFO_STAMP_ASSUMED,
                file: Vec::new(),
            })
        );
    }

    #[test]
    fn the_file_parses_as_script_144_reads_it() {
        let games = parse_game_info(GAME_INFO_TEXT.as_bytes());
        assert_eq!(
            games,
            [
                game!(5, 0, "Red_Baron", 2, 4, 0, 910, 910, 101, 0),
                game!(5, 1, "3-D_Golf", 1, 4, 0, 911, -911, 102, 0),
                game!(5, 2, "Football", 2, 2, 0, 912, -912, 104, 0),
            ]
        );
        let shown: Vec<_> = games
            .iter()
            .map(|game| game.name.replace('_', " "))
            .collect();
        assert_eq!(shown, ["Red Baron", "3-D Golf", "Football"]);

        let tabs = b"5\t0\tRed_Baron\t2\t4\t0\t910\t910\t101\t0\r\n";
        assert_eq!(
            parse_game_info(tabs),
            [game!(5, 0, "Red_Baron", 2, 4, 0, 910, 910, 101, 0)]
        );
        let missing = b"; skipped\r\n5 1\r\n\r\n5 2 Football 2 2 0 912 -912 104 0\r\n";
        assert_eq!(
            parse_game_info(missing),
            [game!(5, 1, "?", 0, 0, 0, 0x38a, 0, -1, 0)]
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ParsedGame {
        land: i16,
        position: i16,
        name: String,
        minimum: i16,
        maximum: i16,
        queue: i16,
        invite: i16,
        score: i16,
        chain: i16,
        add_players: i16,
    }

    /// `script.144` export 1: skip a `;` line, stop on a blank line, split on space and tab.
    /// A missing name token is `?`. Integers are optional-sign digits, and anything else is 0.
    fn parse_game_info(bytes: &[u8]) -> Vec<ParsedGame> {
        let mut games = Vec::new();
        for line in bytes.split(|&byte| byte == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                break;
            }
            if line[0] == b';' {
                continue;
            }
            let line = trim_end(line);
            let tokens = tokens_of(line);
            let token = |index: usize| tokens.get(index).copied();
            games.push(ParsedGame {
                land: token(0).map(sci_integer).unwrap_or(0),
                position: token(1).map(sci_integer).unwrap_or(-1),
                name: latin1(token(2).unwrap_or(b"?")),
                minimum: token(3).map(sci_integer).unwrap_or(0),
                maximum: token(4).map(sci_integer).unwrap_or(0),
                queue: token(5).map(sci_integer).unwrap_or(0),
                invite: token(6).map(sci_integer).unwrap_or(0x38a),
                score: token(7).map(sci_integer).unwrap_or(0),
                chain: token(8).map(sci_integer).unwrap_or(-1),
                add_players: token(9).map(sci_integer).unwrap_or(0),
            });
        }
        games
    }

    fn tokens_of(line: &[u8]) -> Vec<&[u8]> {
        line.split(|&byte| byte == b' ' || byte == b'\t')
            .filter(|token| !token.is_empty())
            .collect()
    }

    fn latin1(bytes: &[u8]) -> String {
        bytes.iter().copied().map(char::from).collect()
    }

    fn trim_end(line: &[u8]) -> &[u8] {
        let end = line
            .iter()
            .rposition(|&byte| byte != b' ' && byte != b'\t')
            .map_or(0, |index| index + 1);
        &line[..end]
    }

    fn sci_integer(token: &[u8]) -> i16 {
        let (negative, digits) = match token.first() {
            Some(b'-') => (true, &token[1..]),
            Some(b'+') => (false, &token[1..]),
            _ => (false, token),
        };
        let mut value: i32 = 0;
        let mut saw_digit = false;
        for byte in digits {
            if !byte.is_ascii_digit() {
                break;
            }
            saw_digit = true;
            value = value
                .saturating_mul(10)
                .saturating_add(i32::from(byte - b'0'));
        }
        if !saw_digit {
            return 0;
        }
        let signed = if negative { -value } else { value };
        i16::try_from(signed).unwrap_or(0)
    }
}
