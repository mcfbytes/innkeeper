//! A Shoppers Advantage session whose mall has nothing to sell.
//!
//! LSCI's mall gate (`54/27`) is answered with `54/19`, which starts Shop. An idle Shop displays
//! a `54/16` kind 1 with no catalog (`shopadv.md` sections 9 and 11), so the closed mall is that
//! notice. `54/25`, `54/24` and `54/26` are answered as well, so Shop never waits out a silence.

use crate::assumptions::{
    MALL_CLOSED_TEXT_ASSUMED, MALL_HEADER_SLOT_ASSUMED, MALL_LAUNCHES_SHOP_ASSUMED,
    MALL_NAK_UNREAD_BYTES_ASSUMED, MALL_NOTICE_FOLLOWS_JOIN_ASSUMED, MALL_OPEN_KIND_ASSUMED,
};
use crate::message::{MallHeader, MallReply, MallRequest};
use crate::HostMessage;

/// Shop appends kind 1. Kind 2 is the "Mall Down" quit (`shopadv.md` section 7.2).
const APPEND: u8 = 1;
/// Kind `0x1A` acknowledges an exit, on `54/19` and on `54/20` (`shopadv.md` section 7.2).
const EXIT_ACK: u8 = 0x1A;
/// A typed exit. The button sends `/EXIT\n` and then `54/26`; only the typed line needs `54/20`.
const TYPED_EXIT: &[u8] = b"/EXIT\r\n";

pub(crate) fn answer(request: &MallRequest) -> Vec<HostMessage> {
    match request {
        MallRequest::Status(header) => open(header),
        MallRequest::Join(header) => joined(header),
        MallRequest::Text { header, text } => line(header, text),
        MallRequest::Exit(header) => vec![control(header, EXIT_ACK)],
    }
}

fn open(header: &MallHeader) -> Vec<HostMessage> {
    let mut replies = Vec::new();
    if MALL_LAUNCHES_SHOP_ASSUMED {
        replies.push(control(header, MALL_OPEN_KIND_ASSUMED));
    }
    replies
}

fn joined(header: &MallHeader) -> Vec<HostMessage> {
    let mut replies = vec![HostMessage::Mall(MallReply::Joined(reply_header(header)))];
    if MALL_NOTICE_FOLLOWS_JOIN_ASSUMED {
        replies.push(notice(header));
    }
    replies
}

fn line(header: &MallHeader, text: &[u8]) -> Vec<HostMessage> {
    let mut replies = vec![notice(header)];
    if text == TYPED_EXIT {
        replies.push(HostMessage::Mall(MallReply::Refused {
            header: reply_header(header),
            kind: EXIT_ACK,
            unread: MALL_NAK_UNREAD_BYTES_ASSUMED,
            text: MALL_CLOSED_TEXT_ASSUMED.into(),
        }));
    }
    replies
}

fn notice(header: &MallHeader) -> HostMessage {
    HostMessage::Mall(MallReply::Output {
        header: reply_header(header),
        kind: APPEND,
        text: MALL_CLOSED_TEXT_ASSUMED.into(),
    })
}

fn control(header: &MallHeader, kind: u8) -> HostMessage {
    HostMessage::Mall(MallReply::Control {
        header: reply_header(header),
        kind,
    })
}

fn reply_header(request: &MallHeader) -> MallHeader {
    MallHeader {
        slot: MALL_HEADER_SLOT_ASSUMED,
        ..*request
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccountId, ClientMessage, ClientVersion, ConnectionId, EncodedPassword, Login, ObjectStore,
        PasswordSource, PlayerSession, World,
    };

    const ME: ConnectionId = ConnectionId(1);

    fn header() -> MallHeader {
        MallHeader {
            slot: 5,
            tokens: [0xFFFF, 0xEEEE, 0xDDDD],
            route: 0,
        }
    }

    fn logged_in() -> (World, ObjectStore, PlayerSession) {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let login = ClientMessage::Login(Login {
            land_type: crate::LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(100_001),
            password_source: PasswordSource::StoredFile,
            password: EncodedPassword([7; 10]),
            name: "guybrush".into(),
            prodigy_id: None,
        });
        let _ = player.handle(&world, &mut objects, ME, &login);
        (world, objects, player)
    }

    fn replies(request: MallRequest) -> Vec<Vec<u8>> {
        let (world, mut objects, mut player) = logged_in();
        let message = ClientMessage::Mall(request);
        player
            .handle(&world, &mut objects, ME, &message)
            .into_iter()
            .map(|delivery| delivery.message.encode())
            .collect()
    }

    fn byte(message: &[u8], index: usize) -> u8 {
        message.get(index).copied().unwrap_or(0xFF)
    }

    #[test]
    fn the_gate_launches_shop_and_shop_can_leave() {
        let launched = replies(MallRequest::Status(header()));
        let [open] = launched.as_slice() else {
            panic!("the gate answers once");
        };
        assert_eq!(open.len(), 13, "54/19 needs its kind byte");
        assert_eq!(byte(open, 1), 19);
        assert_eq!(u16::from_le_bytes([byte(open, 2), byte(open, 3)]), 5);
        assert_eq!(byte(open, 12), MALL_OPEN_KIND_ASSUMED);

        let joined = replies(MallRequest::Join(header()));
        let [token, notice] = joined.as_slice() else {
            panic!("a join answers with the token and the notice");
        };
        assert_eq!((byte(token, 1), byte(notice, 1)), (17, 16));
        assert_eq!(byte(notice, 12), APPEND);
        assert_eq!(byte(notice, 13), b'T');

        let left = replies(MallRequest::Exit(header()));
        let [ack] = left.as_slice() else {
            panic!("an exit answers once");
        };
        assert_eq!(byte(ack, 1), 19);
        assert_eq!(byte(ack, 12), EXIT_ACK);
        assert!(ack.len() >= 13);
    }

    #[test]
    fn a_typed_exit_ends_the_session_and_a_button_does_not() {
        let typed = MallRequest::Text {
            header: header(),
            text: TYPED_EXIT.to_vec(),
        };
        let ended = replies(typed);
        let [notice, nak] = ended.as_slice() else {
            panic!("a typed exit is a notice and then a nak");
        };
        assert_eq!(byte(notice, 1), 16);
        assert_eq!(byte(nak, 1), 20);
        assert_eq!(byte(nak, 12), EXIT_ACK);
        assert_eq!(byte(nak, 15), b'T');

        let button = MallRequest::Text {
            header: header(),
            text: b"/EXIT\n".to_vec(),
        };
        let shown = replies(button);
        let [notice] = shown.as_slice() else {
            panic!("a button exit is only the notice");
        };
        assert_eq!(byte(notice, 1), 16);
    }

    #[test]
    fn shop_messages_before_login_get_no_reply() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let gate = ClientMessage::Mall(MallRequest::Status(header()));
        assert!(player.handle(&world, &mut objects, ME, &gate).is_empty());
    }
}
