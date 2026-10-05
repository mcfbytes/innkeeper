use std::collections::HashMap;
use std::sync::Arc;

use tracing::warn;

use crate::{
    AccountId, AccountRecord, EncodedPassword, Login, LoginAck, LoginStatus, Store, StoreError,
};

/// Who may log in: a stand-in for dev, a fixed list for tests, or the accounts kept in a store.
#[derive(Clone, Debug)]
pub struct AccountBook {
    admission: Admission,
}

#[derive(Clone, Debug)]
enum Admission {
    /// Every account number with any password: the dev default.
    Anyone,
    Listed(HashMap<AccountId, EncodedPassword>),
    Stored {
        store: Arc<dyn Store>,
        enrolment: Enrolment,
    },
}

/// What a stored book does with an account number it has never seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enrolment {
    Closed,
    /// Create the account from the password the Login presents.
    Open,
}

/// A logged-in account and the persona it plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub persona: String,
    pub user_flags: u16,
    pub rating: u16,
    pub status: LoginStatus,
}

impl Account {
    fn plain(login: &Login) -> Self {
        Account::from_record(login, &AccountRecord::new(login.password, ""))
    }

    fn from_record(login: &Login, record: &AccountRecord) -> Self {
        let status = match record.password_expired {
            true => LoginStatus::PASSWORD_OUT_OF_DATE,
            false => record.status,
        };
        Account {
            id: login.account,
            persona: login.name.clone(),
            user_flags: record.user_flags,
            rating: record.rating,
            status,
        }
    }

    /// What the login Ack tells the client about this account.
    pub fn login_ack(&self) -> LoginAck {
        LoginAck {
            user_flags: self.user_flags,
            status: self.status,
            rating: self.rating,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    UnknownAccount,
    WrongPassword,
    /// The store failed; the account may well exist.
    StoreUnavailable,
}

impl AccountBook {
    pub fn anyone() -> Self {
        AccountBook {
            admission: Admission::Anyone,
        }
    }

    pub fn listed(accounts: impl IntoIterator<Item = (AccountId, EncodedPassword)>) -> Self {
        AccountBook {
            admission: Admission::Listed(accounts.into_iter().collect()),
        }
    }

    pub fn stored(store: Arc<dyn Store>, enrolment: Enrolment) -> Self {
        AccountBook {
            admission: Admission::Stored { store, enrolment },
        }
    }

    pub fn admit(&self, login: &Login) -> Result<Account, Refusal> {
        match &self.admission {
            Admission::Anyone => Ok(Account::plain(login)),
            Admission::Listed(passwords) => {
                let stored = passwords
                    .get(&login.account)
                    .ok_or(Refusal::UnknownAccount)?;
                match *stored == login.password {
                    true => Ok(Account::plain(login)),
                    false => Err(Refusal::WrongPassword),
                }
            }
            Admission::Stored { store, enrolment } => {
                admit_stored(store.as_ref(), *enrolment, login).map_err(refusal_of)
            }
        }
    }

    /// Keeps a new password for an account the book stores; other books have nothing to update.
    pub fn change_password(&self, id: AccountId, password: EncodedPassword) -> Result<(), Refusal> {
        match &self.admission {
            Admission::Stored { store, .. } => match store.update_password(id, password) {
                Err(StoreError::UnknownAccount(_)) => Err(Refusal::UnknownAccount),
                other => other.map_err(|error| refusal_of(Failure::Store(error))),
            },
            Admission::Anyone | Admission::Listed(_) => Ok(()),
        }
    }
}

/// Why a stored admission did not succeed: a verdict about the account, or the store itself.
enum Failure {
    Refused(Refusal),
    Store(StoreError),
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Failure::Store(error)
    }
}

fn refusal_of(failure: Failure) -> Refusal {
    match failure {
        Failure::Refused(refusal) => refusal,
        Failure::Store(error) => {
            warn!(%error, "the account store failed");
            Refusal::StoreUnavailable
        }
    }
}

fn admit_stored(
    store: &dyn Store,
    enrolment: Enrolment,
    login: &Login,
) -> Result<Account, Failure> {
    match store.account(login.account)? {
        Some(record) if record.password == login.password => {
            Ok(Account::from_record(login, &record))
        }
        Some(_) => Err(Failure::Refused(Refusal::WrongPassword)),
        None if enrolment == Enrolment::Open => {
            let record = AccountRecord::new(login.password, login.name.as_str());
            store.create_account(login.account, record.clone())?;
            Ok(Account::from_record(login, &record))
        }
        None => Err(Failure::Refused(Refusal::UnknownAccount)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientVersion, LandType, MemoryStore, PasswordSource};

    fn login(account: u32, password: u8) -> Login {
        Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(account),
            password_source: PasswordSource::StoredFile,
            password: EncodedPassword([password; 10]),
            name: "guybrush".into(),
            prodigy_id: None,
        }
    }

    #[test]
    fn an_open_book_admits_anyone_under_the_persona_name() {
        let account = AccountBook::anyone().admit(&login(100_001, 7));
        let expected = Account {
            id: AccountId(100_001),
            persona: "guybrush".into(),
            user_flags: 0,
            rating: 0,
            status: LoginStatus::default(),
        };
        assert_eq!(account, Ok(expected));
    }

    #[test]
    fn a_listed_book_checks_number_and_password() {
        let book = AccountBook::listed([(AccountId(100_001), EncodedPassword([7; 10]))]);
        assert!(book.admit(&login(100_001, 7)).is_ok());
        assert_eq!(book.admit(&login(100_001, 8)), Err(Refusal::WrongPassword));
        assert_eq!(book.admit(&login(100_002, 7)), Err(Refusal::UnknownAccount));
    }

    fn stored_book(enrolment: Enrolment) -> (AccountBook, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::new());
        let record = AccountRecord {
            user_flags: 0x0086,
            rating: 4,
            ..AccountRecord::new(EncodedPassword([7; 10]), "stored")
        };
        store.create_account(AccountId(100_001), record).unwrap();
        (AccountBook::stored(store.clone(), enrolment), store)
    }

