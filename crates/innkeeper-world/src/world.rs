use crate::host_time::{Clock, SystemClock};
use crate::{AccountBook, HostNumber, LandCatalog};

/// The first host of the stock `HOSTADDR` (`Sierra7`); every stock land runs here.
const STOCK_HOST: HostNumber = HostNumber(7);

/// What every session on this host shares and nobody changes at run time.
#[derive(Debug)]
pub struct World {
    pub host: HostNumber,
    pub accounts: AccountBook,
    pub lands: LandCatalog,
    pub clock: Box<dyn Clock>,
}

impl World {
    pub fn stock() -> Self {
        World {
            host: STOCK_HOST,
            accounts: AccountBook::anyone(),
            lands: LandCatalog::stock(),
            clock: Box::new(SystemClock),
        }
    }
}
