//! The field maps of the character files: which bytes of the header and of a slot's record mean
//! what, as JSON beside this file (one per game). The editor shows and changes values through
//! them; a byte nobody has named is still in the map as an `unknown_*` run of raw bytes, so every
//! byte of a record can be edited. docs/server/ybarra.md section 4.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{Game, HEADER_BYTES, SLOT_BYTES};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    /// A NUL-terminated string of at most `len` bytes, NUL included.
    Str,
    /// Raw bytes, shown and edited as hex.
    Bytes,
}

impl Kind {
    /// Bytes of one element; `len` for strings and raw bytes.
    pub(crate) fn width(self, len: usize) -> usize {
        match self {
            Kind::U8 | Kind::I8 => 1,
            Kind::U16 | Kind::I16 => 2,
            Kind::U32 | Kind::I32 => 4,
            Kind::Str | Kind::Bytes => len,
        }
    }

    fn range(self) -> (i64, i64) {
        match self {
            Kind::U8 => (0, 0xff),
            Kind::I8 => (-0x80, 0x7f),
            Kind::U16 => (0, 0xffff),
            Kind::I16 => (-0x8000, 0x7fff),
            Kind::U32 => (0, 0xffff_ffff),
            Kind::I32 => (-0x8000_0000, 0x7fff_ffff),
            Kind::Str | Kind::Bytes => (0, 0),
        }
    }

    pub(crate) fn is_number(self) -> bool {
        !matches!(self, Kind::Str | Kind::Bytes)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Field {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) group: Option<String>,
    pub(crate) offset: usize,
    #[serde(rename = "type")]
    pub(crate) kind: Kind,
    #[serde(default = "one")]
    pub(crate) len: usize,
    #[serde(default = "one")]
    pub(crate) count: usize,
    /// Bytes from one array element to the next; 0 means packed (the element width).
    #[serde(default)]
    pub(crate) stride: usize,
    #[serde(default)]
    pub(crate) min: Option<i64>,
    #[serde(default)]
    pub(crate) max: Option<i64>,
    /// Names of values, keyed by the value in decimal.
    #[serde(default, rename = "enum")]
    pub(crate) values: BTreeMap<String, String>,
    /// Names of array elements.
    #[serde(default)]
    pub(crate) items: Vec<String>,
    #[serde(default)]
    pub(crate) confidence: Option<String>,
    #[serde(default)]
    pub(crate) evidence: Option<String>,
    /// Another field this one is a copy of: the editor writes both from the other's value and
    /// does not offer this one.
    #[serde(default)]
    pub(crate) mirror_of: Option<String>,
    /// Shown in the list of a file's characters.
    #[serde(default)]
    pub(crate) summary: bool,
}

fn one() -> usize {
    1
}

impl Field {
    pub(crate) fn width(&self) -> usize {
        self.kind.width(self.len)
    }

    pub(crate) fn stride(&self) -> usize {
        if self.stride == 0 {
            self.width()
        } else {
            self.stride
        }
    }

    pub(crate) fn element_at(&self, element: usize) -> usize {
        self.offset + element * self.stride()
    }

    /// The bytes the field covers, every element.
    pub(crate) fn span(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.count).flat_map(move |element| {
            let at = self.element_at(element);
            at..at + self.width()
        })
    }

    /// The allowed values: the map's bounds within the type's.
    pub(crate) fn bounds(&self) -> (i64, i64) {
        let (low, high) = self.kind.range();
        (
            self.min.map_or(low, |min| min.max(low)),
            self.max.map_or(high, |max| max.min(high)),
        )
    }

    pub(crate) fn item_name(&self, element: usize) -> String {
        self.items
            .get(element)
            .cloned()
            .unwrap_or_else(|| format!("#{}", element + 1))
    }

    pub(crate) fn read(&self, bytes: &[u8], element: usize) -> Value {
        let at = self.element_at(element);
        let Some(field) = bytes.get(at..at + self.width()) else {
            return Value::Raw(Vec::new());
        };
        match self.kind {
            Kind::Str => Value::Text(
                field
                    .iter()
                    .take_while(|byte| **byte != 0)
                    .map(|byte| char::from(*byte))
                    .collect(),
            ),
            Kind::Bytes => Value::Raw(field.to_vec()),
            kind => {
                let mut wide = [0u8; 8];
                for (to, from) in wide.iter_mut().zip(field) {
                    *to = *from;
                }
                let value = i64::from_le_bytes(wide);
                let unused = 64 - 8 * u32::try_from(field.len()).unwrap_or(8);
                let signed = matches!(kind, Kind::I8 | Kind::I16 | Kind::I32);
                Value::Number(if signed { (value << unused) >> unused } else { value })
            }
        }
    }

