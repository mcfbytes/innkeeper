//! The CasinoLand bankroll ("CasinoBucks"): command 40 sub 3 reads it and sub 2 writes it.
//!
//! The client keeps its own copy in `ll$$<account>.dat` and asks the host only when that file is
//! missing, so a read is the first time the host sees a player's balance. `messages.md` section 3.3.1
//! has the layouts and the script evidence.

use tracing::{info, warn};

use crate::assumptions::MONEY_STARTING_BALANCE;
use crate::{
    Account, AccountId, Ack, HostMessage, MoneyReply, Nak, ReadMoney, SaveMoney, Store, StoreError,
    World,
};

/// The `whichCmd` and `whichSub` the client's save waits for, as an Ack or a Nak (`proc_14`).
const MONEY_SAVED: (u8, u8) = (40, 2);

/// The bankroll to report. An account the store has nothing for is new to the CasinoLand and gets the
/// starting amount; a failing store gets no reply at all, because the client would keep a guess and
/// overwrite the balance it has (`LL::handleMsg` applies whatever arrives).
pub(crate) fn read(world: &World, account: &Account, request: ReadMoney) -> Vec<HostMessage> {
    let amount = match world.store.money(account.id) {
        Ok(Some(amount)) => amount,
        Ok(None) => MONEY_STARTING_BALANCE,
        Err(error) => {
            warn!(%error, account = account.id.0, "no reply: the client keeps its own bankroll");
            return Vec::new();
        }
    };
    info!(account = account.id.0, amount = amount.0, "bankroll read");
    vec![HostMessage::Money(MoneyReply {
        to: request.sid,
        amount,
    })]
}

/// Stores the bankroll. The client waits for a reply that names command 40, so it always gets one.
pub(crate) fn save(world: &World, account: &Account, request: SaveMoney) -> Vec<HostMessage> {
    let (which_cmd, which_sub) = MONEY_SAVED;
    match world.store.set_money(account.id, request.amount) {
        Ok(()) => {
            info!(
                account = account.id.0,
                amount = request.amount.0,
                "bankroll saved"
            );
            vec![HostMessage::Ack(Ack {
                to: request.sid,
                which_cmd,
                which_sub,
                tail: Vec::new(),
            })]
        }
        Err(error) => {
            warn!(%error, account = account.id.0, "bankroll not saved");
            vec![HostMessage::Nak(Nak {
                to: request.sid,
                which_cmd,
                which_sub,
                num_tries: 0,
                text: "Your bankroll could not be saved.".to_owned(),
            })]
        }
    }
}

/// Stores the starting CasinoBucks when the account has no balance yet.
/// A balance already stored, including 0, is left as it is.
pub(crate) fn grant_starting_balance(store: &dyn Store, id: AccountId) -> Result<(), StoreError> {
    if store.money(id)?.is_some() {
        return Ok(());
    }
    store.set_money(id, MONEY_STARTING_BALANCE)
}
