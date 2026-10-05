use crate::MessageError;

/// The `kind` byte of joinNet: how the host scopes the new object. Which game thing a kind stands
/// for depends on the land that sends it (docs/protocol/messages.md section 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectKind {
    Object = 1,
    Group = 2,
    SoloObject = 3,
    PrivateGroup = 4,
    LandGroup = 5,
    GameObject = 129,
}

const OBJECT_KINDS: [ObjectKind; 6] = [
    ObjectKind::Object,
    ObjectKind::Group,
    ObjectKind::SoloObject,
    ObjectKind::PrivateGroup,
    ObjectKind::LandGroup,
    ObjectKind::GameObject,
];

impl ObjectKind {
    pub(crate) const fn byte(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for ObjectKind {
    type Error = MessageError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        OBJECT_KINDS
            .into_iter()
            .find(|kind| kind.byte() == byte)
            .ok_or(MessageError::UnknownValue {
                field: "kind",
                value: u16::from(byte),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_and_the_gaps_are_errors() {
        for kind in OBJECT_KINDS {
            assert_eq!(ObjectKind::try_from(kind.byte()), Ok(kind));
        }
        for byte in [0, 6, 128, 130] {
            assert_eq!(
                ObjectKind::try_from(byte),
                Err(MessageError::UnknownValue {
                    field: "kind",
                    value: u16::from(byte)
                })
            );
        }
    }
}
