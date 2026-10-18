//! The RPG bulletin boards over the store: a fixed directory per game location, messages numbered
//! from 1 in posting order. Layouts, evidence and the INFERRED choices: docs/protocol/yserbius.md.

use tracing::{info, warn};

use crate::assumptions::{
    COMPLAINTS_ARE_KEPT_ASSUMED, COMPLAINT_SECTION_ASSUMED, COMPLAINT_TEXT_LIMIT_ASSUMED,
    ERASE_NEEDS_AUTHOR_OR_PRIVILEGE_ASSUMED, MEMBER_SERVICES_SECTION_ASSUMED,
};
use crate::message::Command;
use crate::{
    Account, AccountId, BbsReply, BbsRequest, BoardId, BoardListing, BoardRef, HostMessage,
    LandType, MessagePosition, Nak, ObjectStore, Place, Post, ReadStep, Sid, StoreError, World,
};

/// Location of the operator-only Complaints board. Posts there are kept for a year
/// (docs/server/privacy.md). The RPG client reaches it as a command 27 post, not as 27/9.
pub const COMPLAINT_BOARD_LOCATION: u16 = 0x63;

/// True for a post on the Complaints board of any land.
pub fn is_complaint_board(board: BoardId) -> bool {
    ((board.0 >> 8) & 0xFFFF) == u32::from(COMPLAINT_BOARD_LOCATION)
}

/// What a post on a board is, which decides its retention clock and whether it may name an account.
/// The SQLite store derives the same value in `posts.kind` (docs/server/store.md section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostKind {
    /// A message on a board players read.
    Post,
    /// A moderator's conference or trivia topic text (location 0, sections 67, 84 and 85).
    Topic,
    /// The Complaints board of any land: the hub's complaint form, or an RPG post to it.
    Complaint,
    /// Town Hall "Ask Member Services", kept on the complaint board's land 0 section 2.
    MemberServices,
}

impl PostKind {
    /// The `posts.kind` text.
    pub fn as_str(self) -> &'static str {
        match self {
            PostKind::Post => "post",
            PostKind::Topic => "topic",
            PostKind::Complaint => "complaint",
            PostKind::MemberServices => "member_services",
        }
    }
}

/// The kind of every post on `board`.
pub fn post_kind(board: BoardId) -> PostKind {
    let member_services = board_id(
        LandType(0),
        BoardRef {
            location: COMPLAINT_BOARD_LOCATION,
            section: MEMBER_SERVICES_SECTION_ASSUMED,
        },
    );
    if board == member_services {
        return PostKind::MemberServices;
    }
    if is_complaint_board(board) {
        return PostKind::Complaint;
    }
    let topic = TOPIC_SECTIONS.iter().any(|&section| {
        board
            == board_id(
                LandType(0),
                BoardRef {
                    location: 0,
                    section,
                },
            )
    });
    match topic {
        true => PostKind::Topic,
        false => PostKind::Post,
    }
}

/// User-flag bits that make the client read a message's text 4 bytes later (`userFlags` & 2 or 4).
const PRIVILEGED_USER_FLAGS: u16 = 0x0006;
/// The user flag the client's own sysop level 3 needs (`RoomZeroHandler`, `hub/script.011`), which is
/// what shows its "Maintain Conferences" dialog.
const MODERATOR_USER_FLAGS: u16 = 0x0004;
/// Location 0 sections whose one message is topic text: conference 67, SL trivia 84, hub trivia 85.
/// A moderator's "Change Topics" replaces it (`hub/script.120`, `SL/script.120`).
const TOPIC_SECTIONS: [u16; 3] = [67, 84, 85];
/// Trivia pickers. An empty one is served as the three rooms below, not as a blank list.
const TRIVIA_TOPIC_SECTIONS: [u16; 2] = [84, 85];
/// Info line, then one topic per `\x02`. The client splits on bytes 1 and 2 (`proc_309`).
const DEFAULT_TRIVIA_TOPICS: &[u8] =
    b"\x01Choose a room\x02Adventure games\x02General trivia\x02Games and screens\x02";
