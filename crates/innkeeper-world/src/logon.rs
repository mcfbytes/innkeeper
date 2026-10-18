//! The logon policy: who is let in, what the client is told, and the password change that follows.

use tracing::{info, warn};

use crate::assumptions::{PASSWORD_REFUSED_SUB_ASSUMED, UNLISTED_LAND_NAK_ASSUMED};
use crate::{
    is_reserved_name, Account, AccountRecord, Ack, ChangePassword, HostMessage, Login,
    LoginNakReason, Nak, Refusal, StoreError, World,
};

const PASSWORD_TRIES: u8 = 3;
/// The Login Nak for a player name that passes for staff; the client shows it and hangs up, and the
/// player picks another name in Select Player.
const RESERVED_NAME_TEXT: &str = "That player name is kept for the staff. Choose another one.";
/// The `whichCmd` and `whichSub` of the Ack that makes the client store the new password.
const PASSWORD_CHANGED: (u8, u8) = (44, 1);

/// The account a Login opens, or the Nak that refuses it.
pub(crate) fn admit(world: &World, login: &Login) -> Result<Account, Nak> {
    if !world.lands.runs(login.land_type) {
        warn!(land_type = login.land_type.0, "login for an unlisted land");
        let text = "This land is not known here.";
        return Err(Nak::login(UNLISTED_LAND_NAK_ASSUMED, 0, text));
    }
    if is_reserved_name(&login.name) {
        warn!(
            account = login.account.0,
            "login refused: a staff or service name"
        );
        return Err(Nak::login(
            LoginNakReason::UnknownAccount,
            0,
            RESERVED_NAME_TEXT,
        ));
    }
    let account = world.accounts.admit(login).map_err(|refusal| {
        warn!(account = login.account.0, ?refusal, "login refused");
        nak_for(refusal)
    })?;
    if !world.accounts.is_stored() {
        give_a_row(world, login);
    }
    Ok(account)
}

/// A book that is not the store's (the dev default, a test's list) still gives the account a row,
/// so its bankroll, mail, posts and saves have a parent: the store keeps no orphans
/// (docs/server/store.md section 2). A row already there, live or deleted, is left alone.
fn give_a_row(world: &World, login: &Login) {
    let record = AccountRecord::new(login.password, login.name.as_str());
    match world.store.create_account(login.account, record) {
        Ok(()) | Err(StoreError::AccountExists(_)) => {}
        Err(error) => warn!(account = login.account.0, %error, "no store row for the account"),
    }
}

/// Stores the new password for the logged-in account; the Ack tells the client to keep it too.
pub(crate) fn change_password(
    world: &World,
    account: &Account,
    change: ChangePassword,
) -> HostMessage {
    let (which_cmd, which_sub) = PASSWORD_CHANGED;
    match world.accounts.change_password(account.id, change.password) {
        Ok(()) => {
            info!(account = account.id.0, "password changed");
            HostMessage::Ack(Ack {
                to: change.sid,
                which_cmd,
                which_sub,
                tail: Vec::new(),
            })
        }
        Err(refusal) => {
            warn!(account = account.id.0, ?refusal, "password change refused");
            HostMessage::Nak(Nak {
                to: change.sid,
                which_cmd,
                which_sub: PASSWORD_REFUSED_SUB_ASSUMED,
                num_tries: 0,
                text: "The password could not be changed.".to_owned(),
            })
        }
    }
}

fn nak_for(refusal: Refusal) -> Nak {
    match refusal {
        Refusal::WrongPassword => Nak::login(
            LoginNakReason::RetryPassword,
            PASSWORD_TRIES,
            "That password is not right.",
        ),
        Refusal::UnknownAccount => Nak::login(
            LoginNakReason::UnknownAccount,
            0,
            "This account is not known here.",
        ),
        Refusal::Suspended => Nak::login(
            LoginNakReason::UnknownAccount,
            0,
            "This account is suspended. Write to Member Services.",
        ),
        Refusal::StoreUnavailable => Nak::login(
            LoginNakReason::UnknownAccount,
            0,
            "Accounts cannot be read right now.",
        ),
    }
}
