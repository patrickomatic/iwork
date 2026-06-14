//! `Sheet` (message type 2) — a Numbers sheet's name and table membership.
//!
//! The sheet name is `Sheet.field 1`, which is UTF-8 across every fixture.
//! `Sheet.field 2` is a repeated list of object references. The full list is
//! retained, and filtering those references to known `TableInfo` objects
//! recovers the sheet's table order.

use std::collections::{HashMap, HashSet};

use crate::iwa::{IwaArchive, IwaObject};
use crate::protobuf::{ProtoMessage, ProtoValue};

pub(crate) const SHEET_TYPE: u64 = 2;
pub(crate) const TABLE_INFO_TYPE: u64 = 6000;

const FIELD_NAME: u32 = 1;
const FIELD_OBJECT_REFERENCES: u32 = 2;
const REFERENCE_FIELD_ID: u32 = 1;

/// A decoded Numbers sheet: its name and the table models it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    id: u64,
    name: Option<String>,
    object_reference_ids: Vec<u64>,
    table_info_ids: Vec<u64>,
    table_model_ids: Vec<u64>,
}

impl Sheet {
    /// Decodes every `Sheet` object stored in an archive.
    pub(crate) fn collect(
        archive: &IwaArchive,
        table_info_to_model_ids: &HashMap<u64, Vec<u64>>,
    ) -> Vec<Self> {
        archive
            .objects()
            .iter()
            .filter_map(|object| Self::from_object(object, table_info_to_model_ids))
            .collect()
    }

    fn from_object(
        object: &IwaObject,
        table_info_to_model_ids: &HashMap<u64, Vec<u64>>,
    ) -> Option<Self> {
        if object.message_type != Some(SHEET_TYPE) {
            return None;
        }

        let id = object.identifier?;
        let message = ProtoMessage::decode(&object.payload).ok()?;
        let name = message
            .field(FIELD_NAME)
            .and_then(|field| field.value.as_bytes())
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(str::to_owned);

        let object_reference_ids: Vec<u64> = message
            .fields_by_number(FIELD_OBJECT_REFERENCES)
            .filter_map(|field| decode_reference_id(&field.value))
            .collect();

        let table_info_ids: Vec<u64> = object_reference_ids
            .iter()
            .copied()
            .filter(|id| table_info_to_model_ids.contains_key(id))
            .collect();

        let mut seen = HashSet::new();
        let table_model_ids = table_info_ids
            .iter()
            .filter_map(|table_info_id| table_info_to_model_ids.get(table_info_id))
            .flat_map(|model_ids| model_ids.iter().copied())
            .filter(|model_id| seen.insert(*model_id))
            .collect();

        Some(Self {
            id,
            name,
            object_reference_ids,
            table_info_ids,
            table_model_ids,
        })
    }

    /// The sheet object's identifier within the package.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The sheet's display name as shown in Numbers, if present.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Identifiers from the sheet's raw object-reference list, in stored order.
    ///
    /// This includes table wrappers plus other sheet-level objects whose exact
    /// roles are still being mapped.
    pub fn object_reference_ids(&self) -> &[u64] {
        &self.object_reference_ids
    }

    /// Identifiers of the `TableInfo` wrapper objects referenced by this sheet,
    /// in sheet order.
    pub fn table_info_ids(&self) -> &[u64] {
        &self.table_info_ids
    }

    /// Sheet-level object references that are not known `TableInfo` wrappers.
    pub fn non_table_object_reference_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.object_reference_ids
            .iter()
            .copied()
            .filter(|id| !self.table_info_ids.contains(id))
    }

    /// Identifiers of the `TableModel` objects belonging to this sheet, in sheet
    /// order after resolving through `TableInfo`.
    pub fn table_model_ids(&self) -> &[u64] {
        &self.table_model_ids
    }
}

/// Builds `TableInfo id -> TableModel id(s)` using protobuf-level object
/// references. Object IDs are unique within a package, so a nested varint that
/// equals a known `TableModel` id is structural reference evidence.
pub(crate) fn table_info_to_model_ids(archives: &[&IwaArchive]) -> HashMap<u64, Vec<u64>> {
    let objects: Vec<IwaObject> = archives
        .iter()
        .flat_map(|archive| archive.objects())
        .collect();
    let table_model_ids: HashSet<u64> = objects
        .iter()
        .filter(|object| object.message_type == Some(super::table_model::TABLE_MODEL_TYPE))
        .filter_map(|object| object.identifier)
        .collect();

    objects
        .iter()
        .filter(|object| object.message_type == Some(TABLE_INFO_TYPE))
        .filter_map(|object| {
            let table_info_id = object.identifier?;
            let message = ProtoMessage::decode(&object.payload).ok()?;
            let mut seen = HashSet::new();
            let model_ids: Vec<u64> = collect_varints(&message)
                .into_iter()
                .filter(|id| table_model_ids.contains(id))
                .filter(|id| seen.insert(*id))
                .collect();
            (!model_ids.is_empty()).then_some((table_info_id, model_ids))
        })
        .collect()
}

fn decode_reference_id(value: &ProtoValue) -> Option<u64> {
    let message = value
        .as_bytes()
        .and_then(|bytes| ProtoMessage::decode(bytes).ok())?;
    message
        .field(REFERENCE_FIELD_ID)
        .and_then(|field| field.value.as_varint())
}

fn collect_varints(message: &ProtoMessage) -> Vec<u64> {
    let mut out = Vec::new();
    collect_varints_into(message, &mut out);
    out
}

fn collect_varints_into(message: &ProtoMessage, out: &mut Vec<u64>) {
    for field in message.fields() {
        match &field.value {
            ProtoValue::Varint(value) => out.push(*value),
            ProtoValue::LengthDelimited(bytes) => {
                if let Ok(nested) = ProtoMessage::decode(bytes) {
                    collect_varints_into(&nested, out);
                }
            }
            ProtoValue::Fixed32(_) | ProtoValue::Fixed64(_) => {}
        }
    }
}
