use std::collections::BTreeMap;

use tracing::{info, warn};

use super::{ConnectionId, Delivery, ObjectStore};
use crate::assumptions::UNLOCK_IS_ACKNOWLEDGED_ASSUMED;
use crate::message::Command;
use crate::{HostMessage, LockId, LockRequest};

/// Which connection holds each host lock. A connection, not a SID, holds it, because the invitation
/// scripts lock from an object without a SID.
#[derive(Debug, Default)]
pub(super) struct LockTable {
    holders: BTreeMap<LockId, ConnectionId>,
}

impl LockTable {
    /// Takes every lock for `connection`, or none and names the first one another connection holds.
    fn take(&mut self, connection: ConnectionId, locks: &[LockId]) -> Result<(), LockId> {
        let held_by_other = |lock: &&LockId| {
            self.holders
                .get(lock)
                .is_some_and(|&holder| holder != connection)
        };
        if let Some(&taken) = locks.iter().find(held_by_other) {
            return Err(taken);
        }
        for &lock in locks {
            self.holders.insert(lock, connection);
        }
        Ok(())
    }

    fn release(&mut self, connection: ConnectionId, locks: &[LockId]) {
        for lock in locks {
            if self.holders.get(lock) == Some(&connection) {
                self.holders.remove(lock);
            }
        }
    }

    pub(super) fn release_all(&mut self, connection: ConnectionId) {
        self.holders.retain(|_, holder| *holder != connection);
    }
}

impl ObjectStore {
    /// lock (4): an Ack when every requested lock is free or already the connection's, else a Nak.
    pub(crate) fn lock(
        &mut self,
        connection: ConnectionId,
        request: &LockRequest,
    ) -> Vec<Delivery> {
        let reply = match self.locks.take(connection, &request.locks) {
            Ok(()) => {
                info!(?request, "locks taken");
                HostMessage::Ack(request.acknowledged(Command::Lock))
            }
            Err(taken) => {
                warn!(?request, taken = taken.0, "lock held by another connection");
                HostMessage::LockRefused(request.refused(taken))
            }
        };
        vec![Delivery::new(connection, reply)]
    }

    /// unlock (6): frees the requested locks the connection holds; another connection's stay.
    pub(crate) fn unlock(
        &mut self,
        connection: ConnectionId,
        request: &LockRequest,
    ) -> Vec<Delivery> {
        self.locks.release(connection, &request.locks);
        info!(?request, "locks released");
        match UNLOCK_IS_ACKNOWLEDGED_ASSUMED {
            true => {
                let ack = request.acknowledged(Command::Unlock);
                vec![Delivery::new(connection, HostMessage::Ack(ack))]
            }
            false => Vec::new(),
        }
    }
}
