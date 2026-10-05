use crate::assumptions::PROPERTIES_UNREAD_WORD_ASSUMED;
use crate::message::wire::{fitting_count, row_count, WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// Byte 1 of every member-properties request: the client sends command 31 as the word `0x011F`.
const MEMBER_PROPERTIES_REQUEST_FLAG: u8 = 1;

/// How the client reads a property value: a word, or bytes up to a NUL. Any other code would make
/// the client's `SetMsg` loop never end, so the codec knows only these two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PropertyKind {
    Int = 0,
    Text = 1,
}

/// A property value as `SetInt` or `SetStr` last set it; text is kept without its NUL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyValue {
    Int(u16),
    Text(Vec<u8>),
}

/// One property of an object: its byte offset in the object and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub offset: u16,
    pub value: PropertyValue,
}

/// Command 32 (getProp) about `target`, or 31 (GrpGetProp) about every member of the group `target`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyRequest {
    pub target: Sid,
    pub from: Sid,
    pub offsets: Vec<u16>,
}

/// Command 33 (`SetMsg`): property values the client applies to the object `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyValues {
    pub to: Sid,
    pub properties: Vec<Property>,
}

/// Command 31 from the host: a table with one column per requested property and one row per member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberProperties {
    pub group: Sid,
    pub from: Sid,
    pub columns: Vec<PropertyColumn>,
    pub rows: Vec<MemberRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyColumn {
    pub offset: u16,
    pub kind: PropertyKind,
}

/// One member's values, in column order; each value has its column's kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub member: Sid,
    pub values: Vec<PropertyValue>,
}

impl PropertyKind {
    fn parse(code: u16) -> Result<Self, MessageError> {
        match code {
            0 => Ok(PropertyKind::Int),
            1 => Ok(PropertyKind::Text),
            value => Err(MessageError::UnknownValue {
                field: "property type",
                value,
            }),
        }
    }
}

impl PropertyValue {
    pub fn kind(&self) -> PropertyKind {
        match self {
            PropertyValue::Int(_) => PropertyKind::Int,
            PropertyValue::Text(_) => PropertyKind::Text,
        }
    }

    /// The value of a `SetStr`: the client reads text only up to its first NUL.
    pub fn text(bytes: &[u8]) -> Self {
        let text = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
        PropertyValue::Text(text.to_vec())
    }

    fn parse(reader: &mut WireReader, kind: PropertyKind) -> Result<Self, MessageError> {
        match kind {
            PropertyKind::Int => Ok(PropertyValue::Int(reader.word("value")?)),
            PropertyKind::Text => Ok(PropertyValue::Text(reader.until_nul("value")?)),
        }
    }

    fn write(&self, writer: &mut WireWriter) {
        match self {
            PropertyValue::Int(value) => {
                writer.word(*value);
            }
            PropertyValue::Text(bytes) => {
                writer.array(bytes).byte(0);
            }
        }
    }
}

impl PropertyRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let target = Sid(reader.word("sid")?);
        let from = Sid(reader.word("fromSID")?);
        let mut offsets = Vec::new();
        while !reader.is_empty() {
            offsets.push(reader.word("propOffset")?);
        }
        Ok(PropertyRequest {
            target,
            from,
            offsets,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter, command: Command) {
        let flag = match command {
            Command::MemberProperties => MEMBER_PROPERTIES_REQUEST_FLAG,
            _ => 0,
        };
        writer.byte(command.byte()).byte(flag);
        writer.word(self.target.0).word(self.from.0);
        for offset in &self.offsets {
            writer.word(*offset);
        }
    }
}

impl PropertyValues {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let to = Sid(reader.word("toSID")?);
        reader.word("unused")?;
        let mut properties = Vec::new();
        while !reader.is_empty() {
            let kind = PropertyKind::parse(u16::from(reader.byte("type")?))?;
            let offset = reader.word("propOffset")?;
            let value = PropertyValue::parse(reader, kind)?;
            properties.push(Property { offset, value });
        }
        Ok(PropertyValues { to, properties })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Properties.byte()).byte(0);
        writer.word(self.to.0).word(PROPERTIES_UNREAD_WORD_ASSUMED);
        for property in &self.properties {
            writer
                .byte(property.value.kind() as u8)
                .word(property.offset);
            property.value.write(writer);
        }
    }
}

impl MemberProperties {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let group = Sid(reader.word("toSID")?);
        let from = Sid(reader.word("fromSID")?);
        let row_total = reader.word("members")?;
        let column_total = reader.word("properties")?;
        let columns = (0..column_total).map(|_| {
            let offset = reader.word("propOffset")?;
            let kind = PropertyKind::parse(reader.word("type")?)?;
            Ok(PropertyColumn { offset, kind })
        });
        let columns: Vec<PropertyColumn> = columns.collect::<Result<_, MessageError>>()?;
        let rows = (0..row_total).map(|_| MemberRow::parse(reader, &columns));
        Ok(MemberProperties {
            group,
            from,
            rows: rows.collect::<Result<_, _>>()?,
            columns,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        let (columns, rows) = (fitting_count(&self.columns), fitting_count(&self.rows));
        writer.byte(Command::MemberProperties.byte()).byte(0);
        writer.word(self.group.0).word(self.from.0);
        writer.word(row_count(rows)).word(row_count(columns));
        for column in columns {
            writer.word(column.offset).word(column.kind as u16);
        }
        for row in rows {
            writer.word(row.member.0);
            row.values.iter().for_each(|value| value.write(writer));
        }
    }
}

impl MemberRow {
    fn parse(reader: &mut WireReader, columns: &[PropertyColumn]) -> Result<Self, MessageError> {
        let member = Sid(reader.word("member")?);
        let values = columns
            .iter()
            .map(|column| PropertyValue::parse(reader, column.kind));
        Ok(MemberRow {
            member,
            values: values.collect::<Result<_, _>>()?,
        })
    }
}
