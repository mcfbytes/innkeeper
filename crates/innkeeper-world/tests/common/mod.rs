//! Scripted players on one object store, every delivery checked byte for byte and in order.
//! Line syntax: "A > name | hex" is a message from player A, "B < name | hex" a delivery to B
//! that the latest step caused, and "A ! hang up" drops A's connection.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort
#![allow(dead_code)] // each test binary uses its own part

use std::collections::VecDeque;

use innkeeper_world::{ClientMessage, ConnectionId, Delivery, ObjectStore, PlayerSession, World};

pub const LOGIN: &str = "35 00 00 00 01 02 03 12 a1 86 01 00 01 1d 7a 01 66 16 66 18 73 03 00 00 \
                     67 75 79 62 72 75 73 68 00";
pub const LOGIN_ACK: &str = "00 00 00 00 16 00 00 00 00 00";

pub struct Table {
    pub world: World,
    pub objects: ObjectStore,
    players: Vec<(char, PlayerSession)>,
    pending: VecDeque<Delivery>,
}

impl Table {
    pub fn new() -> Self {
        Table {
            world: World::stock(),
            objects: ObjectStore::new(),
            players: Vec::new(),
            pending: VecDeque::new(),
        }
    }

    pub fn run(&mut self, transcript: &str) {
        let transcript = transcript
            .replace("{login}", LOGIN)
            .replace("{login ack}", LOGIN_ACK);
        for (index, line) in transcript.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            self.step(line, index + 1);
        }
        self.assert_nothing_pending(transcript.lines().count());
    }

    fn step(&mut self, line: &str, number: usize) {
        let (head, bytes) = line.split_once('|').unwrap_or((line, ""));
        let mut words = head.split_whitespace();
        let who = words.next().unwrap().chars().next().unwrap();
        let bytes = hex(bytes);
        match words.next().unwrap() {
            ">" => {
                self.assert_nothing_pending(number);
                let message = ClientMessage::parse(&bytes).unwrap();
                let connection = connection(who);
                let player = seat(&mut self.players, who);
                let caused = player.handle(&self.world, &mut self.objects, connection, &message);
                self.pending.extend(caused);
            }
            "<" => {
                let delivery = self.pending.pop_front();
                let delivery = delivery.unwrap_or_else(|| panic!("line {number}: nothing sent"));
                let to = delivery.to;
                let sent = delivery.message.encode();
                assert_eq!(
                    (to, spaced(&sent)),
                    (connection(who), spaced(&bytes)),
                    "line {number}"
                );
            }
            "!" => {
                self.assert_nothing_pending(number);
                self.pending
                    .extend(self.objects.disconnect(connection(who)));
            }
            other => panic!("line {number}: unknown direction {other:?}"),
        }
    }

    fn assert_nothing_pending(&self, number: usize) {
        assert!(
            self.pending.is_empty(),
            "line {number}: also sent {:?}",
            self.pending
        );
    }
}

fn seat(players: &mut Vec<(char, PlayerSession)>, who: char) -> &mut PlayerSession {
    if !players.iter().any(|(name, _)| *name == who) {
        players.push((who, PlayerSession::new()));
    }
    let (_, player) = players.iter_mut().find(|(name, _)| *name == who).unwrap();
    player
}

fn connection(who: char) -> ConnectionId {
    ConnectionId(u64::from(who))
}

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect()
}

fn spaced(bytes: &[u8]) -> String {
    let pairs: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    pairs.join(" ")
}
