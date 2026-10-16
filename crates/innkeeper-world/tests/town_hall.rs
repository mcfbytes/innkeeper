//! The Town Hall: the account texts (34) and the Member Services and Accounting forms (37/30).
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

use std::sync::Arc;

use innkeeper_world::{
    ClientMessage, ConnectionId, MemoryStore, ObjectStore, PlayerSession, Store, World,
    COMPLAINT_BOARD_LOCATION,
};

const LOGIN: &str =
    "35 00 00 00 01 02 03 12 a1 86 01 00 01 07 07 07 07 07 07 07 07 07 07 00 41 6c 69 63 65 00";

struct Host {
    world: World,
    objects: ObjectStore,
    player: PlayerSession,
}

impl Host {
    fn new(store: Arc<MemoryStore>) -> Self {
        let mut host = Host {
            world: World::stored(store),
            objects: ObjectStore::new(),
            player: PlayerSession::new(),
        };
        let login: Vec<u8> = LOGIN
            .split_whitespace()
            .map(|pair| u8::from_str_radix(pair, 16).unwrap())
            .collect();
        host.say(&login);
        host
    }

    fn say(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let message = ClientMessage::parse(bytes).unwrap();
        let deliveries =
            self.player
                .handle(&self.world, &mut self.objects, ConnectionId(1), &message);
        deliveries
            .iter()
            .map(|delivery| delivery.message.encode())
            .collect()
    }
}

fn text_of(reply: &[u8], from: usize) -> String {
    String::from_utf8(reply[from..reply.len() - 1].to_vec()).unwrap()
}

#[test]
fn the_logon_rates_request_is_left_alone_and_later_ones_are_answered() {
    let mut host = Host::new(Arc::new(MemoryStore::new()));
    assert!(host.say(&[0x22, 4, 0, 1]).is_empty());
    let [rates] = host.say(&[0x22, 4, 0, 1]).try_into().unwrap();
    assert_eq!(rates[..4], [0x22, 4, 0, 1]);
    assert!(text_of(&rates, 4).contains("No charges"));
    assert!(host.say(&[0x22, 8, 0, 1]).is_empty());
}

#[test]
fn status_and_both_summaries_name_the_account_and_say_there_is_nothing_to_pay() {
    let mut host = Host::new(Arc::new(MemoryStore::new()));
    let [status] = host.say(&[0x22, 5, 0, 0]).try_into().unwrap();
    assert_eq!(status[..4], [0x22, 5, 0, 0]);
    let text = text_of(&status, 4);
    assert!(text.contains("100001") && text.contains("Alice") && text.contains("No charges"));
    for sub in [12u8, 15] {
        let request = [0x22, sub, 0, 1, 0xA1, 0x86, 1, 0, 0, 100, 0];
        let [summary] = host.say(&request).try_into().unwrap();
        assert_eq!(summary[..4], [0x22, sub, 0, 1]);
        let length = usize::from(u16::from_le_bytes([summary[16], summary[17]]));
        assert_eq!(summary.len(), 18 + length);
        let text = String::from_utf8(summary[18..].to_vec()).unwrap();
        assert!(text.contains("100001") && text.contains("No charges"));
    }
}

fn form(to: u8, text: &[u8]) -> Vec<u8> {
    let mut bytes = vec![37, 30, 0, 1, to, 0, 0xFE, 0];
    bytes.extend([0; 10]);
    bytes.extend([1, 0, 0, 0]);
    bytes.extend([0; 10]);
    bytes.extend([0, 0]);
    bytes.extend(b"MemberServ\0\0\0\0\0\0\0\0\0\0\0"[..10].iter());
    bytes.extend(b"User\0\0\0\0\0\0");
    bytes.extend(text);
    bytes
}

fn complaints(store: &MemoryStore) -> Vec<Vec<u8>> {
    let board = innkeeper_world::BoardId(u32::from(COMPLAINT_BOARD_LOCATION) << 8 | 2);
    store
        .posts(board)
        .unwrap()
        .into_iter()
        .map(|post| post.body)
        .collect()
}

#[test]
fn a_member_services_question_is_kept_as_a_complaint_and_a_billing_form_is_not() {
    let store = Arc::new(MemoryStore::new());
    let mut host = Host::new(Arc::clone(&store));
    assert_eq!(host.say(&form(1, b"Where is the map?\0")).len(), 1);
    assert_eq!(host.say(&form(2, b"card 4111 1111 1111 1111\0")).len(), 1);
    assert_eq!(
        complaints(&store),
        [b"Alice|Member Services: Where is the map?".to_vec()]
    );
    let board = innkeeper_world::BoardId(u32::from(COMPLAINT_BOARD_LOCATION) << 8 | 2);
    let kept = store.posts(board).unwrap();
    assert_eq!(kept[0].account, Some(innkeeper_world::AccountId(100_001)));
    // Nothing of the billing form reached any board.
    let board = innkeeper_world::BoardId(u32::from(COMPLAINT_BOARD_LOCATION) << 8 | 1);
    assert!(store.posts(board).unwrap().is_empty());
}

#[test]
fn a_password_change_is_acknowledged_with_sub_command_1() {
    let mut host = Host::new(Arc::new(MemoryStore::new()));
    let change = [44, 2, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let [ack] = host.say(&change).try_into().unwrap();
    assert_eq!(ack, [0, 0, 0, 1, 44, 1]);
}
