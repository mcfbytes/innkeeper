//! The conference control objects the Clubhouse looks up before it lists conferences; the
//! lookup and its replies are in docs/protocol/messages.md section 3.2.1.

use std::collections::BTreeMap;

use crate::assumptions::{CONFERENCE_CONTROL_PUBLISHED_ASSUMED, CONFERENCE_CONTROL_SID_ASSUMED};
use crate::{LandType, ObjectLocated, Sid};

/// The land type of the only land whose scripts hold the conference code.
const CLUBHOUSE: LandType = LandType(1);
/// The second word of the lookup name `GotoConference::init` builds; the client never says why 15.
const LOOKUP_KIND_WORD: u16 = 15;
/// The third word of the lookup name.
const LOOKUP_TAIL_WORD: u16 = 0;
/// The reply SID that says no object has the name; the client then goes on without one.
const NOT_FOUND: Sid = Sid(0);

/// The SIDs of the host's conference control objects, found by the name a client asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conferences {
    control_objects: BTreeMap<Vec<u8>, Sid>,
}

impl Conferences {
    /// No conference control object: every name is free.
    pub fn new() -> Self {
        Conferences::default()
    }

    /// The stock host: the Clubhouse's control object, when the host publishes it.
    pub fn stock() -> Self {
        let mut conferences = Conferences::new();
        if CONFERENCE_CONTROL_PUBLISHED_ASSUMED {
            conferences.publish(lookup_name(CLUBHOUSE), CONFERENCE_CONTROL_SID_ASSUMED);
        }
        conferences
    }

    /// Makes `name` taken: lookups of it find the object `sid`.
    pub fn publish(&mut self, name: Vec<u8>, sid: Sid) {
        self.control_objects.insert(name, sid);
    }

    /// The 41/0 reply to a lookup of `name`, addressed to `to`; the SID is 0 while the name is free.
    pub fn locate(&self, to: Sid, name: &[u8]) -> ObjectLocated {
        let sid = self.control_objects.get(name).copied();
        ObjectLocated {
            to,
            sid: sid.unwrap_or(NOT_FOUND),
        }
    }
}

/// The name the client of a land type sends: three little-endian words.
pub fn lookup_name(land_type: LandType) -> Vec<u8> {
    let words = [u16::from(land_type.0), LOOKUP_KIND_WORD, LOOKUP_TAIL_WORD];
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASKER: Sid = Sid(0x0100);
    const CONTROL: Sid = Sid(0x00C0);

    #[test]
    fn the_clubhouse_asks_for_three_words() {
        assert_eq!(lookup_name(CLUBHOUSE), [1, 0, 15, 0, 0, 0]);
        assert_eq!(lookup_name(LandType(3)), [3, 0, 15, 0, 0, 0]);
    }

    #[test]
    fn a_free_name_is_located_at_sid_0_and_a_published_one_at_its_object() {
        let mut conferences = Conferences::new();
        let name = lookup_name(CLUBHOUSE);
        assert_eq!(conferences.locate(ASKER, &name).sid, NOT_FOUND);
        conferences.publish(name.clone(), CONTROL);
        assert_eq!(
            conferences.locate(ASKER, &name),
            ObjectLocated {
                to: ASKER,
                sid: CONTROL
            }
        );
        assert_eq!(
            conferences.locate(ASKER, &lookup_name(LandType(2))).sid,
            NOT_FOUND
        );
    }
}
