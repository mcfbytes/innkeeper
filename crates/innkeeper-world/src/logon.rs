//! The logon policy: who is let in, what the client is told, the password change that follows, and
//! which host a call reaches (docs/protocol/link-layer.md 3.2, 5.1).

use std::fmt;

use tracing::{info, warn};

use crate::assumptions::{
    HOME_MNEMONIC_ASSUMED, HOST_NUMBER_PREFIX_ASSUMED, UNLISTED_LAND_NAK_ASSUMED,
};
use crate::{
    Account, Ack, ChangePassword, HostMessage, HostNumber, Login, LoginNakReason, Nak, Refusal,
    World,
};

/// The com driver drops this many leading digits, the DNIC, from a numeric address (`MODEM.DRV:0B0F`).
const DNIC_DIGITS: usize = 4;
/// The driver keeps at most this many characters of an address (`MODEM.DRV:0B0F`).
const MAX_ADDRESS_LEN: usize = 14;
const PASSWORD_TRIES: u8 = 3;
/// The `whichCmd` and `whichSub` of the Ack that makes the client store the new password.
const PASSWORD_CHANGED: (u8, u8) = (44, 1);

/// What the PAD is asked to call: an X.25 number without its DNIC, or a mnemonic such as `SIERRA`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallAddress(String);

impl CallAddress {
    /// The address of a PAD call command, which the driver has already shortened.
    pub fn new(text: &str) -> Self {
        CallAddress(text.to_owned())
    }

    /// The driver's reading of a dial-string host or a SwitchHost argument: 4 to 14 digits lose the
    /// DNIC and up to 14 letters stay; for anything else, the empty text too, it calls its default host.
    pub fn from_driver_argument(text: &str) -> Option<Self> {
        let fits = |least: usize| (least..=MAX_ADDRESS_LEN).contains(&text.len());
        if fits(DNIC_DIGITS) && text.bytes().all(|byte| byte.is_ascii_digit()) {
            text.get(DNIC_DIGITS..).map(CallAddress::new)
        } else if fits(1) && text.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            Some(CallAddress::new(text))
        } else {
            None
        }
    }

    /// The addresses that reach this host: the home mnemonic and the host's own number.
    pub fn reaching(world: &World) -> Vec<Self> {
        let number = format!("{HOST_NUMBER_PREFIX_ASSUMED}{:02}", world.host.0);
        vec![CallAddress::new(HOME_MNEMONIC_ASSUMED), CallAddress(number)]
    }

    /// The host this address names; none for a mnemonic or number the stock network does not know.
    pub fn host(&self, world: &World) -> Option<HostNumber> {
        if self.0.eq_ignore_ascii_case(HOME_MNEMONIC_ASSUMED) {
            return Some(world.host);
        }
        let digits = self.0.strip_prefix(HOST_NUMBER_PREFIX_ASSUMED)?;
        let decimal = digits.bytes().all(|byte| byte.is_ascii_digit());
        decimal
            .then(|| digits.parse().ok().map(HostNumber))
            .flatten()
    }

    /// Whether a call to this address reaches this server; every other host is refused.
    pub fn reaches(&self, world: &World) -> bool {
        self.host(world) == Some(world.host)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CallAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn read(argument: &str) -> Option<String> {
        CallAddress::from_driver_argument(argument).map(|address| address.0)
    }

    #[test]
    fn the_driver_drops_the_dnic_keeps_mnemonics_and_defaults_the_rest() {
        assert_eq!(read("311083420207").as_deref(), Some("83420207"));
        assert_eq!(read("SIERRA").as_deref(), Some("SIERRA"));
        assert_eq!(read("3110").as_deref(), Some(""));
        assert_eq!(read(""), None);
        assert_eq!(read("311"), None);
        assert_eq!(read("Sierra7"), None);
        assert_eq!(read("311083420207311"), None);
        assert_eq!(read("ABCDEFGHIJKLMNO"), None);
    }

    #[test]
    fn stock_numbers_name_their_host_and_only_this_one_is_reached() {
        let world = World::stock();
        let host = |text: &str| CallAddress::new(text).host(&world);
        assert_eq!(host("83420207"), Some(HostNumber(7)));
        assert_eq!(host("83420214"), Some(HostNumber(14)));
        assert_eq!(host("sierra"), Some(world.host));
        assert_eq!(host("834202"), None);
        assert_eq!(host("TELENET"), None);
        assert!(CallAddress::new("83420207").reaches(&world));
        assert!(!CallAddress::new("83420208").reaches(&world));
        let reaching = CallAddress::reaching(&world);
        assert!(reaching.iter().all(|address| address.reaches(&world)));
        assert_eq!(reaching[1].as_str(), "83420207");
    }
}
