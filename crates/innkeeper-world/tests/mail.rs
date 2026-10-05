//! Mail transcripts on a stored world: replies are checked byte for byte; layouts are in golden/mail.txt.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

use std::sync::Arc;

use innkeeper_world::{
    ClientMessage, ConnectionId, FixedClock, HostTime, MemoryStore, ObjectStore, PlayerSession,
    World,
};

const LOGIN_A: &str =
    "35 00 00 00 01 02 03 12 a1 86 01 00 01 07 07 07 07 07 07 07 07 07 07 00 41 6c 69 63 65 00";
const LOGIN_B: &str =
    "35 00 00 00 01 02 03 12 a2 86 01 00 01 07 07 07 07 07 07 07 07 07 07 00 42 6f 62 00";
const LOGIN_C: &str =
    "35 00 00 00 01 02 03 12 a3 86 01 00 01 07 07 07 07 07 07 07 07 07 07 00 43 61 72 6f 6c 00";
const ASSIGN: &str = "2d 01 00 01";
const SID: [u8; 2] = [0x00, 0x01];
const NOON: HostTime = HostTime {
    year_1900: 94,
    month0: 1,
    mday: 2,
    hour: 12,
    minute: 30,
    second: 15,
};

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect()
}

fn field(text: &[u8]) -> Vec<u8> {
    let mut field = text.to_vec();
    field.resize(10, 0);
    field
}

