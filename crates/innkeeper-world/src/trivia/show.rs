//! One Countdown game per room, driven by `Shows::tick` and the world's clock.

use std::cmp::Reverse;

use crate::message::{NtnMessage, RoundWinner, ScoreWords};
use crate::{ConnectionId, Delivery};

use crate::message::TriviaHeader;

use super::bank::shuffle_answers;
use super::text::LINE;
use super::{game, Seat, Shows, HANDLE_LIMIT};

/// Countdown clock in seconds. The manuals: a faster correct answer scores more.
pub(crate) const QUESTION_SECONDS: u16 = 20;
/// Starting payout of a correct Countdown answer, when type 23 starts the clock.
const POINTS_AT_START: u16 = 1000;
/// Payout of a correct Countdown answer when the clock hits zero.
const POINTS_AT_END: u16 = 100;
/// Countdown penalty for a wrong answer. The game total floors at zero.
const WRONG_PENALTY: u32 = 250;
/// Pause after the score packets before the next question.
const BETWEEN_QUESTIONS_MS: u64 = 5_000;
/// Break after type 24. Type 22 sends this many minutes.
const BETWEEN_GAMES_MS: u64 = 60_000;
const BETWEEN_GAMES_MINUTES: u8 = 1;
/// Questions in one Countdown game.
const QUESTIONS_PER_GAME: u8 = 15;
/// Pause after type 39/2 so the handle dialog can close before type 4.
const AFTER_HANDLE_MS: u64 = 2_000;

#[derive(Debug, Default)]
pub(super) enum Phase {
    #[default]
    Idle,
    /// First handle is in; wait so type 39 and type 2 land before the question.
    Starting {
        until_ms: u64,
    },
    Asking(Asked),
    Pause {
        next: u8,
        until_ms: u64,
    },
    Intermission {
        until_ms: u64,
    },
}

#[derive(Debug)]
pub(super) struct Asked {
    number: u8,
    total: u8,
    correct: u8,
    started_ms: u64,
    ends_ms: u64,
    answers: Vec<[u8; LINE]>,
    lines: [[u8; LINE]; 4],
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Answer {
    choice: u8,
    at_ms: u64,
}

impl Shows {
    pub(crate) fn tick(&mut self, now: u64) -> Vec<Delivery> {
        let mut out = Vec::new();
        for index in 0..super::ROOMS {
            out.extend(self.advance(index, now));
        }
        out
    }

    pub(crate) fn take_answer(&mut self, connection: ConnectionId, text: &[u8], now: u64) {
        let Some(place) = self.place_of(connection) else {
            return;
        };
        let Some(choice) = choice_of(text) else {
            return;
        };
        let Some(room) = self.rooms.get_mut(place.room) else {
            return;
        };
        let Phase::Asking(asked) = &room.phase else {
            return;
        };
        if now > asked.ends_ms {
            return;
        }
        let count = asked.answers.len();
        let Some(seat) = room.seats.get_mut(place.seat) else {
            return;
        };
        if seat.answered.is_some() || seat.handle.is_none() {
            return;
        }
        if usize::from(choice) > count {
            return;
        }
        seat.answered = Some(Answer { choice, at_ms: now });
    }

    fn advance(&mut self, index: usize, now: u64) -> Vec<Delivery> {
        let phase = self.rooms.get(index).map(|room| &room.phase);
        match phase {
            Some(Phase::Idle) => self.queue_start(index, now),
            Some(Phase::Starting { until_ms }) if now >= *until_ms => {
                self.begin(index, 1, now, true)
            }
            Some(Phase::Asking(asked)) if now >= asked.ends_ms => self.finish(index, now),
            Some(Phase::Pause { next, until_ms }) if now >= *until_ms => {
                let next = *next;
                self.begin(index, next, now, false)
            }
            Some(Phase::Intermission { until_ms }) if now >= *until_ms => {
                self.begin(index, 1, now, true)
            }
            _ => Vec::new(),
        }
    }

    fn queue_start(&mut self, index: usize, now: u64) -> Vec<Delivery> {
        let ready = self
            .rooms
            .get(index)
            .is_some_and(|room| room.seats.iter().any(|seat| seat.handle.is_some()));
        if !ready {
            return Vec::new();
        }
        if let Some(room) = self.rooms.get_mut(index) {
            room.phase = Phase::Starting {
                until_ms: now.saturating_add(AFTER_HANDLE_MS),
            };
        }
        Vec::new()
    }

