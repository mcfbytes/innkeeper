//! The RPG bulletin boards over the store: a fixed directory per game location, messages numbered
//! from 1 in posting order. Layouts, evidence and the INFERRED choices: docs/protocol/yserbius.md.

use tracing::{info, warn};

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
    let Some(place) = place else {
        warn!(request = %world.log.debug(request), "bulletin board request outside any land: offline");
        return offline(request);
    };
    let boards = Boards {
        world,
        land: place.land_type,
        privileged: account.user_flags & PRIVILEGED_USER_FLAGS != 0,
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
        if !is_listed(board) {
            warn!(?board, "post to a board no directory lists: dropped");
            return Ok(());
        }
        let id = self.world.store.post(self.board_id(board), body)?;
        info!(?board, post = id.0, "board post stored");
        Ok(())
    }

    fn posts(&self, board: BoardRef) -> Result<Vec<Post>, StoreError> {
        if !is_listed(board) {
            return Ok(Vec::new());
        }
        self.world.store.posts(self.board_id(board))
    }

    /// The land type in the top byte keeps Yserbius and Twinion boards of one location apart.
    fn board_id(&self, board: BoardRef) -> BoardId {
        board_id(self.land, board)
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