/// The most `author|text` bytes a post's 309-byte request carries after its header and NUL.
const POST_BODY_LIMIT: usize = 300;
/// The client keeps one byte per indexed message in a 120-byte buffer.
const INDEX_LIMIT: usize = 120;

/// One board the directory of a location lists; the client asks for locations 0x59, 0x5A and 0x63.
struct Section {
    location: u16,
    number: u16,
    name: &'static str,
    readable: bool,
}

/// The boards the host offers: the clients name the locations, the host picks the sections.
const SECTIONS: [Section; 3] = [
    Section {
        location: 0x59,
        number: 1,
        name: "Tavern_Posts",
        readable: true,
    },
    Section {
        location: 0x5A,
        number: 1,
        name: "Hall_of_Heroes",
        readable: true,
    },
    Section {
        location: COMPLAINT_BOARD_LOCATION,
        number: 1,
        name: "Complaints",
        readable: false,
    },
];

/// The replies one command 27 request is owed: none for a post, else exactly one.
pub(crate) fn answer(
    world: &World,
    objects: &ObjectStore,
    account: &Account,
    place: Option<Place>,
    request: &BbsRequest,
) -> Vec<HostMessage> {
    if let BbsRequest::Complaint { about, text, .. } = request {
        complaint(world, objects, account, about.is_some(), &text.0);
        return Vec::new();
    }
    let Some(place) = place else {
        warn!(request = %world.log.debug(request), "bulletin board request outside any land: offline");
        return offline(request);
    };
    let boards = Boards {
        world,
        land: place.land_type,
        privileged: account.user_flags & PRIVILEGED_USER_FLAGS != 0,
        moderator: account.user_flags & MODERATOR_USER_FLAGS != 0,
        account: account.id,
        persona: &account.persona,
    };
    match boards.handle(request) {
        Ok(reply) => reply.into_iter().map(HostMessage::Bbs).collect(),
        Err(error) => {
            warn!(%error, request = %world.log.debug(request), "bulletin board store failed: offline");
            offline(request)
        }
    }
}

/// The boards of one land, as one reader sees them.
struct Boards<'w> {
    world: &'w World,
    land: LandType,
    privileged: bool,
    moderator: bool,
    account: AccountId,
    /// The session's persona: the author a post it writes is shown under.
    persona: &'w str,
}

