use crate::assumptions::{
    ANY_VERSION_MAX_ASSUMED, ANY_VERSION_MIN_ASSUMED, OPEN_LAND_FLAGS_ASSUMED,
};
use crate::presence::waiting_room;
use crate::{
    ClientVersion, HostNumber, LandDirectory, LandEntry, LandNumber, LandType, ObjectStore,
    Occupancy, Stamp,
};

/// One land this host runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Land {
    land_type: LandType,
    land_number: LandNumber,
    /// One word, `_` for a space: the client stores it in a space-separated file.
    description: &'static str,
    maximum: u8,
    min_version: ClientVersion,
    max_version: ClientVersion,
}

/// The lands of the stock CD client's `LAND.CFG` that run in `LSCITV` itself.
const STOCK_LANDS: [Land; 3] = [
    Land {
        land_type: LandType(1),
        land_number: LandNumber(1),
        description: "Clubhouse",
        maximum: 64,
        min_version: ANY_VERSION_MIN_ASSUMED,
        max_version: ANY_VERSION_MAX_ASSUMED,
    },
    Land {
        land_type: LandType(2),
        land_number: LandNumber(1),
        description: "SierraLand",
        maximum: 64,
        min_version: ANY_VERSION_MIN_ASSUMED,
        max_version: ANY_VERSION_MAX_ASSUMED,
    },
    Land {
        land_type: LandType(3),
        land_number: LandNumber(1),
        description: "CasinoLand",
        maximum: 64,
        min_version: ANY_VERSION_MIN_ASSUMED,
        max_version: ANY_VERSION_MAX_ASSUMED,
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

    /// The same catalog with the lands of this type open only to clients from `min` to `max`.
    #[must_use]
    pub fn limited_to_versions(
        mut self,
        land_type: LandType,
        min: ClientVersion,
        max: ClientVersion,
    ) -> Self {
        for land in self
            .lands
            .iter_mut()
            .filter(|land| land.land_type == land_type)
        {
            (land.min_version, land.max_version) = (min, max);
        }
        self
    }

    /// Whether some land of this type runs here.
    pub fn runs(&self, land_type: LandType) -> bool {
        self.lands.iter().any(|land| land.land_type == land_type)
    }

    pub fn directory(&self, host: HostNumber) -> LandDirectory {
        let entry = |land: &Land| LandEntry {
            host,
            land_type: land.land_type,
            land_number: land.land_number,
            min_version: land.min_version,
            max_version: land.max_version,
            flags: OPEN_LAND_FLAGS_ASSUMED,
            description: land.description.to_owned(),
        };
        LandDirectory {
            stamp: self.stamp,
            lands: self.lands.iter().map(entry).collect(),
        }
    }

    /// Whether the directory row of this land type lets a client of this version in; a type this
    /// host does not run has no range to break.
    pub fn admits(&self, land_type: LandType, version: ClientVersion) -> bool {
        let mut rows = self.lands.iter().filter(|land| land.land_type == land_type);
        rows.all(|land| (land.min_version..=land.max_version).contains(&version))
    }

    /// `current` is the member count of the land's waiting room; `maximum` stays the catalog's.
    pub fn occupancy(&self, host: HostNumber, objects: &ObjectStore) -> Vec<Occupancy> {
        let row = |land: &Land| {
            let waiting = objects.member_count(waiting_room(land.land_type, land.land_number));
            Occupancy {
                host,
                land_type: land.land_type,
                land_number: land.land_number,
                maximum: land.maximum,
                current: u8::try_from(waiting).unwrap_or(u8::MAX),
            }
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
        let occupancy = table.occupancy(HostNumber(7), &ObjectStore::new());
        assert_eq!(occupancy[0].land_type, LandType(1));
        assert_eq!(occupancy[0].maximum, 64);
        assert!(occupancy.iter().all(|row| row.current == 0));
        assert!(table.runs(LandType(3)) && !table.runs(LandType(0x8B)));
    }
}
