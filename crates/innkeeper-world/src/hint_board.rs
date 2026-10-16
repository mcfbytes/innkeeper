//! The Hint Board room (command 38): a three-level menu of series, games and areas, then the hints
//! of one area, each a problem and its answer. The room sends no room number, only the choices made
//! on its own menus, so one catalogue serves every room. Layouts and the client's waits:
//! docs/server/hub-services.md. Every text here is this host's own; edit the table below to change
//! what the board says.

use crate::message::{HintKind, HintReply, HintRequest, HINT_NO_MORE, HINT_UNAVAILABLE};
use crate::{HostMessage, Nak, Sid};

/// One hint: the problem the player reads first, and the answer the Show button reveals.
struct Hint {
    problem: &'static str,
    answer: &'static str,
}

/// A menu entry at the third level; its hints are numbered from 1 in this order.
struct Area {
    name: &'static str,
    hints: &'static [Hint],
}

/// A menu entry at the second level.
struct Game {
    name: &'static str,
    areas: &'static [Area],
}

/// A menu entry at the first level.
struct Series {
    name: &'static str,
    games: &'static [Game],
}

/// The whole board. Menu lines should stay under about 24 characters so the buttons hold them; a
/// problem under about 110 characters and an answer under about 250 fit the boxes the client draws.
/// Every menu holds at most 255 entries (a choice is one byte), and the client pages them by ten.
const SERIES: &[Series] = &[
    Series {
        name: "Getting Around",
        games: &[
            Game {
                name: "The Map",
                areas: &[
                    Area {
                        name: "Finding Places",
                        hints: &[
                            Hint {
                                problem: "I am looking at the map and every building looks the same to me.",
                                answer: "Squint less. Each building is a door to somewhere: the post office, the town hall, the clubhouse and more. Click one and walk in, no knocking required.",
                            },
                            Hint {
                                problem: "I clicked a building and the map just sat there, humming.",
                                answer: "Give it a moment; the map thinks before it travels. If it is still humming, click the building once more, firmly but kindly.",
                            },
                        ],
                    },
                    Area {
                        name: "Stuck on the Map",
                        hints: &[Hint {
                            problem: "I wandered into a room and cannot get back to the map.",
                            answer: "In the clubhouse, click Go to and choose Map. Most rooms have a button like it, and it will take you back, no questions asked.",
                        }],
                    },
                ],
            },
            Game {
                name: "The Post Office",
                areas: &[
                    Area {
                        name: "Reading Mail",
                        hints: &[
                            Hint {
                                problem: "Is there any mail for me? How would I even know?",
                                answer: "When a letter is waiting, a little mail icon lights up on the map. No light, no letters; the postmaster sends his regrets and a very short list.",
                            },
                            Hint {
                                problem: "I opened my mailbox and it says it is empty.",
                                answer: "Then it is empty, and quite proud of it. Letters appear when somebody writes to your box number, so hand your number to a friend.",
                            },
                        ],
                    },
                    Area {
                        name: "Writing Mail",
                        hints: &[Hint {
                            problem: "I want to write a letter but I do not know the address.",
                            answer: "Letters go to box numbers. Ask your friend for theirs; it is shown on their own mailbox screen. Type it in the box line, then write away.",
                        }],
                    },
                ],
            },
            Game {
                name: "The Town Hall",
                areas: &[
                    Area {
                        name: "Passwords",
                        hints: &[Hint {
                            problem: "I would like a new password, preferably not my cat's name.",
                            answer: "Visit the Town Hall and choose Change Password. Type the old one, then the new one, and the clerk will stamp it. Your cat will never know.",
                        }],
                    },
                    Area {
                        name: "Questions and Bills",
                        hints: &[
                            Hint {
                                problem: "Where do I send a question to the people who run this place?",
                                answer: "The Town Hall has an Ask Member Services window. Write your question and send it; it goes to a very patient inbox, and the answer comes in due course.",
                            },
                            Hint {
                                problem: "How much do I owe?",
                                answer: "Nothing. The Check My Account Status window will tell you so at length, and the meter is asleep in the corner.",
                            },
                        ],
                    },
                ],
            },
        ],
    },
    Series {
        name: "Meeting Folks",
        games: &[
            Game {
                name: "Talking",
                areas: &[
                    Area {
                        name: "Chat Basics",
                        hints: &[
                            Hint {
                                problem: "I typed hello and nobody answered.",
                                answer: "Pick a player in the room, or All, then click Talk and type your message. In an empty room you are talking to the furniture, which is a poor listener.",
                            },
                            Hint {
                                problem: "Somebody keeps saying rude things at me.",
                                answer: "When their message pops up, click Complain, fill in the form and send it. A note goes to the people who tidy up here. Or click Mute, and the rude person goes quiet.",
                            },
                        ],
                    },
                    Area {
                        name: "Finding People",
                        hints: &[Hint {
                            problem: "How do I find out if my friend is online?",
                            answer: "Click Go to, then A Person, and type the name. It tells you where that person is right now, or that they are out.",
                        }],
                    },
                ],
            },
        ],
    },
    Series {
        name: "Games and Pastimes",
        games: &[
            Game {
                name: "The Clubhouse",
                areas: &[
                    Area {
                        name: "Starting a Game",
                        hints: &[Hint {
                            problem: "I want to play but I have nobody to play with.",
                            answer: "Wait in the clubhouse; the room lists whoever else is about. Select a player, click Invite and wait for an answer. If it is yes, off you go.",
                        }],
                    },
                    Area {
                        name: "Joining a Game",
                        hints: &[Hint {
                            problem: "Someone invited me and I do not know what to do.",
                            answer: "A box asks whether you will join. Say yes to play, or no to decline; declining is perfectly polite. Nobody has ever been sent to bed for it.",
                        }],
                    },
                ],
            },
            Game {
                name: "Sketching",
                areas: &[Area {
                    name: "Drawing Together",
                    hints: &[
                        Hint {
                            problem: "My drawing does not show up on my friend's screen.",
                            answer: "Both of you must be in the same sketch room at the same time. Wave at each other first, then draw something wonderful and slightly lopsided.",
                        },
                        Hint {
                            problem: "Someone drew something that should not be drawn.",
                            answer: "Click Options and choose Complain. It sends a note naming the artists to the people who tidy up, who arrive with buckets.",
                        },
                    ],
                }],
            },
        ],
    },
    Series {
        name: "Odds and Ends",
        games: &[Game {
            name: "General Hints",
            areas: &[
                Area {
                    name: "Click on Things",
                    hints: &[
                        Hint {
                            problem: "I clicked everywhere and nothing happened.",
                            answer: "Click on things. Most things are clickable; a few click back. Nothing here bites, and the map is always one Go to away.",
                        },
                        Hint {
                            problem: "There is a button here that I do not understand.",
                            answer: "Click it and find out. The worst a button can do is take you somewhere new, and you can always go back with Go to and Map.",
                        },
                    ],
                },
                Area {
                    name: "When All Else Fails",
                    hints: &[Hint {
                        problem: "Everything is slow and I am losing my patience.",
                        answer: "Take a breath, count to ten, and wait for the next screen. Some things take a little while here, and a watched pot is a notorious sulker.",
                    }],
                },
            ],
        }],
    },
];

