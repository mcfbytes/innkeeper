use crate::assumptions::{
    ANY_VERSION_MAX_ASSUMED, ANY_VERSION_MIN_ASSUMED, OPEN_LAND_FLAGS_ASSUMED,
};
use crate::{HostNumber, LandDirectory, LandEntry, LandNumber, LandType, Occupancy, Stamp};

/// One land this host runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Land {
    land_type: LandType,
    land_number: LandNumber,
    /// One word, `_` for a space: the client stores it in a space-separated file.
    description: &'static str,
    maximum: u8,
}

/// The lands of the stock CD client's `LAND.CFG` that run in `LSCITV` itself.
const STOCK_LANDS: [Land; 3] = [
    Land {
        land_type: LandType(1),
        land_number: LandNumber(1),
        description: "Clubhouse",
        maximum: 64,
    },
    Land {
        land_type: LandType(2),
        land_number: LandNumber(1),
        description: "SierraLand",
        maximum: 64,
    },
    Land {
        land_type: LandType(3),
        land_number: LandNumber(1),
        description: "CasinoLand",
        maximum: 64,
    },
];

/// The land directory and occupancy the host reports; the client polls both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandCatalog {
    stamp: Stamp,
    lands: Vec<Land>,
}

impl LandCatalog {
    pub fn stock() -> Self {
        LandCatalog {
            stamp: Stamp(1),
            lands: STOCK_LANDS.to_vec(),
        }
    }

    pub fn directory(&self, host: HostNumber) -> LandDirectory {
        let entry = |land: &Land| LandEntry {
            host,
            land_type: land.land_type,
            land_number: land.land_number,
            min_version: ANY_VERSION_MIN_ASSUMED,
            max_version: ANY_VERSION_MAX_ASSUMED,
            flags: OPEN_LAND_FLAGS_ASSUMED,
            description: land.description.to_owned(),
        };
        LandDirectory {
            stamp: self.stamp,
            lands: self.lands.iter().map(entry).collect(),
        }
    }

    /// Nobody is counted yet: each session sees only its own player.
    pub fn occupancy(&self, host: HostNumber) -> Vec<Occupancy> {
        let row = |land: &Land| Occupancy {
            host,
            land_type: land.land_type,
            land_number: land.land_number,
            maximum: land.maximum,
            current: 0,
        };
        self.lands.iter().map(row).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_descriptions_survive_the_clients_land_file() {
        for land in STOCK_LANDS {
            let description = land.description;
            assert!(description.is_ascii() && !description.contains([' ', '\0']));
            assert!(description.len() <= 16, "{description} is cut at 16");
        }
    }

    #[test]
    fn directory_and_occupancy_list_every_land_on_this_host() {
        let table = LandCatalog::stock();
        let directory = table.directory(HostNumber(7));
        assert_eq!(directory.lands.len(), 3);
        assert!(directory
            .lands
            .iter()
            .all(|land| land.host == HostNumber(7)));
        let occupancy = table.occupancy(HostNumber(7));
        assert_eq!(occupancy[0].land_type, LandType(1));
        assert_eq!(occupancy[0].maximum, 64);
    }
}
