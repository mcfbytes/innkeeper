use std::collections::HashMap;

use crate::{AccountId, EncodedPassword, Login};

/// Who may log in. Nothing is stored on disk yet, so a fresh host knows no passwords.
#[derive(Clone, Debug)]
pub struct AccountBook {
    admission: Admission,
}

#[derive(Clone, Debug)]
enum Admission {
    /// Every account number with any password: the stand-in until accounts persist.
    Anyone,
    Listed(HashMap<AccountId, EncodedPassword>),
}

/// A logged-in account and the persona it plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub persona: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    UnknownAccount,
    WrongPassword,
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

    pub fn admit(&self, login: &Login) -> Result<Account, Refusal> {
        if let Admission::Listed(passwords) = &self.admission {
            let stored = passwords
                .get(&login.account)
                .ok_or(Refusal::UnknownAccount)?;
            if *stored != login.password {
                return Err(Refusal::WrongPassword);
            }
        }
        Ok(Account {
            id: login.account,
            persona: login.name.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientVersion, LandType, PasswordSource};

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
}
