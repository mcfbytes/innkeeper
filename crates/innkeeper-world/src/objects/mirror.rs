use std::collections::{BTreeMap, BTreeSet};

use tracing::warn;

use super::{ConnectionId, Delivery, ObjectStore, Role};
use crate::assumptions::{REPLICAS_INCLUDE_GROUP_FELLOWS_ASSUMED, UNSET_INT_PROPERTY_ASSUMED};
use crate::{
    HostMessage, MemberProperties, MemberRow, Property, PropertyColumn, PropertyKind,
    PropertyRequest, PropertyValue, PropertyValues, SetInt, SetStr, Sid,
};

/// The last value `SetInt` or `SetStr` gave each property of each live object, by byte offset.
#[derive(Debug, Default)]
pub(super) struct PropertyMirror {
    objects: BTreeMap<Sid, BTreeMap<u16, PropertyValue>>,
}

impl PropertyMirror {
    fn set(&mut self, sid: Sid, offset: u16, value: PropertyValue) {
        self.objects.entry(sid).or_default().insert(offset, value);
    }

    fn get(&self, sid: Sid, offset: u16) -> Option<&PropertyValue> {
        self.objects.get(&sid)?.get(&offset)
    }

    pub(super) fn forget(&mut self, sid: Sid) {
        self.objects.remove(&sid);
    }
}

impl ObjectStore {
    /// The connections with a copy of `sid`: its holders, the holders of its members, and the
    /// connections that share a group with it.
    pub fn replicas(&self, sid: Sid) -> BTreeSet<ConnectionId> {
        let Some(object) = self.objects.get(&sid) else {
            return BTreeSet::new();
        };
        let mut replicas: BTreeSet<ConnectionId> = object.holders.keys().copied().collect();
        replicas.extend(self.connections_of(self.members_of(sid)));
        if REPLICAS_INCLUDE_GROUP_FELLOWS_ASSUMED {
            for group in self.groups_containing(sid) {
                replicas.extend(self.connections_of(self.members_of(group)));
            }
        }
        replicas
    }

    pub(crate) fn mirror_ints(&mut self, set: &SetInt) {
        if self.is_live(set.target) {
            for property in &set.properties {
                let value = PropertyValue::Int(property.value);
                self.mirror.set(set.target, property.offset, value);
            }
        }
    }

    pub(crate) fn mirror_text(&mut self, set: &SetStr) {
        if self.is_live(set.target) {
            let value = PropertyValue::text(&set.value);
            self.mirror.set(set.target, set.offset, value);
        }
    }

    /// getProp (32): the mirrored values of the requested properties, in request order; a property
    /// nobody has set is left out.
    pub(crate) fn properties(
        &self,
        connection: ConnectionId,
        request: &PropertyRequest,
    ) -> Vec<Delivery> {
        if !self.is_live(request.target) {
            warn!(?request, "getProp for a SID nobody holds");
            return Vec::new();
        }
        let properties = request.offsets.iter().filter_map(|&offset| {
            let value = self.mirror.get(request.target, offset)?.clone();
            Some(Property { offset, value })
        });
        let values = PropertyValues {
            to: request.target,
            properties: properties.collect(),
        };
        vec![Delivery::new(connection, HostMessage::Properties(values))]
    }

    /// GrpGetProp (31): one row per member in joining order, one column per requested property.
    pub(crate) fn member_properties(
        &self,
        connection: ConnectionId,
        request: &PropertyRequest,
    ) -> Vec<Delivery> {
        let Some(Role::Group { members, .. }) = self.objects.get(&request.target).map(|o| &o.role)
        else {
            warn!(
                ?request,
                "member properties of something that is not a group"
            );
            return Vec::new();
        };
        let columns: Vec<PropertyColumn> = request
            .offsets
            .iter()
            .map(|&offset| self.column(members, offset))
            .collect();
        let rows = members.iter().map(|&member| MemberRow {
            member,
            values: columns
                .iter()
                .map(|column| self.cell(member, *column))
                .collect(),
        });
        let table = MemberProperties {
            group: request.target,
            from: request.from,
            rows: rows.collect(),
            columns,
        };
        vec![Delivery::new(
            connection,
            HostMessage::MemberProperties(table),
        )]
    }

    /// A column holds text when any member's mirror has text there.
    fn column(&self, members: &[Sid], offset: u16) -> PropertyColumn {
        let values = members
            .iter()
            .filter_map(|&member| self.mirror.get(member, offset));
        let has_text = values
            .map(PropertyValue::kind)
            .any(|kind| kind == PropertyKind::Text);
        let kind = if has_text {
            PropertyKind::Text
        } else {
            PropertyKind::Int
        };
        PropertyColumn { offset, kind }
    }

    fn cell(&self, member: Sid, column: PropertyColumn) -> PropertyValue {
        match (self.mirror.get(member, column.offset), column.kind) {
            (Some(value), kind) if value.kind() == kind => value.clone(),
            (_, PropertyKind::Int) => PropertyValue::Int(UNSET_INT_PROPERTY_ASSUMED),
            (_, PropertyKind::Text) => PropertyValue::Text(Vec::new()),
        }
    }

    fn is_live(&self, sid: Sid) -> bool {
        self.objects.contains_key(&sid)
    }
}
