//! The RPG bulletin boards over the store: a fixed directory per game location, messages numbered
//! from 1 in posting order. Layouts, evidence and the INFERRED choices: docs/protocol/yserbius.md.

use tracing::{info, warn};

use crate::assumptions::{
    COMPLAINTS_ARE_KEPT_ASSUMED, COMPLAINT_SECTION_ASSUMED, COMPLAINT_TEXT_LIMIT_ASSUMED,
    ERASE_NEEDS_AUTHOR_OR_PRIVILEGE_ASSUMED,
};
use crate::message::Command;
use crate::{
    Account, BbsReply, BbsRequest, BoardId, BoardListing, BoardRef, HostMessage, LandType,
    MessagePosition, Nak, Place, Post, ReadStep, Sid, StoreError, World,
};

/// Location of the operator-only Complaints board. Posts there are kept for a year
/// (docs/server/privacy.md). The RPG client reaches it as a command 27 post, not as 27/9.
pub const COMPLAINT_BOARD_LOCATION: u16 = 0x63;

/// True for a post on the Complaints board of any land.
pub fn is_complaint_board(board: BoardId) -> bool {
    ((board.0 >> 8) & 0xFFFF) == u32::from(COMPLAINT_BOARD_LOCATION)
}

/// User-flag bits that make the client read a message's text 4 bytes later (`userFlags` & 2 or 4).
const PRIVILEGED_USER_FLAGS: u16 = 0x0006;
/// The user flag the client's own sysop level 3 needs (`RoomZeroHandler`, `hub/script.011`), which is
/// what shows its "Maintain Conferences" dialog.
const MODERATOR_USER_FLAGS: u16 = 0x0004;
/// Sections of location 0 that hold the conference (67) and trivia (84) topic texts: the hub's
/// conference list and SL's trivia list read message 1 of them, and a moderator's "Change Topics"
/// replaces it (`hub/script.120`, `SL/script.120`). No directory lists them.
const TOPIC_SECTIONS: [u16; 2] = [67, 84];
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
    account: &Account,
    place: Option<Place>,
    request: &BbsRequest,
) -> Vec<HostMessage> {
    if let BbsRequest::Complaint { about, text, .. } = request {
        complaint(world, account, about.is_some(), &text.0);
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
        let total = word(posts.len());
        let number = step_to(step, message, total);
        let body = usize::from(number)
            .checked_sub(1)
            .and_then(|index| posts.get(index))
            .map(|post| post.body.clone())
            .unwrap_or_default();
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
        let id = self.world.store.post(self.board_id(board), body)?;
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
        let post = self.world.store.post(id, body)?;
        info!(
            ?board,
            post = post.0,
            length = body.len(),
            "topic text stored"
        );
        Ok(())
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
        let allowed = !ERASE_NEEDS_AUTHOR_OR_PRIVILEGE_ASSUMED
            || self.privileged
            || is_author(&post.body, self.persona);
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

    /// The land type in the top byte keeps Yserbius and Twinion boards of one location apart.
    fn board_id(&self, board: BoardRef) -> BoardId {
        match is_topic_board(board) {
            true => board_id(LandType(0), board),
            false => board_id(self.land, board),
        }
    }
}

/// True when a post's `author|text` names `persona` as its author (case-insensitive).
fn is_author(body: &[u8], persona: &str) -> bool {
    let persona = persona.trim();
    let author = body.split(|&byte| byte == b'|').next().unwrap_or_default();
    !persona.is_empty() && author.trim_ascii().eq_ignore_ascii_case(persona.as_bytes())
}

/// A complaint from the hub or the Sketch Pad. The client says "Your complaint has been forwarded"
/// the moment it sends and waits for nothing, so nothing is answered. The text is kept as
/// `persona|Complaint: text` on the complaint board, where the account's export and delete find it
/// by persona, and only the account, the length and whether someone was named are logged.
fn complaint(world: &World, account: &Account, names_someone: bool, text: &[u8]) {
    if !COMPLAINTS_ARE_KEPT_ASSUMED {
        return;
    }
    let text = text.get(..COMPLAINT_TEXT_LIMIT_ASSUMED).unwrap_or(text);
    let text = text.strip_prefix(b"Complaint|").unwrap_or(text);
    let author = match account.persona.trim() {
        "" => format!("account {}", account.id.0),
        persona => persona.replace('|', "_"),
    };
    let mut body = format!("{author}|Complaint: ").into_bytes();
    body.extend_from_slice(text);
    let board = board_id(
        LandType(0),
        BoardRef {
            location: COMPLAINT_BOARD_LOCATION,
            section: COMPLAINT_SECTION_ASSUMED,
        },
    );
    match world.store.post(board, &body) {
        Ok(post) => info!(
            account = account.id.0,
            post = post.0,
            length = text.len(),
            names_someone,
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
    fn every_section_name_fits_the_clients_21_bytes() {
        for section in SECTIONS {
            assert!(section.name.len() <= 21 && section.name.is_ascii());
            assert!(!section.name.contains([' ', '\r']));
        }
    }
}
