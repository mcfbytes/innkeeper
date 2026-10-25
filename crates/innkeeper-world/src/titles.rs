//! Every game title this host serves: where it is offered, what runs it, and the bytes the clients
//! name it by. Sources and labels: docs/server/game-services.md section 1.

use crate::assumptions::UNSEEN_GAME_BYTES_NAME_THEIR_TITLE_ASSUMED;
use crate::services::ServiceObject;
use crate::LandType;

/// What runs a title on the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramKind {
    /// A script of the land's own `LSCITV`.
    LsciScript,
    /// A DOS program the land chains to through a `TSN.PRG` block.
    DosProgram,
    /// An RPG title's own programs, on a land of its own.
    Rpg,
    /// A land script that talks to a PMS server with command 54.
    PmsServer,
}

/// One title. A catalogue entry with two script numbers is still one title.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TitleId {
    MiniGolf,
    PaintBall,
    Boogers,
    Stratego,
    SneakATac,
    Graffiti,
    RocketQuiz,
    NtnTrivia,
    RedBaron,
    Golf3d,
    Football,
    Checkers,
    Chess,
    BackGammon,
    FlipFlop,
    Go,
    Spades,
    Euchre,
    Bridge,
    Cribbage,
    Hearts,
    BlackJack,
    Slots,
    Roulette,
    Roulette00,
    Poker,
    LeftysBar,
    ShoppersAdvantage,
    Yserbius,
    Cawdor,
    Twinion,
}

/// The fewest and most players the land's `GameInfo` (or the Arena's row) allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Players {
    pub minimum: u8,
    pub maximum: u8,
}

/// The columns of the Arena's `GAMEINFO.CFG` row that the title's other fields do not give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaSlot {
    pub position: i16,
    pub queue: i16,
    pub invite: i16,
    /// A positive number is a high-score board, a negative one a low-score board.
    pub score: i16,
    pub add_players: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Title {
    pub id: TitleId,
    /// As `GAME.CFG` spells it.
    pub name: &'static str,
    pub kind: ProgramKind,
    /// The land types whose places offer it.
    pub lands: &'static [LandType],
    /// The land's `GameInfo` script numbers, one per catalogue entry.
    pub scripts: &'static [u16],
    /// The land-type byte of the own group a game makes (`GameInfo` tID), one per catalogue entry.
    pub game_bytes: &'static [u8],
    /// Whether a live run has shown those bytes on a game group.
    pub game_bytes_seen: bool,
    pub players: Option<Players>,
    /// The land type of the objects the title's DOS program makes.
    pub dos_land_type: Option<LandType>,
    /// The `TSN.PRG` block the land chains to.
    pub chain: Option<&'static str>,
    pub service: Option<ServiceObject>,
    /// The server word of the title's command 54 traffic.
    pub pms_server: Option<u16>,
    pub arena: Option<ArenaSlot>,
}

const CLUBHOUSE: &[LandType] = &[LandType(1)];
const SIERRALAND: &[LandType] = &[LandType(2)];
const CASINOLAND: &[LandType] = &[LandType(3)];
const SIERRALAND_AND_ARENA: &[LandType] = &[LandType(2), LandType(5)];
const ARENA: &[LandType] = &[LandType(5)];
const CLUBHOUSE_AND_SIERRALAND: &[LandType] = &[LandType(1), LandType(2)];

impl Title {
    const fn place(
        id: TitleId,
        name: &'static str,
        kind: ProgramKind,
        lands: &'static [LandType],
    ) -> Self {
        Title {
            id,
            name,
            kind,
            lands,
            scripts: &[],
            game_bytes: &[],
            game_bytes_seen: false,
            players: None,
            dos_land_type: None,
            chain: None,
            service: None,
            pms_server: None,
            arena: None,
        }
    }

    const fn game(
        id: TitleId,
        name: &'static str,
        lands: &'static [LandType],
        scripts: &'static [u16],
        game_bytes: &'static [u8],
        (minimum, maximum): (u8, u8),
    ) -> Self {
        Title {
            scripts,
            game_bytes,
            players: Some(Players { minimum, maximum }),
            ..Title::place(id, name, ProgramKind::LsciScript, lands)
        }
    }

    const fn seen(self) -> Self {
        Title {
            game_bytes_seen: true,
            ..self
        }
    }

    const fn dos(self, objects: Option<LandType>, chain: &'static str) -> Self {
        Title {
            kind: ProgramKind::DosProgram,
            dos_land_type: objects,
            chain: Some(chain),
            ..self
        }
    }

    const fn chained(self, chain: &'static str) -> Self {
        Title {
            chain: Some(chain),
            ..self
        }
    }

    const fn served_by(self, service: ServiceObject) -> Self {
        Title {
            service: Some(service),
            ..self
        }
    }

    const fn pms(self, server: u16) -> Self {
        Title {
            kind: ProgramKind::PmsServer,
            pms_server: Some(server),
            ..self
        }
    }

    const fn in_arena(self, position: i16, invite: i16, score: i16) -> Self {
        Title {
            arena: Some(ArenaSlot {
                position,
                queue: 0,
                invite,
                score,
                add_players: 0,
            }),
            ..self
        }
    }
}

