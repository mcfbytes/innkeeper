use crate::Int14hError;

/// The 17 far-call entries of the table that INT 14h returns, in table order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Export {
    GetStatus,
    GetSharedData,
    SetSharedData,
    Connect,
    Send,
    Receive,
    SetAckTimeout,
    Disconnect,
    Poll,
    SetNextProgram,
    Service,
    GetPreviousProgram,
    IsTransmitIdle,
    SwitchHost,
    Flush,
    SetCallbacks,
    GetLineRate,
}

/// How an export treats TSNEXEC's reentrancy lock (`031D:0710`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockUse {
    Ignores,
    /// Does nothing and returns 0 while the lock is held.
    Takes,
    Spins,
}

/// One row of the export table in int14h-api.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportRow {
    pub export: Export,
    pub name: &'static str,
    /// Entry point in the Feb-1994 TSNEXEC image.
    pub entry: &'static str,
    pub lock: LockUse,
}

const fn row(export: Export, name: &'static str, entry: &'static str, lock: LockUse) -> ExportRow {
    ExportRow {
        export,
        name,
        entry,
        lock,
    }
}

/// The export table, indexed by export number; `Export::index` relies on the order.
pub const EXPORT_TABLE: [ExportRow; 17] = [
    row(
        Export::GetStatus,
        "GetStatus",
        "0000:2A4A",
        LockUse::Ignores,
    ),
    row(
        Export::GetSharedData,
        "GetSharedData",
        "0000:0273",
        LockUse::Ignores,
    ),
    row(
        Export::SetSharedData,
        "SetSharedData",
        "0000:022F",
        LockUse::Ignores,
    ),
    row(Export::Connect, "Connect", "0000:2A60", LockUse::Ignores),
    row(Export::Send, "Send", "0000:23D1", LockUse::Takes),
    row(Export::Receive, "Receive", "0000:2455", LockUse::Takes),
    row(
        Export::SetAckTimeout,
        "SetAckTimeout",
        "0000:22CC",
        LockUse::Ignores,
    ),
    row(
        Export::Disconnect,
        "Disconnect",
        "0000:25A3",
        LockUse::Ignores,
    ),
    row(Export::Poll, "Poll", "0000:24D9", LockUse::Takes),
    row(
        Export::SetNextProgram,
        "SetNextProgram",
        "0000:0290",
        LockUse::Ignores,
    ),
    row(Export::Service, "Service", "0000:2507", LockUse::Takes),
    row(
        Export::GetPreviousProgram,
        "GetPreviousProgram",
        "0000:02DF",
        LockUse::Ignores,
    ),
    row(
        Export::IsTransmitIdle,
        "IsTransmitIdle",
        "0000:22E5",
        LockUse::Ignores,
    ),
    row(
        Export::SwitchHost,
        "SwitchHost",
        "0000:2A79",
        LockUse::Spins,
    ),
    row(Export::Flush, "Flush", "0000:2531", LockUse::Takes),
    row(
        Export::SetCallbacks,
        "SetCallbacks",
        "0000:03C3",
        LockUse::Ignores,
    ),
    row(
        Export::GetLineRate,
        "GetLineRate",
        "0000:2A55",
        LockUse::Ignores,
    ),
];

impl Export {
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// Byte offset of the far pointer within the table at `0000:1660`.
    pub const fn table_offset(self) -> u8 {
        self.index() * 4
    }

    pub fn row(self) -> &'static ExportRow {
        match EXPORT_TABLE.get(usize::from(self.index())) {
            Some(row) => row,
            None => unreachable!("EXPORT_TABLE has one row per Export variant"),
        }
    }

    pub fn name(self) -> &'static str {
        self.row().name
    }
}

impl TryFrom<u8> for Export {
    type Error = Int14hError;

    fn try_from(index: u8) -> Result<Self, Self::Error> {
        let row = EXPORT_TABLE.get(usize::from(index));
        row.map(|row| row.export)
            .ok_or(Int14hError::UnknownExport(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_rows_sit_at_their_export_index() {
        for (index, row) in EXPORT_TABLE.iter().enumerate() {
            assert_eq!(usize::from(row.export.index()), index, "{}", row.name);
            assert_eq!(format!("{:?}", row.export), row.name);
            assert_eq!(Export::try_from(index as u8), Ok(row.export));
        }
        assert_eq!(Export::try_from(17), Err(Int14hError::UnknownExport(17)));
    }

    #[test]
    fn offsets_and_locks_match_the_document() {
        assert_eq!(Export::GetLineRate.table_offset(), 0x40);
        let locking: Vec<u8> = EXPORT_TABLE
            .iter()
            .filter(|row| row.lock == LockUse::Takes)
            .map(|row| row.export.index())
            .collect();
        assert_eq!(locking, [4, 5, 8, 10, 14]);
    }
}