fn long(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn envelope(from: u32, addressee: &[u8], sender: &[u8], text: &[u8]) -> Vec<u8> {
    let parts = [
        vec![0; 10],
        long(from),
        field(b"subject"),
        vec![0, 0],
        field(addressee),
        field(sender),
        text.to_vec(),
    ];
    parts.concat()
}

/// `b 37, b sub, w sid` and then the fields.
fn request(sub: u8, fields: &[Vec<u8>]) -> Vec<u8> {
    [&[37, sub][..], &SID, &fields.concat()].concat()
}

fn send(sub: u8, to: u32, envelope: Vec<u8>) -> Vec<u8> {
    request(sub, &[long(to), envelope])
}

fn about_letter(sub: u8, mailbox: u32, letter: u32) -> Vec<u8> {
    request(sub, &[long(mailbox), long(letter)])
}

fn about_box(sub: u8, mailbox: u32) -> Vec<u8> {
    request(sub, &[long(mailbox)])
}

fn reply(sub: u8, fields: &[Vec<u8>]) -> Vec<u8> {
    request(sub, fields)
}

/// A host and its connected players, each with its own session.
struct Host {
    world: World,
    objects: ObjectStore,
    players: Vec<PlayerSession>,
}

impl Host {
    fn stored() -> Self {
        let mut world = World::stored(Arc::new(MemoryStore::new()));
        world.clock = Box::new(FixedClock { time: NOON });
        Host::on(world)
    }

    fn on(world: World) -> Self {
        Host {
            world,
            objects: ObjectStore::new(),
            players: Vec::new(),
        }
    }

    /// Logs a player on and has the client ask for its mailbox, as the logon does; the player's index.
    fn join(&mut self, login: &str) -> usize {
        self.players.push(PlayerSession::new());
        let player = self.players.len() - 1;
        self.say(player, &hex(login));
        player
    }

    fn say(&mut self, player: usize, bytes: &[u8]) -> Vec<Vec<u8>> {
        let message = ClientMessage::parse(bytes).unwrap();
        let session = &mut self.players[player];
        let deliveries = session.handle(&self.world, &mut self.objects, ConnectionId(1), &message);
        deliveries
            .iter()
            .map(|delivery| delivery.message.encode())
            .collect()
    }

    fn mailbox_of(&mut self, player: usize) -> u32 {
        let replies = self.say(player, &hex(ASSIGN));
        let [answer] = replies.as_slice() else {
            panic!("expected one answer, got {replies:?}");
        };
        assert_eq!(answer[..5], [45, 1, 0, 1, 1]);
        u32::from_le_bytes(answer[5..9].try_into().unwrap())
    }
}

#[test]
fn a_letter_goes_from_one_account_to_another() {
    let mut host = Host::stored();
    let (alice, bob) = (host.join(LOGIN_A), host.join(LOGIN_B));
    assert_eq!(host.mailbox_of(alice), 1);
    assert_eq!(host.mailbox_of(bob), 2);

    let nothing = || reply(18, &[long(2), vec![0, 0]]);
    assert_eq!(host.say(bob, &about_box(18, 2)), [nothing()]);

    let text = b"Caf\xe9 at noon\r\nsecond line\0";
    let sent = host.say(alice, &send(24, 2, envelope(1, b"Bob", b"Alice", text)));
    let delivered = [0, 0, 0, 1, 37, 24, 0, 2, 0, 0, 0];
    assert_eq!(sent, [delivered]);

    let waiting = reply(18, &[long(2), vec![3, 0]]);
    assert_eq!(host.say(bob, &about_box(18, 2)), [waiting]);

    let listing = host.say(bob, &about_box(19, 2));
    let row = [
        vec![0],
        long(1),
        long(1),
        field(b"subject"),
        field(b"Alice"),
        vec![0, 0],
        vec![94, 1, 2, 12, 30, 15],
        field(b"Bob"),
    ];
    assert_eq!(listing, [reply(19, &[long(2), row.concat()])]);

    let read = host.say(bob, &about_letter(25, 2, 1));
    assert_eq!(read, [reply(25, &[long(2), long(1), text.to_vec()])]);
    assert_eq!(host.say(bob, &about_letter(25, 2, 1)), read);

    assert!(host.say(bob, &about_letter(26, 2, 1)).is_empty());
    assert_eq!(host.say(bob, &about_box(18, 2)), [nothing()]);
    let gone = [1, 0, 0, 1, 37, 25, 14, 2, 0, 0, 0];
    assert_eq!(host.say(bob, &about_letter(25, 2, 1)), [gone]);
}

#[test]
fn the_mailbox_number_is_stable_across_a_second_assignment() {
    let mut host = Host::stored();
    let (alice, bob) = (host.join(LOGIN_A), host.join(LOGIN_B));
    let first = host.mailbox_of(alice);
    assert_eq!(host.mailbox_of(bob), first + 1);
    assert_eq!(host.mailbox_of(alice), first);
    let again = host.join(LOGIN_A);
    assert_eq!(host.mailbox_of(again), first);
}

#[test]
fn letters_arrive_in_order_and_each_reader_sees_only_their_box() {
    let mut host = Host::stored();
    let (alice, bob) = (host.join(LOGIN_A), host.join(LOGIN_B));
    host.mailbox_of(alice);
    host.mailbox_of(bob);
    for text in [&b"one\0"[..], b"two\0"] {
        host.say(alice, &send(24, 2, envelope(1, b"Bob", b"Alice", text)));
    }
    let listing = host.say(bob, &about_box(19, 2));
    let rows = &listing[0][8..];
    assert_eq!(rows.len(), 2 * 47);
    assert_eq!(rows[1..5], long(1));
    assert_eq!(rows[48..52], long(2));

    let mine = reply(19, &[long(1)]);
    assert_eq!(host.say(alice, &about_box(19, 1)), [mine]);
}

#[test]
fn only_the_requesters_own_box_can_be_read_or_emptied() {
    let mut host = Host::stored();
    let (alice, bob) = (host.join(LOGIN_A), host.join(LOGIN_B));
    host.mailbox_of(alice);
    host.mailbox_of(bob);
    host.say(alice, &send(24, 2, envelope(1, b"Bob", b"Alice", b"hi\0")));
    for sub in [18, 19] {
        let refused = [1, 0, 0, 1, 37, sub, 12, 2, 0, 0, 0];
        assert_eq!(host.say(alice, &about_box(sub, 2)), [refused]);
    }
    for sub in [25, 26] {
        let refused = [1, 0, 0, 1, 37, sub, 12, 2, 0, 0, 0];
        assert_eq!(host.say(alice, &about_letter(sub, 2, 1)), [refused]);
    }
    assert_eq!(host.say(bob, &about_box(19, 2))[0].len(), 8 + 47);
}

#[test]
fn a_letter_to_a_box_nobody_has_is_refused() {
    let mut host = Host::stored();
    let alice = host.join(LOGIN_A);
    host.mailbox_of(alice);
    let letter = envelope(1, b"Nobody", b"Alice", b"hi\0");
    let no_such_box = [1, 0, 0, 1, 37, 24, 6, 9, 0, 0, 0];
    assert_eq!(host.say(alice, &send(24, 9, letter.clone())), [no_such_box]);
    let out_of_range = [1, 0, 0, 1, 37, 24, 9, 0, 0, 0, 0];
    assert_eq!(host.say(alice, &send(24, 0, letter)), [out_of_range]);
}

#[test]
fn a_forwarded_letter_reaches_the_third_box_under_the_new_name() {
    let mut host = Host::stored();
    let (alice, bob, carol) = (host.join(LOGIN_A), host.join(LOGIN_B), host.join(LOGIN_C));
    for player in [alice, bob, carol] {
        host.mailbox_of(player);
    }
    host.say(alice, &send(24, 2, envelope(1, b"Bob", b"Alice", b"hi\0")));
    let forward = request(
        27,
        &[
            long(2),
            long(1),
            long(3),
            field(b"Carol"),
            b"Hmmm...\0".to_vec(),
        ],
    );
    let forwarded = [0, 0, 0, 1, 37, 27, 0, 3, 0, 0, 0];
    assert_eq!(host.say(bob, &forward), [forwarded]);
    let listing = host.say(carol, &about_box(19, 3));
    assert_eq!(listing[0][8 + 29 + 8..8 + 47], field(b"Carol"));
    assert_eq!(host.say(bob, &about_box(19, 2))[0].len(), 8 + 47);
}

#[test]
fn a_form_for_a_service_box_is_dropped_and_the_service_lists_are_empty() {
    let mut host = Host::stored();
    let alice = host.join(LOGIN_A);
    host.mailbox_of(alice);
    let form = send(
        30,
        0x10000,
        envelope(1, b"MemberServ", b"User", b"bill me\0"),
    );
    assert!(host.say(alice, &form).is_empty());
    assert_eq!(host.say(alice, &about_box(31, 1)), [reply(19, &[long(1)])]);
    let system_list = [37, 32, 0, 0, 0, 0, 0, 0];
    assert_eq!(host.say(alice, &system_list), [system_list]);
}

#[test]
fn a_listing_shows_the_first_64_letters() {
    let mut host = Host::stored();
    let (alice, bob) = (host.join(LOGIN_A), host.join(LOGIN_B));
    host.mailbox_of(alice);
    host.mailbox_of(bob);
    for _ in 0..65 {
        host.say(alice, &send(24, 2, envelope(1, b"Bob", b"Alice", b"hi\0")));
    }
    assert_eq!(host.say(bob, &about_box(19, 2))[0].len(), 8 + 64 * 47);
}

#[test]
fn the_open_book_has_no_mailboxes() {
    let mut host = Host::on(World::stock());
    let alice = host.join(LOGIN_A);
    let refused = [45, 1, 0, 1, 2, 0, 0, 0, 0];
    assert_eq!(host.say(alice, &hex(ASSIGN)), [refused]);
    let closed = [1, 0, 0, 1, 37, 18, 21, 1, 0, 0, 0];
    assert_eq!(host.say(alice, &about_box(18, 1)), [closed]);
    let letter = envelope(1, b"Bob", b"Alice", b"hi\0");
    let undelivered = [1, 0, 0, 1, 37, 24, 21, 2, 0, 0, 0];
    assert_eq!(host.say(alice, &send(24, 2, letter)), [undelivered]);
}

#[test]
fn requests_before_login_get_no_reply() {
    let mut host = Host::stored();
    host.players.push(PlayerSession::new());
    assert!(host.say(0, &hex(ASSIGN)).is_empty());
    assert!(host.say(0, &about_box(18, 1)).is_empty());
}

#[test]
fn the_mail_room_learns_the_boxes_of_its_own_account_only() {
    let mut host = Host::stored();
    let (alice, bob) = (host.join(LOGIN_A), host.join(LOGIN_B));
    host.mailbox_of(alice);
    host.mailbox_of(bob);
    let boxes = reply(17, &[long(100_001), long(1)]);
    assert_eq!(host.say(alice, &about_box(17, 100_001)), [boxes]);
    let refused = [1, 0, 0, 1, 37, 17, 12, 0xa2, 0x86, 1, 0];
    assert_eq!(host.say(alice, &about_box(17, 100_002)), [refused]);
}