use ProgramKind::{DosProgram, Rpg};
use TitleId::*;

/// One row per [`TitleId`], in its order.
pub static TITLES: [Title; 31] = [
    Title::game(MiniGolf, "MiniGolf", SIERRALAND, &[779], &[23], (1, 2)).seen(),
    Title::game(PaintBall, "PaintBall", SIERRALAND, &[780], &[24], (2, 6)),
    Title::game(Boogers, "Boogers", SIERRALAND, &[790], &[25], (2, 4)),
    Title::game(Stratego, "Stratego", SIERRALAND, &[800], &[26], (1, 2)).seen(),
    Title::game(SneakATac, "SneakATac", SIERRALAND, &[775], &[27], (2, 2)),
    Title::game(Graffiti, "Graffiti", SIERRALAND, &[530], &[28], (1, 4)).seen(),
    Title::game(RocketQuiz, "RocketQuiz", SIERRALAND, &[555], &[29], (2, 3)),
    Title::game(
        NtnTrivia,
        "NTN Trivia",
        CLUBHOUSE_AND_SIERRALAND,
        &[8920],
        &[31],
        (1, 12),
    )
    .pms(4),
    Title::game(
        RedBaron,
        "Red Baron",
        SIERRALAND_AND_ARENA,
        &[610, 615],
        &[101],
        (2, 4),
    )
    .seen()
    .dos(Some(LandType(101)), "RedBaron")
    .served_by(ServiceObject::RedBaron)
    .in_arena(0, 910, 910),
    Title::game(
        Golf3d,
        "3-D Golf",
        SIERRALAND_AND_ARENA,
        &[605],
        &[102],
        (1, 4),
    )
    .seen()
    .dos(Some(LandType(102)), "Golf")
    .in_arena(1, 911, -911),
    Title::game(Football, "Football", ARENA, &[], &[], (2, 2))
        .dos(Some(LandType(104)), "football")
        .in_arena(2, 912, -912),
    Title::game(Checkers, "Checkers", CLUBHOUSE, &[400], &[4], (2, 2)),
    Title::game(Chess, "Chess", CLUBHOUSE, &[405, 406], &[5, 16], (2, 2)),
    Title::game(BackGammon, "BackGammon", CLUBHOUSE, &[410], &[3], (2, 2)),
    Title::game(FlipFlop, "FlipFlop", CLUBHOUSE, &[415], &[8], (2, 2)),
    Title::game(Go, "Go", CLUBHOUSE, &[420], &[9], (2, 2)),
    Title::game(Spades, "Spades", CLUBHOUSE, &[515], &[18], (4, 4)),
    Title::game(Euchre, "Euchre", CLUBHOUSE, &[520], &[19], (4, 4)),
    Title::game(Bridge, "Bridge", CLUBHOUSE, &[500, 503], &[14, 15], (4, 4)),
    Title::game(Cribbage, "Cribbage", CLUBHOUSE, &[505], &[6], (2, 2)),
    Title::game(Hearts, "Hearts", CLUBHOUSE, &[510, 511], &[7, 17], (3, 4)),
    Title::game(BlackJack, "BlackJack", CASINOLAND, &[911], &[32], (1, 4)),
    Title::game(Slots, "Slots", CASINOLAND, &[1030], &[33], (1, 7)),
    Title::game(Roulette, "Roulette", CASINOLAND, &[1060], &[34], (1, 4)),
    Title::game(
        Roulette00,
        "Roulette 00",
        CASINOLAND,
        &[1061],
        &[35],
        (1, 4),
    ),
    Title::game(Poker, "Poker", CASINOLAND, &[1040], &[36], (2, 5)),
    Title::game(LeftysBar, "Lefty's Bar", CASINOLAND, &[1100], &[37], (2, 6)),
    Title::place(
        ShoppersAdvantage,
        "Shoppers Advantage",
        DosProgram,
        CLUBHOUSE,
    )
    .pms(5)
    .dos(None, "ShopAdv"),
    Title::place(Yserbius, "Yserbius", Rpg, &[LandType(4)])
        .chained("Yserbius")
        .served_by(ServiceObject::Rpg),
    Title::place(Cawdor, "Cawdor", Rpg, &[LandType(6)])
        .chained("Cawdor")
        .served_by(ServiceObject::Rpg),
    Title::place(Twinion, "Twinion", Rpg, &[LandType(8)])
        .chained("Twinion")
        .served_by(ServiceObject::Rpg),
];

/// The title whose land made a game group of this land-type byte. A byte no live run has shown is
/// trusted under `UNSEEN_GAME_BYTES_NAME_THEIR_TITLE_ASSUMED`.
pub fn by_game_byte(byte: u8) -> Option<&'static Title> {
    TITLES.iter().find(|title| {
        title.game_bytes.contains(&byte)
            && (title.game_bytes_seen || UNSEEN_GAME_BYTES_NAME_THEIR_TITLE_ASSUMED)
    })
}