    fn begin(&mut self, index: usize, number: u8, now: u64, new_game: bool) -> Vec<Delivery> {
        let ready = self
            .rooms
            .get(index)
            .is_some_and(|room| room.seats.iter().any(|seat| seat.handle.is_some()));
        if !ready {
            if let Some(room) = self.rooms.get_mut(index) {
                room.phase = Phase::Idle;
            }
            return Vec::new();
        }
        let Some(item) = self.bags.get_mut(index).and_then(|bag| bag.draw()) else {
            if let Some(room) = self.rooms.get_mut(index) {
                room.phase = Phase::Idle;
            }
            return Vec::new();
        };
        let mut answers = item.answers;
        let seed = now ^ u64::from(u32::try_from(index).unwrap_or(0)) << 8 ^ u64::from(number);
        let correct = shuffle_answers(&mut answers, seed);
        let asked = Asked {
            number,
            total: QUESTIONS_PER_GAME,
            correct,
            started_ms: now,
            ends_ms: now.saturating_add(u64::from(QUESTION_SECONDS) * 1000),
            answers,
            lines: item.lines,
        };
        let Some(room) = self.rooms.get_mut(index) else {
            return Vec::new();
        };
        if new_game {
            for seat in &mut room.seats {
                seat.score = 0;
                seat.answered = None;
            }
        }
        room.phase = Phase::Asking(asked);
        self.deal(index)
    }

    fn deal(&self, index: usize) -> Vec<Delivery> {
        let Some(room) = self.rooms.get(index) else {
            return Vec::new();
        };
        let Phase::Asking(asked) = &room.phase else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for seat in room.seats.iter().filter(|seat| seat.handle.is_some()) {
            let header = seat.header();
            out.push(to(
                seat.connection,
                header,
                NtnMessage::TickdownSetup {
                    kind: 0,
                    seconds: QUESTION_SECONDS,
                    points: POINTS_AT_START,
                    clues: 0,
                },
            ));
            out.push(to(
                seat.connection,
                header,
                NtnMessage::Question {
                    number: asked.number,
                    total: asked.total,
                    lines: asked.lines,
                },
            ));
            out.push(to(
                seat.connection,
                header,
                NtnMessage::Answers {
                    lines: asked.answers.clone(),
                },
            ));
            out.push(to(
                seat.connection,
                header,
                NtnMessage::StartTickdown { iteration: 0 },
            ));
        }
        out
    }

