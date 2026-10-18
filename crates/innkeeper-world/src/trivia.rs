//! NTN Trivia rooms. A kind-2 join picks the room; JOIN runs the handle logon.
//! The show is `tick`; layouts are `message::trivia`.

mod bank;
mod show;
mod text;

use std::sync::{Mutex, MutexGuard, PoisonError};

use tracing::{info, warn};

use crate::message::{CheckedFailure, NtnMessage, TriviaHeader, TriviaReply, TriviaRequest};
use crate::{
    is_reserved_name, ConnectionId, Delivery, HostMessage, JoinNet, LogPolicy, ObjectKind,
    NTN_SERVER,
};

pub use bank::{OpenQuestion, PoolCounts, TriviaPools};
pub use text::{fit, wrap, Item, RawQuestion};

/// The three rooms, behind a mutex so `World` can keep the field public.
#[derive(Debug)]
pub struct TriviaShows {
    inner: Mutex<Shows>,
}

impl TriviaShows {
    pub fn new() -> Self {
        TriviaShows {
            inner: Mutex::new(Shows::new()),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Shows> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for TriviaShows {
    fn default() -> Self {
        Self::new()
    }
}

/// Hub `script.120` pushes these for topics 1, 2 and 3.
const HUB_GROUPS: [u16; 3] = [16, 17, 18];
/// SierraLand and Feb-94 push these for the same three topics.
const SIERRA_GROUPS: [u16; 3] = [7, 8, 9];

pub(super) const HANDLE_LIMIT: usize = 6;
/// The word of a JOIN failure (whichCmd 18) that makes the client say NTN Trivia has gone down.
const SHOW_DOWN: u16 = 42;
/// whichCmd 16 bytes 14..20. The client ignores them (`proc_237`).
const IGNORED_PAD: usize = 7;
pub(super) const ROOMS: usize = 3;

/// The topic index, 1..=3, when `join` is a trivia waiting-room group.
pub(crate) fn room_for(join: &JoinNet) -> Option<u8> {
    if join.kind != ObjectKind::Group {
        return None;
    }
    let index = HUB_GROUPS
        .iter()
        .position(|&parameter| parameter == join.parameter)
        .or_else(|| {
            SIERRA_GROUPS
                .iter()
                .position(|&parameter| parameter == join.parameter)
        })?;
    u8::try_from(index.saturating_add(1)).ok()
}

/// One show's seats. The three rooms share nothing but the session-number counter.
#[derive(Debug)]
pub(crate) struct Shows {
    next_number: u16,
    rooms: [Room; ROOMS],
    bags: [bank::RoomBag; ROOMS],
}

#[derive(Debug, Default)]
struct Room {
    seats: Vec<Seat>,
    phase: show::Phase,
}

#[derive(Debug)]
struct Seat {
    connection: ConnectionId,
    number: u16,
    sid: u16,
    host: u16,
    /// No second NTN host, so replies keep this 0.
    remote: u16,
    handle: Option<Vec<u8>>,
    awaiting: bool,
    score: u32,
    last: u16,
    answered: Option<show::Answer>,
}

#[derive(Clone, Copy)]
struct Place {
    room: usize,
    seat: usize,
}

struct View {
    header: TriviaHeader,
    awaiting: bool,
}

impl Shows {
    pub(crate) fn new() -> Self {
        Shows {
            next_number: 1,
            rooms: Default::default(),
            bags: [
                bank::RoomBag::Empty,
                bank::RoomBag::Empty,
                bank::RoomBag::Empty,
            ],
        }
    }

    pub(crate) fn on_request(
        &mut self,
        connection: ConnectionId,
        room: Option<u8>,
        (host, log): (u16, LogPolicy),
        request: &TriviaRequest,
        now: u64,
    ) -> Vec<Delivery> {
        match request {
            TriviaRequest::Join { header, .. } => {
                into(connection, self.join(connection, room, host, *header))
            }
            TriviaRequest::Exit(_) => {
                self.disconnect(connection);
                Vec::new()
            }
            TriviaRequest::Text { text, .. } => self.text(connection, text, now, log),
        }
    }

    pub(crate) fn set_pools(&mut self, pools: TriviaPools, seed: u64) {
        self.bags = [
            bank::RoomBag::single(pools.adventure, seed),
            bank::RoomBag::single(pools.general, seed ^ 0x9e37_79b9),
            bank::RoomBag::mixed(pools.screens_ours, pools.screens, seed ^ 0x85eb_ca6b),
        ];
    }

    pub(crate) fn disconnect(&mut self, connection: ConnectionId) {
        let mut removed = false;
        for room in &mut self.rooms {
            let before = room.seats.len();
            room.seats.retain(|seat| seat.connection != connection);
            if !room.seats.iter().any(|seat| seat.handle.is_some()) {
                room.phase = show::Phase::Idle;
            }
            removed |= room.seats.len() != before;
        }
        if removed {
            info!(connection = connection.0, "trivia player left");
        }
    }

    fn join(
        &mut self,
        connection: ConnectionId,
        room: Option<u8>,
        host: u16,
        request: TriviaHeader,
    ) -> Vec<HostMessage> {
        self.disconnect(connection);
        let index = room_index(room);
        if self.bags.get(index).is_none_or(bank::RoomBag::is_empty) {
            // No questions to deal: the player would wait forever, so the show says it is down.
            warn!(
                room = index + 1,
                "trivia JOIN refused: the room has no questions"
            );
            let header = TriviaHeader {
                server: NTN_SERVER,
                number: 0,
                host,
                remote: 0,
                sid: request.sid,
            };
            return vec![HostMessage::Trivia(TriviaReply::Checked {
                which: CheckedFailure::Join,
                header,
                word: SHOW_DOWN,
            })];
        }
        let number = self.allocate();
        let seat = Seat {
            connection,
            number,
            sid: request.sid,
            host,
            remote: 0,
            handle: None,
            awaiting: true,
            score: 0,
            last: 0,
            answered: None,
        };
        let header = seat.header();
        let Some(room) = self.rooms.get_mut(index) else {
            return Vec::new();
        };
        room.seats.push(seat);
        info!(
            connection = connection.0,
            room = index + 1,
            number,
            "trivia player joined"
        );
        // Type 1 opens the handle dialog: vocab.729 local5 starts at 1. A later
        // type 0 is the reject path (`local3 == 1`) and is sent only on refusal.
        vec![
            joined(header),
            game(header, NtnMessage::Version),
            game(header, NtnMessage::Logon),
        ]
    }

    fn text(
        &mut self,
        connection: ConnectionId,
        text: &[u8],
        now: u64,
        log: LogPolicy,
    ) -> Vec<Delivery> {
        let Some(place) = self.place_of(connection) else {
            warn!("trivia line before JOIN: ignored");
            return Vec::new();
        };
        let Some(view) = self.view(place) else {
            return Vec::new();
        };
        if !view.awaiting {
            if !version_line(text) {
                self.take_answer(connection, text, now);
            }
            return Vec::new();
        }
        if version_line(text) {
            return Vec::new();
        }
        let Some(handle) = normalize_handle(text) else {
            warn!(bytes = text.len(), "trivia handle refused");
            return into(connection, vec![game(view.header, NtnMessage::Version)]);
        };
        if self.taken(place.room, &same_handle(&handle)) {
            warn!(
                bytes = handle.len(),
                "trivia handle already used in the room"
            );
            return into(connection, vec![game(view.header, NtnMessage::Version)]);
        }
        self.accept(place, handle.clone());
        match log {
            LogPolicy::Verbose => info!(
                connection = connection.0,
                handle = %String::from_utf8_lossy(&handle),
                "trivia handle accepted"
            ),
            LogPolicy::Public => info!(
                connection = connection.0,
                bytes = handle.len(),
                "trivia handle accepted"
            ),
        }
        into(
            connection,
            vec![
                game(view.header, NtnMessage::HandleRegistered { handle }),
                game(
                    view.header,
                    NtnMessage::ScoreTemplate {
                        mode: 0,
                        cnf: 0,
                        help: 0,
                        game_id: 0,
                        scope: 0,
                    },
                ),
            ],
        )
    }

    fn accept(&mut self, place: Place, handle: Vec<u8>) {
        let Some(room) = self.rooms.get_mut(place.room) else {
            return;
        };
        let Some(seat) = room.seats.get_mut(place.seat) else {
            return;
        };
        seat.handle = Some(handle);
        seat.awaiting = false;
    }

    /// `wanted` is in `same_handle` form: handles that differ only in spaces are the same handle.
    fn taken(&self, room: usize, wanted: &[u8]) -> bool {
        let Some(room) = self.rooms.get(room) else {
            return false;
        };
        room.seats.iter().any(|seat| {
            seat.handle
                .as_deref()
                .is_some_and(|handle| same_handle(handle) == wanted)
        })
    }

    fn view(&self, place: Place) -> Option<View> {
        let seat = self.rooms.get(place.room)?.seats.get(place.seat)?;
        Some(View {
            header: seat.header(),
            awaiting: seat.awaiting,
        })
    }

    fn place_of(&self, connection: ConnectionId) -> Option<Place> {
        self.rooms.iter().enumerate().find_map(|(room, group)| {
            let seat = group
                .seats
                .iter()
                .position(|seat| seat.connection == connection)?;
            Some(Place { room, seat })
        })
    }

    fn allocate(&mut self) -> u16 {
        for _ in 0..u16::MAX {
            let number = match self.next_number {
                0 => 1,
                number => number,
            };
            self.next_number = number.wrapping_add(1);
            if self.next_number == 0 {
                self.next_number = 1;
            }
            if !self.number_used(number) {
                return number;
            }
        }
        1
    }

    fn number_used(&self, number: u16) -> bool {
        self.rooms
            .iter()
            .any(|room| room.seats.iter().any(|seat| seat.number == number))
    }
}

impl Seat {
    fn header(&self) -> TriviaHeader {
        TriviaHeader {
            server: NTN_SERVER,
            number: self.number,
            host: self.host,
            remote: self.remote,
            sid: self.sid,
        }
    }
}

fn room_index(room: Option<u8>) -> usize {
    match room {
        Some(number @ 1..=3) => usize::from(number - 1),
        other => {
            warn!(
                room = other.unwrap_or(0),
                "trivia JOIN without a room, using room 1"
            );
            0
        }
    }
}

fn version_line(text: &[u8]) -> bool {
    line_of(text).starts_with(b"TRIVIA%")
}

/// The client rejects `TSN` and `INN` prefixes for non-staff. This host has no staff
/// exception on the trivia path, so every such name is refused and the dialog is reopened, as is
/// any other staff or service name. Leading and trailing spaces are dropped.
fn normalize_handle(text: &[u8]) -> Option<Vec<u8>> {
    let line = line_of(text).trim_ascii();
    let fits = (1..=HANDLE_LIMIT).contains(&line.len());
    let graphic = line
        .iter()
        .all(|byte| byte.is_ascii_graphic() || *byte == b' ');
    if !fits || !graphic {
        return None;
    }
    let handle: Vec<u8> = line.iter().map(u8::to_ascii_uppercase).collect();
    let squeezed = same_handle(&handle);
    let staff = std::str::from_utf8(&handle).map_or(true, is_reserved_name);
    let reserved = squeezed.starts_with(b"TSN") || squeezed.starts_with(b"INN") || staff;
    match reserved {
        true => None,
        false => Some(handle),
    }
}

/// A handle with its spaces removed: what two handles are compared by, so `G UY` is `GUY`.
fn same_handle(handle: &[u8]) -> Vec<u8> {
    handle
        .iter()
        .copied()
        .filter(|byte| *byte != b' ')
        .collect()
}

pub(super) fn line_of(text: &[u8]) -> &[u8] {
    let end = text
        .iter()
        .position(|byte| *byte == b'\r' || *byte == 0)
        .unwrap_or(text.len());
    text.get(..end).unwrap_or(text)
}

fn joined(header: TriviaHeader) -> HostMessage {
    HostMessage::Trivia(TriviaReply::Joined(header))
}

fn into(connection: ConnectionId, messages: Vec<HostMessage>) -> Vec<Delivery> {
    messages
        .into_iter()
        .map(|message| Delivery::new(connection, message))
        .collect()
}

pub(super) fn game(header: TriviaHeader, message: NtnMessage) -> HostMessage {
    HostMessage::Trivia(TriviaReply::Game {
        header,
        ignored: 0,
        pad: [0; IGNORED_PAD],
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ConnectionId = ConnectionId(1);
    const B: ConnectionId = ConnectionId(2);

    fn header(sid: u16) -> TriviaHeader {
        TriviaHeader {
            server: NTN_SERVER,
            number: 0,
            host: 0,
            remote: 0,
            sid,
        }
    }

    fn join(shows: &mut Shows, connection: ConnectionId, room: u8) {
        let replies = shows.join(connection, Some(room), 7, header(0x10));
        assert_eq!(replies.len(), 3);
    }

    /// Three rooms with one question each, so a JOIN is seated.
    fn stocked() -> Shows {
        let item = text::fit(&RawQuestion {
            question: "Who founded Sierra?".into(),
            correct: "Ken Williams".into(),
            incorrect: vec!["Al Lowe".into()],
            boolean: false,
        })
        .unwrap();
        let mut shows = Shows::new();
        let pools = TriviaPools {
            adventure: vec![item.clone()],
            general: vec![item.clone()],
            screens: vec![item],
            screens_ours: Vec::new(),
        };
        shows.set_pools(pools, 1);
        shows
    }

    fn handle(shows: &mut Shows, connection: ConnectionId, text: &[u8]) -> Vec<HostMessage> {
        shows
            .text(connection, text, 0, LogPolicy::Public)
            .into_iter()
            .map(|delivery| delivery.message)
            .collect()
    }

    #[test]
    fn hub_and_sierra_parameters_are_the_same_three_rooms() {
        let group = |parameter| JoinNet {
            cookie: crate::Cookie(1),
            kind: ObjectKind::Group,
            land_type: crate::LandType(1),
            parameter,
            size: 20,
        };
        for (hub, sierra, room) in [(16, 7, 1), (17, 8, 2), (18, 9, 3)] {
            assert_eq!(room_for(&group(hub)), Some(room));
            assert_eq!(room_for(&group(sierra)), Some(room));
        }
        assert_eq!(room_for(&group(1)), None);
    }

    #[test]
    fn a_refused_handle_reopens_the_dialog_and_a_taken_one_does_too() {
        let mut shows = stocked();
        join(&mut shows, A, 1);
        assert!(handle(&mut shows, A, b"TRIVIA%V01.200\r\n").is_empty());
        assert!(handle(&mut shows, A, b"TSNBOB\r\n").len() == 1);
        assert!(handle(&mut shows, A, b"inn\r\n").len() == 1);
        assert!(handle(&mut shows, A, b"\r\n").len() == 1);
        let accepted = handle(&mut shows, A, b"Guy\r\n");
        assert!(matches!(
            accepted.first(),
            Some(HostMessage::Trivia(TriviaReply::Game {
                message: NtnMessage::HandleRegistered { handle },
                ..
            })) if handle == b"GUY"
        ));
        join(&mut shows, B, 1);
        assert_eq!(handle(&mut shows, B, b"guy\r\n").len(), 1);
        join(&mut shows, B, 2);
        assert_eq!(handle(&mut shows, B, b"guy\r\n").len(), 2);
    }

    #[test]
    fn handles_that_differ_only_in_spaces_are_one_handle_and_staff_names_are_refused() {
        let mut shows = stocked();
        join(&mut shows, A, 1);
        let accepted = handle(&mut shows, A, b" GUY \r\n");
        assert!(matches!(
            accepted.first(),
            Some(HostMessage::Trivia(TriviaReply::Game {
                message: NtnMessage::HandleRegistered { handle },
                ..
            })) if handle == b"GUY"
        ));
        join(&mut shows, B, 1);
        for look_alike in [&b"GUY \r\n"[..], b" guy\r\n", b"G UY\r\n", b"g u y\r\n"] {
            assert_eq!(handle(&mut shows, B, look_alike).len(), 1, "{look_alike:?}");
        }
        for staff in [&b"T SN\r\n"[..], b"SYSOP\r\n", b"ADMIN\r\n", b"   \r\n"] {
            assert_eq!(handle(&mut shows, B, staff).len(), 1, "{staff:?}");
        }
        assert_eq!(handle(&mut shows, B, b"GUYS\r\n").len(), 2);
    }

    #[test]
    fn a_room_with_no_questions_says_the_show_is_down_and_seats_nobody() {
        let mut shows = Shows::new();
        let replies = shows.join(A, Some(2), 7, header(0x10));
        assert!(matches!(
            replies.as_slice(),
            [HostMessage::Trivia(TriviaReply::Checked {
                which: CheckedFailure::Join,
                word: SHOW_DOWN,
                header,
            })] if header.sid == 0x10 && header.server == NTN_SERVER
        ));
        assert!(shows.place_of(A).is_none());
        let mut stocked = stocked();
        assert_eq!(stocked.join(A, Some(2), 7, header(0x10)).len(), 3);
    }

    #[test]
    fn exit_and_disconnect_free_the_handle() {
        let mut shows = stocked();
        join(&mut shows, A, 1);
        assert_eq!(handle(&mut shows, A, b"QUINN\r\n").len(), 2);
        shows.disconnect(A);
        join(&mut shows, B, 1);
        assert_eq!(handle(&mut shows, B, b"QUINN\r\n").len(), 2);
        let exit = TriviaRequest::Exit(header(1));
        shows.on_request(B, Some(1), (7, LogPolicy::Public), &exit, 0);
        join(&mut shows, A, 1);
        assert_eq!(handle(&mut shows, A, b"QUINN\r\n").len(), 2);
    }
}
