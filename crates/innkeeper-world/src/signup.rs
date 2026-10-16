//! Command 55/1: issue an account number. The form has no password, so the row keeps a placeholder
//! until the first Login, which presents the password the client actually has.

use tracing::warn;

use crate::assumptions::{
    SIGNUP_ADOPTS_PRESENTED_PASSWORD_ASSUMED, SIGNUP_CREATES_WHEN_ENROLMENT_IS_CLOSED_ASSUMED,
    SIGNUP_FIRST_ACCOUNT_ASSUMED, SIGNUP_KEEPS_ONLY_THE_ACCOUNT_ASSUMED, SIGNUP_PASSWORD_ASSUMED,
};
use crate::message::Command;
use crate::money::grant_starting_balance;
use crate::{
    AccountId, AccountRecord, Ack, EncodedPassword, HostMessage, Sid, Signup, StoreError, World,
};

const ATTEMPTS: u8 = 4;
const SIGNUP_SUB: u8 = 1;

pub(crate) fn answer(world: &World, signup: &Signup) -> Vec<HostMessage> {
    if SIGNUP_KEEPS_ONLY_THE_ACCOUNT_ASSUMED {
        let _ = &signup.form;
    }
    // Closed enrolment only refuses a Login for an unknown number. Signup asks for one.
    let _when_closed = SIGNUP_CREATES_WHEN_ENROLMENT_IS_CLOSED_ASSUMED;
    match create(world) {
        Ok(id) => vec![acknowledged(id)],
        Err(error) => {
            warn!(%error, "signup could not create an account");
            Vec::new()
        }
    }
}

/// True when this row is the placeholder signup stored, so a Login may replace its password.
pub(crate) fn is_placeholder(password: &EncodedPassword) -> bool {
    SIGNUP_ADOPTS_PRESENTED_PASSWORD_ASSUMED && password.0 == SIGNUP_PASSWORD_ASSUMED
}

