use std::sync::Arc;

use crate::account::Enrolment;
use crate::assumptions::STORED_BOOK_ENROLS_ASSUMED;
use crate::host_time::{Clock, SystemClock};
use crate::{AccountBook, HostNumber, LandCatalog, MemoryStore, Store};

/// The first host of the stock `HOSTADDR` (`Sierra7`); every stock land runs here.
const STOCK_HOST: HostNumber = HostNumber(7);

/// What every session on this host shares and nobody changes at run time.
#[derive(Debug)]
pub struct World {
    pub host: HostNumber,
    pub accounts: AccountBook,
    pub lands: LandCatalog,
    pub clock: Box<dyn Clock>,
    pub store: Arc<dyn Store>,
}

impl World {
    /// The dev host: any account and password is admitted and nothing is kept.
    pub fn stock() -> Self {
        World {
            host: STOCK_HOST,
            accounts: AccountBook::anyone(),
            lands: LandCatalog::stock(),
            clock: Box::new(SystemClock),
            store: Arc::new(MemoryStore::new()),
        }
    }

    /// The stock host with its accounts kept in `store`.
    pub fn stored(store: Arc<dyn Store>) -> Self {
        let enrolment = match STORED_BOOK_ENROLS_ASSUMED {
            true => Enrolment::Open,
            false => Enrolment::Closed,
        };
        World {
            accounts: AccountBook::stored(Arc::clone(&store), enrolment),
            store,
            ..World::stock()
        }
    }
}
