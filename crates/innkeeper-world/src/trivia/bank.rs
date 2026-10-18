//! Shuffle-bags for the three rooms. Fitting is `super::text`.

use super::text::{fit, Item, RawQuestion};

/// Open Trivia DB row, category and all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenQuestion {
    pub category: String,
    pub question: RawQuestion,
}

/// How many questions each room kept after the wrap and ASCII filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolCounts {
    pub adventure: usize,
    pub general: usize,
    pub screens: usize,
    pub dropped_ours: usize,
    pub dropped_open: usize,
}

/// Fitted questions split by room. The show owns the shuffle-bags.
#[derive(Clone, Debug, Default)]
pub struct TriviaPools {
    pub adventure: Vec<Item>,
    pub general: Vec<Item>,
    pub screens: Vec<Item>,
    pub screens_ours: Vec<Item>,
}

impl TriviaPools {
    /// Room 1 is our bank, room 2 is OpenTDB outside Entertainment, room 3 is the screen
    /// categories plus our bank mixed in at draw time.
    pub fn from_banks(ours: Vec<RawQuestion>, open: Vec<OpenQuestion>) -> (Self, PoolCounts) {
        let mut counts = PoolCounts::default();
        let mut adventure = Vec::new();
        for raw in &ours {
            match fit(raw) {
                Some(item) => adventure.push(item),
                None => counts.dropped_ours += 1,
            }
        }
        let mut general = Vec::new();
        let mut screens = Vec::new();
        for row in &open {
            match fit(&row.question) {
                Some(item) if is_screens(&row.category) => screens.push(item),
                Some(item) if is_general(&row.category) => general.push(item),
                Some(_) | None => counts.dropped_open += 1,
            }
        }
        counts.adventure = adventure.len();
        counts.general = general.len();
        counts.screens = screens.len();
        let screens_ours = adventure.clone();
        (
            TriviaPools {
                adventure,
                general,
                screens,
                screens_ours,
            },
            counts,
        )
    }
}

/// Entertainment: Video Games/Film/Television/Board Games/Comics/Cartoon.
fn is_screens(category: &str) -> bool {
    const NAMES: &[&str] = &[
        "Entertainment: Video Games",
        "Entertainment: Film",
        "Entertainment: Television",
        "Entertainment: Board Games",
        "Entertainment: Comics",
        "Entertainment: Cartoon & Animations",
    ];
    NAMES.contains(&category)
}

fn is_general(category: &str) -> bool {
    !category.starts_with("Entertainment:")
}

/// One room's draw order. Room 3 pulls our bank on every fourth question.
#[derive(Debug)]
pub(crate) enum RoomBag {
    Empty,
    One(Bag),
    Mix { ours: Bag, rest: Bag, step: u8 },
}

impl RoomBag {
    pub(crate) fn single(items: Vec<Item>, seed: u64) -> Self {
        match items.is_empty() {
            true => RoomBag::Empty,
            false => RoomBag::One(Bag::new(items, seed)),
        }
    }

    pub(crate) fn mixed(ours: Vec<Item>, rest: Vec<Item>, seed: u64) -> Self {
        match (ours.is_empty(), rest.is_empty()) {
            (true, true) => RoomBag::Empty,
            (true, false) => RoomBag::One(Bag::new(rest, seed)),
            (false, true) => RoomBag::One(Bag::new(ours, seed)),
            (false, false) => RoomBag::Mix {
                ours: Bag::new(ours, seed),
                rest: Bag::new(rest, seed ^ 1),
                step: 0,
            },
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        matches!(self, RoomBag::Empty)
    }

    pub(crate) fn draw(&mut self) -> Option<Item> {
        match self {
            RoomBag::Empty => None,
            RoomBag::One(bag) => bag.draw(),
            RoomBag::Mix { ours, rest, step } => {
                let from_ours = *step == 0;
                *step = (*step + 1) % 4;
                if from_ours {
                    ours.draw().or_else(|| rest.draw())
                } else {
                    rest.draw().or_else(|| ours.draw())
                }
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Bag {
    items: Vec<Item>,
    order: Vec<usize>,
    next: usize,
    rng: Rng,
}

impl Bag {
    fn new(items: Vec<Item>, seed: u64) -> Self {
        let mut bag = Bag {
            items,
            order: Vec::new(),
            next: 0,
            rng: Rng(seed | 1),
        };
        bag.reshuffle();
        bag
    }

    fn draw(&mut self) -> Option<Item> {
        if self.items.is_empty() {
            return None;
        }
        if self.next >= self.order.len() {
            self.reshuffle();
        }
        let index = *self.order.get(self.next)?;
        self.next += 1;
        self.items.get(index).cloned()
    }

    fn reshuffle(&mut self) {
        self.order = (0..self.items.len()).collect();
        shuffle(&mut self.order, &mut self.rng);
        self.next = 0;
    }
}

#[derive(Debug)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        match n {
            0 => 0,
            n => (self.next() as usize) % n,
        }
    }
}

fn shuffle<T>(items: &mut [T], rng: &mut Rng) {
    let len = items.len();
    if len < 2 {
        return;
    }
    for i in (1..len).rev() {
        let j = rng.below(i + 1);
        items.swap(i, j);
    }
}

pub(crate) fn shuffle_answers(answers: &mut Vec<[u8; super::text::LINE]>, seed: u64) -> u8 {
    let mut rng = Rng(seed | 1);
    let mut order: Vec<usize> = (0..answers.len()).collect();
    shuffle(&mut order, &mut rng);
    let correct = order.iter().position(|&index| index == 0).unwrap_or(0);
    let shuffled: Vec<_> = order
        .iter()
        .filter_map(|index| answers.get(*index).copied())
        .collect();
    *answers = shuffled;
    u8::try_from(correct.saturating_add(1)).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trivia::text::RawQuestion;

    fn q(text: &str) -> RawQuestion {
        RawQuestion {
            question: text.into(),
            correct: "Yes".into(),
            incorrect: vec!["No".into()],
            boolean: false,
        }
    }

    #[test]
    fn rooms_split_entertainment_from_the_rest() {
        let ours = vec![q("Who founded Sierra On-Line?")];
        let open = vec![
            OpenQuestion {
                category: "History".into(),
                question: q("In which year did World War II end?"),
            },
            OpenQuestion {
                category: "Entertainment: Film".into(),
                question: q("Who directed Jurassic Park?"),
            },
            OpenQuestion {
                category: "Entertainment: Music".into(),
                question: q("Who wrote Hey Jude?"),
            },
        ];
        let (pools, counts) = TriviaPools::from_banks(ours, open);
        assert_eq!(counts.adventure, 1);
        assert_eq!(counts.general, 1);
        assert_eq!(counts.screens, 1);
        assert_eq!(counts.dropped_open, 1);
        assert_eq!(pools.screens_ours.len(), 1);
    }

    #[test]
    fn a_bag_reshuffles_only_after_it_is_exhausted() {
        let items = vec![
            fit(&q("Question one is short enough?")).unwrap(),
            fit(&q("Question two is short enough?")).unwrap(),
        ];
        let mut bag = Bag::new(items, 7);
        let first = bag.draw().unwrap();
        let second = bag.draw().unwrap();
        assert_ne!(first.lines, second.lines);
        let third = bag.draw().unwrap();
        assert!(third.lines == first.lines || third.lines == second.lines);
    }
}