    fn finish(&mut self, index: usize, now: u64) -> Vec<Delivery> {
        let Some(room) = self.rooms.get_mut(index) else {
            return Vec::new();
        };
        let Phase::Asking(asked) = &room.phase else {
            return Vec::new();
        };
        let asked = Asked {
            number: asked.number,
            total: asked.total,
            correct: asked.correct,
            started_ms: asked.started_ms,
            ends_ms: asked.ends_ms,
            answers: asked.answers.clone(),
            lines: asked.lines,
        };
        apply_scores(room, &asked);
        let mut out = score_packets(room, &asked);
        let last = asked.number >= asked.total;
        if last {
            out.extend(winners(room));
            out.extend(over(room));
            room.phase = Phase::Intermission {
                until_ms: now.saturating_add(BETWEEN_GAMES_MS),
            };
        } else {
            room.phase = Phase::Pause {
                next: asked.number.saturating_add(1),
                until_ms: now.saturating_add(BETWEEN_QUESTIONS_MS),
            };
        }
        for seat in &mut room.seats {
            seat.answered = None;
        }
        out
    }
}

fn apply_scores(room: &mut super::Room, asked: &Asked) {
    let limit = asked.ends_ms.saturating_sub(asked.started_ms);
    for seat in &mut room.seats {
        if seat.handle.is_none() {
            continue;
        }
        let (this, delta) = payout(
            asked.correct,
            seat.answered.as_ref(),
            asked.started_ms,
            limit,
        );
        seat.last = this;
        if delta < 0 {
            seat.score = seat.score.saturating_sub(delta.unsigned_abs());
        } else {
            seat.score = seat.score.saturating_add(u32::try_from(delta).unwrap_or(0));
        }
    }
}

fn payout(correct: u8, answer: Option<&Answer>, started: u64, limit: u64) -> (u16, i32) {
    let Some(answer) = answer else {
        return (0, 0);
    };
    if answer.choice != correct {
        return (0, -i32::try_from(WRONG_PENALTY).unwrap_or(250));
    }
    let elapsed = answer.at_ms.saturating_sub(started).min(limit);
    let drop = u64::from(POINTS_AT_START.saturating_sub(POINTS_AT_END));
    let earned = match limit {
        0 => POINTS_AT_START,
        limit => {
            let value = u64::from(POINTS_AT_START) - drop * elapsed / limit;
            u16::try_from(value).unwrap_or(POINTS_AT_END)
        }
    };
    (earned, i32::from(earned))
}

fn score_packets(room: &super::Room, asked: &Asked) -> Vec<Delivery> {
    let ranks = ranks(&room.seats);
    let mut out = Vec::new();
    for seat in room.seats.iter().filter(|seat| seat.handle.is_some()) {
        out.push(to(
            seat.connection,
            seat.header(),
            NtnMessage::AnswerCorrect {
                answer: asked.correct,
            },
        ));
    }
    for seat in room.seats.iter().filter(|seat| seat.answered.is_some()) {
        let choice = seat.answered.map(|a| a.choice).unwrap_or(0);
        out.push(to(
            seat.connection,
            seat.header(),
            NtnMessage::AnswerRegistered {
                kind: 0,
                answer: choice,
                points: seat.last,
                points_again: seat.last,
            },
        ));
    }
    for (index, seat) in room.seats.iter().enumerate() {
        if seat.handle.is_none() {
            continue;
        }
        let rank = ranks.get(index).copied().unwrap_or(0);
        let total = u16::try_from(seat.score.min(u32::from(u16::MAX))).unwrap_or(u16::MAX);
        out.push(to(
            seat.connection,
            seat.header(),
            NtnMessage::Score(ScoreWords {
                score_low: seat.last,
                score_high: 0,
                round_low: total,
                round_high: 0,
                round_rank: rank,
                total_low: total,
                total_high: 0,
                total_rank: rank,
            }),
        ));
    }
    out
}

fn ranks(seats: &[Seat]) -> Vec<u16> {
    let mut order: Vec<usize> = seats
        .iter()
        .enumerate()
        .filter(|(_, seat)| seat.handle.is_some())
        .map(|(index, _)| index)
        .collect();
    order.sort_by_key(|&index| {
        (
            Reverse(seats.get(index).map(|seat| seat.score).unwrap_or(0)),
            index,
        )
    });
    let mut ranks = vec![0u16; seats.len()];
    for (rank, index) in order.iter().enumerate() {
        if let Some(slot) = ranks.get_mut(*index) {
            *slot = u16::try_from(rank.saturating_add(1)).unwrap_or(u16::MAX);
        }
    }
    ranks
}

fn winners(room: &super::Room) -> Vec<Delivery> {
    let mut order: Vec<&Seat> = room
        .seats
        .iter()
        .filter(|seat| seat.handle.is_some())
        .collect();
    order.sort_by_key(|seat| Reverse(seat.score));
    order.truncate(3);
    let records: Vec<RoundWinner> = order
        .iter()
        .enumerate()
        .map(|(rank, seat)| {
            let total = u16::try_from(seat.score.min(u32::from(u16::MAX))).unwrap_or(u16::MAX);
            RoundWinner {
                label: label(rank),
                score_low: total,
                score_high: 0,
                name: name6(seat.handle.as_deref().unwrap_or(&[])),
                pad: [0; 4],
            }
        })
        .collect();
    let mut out = Vec::new();
    for seat in room.seats.iter().filter(|seat| seat.handle.is_some()) {
        out.push(to(
            seat.connection,
            seat.header(),
            NtnMessage::RoundWinners {
                winners: records.clone(),
            },
        ));
    }
    out
}

fn over(room: &super::Room) -> Vec<Delivery> {
    let mut out = Vec::new();
    for seat in room.seats.iter().filter(|seat| seat.handle.is_some()) {
        let header = seat.header();
        out.push(to(
            seat.connection,
            header,
            NtnMessage::GameOver { flag: 0 },
        ));
        out.push(to(
            seat.connection,
            header,
            NtnMessage::MinutesToNext {
                minutes: BETWEEN_GAMES_MINUTES,
            },
        ));
    }
    out
}

fn label(rank: usize) -> [u8; 10] {
    let mut out = [b' '; 10];
    let digit = b'1'.saturating_add(u8::try_from(rank).unwrap_or(0));
    if let Some(slot) = out.first_mut() {
        *slot = digit;
    }
    out
}

fn name6(handle: &[u8]) -> [u8; HANDLE_LIMIT] {
    let mut out = [b' '; HANDLE_LIMIT];
    let n = handle.len().min(HANDLE_LIMIT);
    if let (Some(slot), Some(src)) = (out.get_mut(..n), handle.get(..n)) {
        slot.copy_from_slice(src);
    }
    out
}

fn choice_of(text: &[u8]) -> Option<u8> {
    let line = super::line_of(text);
    match line {
        b"1" => Some(1),
        b"2" => Some(2),
        b"3" => Some(3),
        b"4" => Some(4),
        b"5" => Some(5),
        _ => None,
    }
}

fn to(connection: ConnectionId, header: TriviaHeader, message: NtnMessage) -> Delivery {
    Delivery::new(connection, game(header, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ScoreWords, TriviaReply};
    use crate::trivia::text::{fit, RawQuestion};
    use crate::trivia::TriviaPools;
    use crate::NTN_SERVER;

    const A: ConnectionId = ConnectionId(1);
    const B: ConnectionId = ConnectionId(2);
    const T0: u64 = 1_000_000;

    fn header(sid: u16) -> TriviaHeader {
        TriviaHeader {
            server: NTN_SERVER,
            number: 0,
            host: 0,
            remote: 0,
            sid,
        }
    }

    fn raw() -> RawQuestion {
        RawQuestion {
            question: "Who founded Sierra?".into(),
            correct: "Ken Williams".into(),
            incorrect: vec!["Al Lowe".into(), "Ron Gilbert".into()],
            boolean: false,
        }
    }

    fn ntn(delivery: &Delivery) -> Option<&NtnMessage> {
        match &delivery.message {
            crate::HostMessage::Trivia(TriviaReply::Game { message, .. }) => Some(message),
            _ => None,
        }
    }

    fn trimmed(line: &[u8]) -> &[u8] {
        match line.iter().rposition(|byte| *byte != b' ') {
            Some(end) => line.get(..=end).unwrap_or(line),
            None => b"",
        }
    }

    fn choice_for(deliveries: &[Delivery], text: &[u8]) -> u8 {
        for delivery in deliveries {
            if let Some(NtnMessage::Answers { lines }) = ntn(delivery) {
                for (index, line) in lines.iter().enumerate() {
                    if trimmed(line) == text {
                        return u8::try_from(index + 1).unwrap_or(1);
                    }
                }
            }
        }
        panic!("correct answer {text:?} not in type 5");
    }

    fn score_of(deliveries: &[Delivery], who: ConnectionId) -> ScoreWords {
        deliveries
            .iter()
            .filter(|delivery| delivery.to == who)
            .filter_map(ntn)
            .find_map(|message| match message {
                NtnMessage::Score(words) => Some(*words),
                _ => None,
            })
            .expect("type 10")
    }

    #[test]
    fn two_players_one_right_fast_one_wrong_get_the_countdown_scores() {
        let item = fit(&raw()).unwrap();
        let mut shows = Shows::new();
        shows.set_pools(
            TriviaPools {
                adventure: vec![item],
                ..TriviaPools::default()
            },
            1,
        );
        assert_eq!(shows.join(A, Some(1), 7, header(0x10)).len(), 3);
        assert_eq!(
            shows
                .text(A, b"GUY\r\n", T0, crate::LogPolicy::Public)
                .len(),
            2
        );
        assert_eq!(shows.join(B, Some(1), 7, header(0x11)).len(), 3);
        assert_eq!(
            shows
                .text(B, b"MAX\r\n", T0, crate::LogPolicy::Public)
                .len(),
            2
        );
        assert!(shows.tick(T0).is_empty());
        let start_at = T0 + AFTER_HANDLE_MS;
        let start = shows.tick(start_at);
        let right = choice_for(&start, b"Ken Williams");
        let wrong = if right == 1 { 2 } else { 1 };
        shows.take_answer(A, format!("{right}\r\n").as_bytes(), start_at);
        shows.take_answer(B, format!("{wrong}\r\n").as_bytes(), start_at + 1_000);
        let end = shows.tick(start_at + u64::from(QUESTION_SECONDS) * 1000);
        let a = score_of(&end, A);
        let b = score_of(&end, B);
        assert_eq!(
            a,
            ScoreWords {
                score_low: POINTS_AT_START,
                score_high: 0,
                round_low: POINTS_AT_START,
                round_high: 0,
                round_rank: 1,
                total_low: POINTS_AT_START,
                total_high: 0,
                total_rank: 1,
            }
        );
        assert_eq!(
            b,
            ScoreWords {
                score_low: 0,
                score_high: 0,
                round_low: 0,
                round_high: 0,
                round_rank: 2,
                total_low: 0,
                total_high: 0,
                total_rank: 2,
            }
        );
        let eights: Vec<_> = end
            .iter()
            .filter_map(ntn)
            .filter(|message| matches!(message, NtnMessage::AnswerCorrect { .. }))
            .collect();
        assert_eq!(eights.len(), 2);
    }
}