fn create(world: &World) -> Result<AccountId, StoreError> {
    let record = AccountRecord::new(EncodedPassword(SIGNUP_PASSWORD_ASSUMED), "");
    for _ in 0..ATTEMPTS {
        let id = next_id(world.store.highest_account_id()?)?;
        match world.store.create_account(id, record.clone()) {
            Ok(()) => {
                grant_starting_balance(world.store.as_ref(), id)?;
                return Ok(id);
            }
            Err(StoreError::AccountExists(_)) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(StoreError::Backend(
        "signup could not reserve an account number".into(),
    ))
}

fn next_id(highest: Option<AccountId>) -> Result<AccountId, StoreError> {
    let floor = SIGNUP_FIRST_ACCOUNT_ASSUMED.saturating_sub(1);
    let base = highest.map(|id| id.0).unwrap_or(0).max(floor);
    let id = base
        .checked_add(1)
        .ok_or_else(|| StoreError::Backend("the account-number space is used up".into()))?;
    Ok(AccountId(id))
}

fn acknowledged(id: AccountId) -> HostMessage {
    let (low, high) = id.words();
    let mut tail = Vec::with_capacity(4);
    tail.extend(low.to_le_bytes());
    tail.extend(high.to_le_bytes());
    HostMessage::Ack(Ack {
        to: Sid::GAME_OBJECT,
        which_cmd: Command::Signup.byte(),
        which_sub: SIGNUP_SUB,
        tail,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::assumptions::{
        MONEY_STARTING_BALANCE, SIGNUP_FIRST_ACCOUNT_ASSUMED, SIGNUP_PASSWORD_ASSUMED,
    };
    use crate::message::Command;
    use crate::{
        AccountBook, AccountId, AccountRecord, ClientMessage, ClientVersion, ConnectionId,
        EncodedPassword, Enrolment, HostMessage, LandType, Login, MemoryStore, ObjectStore,
        PasswordSource, PlayerSession, Sid, Signup, Store, StoreError, World,
    };

    const ME: ConnectionId = ConnectionId(1);

    fn signup(form: &[u8]) -> ClientMessage {
        ClientMessage::Signup(Signup {
            cookie: 0x1234,
            form: form.to_vec(),
        })
    }

    fn login(account: u32, password: u8) -> ClientMessage {
        ClientMessage::Login(Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(account),
            password_source: PasswordSource::Typed,
            password: EncodedPassword([password; 10]),
            name: "guybrush".into(),
            prodigy_id: None,
        })
    }

    fn closed_world() -> (World, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::new());
        let world = World {
            accounts: AccountBook::stored(store.clone(), Enrolment::Closed),
            store: store.clone(),
            ..World::stock()
        };
        (world, store)
    }

    fn ack_id(message: &HostMessage) -> AccountId {
        let HostMessage::Ack(ack) = message else {
            panic!("signup answers with an Ack, got {message:?}");
        };
        assert_eq!(
            (ack.to, ack.which_cmd, ack.which_sub),
            (Sid::GAME_OBJECT, Command::Signup.byte(), SIGNUP_SUB)
        );
        let tail = ack.tail.as_slice();
        let low = tail.get(..2).and_then(|bytes| bytes.try_into().ok());
        let high = tail.get(2..4).and_then(|bytes| bytes.try_into().ok());
        let (Some(low), Some(high)) = (low, high) else {
            panic!("the account number is two words");
        };
        AccountId::from_words(u16::from_le_bytes(low), u16::from_le_bytes(high))
    }

    #[test]
    fn signup_before_login_creates_an_account_and_the_first_login_sets_its_password() {
        let (world, store) = closed_world();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let replies = player.handle(&world, &mut objects, ME, &signup(b"form-bytes"));
        let id = ack_id(&replies[0].message);
        assert_eq!(id, AccountId(SIGNUP_FIRST_ACCOUNT_ASSUMED));
        assert!(
            player.account().is_none(),
            "signup does not log the session in"
        );
        let record = store.account(id).unwrap().unwrap();
        assert_eq!(record.password.0, SIGNUP_PASSWORD_ASSUMED);
        assert_eq!(record.persona, "");
        assert_eq!(store.money(id).unwrap(), Some(MONEY_STARTING_BALANCE));

        let admitted = player.handle(&world, &mut objects, ME, &login(id.0, 9));
        assert!(matches!(
            admitted.first().map(|delivery| &delivery.message),
            Some(HostMessage::LoginAccepted(_))
        ));
        let record = store.account(id).unwrap().unwrap();
        assert_eq!(record.password, EncodedPassword([9; 10]));
        let refused = player.handle(&world, &mut objects, ME, &login(id.0, 8));
        assert!(matches!(
            refused.first().map(|delivery| &delivery.message),
            Some(HostMessage::Nak(_))
        ));
    }

    #[test]
    fn a_second_signup_takes_the_next_number_above_the_floor() {
        let (world, store) = closed_world();
        store
            .create_account(
                AccountId(50),
                AccountRecord::new(EncodedPassword([1; 10]), "low"),
            )
            .unwrap();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let first = player.handle(&world, &mut objects, ME, &signup(b""));
        assert_eq!(ack_id(&first[0].message), AccountId(100_001));
        let second = player.handle(&world, &mut objects, ME, &signup(b""));
        assert_eq!(ack_id(&second[0].message), AccountId(100_002));
    }

    #[test]
    fn signup_skips_a_deleted_account_number() {
        let (world, store) = closed_world();
        let retired = AccountId(SIGNUP_FIRST_ACCOUNT_ASSUMED);
        store
            .create_account(
                retired,
                AccountRecord::new(EncodedPassword([1; 10]), "gone"),
            )
            .unwrap();
        store.retire_account(retired).unwrap();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let replies = player.handle(&world, &mut objects, ME, &signup(b""));
        let id = ack_id(&replies[0].message);
        assert_eq!(id, AccountId(SIGNUP_FIRST_ACCOUNT_ASSUMED + 1));
        assert!(store.account(retired).unwrap().is_none());
        assert!(matches!(
            store.create_account(
                retired,
                AccountRecord::new(EncodedPassword([1; 10]), "gone"),
            ),
            Err(StoreError::AccountExists(_))
        ));
    }

    #[test]
    fn an_open_book_issues_a_number_and_login_does_not_read_it() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let replies = player.handle(&world, &mut objects, ME, &signup(b"form"));
        let id = ack_id(&replies[0].message);
        let stored = world.store.account(id).unwrap().unwrap();
        assert_eq!(stored.password.0, SIGNUP_PASSWORD_ASSUMED);
        let admitted = player.handle(&world, &mut objects, ME, &login(id.0, 3));
        assert!(matches!(
            admitted.first().map(|delivery| &delivery.message),
            Some(HostMessage::LoginAccepted(_))
        ));
        let stored = world.store.account(id).unwrap().unwrap();
        assert_eq!(stored.password.0, SIGNUP_PASSWORD_ASSUMED);
    }
}
