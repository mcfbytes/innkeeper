use crate::assumptions::{
    ANY_VERSION_MAX_ASSUMED, ANY_VERSION_MIN_ASSUMED, OPEN_LAND_FLAGS_ASSUMED,
};
use crate::presence::waiting_room;
use crate::rpg::{RpgTitle, RPG_TITLES};
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
const LSCI_LANDS: [Land; 3] = [
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

impl Land {
    /// Place 1 of an RPG title; its directory row admits any version, the land GrpJoin checks it.
    fn of_title(title: &RpgTitle) -> Self {
        Land {
            land_type: title.land_type,
            land_number: LandNumber(1),
            description: title.description,
            maximum: title.land_capacity,
            min_version: ANY_VERSION_MIN_ASSUMED,
            max_version: ANY_VERSION_MAX_ASSUMED,
        }
    }
}

impl LandCatalog {
    /// Every land of the stock `LAND.CFG`: the three LSCI lands, then the RPG titles.
    pub fn stock() -> Self {
        let mut catalog = LandCatalog::lsci();
        catalog.lands.extend(RPG_TITLES.iter().map(Land::of_title));
        // A client that kept the three-land directory under stamp 1 must not keep it.
        catalog.stamp = Stamp(2);
        catalog
    }

    /// The three LSCI lands only, as the live logon capture was served.
    pub fn lsci() -> Self {
        LandCatalog {
            stamp: Stamp(1),
            lands: LSCI_LANDS.to_vec(),
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

    /// The login Ack's `rating` word as the hub reads it: bit `landType - 1` lets the player enter
    /// lands of that type (`hub/script.050 proc_310`). One bit per land type this host runs.
    pub fn rights(&self) -> u16 {
        let bits = self.lands.iter().filter_map(|land| {
            let bit = u32::from(land.land_type.0).checked_sub(1)?;
            1u16.checked_shl(bit)
        });
        bits.fold(0, |rights, bit| rights | bit)
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
        for land in LandCatalog::stock().lands {
            let description = land.description;
            assert!(description.is_ascii() && !description.contains([' ', '\0']));
            assert!(description.len() <= 16, "{description} is cut at 16");
        }
    }

    #[test]
    fn directory_and_occupancy_list_every_land_on_this_host() {
        let table = LandCatalog::stock();
        let directory = table.directory(HostNumber(7));
        let names: Vec<_> = directory
            .lands
            .iter()
            .map(|land| land.description.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Clubhouse",
                "SierraLand",
                "CasinoLand",
                "Yserbius",
                "Cawdor",
                "Twinion"
            ]
        );
        assert!(directory
            .lands
            .iter()
            .all(|land| land.host == HostNumber(7)));
        let occupancy = table.occupancy(HostNumber(7), &ObjectStore::new());
        assert_eq!(occupancy[0].land_type, LandType(1));
        assert_eq!(occupancy[0].maximum, 64);
        assert!(occupancy.iter().all(|row| row.current == 0));
        assert!(table.runs(LandType(3)) && table.runs(LandType(4)) && table.runs(LandType(6)));
        assert!(!table.runs(LandType(0x8B)));
        assert_eq!(occupancy[3].maximum, 100);
        assert_eq!(occupancy[4].land_type, LandType(6));
        assert_eq!(occupancy[4].maximum, 100);
        assert_eq!(table.rights(), 0x00AF);
        assert_eq!(LandCatalog::lsci().rights(), 0x0007);
    }
}
