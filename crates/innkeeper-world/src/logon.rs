//! The logon policy: who is let in, what the client is told, and the password change that follows.

use tracing::{info, warn};

use crate::assumptions::UNLISTED_LAND_NAK_ASSUMED;
use crate::{
    Account, Ack, ChangePassword, HostMessage, Login, LoginNakReason, Nak, Refusal, World,
};

const PASSWORD_TRIES: u8 = 3;
/// The `whichCmd` and `whichSub` of the Ack that makes the client store the new password.
const PASSWORD_CHANGED: (u8, u8) = (44, 1);

/// The account a Login opens, or the Nak that refuses it.
pub(crate) fn admit(world: &World, login: &Login) -> Result<Account, Nak> {
    if !world.lands.runs(login.land_type) {
        warn!(land_type = login.land_type.0, "login for an unlisted land");
        let text = "This land is not known here.";
        return Err(Nak::login(UNLISTED_LAND_NAK_ASSUMED, 0, text));
    }
    world.accounts.admit(login).map_err(|refusal| {
        warn!(account = login.account.0, ?refusal, "login refused");
        nak_for(refusal)
    })
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
                which_sub: 0,
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
        Refusal::StoreUnavailable => Nak::login(
            LoginNakReason::UnknownAccount,
            0,
            "Accounts cannot be read right now.",
        ),
    }
}