/// The first menu, `kind` of reply 0.
fn series_menu() -> String {
    SERIES
        .iter()
        .map(|series| series.name)
        .collect::<Vec<_>>()
        .join("\n")
}

fn series_at(choice: u8) -> Option<&'static Series> {
    usize::from(choice)
        .checked_sub(1)
        .and_then(|index| SERIES.get(index))
}

fn game_at(series: &'static Series, choice: u8) -> Option<&'static Game> {
    usize::from(choice)
        .checked_sub(1)
        .and_then(|index| series.games.get(index))
}

fn area_at(game: &'static Game, choice: u8) -> Option<&'static Area> {
    usize::from(choice)
        .checked_sub(1)
        .and_then(|index| game.areas.get(index))
}

fn menu_of<T>(entries: &[T], name: impl Fn(&T) -> &'static str) -> String {
    entries.iter().map(name).collect::<Vec<_>>().join("\n")
}

/// The reply to one hint request: the menu or hint it asks for, a question and an answer for a
/// hint, or a refusal the room shows in its own words.
pub(crate) fn answer(request: &HintRequest) -> Vec<HostMessage> {
    let to = request.sid;
    let [series, game, area, number] = request.args;
    let text = |kind, which, text: String| {
        vec![HostMessage::Hint(HintReply {
            to,
            kind,
            which,
            text,
        })]
    };
    match request.step {
        0 => text(HintKind::Series, 0, series_menu()),
        1 => match series_at(series) {
            Some(found) => text(
                HintKind::Games,
                series,
                menu_of(found.games, |game| game.name),
            ),
            None => refuse(to, HINT_UNAVAILABLE),
        },
        2 => match series_at(series).and_then(|found| game_at(found, game)) {
            Some(found) => text(
                HintKind::Areas,
                game,
                menu_of(found.areas, |area| area.name),
            ),
            None => refuse(to, HINT_UNAVAILABLE),
        },
        _ => hint(to, [series, game, area], number),
    }
}

fn hint(to: Sid, path: [u8; 3], number: u8) -> Vec<HostMessage> {
    let [series, game, area] = path;
    let found = series_at(series)
        .and_then(|found| game_at(found, game))
        .and_then(|found| area_at(found, area));
    let Some(found) = found else {
        return refuse(to, HINT_UNAVAILABLE);
    };
    if number == 0 {
        return refuse(to, HINT_UNAVAILABLE);
    }
    let Some(hint) = usize::from(number)
        .checked_sub(1)
        .and_then(|index| found.hints.get(index))
    else {
        return refuse(to, HINT_NO_MORE);
    };
    let reply = |kind, text: &str| {
        HostMessage::Hint(HintReply {
            to,
            kind,
            which: number,
            text: text.to_owned(),
        })
    };
    vec![
        reply(HintKind::Question, hint.problem),
        reply(HintKind::Answer, hint.answer),
    ]
}

/// Nak 38: reason 5 makes the room say the board or menu is unavailable and, past the first menu,
/// go back to it; reason 6 makes it say there are no more hints and offer the way out.
fn refuse(to: Sid, reason: u8) -> Vec<HostMessage> {
    vec![HostMessage::Nak(Nak {
        to,
        which_cmd: 38,
        which_sub: reason,
        num_tries: 0,
        text: String::new(),
    })]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(step: u8, args: [u8; 4]) -> Vec<HostMessage> {
        answer(&HintRequest {
            sid: Sid(0x0123),
            step,
            args,
        })
    }

    fn only_text(messages: Vec<HostMessage>) -> HintReply {
        let [HostMessage::Hint(reply)] = messages.as_slice() else {
            panic!("expected one hint reply, got {messages:?}");
        };
        reply.clone()
    }

    #[test]
    fn the_first_menu_lists_every_series_in_order() {
        let reply = only_text(ask(0, [0; 4]));
        assert_eq!(reply.kind, HintKind::Series);
        assert_eq!(reply.to, Sid(0x0123));
        let lines: Vec<_> = reply.text.lines().collect();
        assert_eq!(lines.len(), SERIES.len());
        assert_eq!(lines[0], "Getting Around");
    }

    #[test]
    fn a_menu_reply_names_the_choice_it_answers() {
        let games = only_text(ask(1, [3, 0, 0, 0]));
        assert_eq!((games.kind, games.which), (HintKind::Games, 3));
        assert!(games.text.starts_with("The Clubhouse\n"));
        let areas = only_text(ask(2, [1, 2, 0, 0]));
        assert_eq!((areas.kind, areas.which), (HintKind::Areas, 2));
        assert_eq!(areas.text, "Reading Mail\nWriting Mail");
    }

    #[test]
    fn a_hint_is_a_problem_then_its_answer_under_the_same_number() {
        let messages = ask(3, [4, 1, 1, 1]);
        let [HostMessage::Hint(problem), HostMessage::Hint(answer)] = messages.as_slice() else {
            panic!("expected a problem and an answer, got {messages:?}");
        };
        assert_eq!((problem.kind, problem.which), (HintKind::Question, 1));
        assert_eq!((answer.kind, answer.which), (HintKind::Answer, 1));
        assert!(problem.text.contains("clicked everywhere"));
        assert!(answer
            .text
            .starts_with("Click on things. Most things are clickable"));
    }

    #[test]
    fn a_menu_or_hint_that_does_not_exist_is_refused_in_the_clients_terms() {
        let nak = |messages: Vec<HostMessage>| match messages.as_slice() {
            [HostMessage::Nak(nak)] => (nak.which_cmd, nak.which_sub, nak.to),
            other => panic!("expected a Nak, got {other:?}"),
        };
        assert_eq!(nak(ask(1, [99, 0, 0, 0])), (38, 5, Sid(0x0123)));
        assert_eq!(nak(ask(1, [0, 0, 0, 0])), (38, 5, Sid(0x0123)));
        assert_eq!(nak(ask(2, [1, 99, 0, 0])), (38, 5, Sid(0x0123)));
        assert_eq!(nak(ask(3, [1, 1, 99, 1])), (38, 5, Sid(0x0123)));
        assert_eq!(nak(ask(3, [1, 1, 1, 0])), (38, 5, Sid(0x0123)));
        assert_eq!(nak(ask(3, [1, 1, 1, 3])), (38, 6, Sid(0x0123)));
    }

    #[test]
    fn every_entry_fits_the_clients_boxes_and_the_ascii_wire() {
        let mut hints = 0;
        for series in SERIES {
            assert!(series.name.len() <= 24 && !series.name.contains('\n'));
            for game in series.games {
                assert!(game.name.len() <= 24 && !game.name.contains('\n'));
                for area in game.areas {
                    assert!(area.name.len() <= 24 && !area.name.contains('\n'));
                    assert!(!area.hints.is_empty());
                    for hint in area.hints {
                        hints += 1;
                        assert!(hint.problem.len() <= 110, "{}", hint.problem);
                        assert!(hint.answer.len() <= 250, "{}", hint.answer);
                        assert!(hint.problem.is_ascii() && hint.answer.is_ascii());
                    }
                }
            }
        }
        assert!(hints >= 15);
    }
}