impl Boards<'_> {
    fn handle(&self, request: &BbsRequest) -> Result<Option<BbsReply>, StoreError> {
        let reply = match *request {
            BbsRequest::Directory { from, location } => Some(self.directory(from, location)?),
            BbsRequest::Index { from, board } => Some(self.index(from, board)?),
            BbsRequest::Read {
                from,
                board,
                step,
                message,
            } => Some(self.read(from, board, step, message)?),
            BbsRequest::Post {
                board, ref body, ..
            } => {
                self.post(board, body)?;
                None
            }
            BbsRequest::Erase { board, message, .. } => {
                self.erase(board, message)?;
                None
            }
            // `answer` keeps a complaint away from the boards of a land.
            BbsRequest::Complaint { .. } => None,
        };
        Ok(reply)
    }

    fn directory(&self, to: Sid, location: u16) -> Result<BbsReply, StoreError> {
        let sections = SECTIONS
            .iter()
            .filter(|section| section.location == location);
        let mut boards = Vec::new();
        for section in sections {
            let board = BoardRef {
                location,
                section: section.number,
            };
            let total = word(self.posts(board)?.len());
            boards.push(BoardListing {
                readable: section.readable,
                total,
                board,
                indexed: total.min(word(INDEX_LIMIT)),
                name: section.name.to_owned(),
            });
        }
        info!(location, boards = boards.len(), "board directory");
        Ok(BbsReply::Directory {
            to,
            location,
            boards,
        })
    }

    fn index(&self, to: Sid, board: BoardRef) -> Result<BbsReply, StoreError> {
        let total = word(self.posts(board)?.len().min(INDEX_LIMIT));
        Ok(BbsReply::Index {
            to,
            board,
            messages: (1..=total).collect(),
        })
    }

    fn read(
        &self,
        to: Sid,
        board: BoardRef,
        step: ReadStep,
        message: u16,
    ) -> Result<BbsReply, StoreError> {
        let posts = self.posts(board)?;
        let stored_total = word(posts.len());
        let stored_number = step_to(step, message, stored_total);
        let stored = usize::from(stored_number)
            .checked_sub(1)
            .and_then(|index| posts.get(index))
            .map(|post| post.body.clone())
            .unwrap_or_default();
        let (number, total, body) = shown_topics(board, stored_number, stored_total, stored);
        Ok(BbsReply::Message {
            to,
            board,
            number,
            position: MessagePosition {
                last: number >= total,
                first: number <= 1,
            },
            privileged: self.privileged,
            body,
        })
    }

    fn post(&self, board: BoardRef, body: &[u8]) -> Result<(), StoreError> {
        if is_topic_board(board) {
            return self.set_topics(board, body);
        }
        if !is_listed(board) {
            warn!(?board, "post to a board no directory lists: dropped");
            return Ok(());
        }
        let body = authored(&author_of(self.account, self.persona), body);
        let id = self
            .world
            .store
            .post(self.board_id(board), self.account, None, &body)?;
        info!(?board, post = id.0, "board post stored");
        Ok(())
    }

    /// A moderator's conference or trivia topic text replaces the current one: the boards hold one
    /// message, which every reader's first read returns. Others' writes are dropped.
    fn set_topics(&self, board: BoardRef, body: &[u8]) -> Result<(), StoreError> {
        if !self.moderator {
            warn!(
                ?board,
                "topic text from a reader without moderator rights: dropped"
            );
            return Ok(());
        }
        let id = self.board_id(board);
        for old in self.world.store.posts(id)? {
            self.world.store.delete_post(id, old.id)?;
        }
        let post = self.world.store.post(id, self.account, None, body)?;
        info!(
            ?board,
            post = post.0,
            length = body.len(),
            "topic text stored"
        );
        Ok(())
    }

    /// Whether this reader wrote the post: the recorded account alone decides. The name at the
    /// head of the body is the client's bytes and two accounts may share it.
    fn wrote(&self, post: &Post) -> bool {
        post.account == self.account
    }

    /// Message `message` of a board goes when its author or a privileged reader asks.
    fn erase(&self, board: BoardRef, message: u16) -> Result<(), StoreError> {
        let posts = self.posts(board)?;
        let post = usize::from(message)
            .checked_sub(1)
            .and_then(|index| posts.get(index));
        let Some(post) = post else {
            info!(
                ?board,
                message, "erase of a message that is not there: ignored"
            );
            return Ok(());
        };
        let allowed =
            !ERASE_NEEDS_AUTHOR_OR_PRIVILEGE_ASSUMED || self.privileged || self.wrote(post);
        if !allowed {
            warn!(
                ?board,
                message, "erase by a reader who is neither author nor privileged: ignored"
            );
            return Ok(());
        }
        let erased = self
            .world
            .store
            .delete_post(self.board_id(board), post.id)?;
        info!(?board, message, erased, "board message erased");
        Ok(())
    }

    fn posts(&self, board: BoardRef) -> Result<Vec<Post>, StoreError> {
        if !is_listed(board) && !is_topic_board(board) {
            return Ok(Vec::new());
        }
        self.world.store.posts(self.board_id(board))
    }

    /// The land type in the top byte keeps each RPG title's boards of one location apart.
    fn board_id(&self, board: BoardRef) -> BoardId {
        match is_topic_board(board) {
            true => board_id(LandType(0), board),
            false => board_id(self.land, board),
        }
    }
}

/// `author|text` with the session's persona as the author, whatever name the client put there, so
/// a post cannot pass for another player's or the staff's. The text keeps the client's length limit.
fn authored(author: &str, body: &[u8]) -> Vec<u8> {
    let text = match body.iter().position(|byte| *byte == b'|') {
        Some(bar) => body.get(bar + 1..).unwrap_or_default(),
        None => body,
    };
    let mut stamped = author.as_bytes().to_vec();
    stamped.push(b'|');
    stamped.extend_from_slice(text);
    stamped.truncate(POST_BODY_LIMIT);
    stamped
}