    #[test]
    fn a_stored_account_brings_its_flags_and_rating_and_checks_its_password() {
        let (book, _) = stored_book(Enrolment::Closed);
        let account = book.admit(&login(100_001, 7)).unwrap();
        assert_eq!((account.user_flags, account.rating), (0x0086, 4));
        assert_eq!(account.persona, "guybrush");
        assert_eq!(book.admit(&login(100_001, 8)), Err(Refusal::WrongPassword));
        assert_eq!(book.admit(&login(100_002, 7)), Err(Refusal::UnknownAccount));
    }

    #[test]
    fn an_enrolling_book_creates_the_unknown_account_from_the_presented_password() {
        let (book, store) = stored_book(Enrolment::Open);
        assert!(book.admit(&login(100_002, 9)).is_ok());
        let record = store.account(AccountId(100_002)).unwrap().unwrap();
        assert_eq!(record.password, EncodedPassword([9; 10]));
        assert_eq!(record.persona, "guybrush");
        assert_eq!(book.admit(&login(100_002, 8)), Err(Refusal::WrongPassword));
    }

    #[test]
    fn a_new_password_clears_the_expired_flag_that_made_the_status_11() {
        let (book, store) = stored_book(Enrolment::Closed);
        let id = AccountId(100_003);
        let expired = AccountRecord {
            password_expired: true,
            ..AccountRecord::new(EncodedPassword([7; 10]), "old")
        };
        store.create_account(id, expired).unwrap();
        let status = book.admit(&login(100_003, 7)).unwrap().status;
        assert_eq!(status, LoginStatus::PASSWORD_OUT_OF_DATE);
        book.change_password(id, EncodedPassword([8; 10])).unwrap();
        assert_eq!(
            book.admit(&login(100_003, 8)).unwrap().status,
            LoginStatus(0)
        );
    }
}