    /// Parses what the form sent for one element and writes it; the bytes are untouched on error.
    pub(crate) fn write(&self, bytes: &mut [u8], element: usize, text: &str) -> Result<(), String> {
        let at = self.element_at(element);
        let width = self.width();
        let target = bytes
            .get_mut(at..at + width)
            .ok_or_else(|| "outside the record".to_owned())?;
        match self.kind {
            Kind::Str => {
                if !text.bytes().all(|byte| (0x20..0x7f).contains(&byte)) {
                    return Err("letters, digits, spaces and punctuation only".to_owned());
                }
                if text.len() >= width {
                    return Err(format!("at most {} characters", width.saturating_sub(1)));
                }
                // The bytes after the NUL are kept: the games leave old text there and some
                // records keep flags past a short name.
                let (letters, rest) = target.split_at_mut(text.len());
                letters.copy_from_slice(text.as_bytes());
                if let Some(nul) = rest.first_mut() {
                    *nul = 0;
                }
            }
            Kind::Bytes => {
                let digits: Vec<u8> = text.bytes().filter(|ch| !ch.is_ascii_whitespace()).collect();
                if digits.len() != width * 2 {
                    return Err(format!("exactly {width} bytes ({} hex digits)", width * 2));
                }
                let parsed = digits
                    .chunks_exact(2)
                    .map(|pair| {
                        std::str::from_utf8(pair)
                            .ok()
                            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    })
                    .collect::<Option<Vec<u8>>>()
                    .ok_or_else(|| "hex digits only".to_owned())?;
                target.copy_from_slice(&parsed);
            }
            _ => {
                let value: i64 = text
                    .trim()
                    .parse()
                    .map_err(|_| "a whole number".to_owned())?;
                let (low, high) = self.bounds();
                if value < low || value > high {
                    return Err(format!("from {low} to {high}"));
                }
                for (to, from) in target.iter_mut().zip(value.to_le_bytes()) {
                    *to = from;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Number(i64),
    Text(String),
    Raw(Vec<u8>),
}

impl Value {
    /// As the form shows and sends it.
    pub(crate) fn form_text(&self) -> String {
        match self {
            Value::Number(number) => number.to_string(),
            Value::Text(text) => text.clone(),
            Value::Raw(bytes) => bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Schema {
    pub(crate) header: Vec<Field>,
    pub(crate) record: Vec<Field>,
    #[serde(default)]
    pub(crate) automap: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) invariants: Vec<String>,
}

/// Why a field map does not fit its file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SchemaError {
    Json(String),
    OutOfRange(String),
    Overlap(String, String),
    Gap(usize),
    Mirror(String),
}

impl Schema {
    /// A map that names nothing: every byte as raw hex, 16 to a field.
    fn raw() -> Schema {
        let runs = |size: usize| -> Vec<Field> {
            (0..size)
                .step_by(16)
                .map(|offset| Field {
                    name: format!("unknown_{offset:#05x}"),
                    group: Some("Raw".to_owned()),
                    offset,
                    kind: Kind::Bytes,
                    len: 16.min(size - offset),
                    count: 1,
                    stride: 0,
                    min: None,
                    max: None,
                    values: BTreeMap::new(),
                    items: Vec::new(),
                    confidence: None,
                    evidence: None,
                    mirror_of: None,
                    summary: false,
                })
                .collect()
        };
        Schema {
            header: runs(HEADER_BYTES),
            record: runs(SLOT_BYTES),
            automap: None,
            invariants: Vec::new(),
        }
    }

    pub(crate) fn parse(text: &str) -> Result<Schema, SchemaError> {
        let schema: Schema =
            serde_json::from_str(text).map_err(|error| SchemaError::Json(error.to_string()))?;
        schema.check()?;
        Ok(schema)
    }

    /// Every byte of the header and of a record belongs to exactly one field, every field stays
    /// inside its part, and a mirror names a field of the same shape.
    fn check(&self) -> Result<(), SchemaError> {
        for (fields, size) in [(&self.header, HEADER_BYTES), (&self.record, SLOT_BYTES)] {
            let mut owner: Vec<Option<&str>> = vec![None; size];
            for field in fields {
                for at in field.span() {
                    let slot = owner
                        .get_mut(at)
                        .ok_or_else(|| SchemaError::OutOfRange(field.name.clone()))?;
                    if let Some(other) = slot {
                        return Err(SchemaError::Overlap((*other).to_owned(), field.name.clone()));
                    }
                    *slot = Some(&field.name);
                }
                if let Some(mirror) = &field.mirror_of {
                    let source = fields
                        .iter()
                        .find(|other| &other.name == mirror)
                        .ok_or_else(|| SchemaError::Mirror(field.name.clone()))?;
                    if source.kind != field.kind
                        || source.width() != field.width()
                        || source.count != field.count
                        || source.mirror_of.is_some()
                    {
                        return Err(SchemaError::Mirror(field.name.clone()));
                    }
                }
            }
            if let Some(gap) = owner.iter().position(Option::is_none) {
                return Err(SchemaError::Gap(gap));
            }
        }
        Ok(())
    }
}

/// The field map of a game, parsed once.
pub(crate) fn schema(game: Game) -> &'static Schema {
    use std::sync::OnceLock;
    static MAPS: [OnceLock<Schema>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let (cell, text) = match game {
        Game::Yserbius => (&MAPS[0], include_str!("yserbius.json")),
        Game::Twinion => (&MAPS[1], include_str!("twinion.json")),
        Game::Cawdor => (&MAPS[2], include_str!("cawdor.json")),
    };
    cell.get_or_init(|| {
        Schema::parse(text).unwrap_or_else(|error| {
            tracing::error!(game = game.id(), ?error, "a field map is broken; showing raw bytes");
            Schema::raw()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_map_parses_and_covers_every_byte() {
        for (game, text) in [
            (Game::Yserbius, include_str!("yserbius.json")),
            (Game::Twinion, include_str!("twinion.json")),
            (Game::Cawdor, include_str!("cawdor.json")),
        ] {
            Schema::parse(text).unwrap_or_else(|error| panic!("{}: {error:?}", game.id()));
        }
        assert!(Schema::raw().check().is_ok());
    }

    fn field(json: &str) -> Field {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn numbers_keep_to_their_bounds() {
        let level = field(r#"{"name": "Level", "offset": 2, "type": "u16", "max": 3200}"#);
        let mut bytes = [0u8; 8];
        level.write(&mut bytes, 0, "3200").unwrap();
        assert_eq!(&bytes[2..4], &3200u16.to_le_bytes());
        assert_eq!(level.read(&bytes, 0), Value::Number(3200));
        assert!(level.write(&mut bytes, 0, "3201").is_err());
        assert!(level.write(&mut bytes, 0, "-1").is_err());
        assert!(level.write(&mut bytes, 0, "x").is_err());
        assert_eq!(level.read(&bytes, 0), Value::Number(3200));
        let signed = field(r#"{"name": "s", "offset": 0, "type": "i8"}"#);
        signed.write(&mut bytes, 0, "-2").unwrap();
        assert_eq!(bytes[0], 0xfe);
    }

    #[test]
    fn arrays_strings_and_raw_bytes() {
        let ranks = field(r#"{"name": "Ranks", "offset": 0, "type": "u8", "count": 3, "stride": 2}"#);
        let mut bytes = [0u8; 12];
        ranks.write(&mut bytes, 2, "7").unwrap();
        assert_eq!(bytes[4], 7);
        assert_eq!(ranks.span().collect::<Vec<_>>(), [0, 2, 4]);

        let name = field(r#"{"name": "Name", "offset": 6, "type": "str", "len": 5}"#);
        bytes[6..11].copy_from_slice(b"abcd\0");
        name.write(&mut bytes, 0, "xy").unwrap();
        assert_eq!(&bytes[6..11], b"xy\0d\0");
        assert_eq!(name.read(&bytes, 0), Value::Text("xy".into()));
        assert!(name.write(&mut bytes, 0, "toolong").is_err());
        assert!(name.write(&mut bytes, 0, "<\u{e9}>").is_err());

        let raw = field(r#"{"name": "unknown", "offset": 11, "type": "bytes", "len": 1}"#);
        raw.write(&mut bytes, 0, "Fe").unwrap();
        assert_eq!(bytes[11], 0xfe);
        assert_eq!(raw.read(&bytes, 0).form_text(), "fe");
        assert!(raw.write(&mut bytes, 0, "f").is_err());
    }

    #[test]
    fn a_map_with_a_gap_or_an_overlap_is_refused() {
        let pad = |from: usize, to: usize| {
            format!(r#"{{"name": "pad{from}", "offset": {from}, "type": "bytes", "len": {}}}"#, to - from)
        };
        let header = pad(0, HEADER_BYTES);
        let whole = format!(r#"{{"header": [{header}], "record": [{}]}}"#, pad(0, SLOT_BYTES));
        assert!(Schema::parse(&whole).is_ok());
        let gap = format!(r#"{{"header": [{header}], "record": [{}]}}"#, pad(1, SLOT_BYTES));
        assert_eq!(Schema::parse(&gap).unwrap_err(), SchemaError::Gap(0));
        let overlap = format!(
            r#"{{"header": [{header}], "record": [{}, {}]}}"#,
            pad(0, 10),
            pad(5, SLOT_BYTES)
        );
        assert!(matches!(Schema::parse(&overlap), Err(SchemaError::Overlap(..))));
    }
}