/// The name a post or complaint is filed under: the persona without `|`, or the account number
/// while the account has no persona yet.
fn author_of(account: AccountId, persona: &str) -> String {
    match persona.trim() {
        "" => format!("account {}", account.0),
        persona => persona.replace('|', "_"),
    }
}

/// A complaint from the hub or the Sketch Pad. The client says "Your complaint has been forwarded"
/// the moment it sends and waits for nothing, so nothing is answered. The text is kept as
/// `persona|Complaint: text` on the complaint board, where the account's export and delete find it
/// by the recorded account, and only the account, the length and whether someone was named are logged.
/// The player named after `about:` is resolved to an account now, while they are online
/// (`offender`), and stored beside the text; the name itself attributes nothing later.
fn complaint(
    world: &World,
    objects: &ObjectStore,
    account: &Account,
    names_someone: bool,
    text: &[u8],
) {
    if !COMPLAINTS_ARE_KEPT_ASSUMED {
        return;
    }
    let text = text.get(..COMPLAINT_TEXT_LIMIT_ASSUMED).unwrap_or(text);
    let text = text.strip_prefix(b"Complaint|").unwrap_or(text);
    let author = author_of(account.id, &account.persona);
    let mut body = format!("{author}|Complaint: ").into_bytes();
    body.extend_from_slice(text);
    let board = board_id(
        LandType(0),
        BoardRef {
            location: COMPLAINT_BOARD_LOCATION,
            section: COMPLAINT_SECTION_ASSUMED,
        },
    );
    let about = offender(objects, account.id, text);
    match world.store.post(board, account.id, about, &body) {
        Ok(post) => info!(
            account = account.id.0,
            post = post.0,
            length = text.len(),
            names_someone,
            about = about.map(|about| about.0),
            "complaint kept"
        ),
        Err(error) => warn!(
            account = account.id.0,
            length = text.len(),
            %error,
            "complaint could not be kept"
        ),
    }
}

/// The account of the player the complaint text names after `about:`, when exactly one other
/// account online plays under that name. The client builds the text (hub-services.md section 2);
/// the 27/9 SID beside it is connection-local, so the name and the phone book are what identify the
/// player, and only at this moment. Nobody, two accounts or the complainer's own: none.
pub(crate) fn offender(
    objects: &ObjectStore,
    complainer: AccountId,
    text: &[u8],
) -> Option<AccountId> {
    let name = named_offender(text)?;
    let mut matches = objects
        .online_players()
        .filter(|(_, player)| player.persona.trim().eq_ignore_ascii_case(name))
        .map(|(_, player)| player.account)
        .filter(|&account| account != complainer);
    let first = matches.next()?;
    match matches.all(|other| other == first) {
        true => Some(first),
        false => None,
    }
}

/// The name after the first `about: ` up to a comma or semicolon: 1 to 16 printable characters.
pub fn named_offender(text: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(text.split(|&byte| byte == 0).next()?).ok()?;
    let after = text.split("about: ").nth(1)?;
    let name = after.split([',', ';']).next()?.trim();
    let valid = !name.is_empty()
        && name.len() <= 16
        && name
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ');
    valid.then_some(name)
}

pub(crate) fn board_id(land: LandType, board: BoardRef) -> BoardId {
    let section = u32::from(board.section & 0xFF);
    let location = u32::from(board.location) << 8;
    BoardId(u32::from(land.0) << 24 | location | section)
}

/// The message a step lands on, kept within 1..=total; 0 for an empty board.
fn step_to(step: ReadStep, message: u16, total: u16) -> u16 {
    let wanted = match step {
        ReadStep::This => message,
        ReadStep::Next => message.saturating_add(1),
        ReadStep::Previous => message.saturating_sub(1),
        ReadStep::First => 1,
        ReadStep::Last => total,
    };
    wanted.clamp(total.min(1), total)
}

fn is_topic_board(board: BoardRef) -> bool {
    board.location == 0 && TOPIC_SECTIONS.contains(&board.section)
}

