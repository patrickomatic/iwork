//! Structural decoder for `HeaderStorageBucket` (message type 6006).
//!
//! Buckets are referenced from `TableModel.field 4` (`DataStore`) and contain
//! repeated entry messages. The entry fields are exposed as structural values
//! until their row/column/header semantics are cross-validated.

use crate::iwa::IwaArchive;
use crate::protobuf::{ProtoMessage, ProtoValue};

pub(crate) const HEADER_STORAGE_BUCKET_TYPE: u64 = 6006;

const FIELD_ENTRIES: u32 = 2;
const ENTRY_FIELD_INDEX: u32 = 1;
const ENTRY_FIELD_FIXED32: u32 = 2;
const ENTRY_FIELD_3: u32 = 3;
const ENTRY_FIELD_4: u32 = 4;

/// A decoded Numbers `HeaderStorageBucket` object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderStorageBucket {
    id: u64,
    entries: Vec<HeaderStorageEntry>,
}

impl HeaderStorageBucket {
    pub(crate) fn from_archive(archive: &IwaArchive) -> Option<Self> {
        let object = archive
            .objects()
            .into_iter()
            .find(|object| object.message_type == Some(HEADER_STORAGE_BUCKET_TYPE))?;
        let id = object.identifier?;
        let message = ProtoMessage::decode(&object.payload).ok()?;
        let entries = message
            .fields_by_number(FIELD_ENTRIES)
            .filter_map(|field| {
                field
                    .value
                    .as_bytes()
                    .and_then(|bytes| ProtoMessage::decode(bytes).ok())
                    .and_then(|entry| HeaderStorageEntry::from_message(&entry))
            })
            .collect();

        Some(Self { id, entries })
    }

    /// The bucket object's identifier within the package.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Structural bucket entries in stored order.
    pub fn entries(&self) -> &[HeaderStorageEntry] {
        &self.entries
    }
}

/// One structural entry in a `HeaderStorageBucket`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderStorageEntry {
    index: u64,
    fixed32_bits: u32,
    field3: u64,
    field4: u64,
}

impl HeaderStorageEntry {
    fn from_message(message: &ProtoMessage) -> Option<Self> {
        Some(Self {
            index: message
                .field(ENTRY_FIELD_INDEX)
                .and_then(|field| field.value.as_varint())?,
            fixed32_bits: message
                .field(ENTRY_FIELD_FIXED32)
                .and_then(as_fixed32_bits)?,
            field3: message
                .field(ENTRY_FIELD_3)
                .and_then(|field| field.value.as_varint())?,
            field4: message
                .field(ENTRY_FIELD_4)
                .and_then(|field| field.value.as_varint())?,
        })
    }

    /// Entry ordinal stored in field 1.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Raw fixed32 bits stored in field 2.
    pub fn fixed32_bits(&self) -> u32 {
        self.fixed32_bits
    }

    /// Raw varint stored in field 3.
    pub fn field3(&self) -> u64 {
        self.field3
    }

    /// Raw varint stored in field 4.
    pub fn field4(&self) -> u64 {
        self.field4
    }
}

fn as_fixed32_bits(field: &crate::protobuf::ProtoField) -> Option<u32> {
    match &field.value {
        ProtoValue::Fixed32(value) => Some(*value),
        _ => None,
    }
}
