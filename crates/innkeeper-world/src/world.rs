use std::sync::Arc;

use crate::account::Enrolment;
use crate::assumptions::STORED_BOOK_ENROLS_ASSUMED;
use crate::host_time::{Clock, SystemClock};
use crate::trivia::{Shows, TriviaPools, TriviaShows};
use crate::{
    AccountBook, ConnectionId, Delivery, HostNumber, LandCatalog, LogPolicy, MemoryStore, Store,
};

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
    pub log: LogPolicy,
    pub trivia: TriviaShows,
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
            log: LogPolicy::Verbose,
            trivia: TriviaShows::new(),
        }
    }

    pub(crate) fn with_trivia<T>(&self, body: impl FnOnce(&mut Shows) -> T) -> T {
        let mut shows = self.trivia.lock();
        body(&mut shows)
    }

    /// Drops a connection's trivia seat. A hang-up and a fresh Login both call this.
    pub fn leave_trivia(&self, connection: ConnectionId) {
        self.with_trivia(|shows| shows.disconnect(connection));
    }

    /// Advances every room's Countdown clock. The daemon delivers the packets.
    pub fn tick_trivia(&self) -> Vec<Delivery> {
        let now = self.clock.millis();
        self.with_trivia(|shows| shows.tick(now))
    }

    /// Installs the fitted question bags. The clock seeds the first shuffle of each room.
    pub fn install_trivia(&self, pools: TriviaPools) {
        let seed = self.clock.millis();
        self.with_trivia(|shows| shows.set_pools(pools, seed));
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