/// An empty trivia picker (84 or 85) shows the three rooms. A stored message, including a
/// moderator's, is returned as posted. Conference 67 is left empty until someone writes it.
fn shown_topics(board: BoardRef, number: u16, total: u16, body: Vec<u8>) -> (u16, u16, Vec<u8>) {
    let picker = board.location == 0 && TRIVIA_TOPIC_SECTIONS.contains(&board.section);
    let blank = body.iter().all(|byte| *byte == 0);
    match picker && blank {
        true => (1, 1, DEFAULT_TRIVIA_TOPICS.to_vec()),
        false => (number, total, body),
    }
}

fn is_listed(board: BoardRef) -> bool {
    let mut sections = SECTIONS.iter();
    sections.any(|section| section.location == board.location && section.number == board.section)
}

/// Nak 27, "BBS Server offline.": it also ends the client's wait for the reply.
fn offline(request: &BbsRequest) -> Vec<HostMessage> {
    let (BbsRequest::Directory { from, .. }
    | BbsRequest::Index { from, .. }
    | BbsRequest::Read { from, .. }) = *request
    else {
        return Vec::new();
    };
    vec![HostMessage::Nak(Nak {
        to: from,
        which_cmd: Command::Bbs.byte(),
        which_sub: request.sub(),
        num_tries: 0,
        text: String::new(),
    })]
}

fn word(count: usize) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_stay_on_the_board() {
        assert_eq!(step_to(ReadStep::First, 9, 3), 1);
        assert_eq!(step_to(ReadStep::Last, 0, 3), 3);
        assert_eq!(step_to(ReadStep::Next, 3, 3), 3);
        assert_eq!(step_to(ReadStep::Previous, 1, 3), 1);
        assert_eq!(step_to(ReadStep::Next, 1, 3), 2);
        assert_eq!(step_to(ReadStep::This, 7, 3), 3);
        assert_eq!(step_to(ReadStep::First, 1, 0), 0);
    }

    #[test]
    fn post_kinds_follow_the_board() {
        let complaint = BoardRef {
            location: COMPLAINT_BOARD_LOCATION,
            section: COMPLAINT_SECTION_ASSUMED,
        };
        let member = BoardRef {
            location: COMPLAINT_BOARD_LOCATION,
            section: MEMBER_SERVICES_SECTION_ASSUMED,
        };
        let tavern = BoardRef {
            location: 0x59,
            section: 1,
        };
        let topic = BoardRef {
            location: 0,
            section: 67,
        };
        assert_eq!(
            post_kind(board_id(LandType(0), complaint)),
            PostKind::Complaint
        );
        assert_eq!(
            post_kind(board_id(LandType(1), member)),
            PostKind::Complaint
        );
        assert_eq!(
            post_kind(board_id(LandType(0), member)),
            PostKind::MemberServices
        );
        assert_eq!(post_kind(board_id(LandType(1), tavern)), PostKind::Post);
        assert_eq!(post_kind(board_id(LandType(0), topic)), PostKind::Topic);
        assert_eq!(post_kind(board_id(LandType(1), topic)), PostKind::Post);
        let trivia = BoardRef {
            location: 0,
            section: 85,
        };
        assert_eq!(post_kind(board_id(LandType(0), trivia)), PostKind::Topic);
        assert_eq!(post_kind(board_id(LandType(1), trivia)), PostKind::Post);
        let elsewhere = BoardRef {
            location: 0x59,
            section: 85,
        };
        assert_eq!(post_kind(board_id(LandType(1), elsewhere)), PostKind::Post);
    }

    #[test]
    fn the_offender_is_the_name_after_about() {
        let text = b"From: Anna in: Tavern; about: Bruno, who said \"x\" Comments: rude\0junk";
        assert_eq!(named_offender(text), Some("Bruno"));
        assert_eq!(named_offender(b"nobody is named"), None);
        assert_eq!(named_offender(b"about: , x"), None);
    }

    #[test]
    fn every_section_name_fits_the_clients_21_bytes() {
        for section in SECTIONS {
            assert!(section.name.len() <= 21 && section.name.is_ascii());
            assert!(!section.name.contains([' ', '\r']));
        }
    }
}