/// The DOS title whose program makes objects of this land type (the 41/2 lookup's and the joinNet's).
pub fn by_dos_land_type(land_type: LandType) -> Option<&'static Title> {
    TITLES
        .iter()
        .find(|title| title.dos_land_type == Some(land_type))
}

/// The title of this name, in any case, with `_` for a space as `GAMEINFO.CFG` writes it.
pub fn by_name(name: &str) -> Option<&'static Title> {
    let spoken = name.replace('_', " ");
    TITLES
        .iter()
        .find(|title| title.name.eq_ignore_ascii_case(&spoken))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::rpg::RPG_TITLES;

    /// Exhaustive, so a new variant does not compile until it has a row.
    fn row_of(id: TitleId) -> usize {
        match id {
            MiniGolf => 0,
            PaintBall => 1,
            Boogers => 2,
            Stratego => 3,
            SneakATac => 4,
            Graffiti => 5,
            RocketQuiz => 6,
            NtnTrivia => 7,
            RedBaron => 8,
            Golf3d => 9,
            Football => 10,
            Checkers => 11,
            Chess => 12,
            BackGammon => 13,
            FlipFlop => 14,
            Go => 15,
            Spades => 16,
            Euchre => 17,
            Bridge => 18,
            Cribbage => 19,
            Hearts => 20,
            BlackJack => 21,
            Slots => 22,
            Roulette => 23,
            Roulette00 => 24,
            Poker => 25,
            LeftysBar => 26,
            ShoppersAdvantage => 27,
            Yserbius => 28,
            Cawdor => 29,
            Twinion => 30,
        }
    }

    #[test]
    fn every_title_variant_has_exactly_one_row() {
        for (index, title) in TITLES.iter().enumerate() {
            assert_eq!(row_of(title.id), index, "{:?} is out of place", title.id);
        }
        let ids: BTreeSet<TitleId> = TITLES.iter().map(|title| title.id).collect();
        assert_eq!(ids.len(), TITLES.len());
    }

    #[test]
    fn names_game_bytes_and_dos_land_types_name_one_title_each() {
        let mut names = BTreeSet::new();
        let mut bytes = BTreeSet::new();
        let mut dos = BTreeSet::new();
        for title in &TITLES {
            assert!(names.insert(title.name.to_ascii_lowercase()));
            for &byte in title.game_bytes {
                assert!(bytes.insert(byte), "game byte {byte} twice");
            }
            if let Some(land_type) = title.dos_land_type {
                assert!(dos.insert(land_type.0));
            }
        }
    }

    #[test]
    fn lookups_find_each_title_by_its_own_keys() {
        for title in &TITLES {
            assert_eq!(by_name(title.name).map(|t| t.id), Some(title.id));
            for &byte in title.game_bytes {
                assert_eq!(by_game_byte(byte).map(|t| t.id), Some(title.id));
            }
            if let Some(land_type) = title.dos_land_type {
                assert_eq!(by_dos_land_type(land_type).map(|t| t.id), Some(title.id));
            }
        }
        assert_eq!(by_name("3-D_Golf").map(|t| t.id), Some(Golf3d));
        assert_eq!(by_name("red baron").map(|t| t.id), Some(RedBaron));
        assert_eq!(by_game_byte(23).map(|t| t.name), Some("MiniGolf"));
        assert_eq!(by_game_byte(102).map(|t| t.name), Some("3-D Golf"));
        assert_eq!(
            by_dos_land_type(LandType(101)).map(|t| t.id),
            Some(RedBaron)
        );
        assert!(by_game_byte(0).is_none() && by_game_byte(0x8B).is_none());
        assert!(by_dos_land_type(LandType(2)).is_none() && by_name("Chat").is_none());
    }

    #[test]
    fn the_rpg_rows_are_the_rpg_lands() {
        let rows: Vec<LandType> = TITLES
            .iter()
            .filter(|title| title.kind == Rpg)
            .flat_map(|title| title.lands.iter().copied())
            .collect();
        let lands: Vec<LandType> = RPG_TITLES.iter().map(|title| title.land_type).collect();
        assert_eq!(rows, lands);
    }

    #[test]
    fn a_title_with_a_service_object_owns_its_land_type() {
        for title in TITLES.iter().filter(|title| title.kind == DosProgram) {
            if let Some(service) = title.service {
                assert_eq!(title.dos_land_type, Some(service.land_type()));
            }
            assert!(title.chain.is_some());
        }
    }

    #[test]
    fn arena_titles_carry_every_column_of_their_row() {
        let mut positions: Vec<i16> = TITLES
            .iter()
            .filter_map(|title| {
                let slot = title.arena?;
                assert!(title.players.is_some() && title.dos_land_type.is_some());
                assert!(title.lands.contains(&LandType(5)));
                Some(slot.position)
            })
            .collect();
        positions.sort_unstable();
        assert_eq!(positions, [0, 1, 2]);
    }

    #[test]
    fn players_fit_their_game() {
        for players in TITLES.iter().filter_map(|title| title.players) {
            assert!(1 <= players.minimum && players.minimum <= players.maximum);
        }
    }
}
