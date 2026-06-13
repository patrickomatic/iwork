//! Structural decoder for formula records (message type 4008).
//!
//! Formula-result cells in table tiles carry a local formula id. Type-4008
//! objects in `Index/CalculationEngine.iwa` carry the matching id in field 2,
//! giving the reader a stable join point before the expression payload itself
//! is fully decoded.

use std::collections::HashSet;

use crate::iwa::{IwaArchive, IwaObject};
use crate::protobuf::{ProtoMessage, read_varint};

pub(crate) const FORMULA_RECORD_TYPE: u64 = 4008;
pub(crate) const FORMULA_AUXILIARY_RECORD_TYPE: u64 = 4009;

const FIELD_FORMULA_ID: u32 = 2;
const FIELD_FORMULA_KIND: u32 = 3;
const FIELD_BOUNDS_7: u32 = 7;
const FIELD_BOUNDS_8: u32 = 8;
const BOUNDS_FIELD_PRIMARY: u32 = 2;
const BOUNDS_FIELD_SECONDARY: u32 = 3;
const AUX_FIELD_1: u32 = 1;
const AUX_FIELD_2: u32 = 2;
const AUX_FIELD_3: u32 = 3;
const AUX_FIELD_ENTRIES: u32 = 4;
const AUX_ENTRY_FIELD_1: u32 = 1;
const AUX_ENTRY_FIELD_2: u32 = 2;
const AUX_ENTRY_FIELD_PAYLOAD: u32 = 6;
const AUX_PAYLOAD_FIELD_1: u32 = 1;
const AUX_PAYLOAD_FIELD_2: u32 = 2;

/// A decoded formula record from `Index/CalculationEngine.iwa`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaRecord {
    object_id: u64,
    formula_id: u32,
    formula_kind: u64,
    field7_bounds: Option<FormulaBoundsPair>,
    field8_bounds: Option<FormulaBoundsPair>,
    auxiliary_record_ids: Vec<u64>,
}

impl FormulaRecord {
    pub(crate) fn collect(archive: &IwaArchive) -> Vec<Self> {
        let auxiliary_ids: HashSet<u64> = FormulaAuxiliaryRecord::collect(archive)
            .into_iter()
            .map(|record| record.object_id())
            .collect();
        archive
            .objects()
            .iter()
            .filter_map(|object| Self::from_object(object, &auxiliary_ids))
            .collect()
    }

    fn from_object(object: &IwaObject, auxiliary_ids: &HashSet<u64>) -> Option<Self> {
        if object.message_type != Some(FORMULA_RECORD_TYPE) {
            return None;
        }
        let object_id = object.identifier?;
        let message = ProtoMessage::decode(&object.payload).ok()?;
        let formula_id = message
            .field(FIELD_FORMULA_ID)
            .and_then(|field| field.value.as_varint())
            .and_then(|id| u32::try_from(id).ok())?;
        let formula_kind = message
            .field(FIELD_FORMULA_KIND)
            .and_then(|field| field.value.as_varint())
            .unwrap_or(0);

        Some(Self {
            object_id,
            formula_id,
            formula_kind,
            field7_bounds: decode_bounds_pair(&message, FIELD_BOUNDS_7),
            field8_bounds: decode_bounds_pair(&message, FIELD_BOUNDS_8),
            auxiliary_record_ids: referenced_auxiliary_ids(object, auxiliary_ids),
        })
    }

    /// Object identifier of this formula record within the package.
    pub fn object_id(&self) -> u64 {
        self.object_id
    }

    /// Local formula id referenced by formula-result cells in tile storage.
    pub fn formula_id(&self) -> u32 {
        self.formula_id
    }

    /// Raw field-3 classifier for the formula record.
    ///
    /// Its expression semantics are not decoded yet; this value is retained so
    /// callers can correlate records while the schema is still being mapped.
    pub fn formula_kind(&self) -> u64 {
        self.formula_kind
    }

    /// Raw bounds pair stored in field 7.
    ///
    /// The four component values are structurally decoded but not semantically
    /// named yet.
    pub fn field7_bounds(&self) -> Option<&FormulaBoundsPair> {
        self.field7_bounds.as_ref()
    }

    /// Raw bounds pair stored in field 8.
    ///
    /// Across current fixtures this mirrors field 7's shape; the exact role is
    /// still under investigation.
    pub fn field8_bounds(&self) -> Option<&FormulaBoundsPair> {
        self.field8_bounds.as_ref()
    }

    /// Type-4009 object ids referenced structurally by this formula record.
    pub fn auxiliary_record_ids(&self) -> &[u64] {
        &self.auxiliary_record_ids
    }
}

/// A structurally decoded type-4009 formula auxiliary record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaAuxiliaryRecord {
    object_id: u64,
    field1: u64,
    field2: u64,
    field3: u64,
    entries: Vec<FormulaAuxiliaryEntry>,
}

impl FormulaAuxiliaryRecord {
    pub(crate) fn collect(archive: &IwaArchive) -> Vec<Self> {
        archive
            .objects()
            .iter()
            .filter_map(Self::from_object)
            .collect()
    }

    fn from_object(object: &IwaObject) -> Option<Self> {
        if object.message_type != Some(FORMULA_AUXILIARY_RECORD_TYPE) {
            return None;
        }
        let object_id = object.identifier?;
        let message = ProtoMessage::decode(&object.payload).ok()?;
        Some(Self {
            object_id,
            field1: message.field(AUX_FIELD_1).and_then(|field| field.value.as_varint())?,
            field2: message.field(AUX_FIELD_2).and_then(|field| field.value.as_varint())?,
            field3: message.field(AUX_FIELD_3).and_then(|field| field.value.as_varint())?,
            entries: message
                .fields_by_number(AUX_FIELD_ENTRIES)
                .filter_map(|field| {
                    field
                        .value
                        .as_bytes()
                        .and_then(|bytes| ProtoMessage::decode(bytes).ok())
                        .and_then(|entry| FormulaAuxiliaryEntry::from_message(&entry))
                })
                .collect(),
        })
    }

    /// Object identifier of this auxiliary record within the package.
    pub fn object_id(&self) -> u64 {
        self.object_id
    }

    /// Raw field 1.
    pub fn field1(&self) -> u64 {
        self.field1
    }

    /// Raw field 2.
    pub fn field2(&self) -> u64 {
        self.field2
    }

    /// Raw field 3.
    pub fn field3(&self) -> u64 {
        self.field3
    }

    /// Repeated field-4 entries.
    pub fn entries(&self) -> &[FormulaAuxiliaryEntry] {
        &self.entries
    }
}

/// One repeated field-4 entry from a type-4009 formula auxiliary record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaAuxiliaryEntry {
    field1: u64,
    field2: u64,
    payload: Option<FormulaAuxiliaryEntryPayload>,
}

impl FormulaAuxiliaryEntry {
    fn from_message(message: &ProtoMessage) -> Option<Self> {
        Some(Self {
            field1: message
                .field(AUX_ENTRY_FIELD_1)
                .and_then(|field| field.value.as_varint())?,
            field2: message
                .field(AUX_ENTRY_FIELD_2)
                .and_then(|field| field.value.as_varint())?,
            payload: decode_auxiliary_entry_payload(message),
        })
    }

    /// Raw entry field 1.
    pub fn field1(&self) -> u64 {
        self.field1
    }

    /// Raw entry field 2.
    pub fn field2(&self) -> u64 {
        self.field2
    }

    /// Optional decoded field-6 payload for entries whose payload is a nested
    /// protobuf message with the known two-varint shape.
    pub fn payload(&self) -> Option<&FormulaAuxiliaryEntryPayload> {
        self.payload.as_ref()
    }
}

/// Decoded nested field-6 payload from a formula auxiliary entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaAuxiliaryEntryPayload {
    field1: u64,
    field2: u64,
}

impl FormulaAuxiliaryEntryPayload {
    /// Raw payload field 1.
    pub fn field1(&self) -> u64 {
        self.field1
    }

    /// Raw payload field 2.
    pub fn field2(&self) -> u64 {
        self.field2
    }
}

/// A pair of four-varint bounds records in a formula record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaBoundsPair {
    primary: FormulaBounds,
    secondary: FormulaBounds,
}

impl FormulaBoundsPair {
    /// First nested bounds record.
    pub fn primary(&self) -> &FormulaBounds {
        &self.primary
    }

    /// Second nested bounds record.
    pub fn secondary(&self) -> &FormulaBounds {
        &self.secondary
    }
}

/// Four raw varints that form a stable formula bounds record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaBounds {
    first: u64,
    second: u64,
    third: u64,
    fourth: u64,
}

impl FormulaBounds {
    /// First raw component.
    pub fn first(&self) -> u64 {
        self.first
    }

    /// Second raw component.
    pub fn second(&self) -> u64 {
        self.second
    }

    /// Third raw component.
    pub fn third(&self) -> u64 {
        self.third
    }

    /// Fourth raw component.
    pub fn fourth(&self) -> u64 {
        self.fourth
    }

    /// Whether this bounds record uses the common sentinel maxima observed in
    /// formula records with no concrete bounds.
    pub fn is_sentinel(&self) -> bool {
        self.first == 32_767
            && self.second == 2_147_483_647
            && self.third == 32_767
            && self.fourth == 2_147_483_647
    }
}

fn decode_bounds_pair(message: &ProtoMessage, field_number: u32) -> Option<FormulaBoundsPair> {
    let pair = message
        .field(field_number)
        .and_then(|field| field.value.as_bytes())
        .and_then(|bytes| ProtoMessage::decode(bytes).ok())?;
    Some(FormulaBoundsPair {
        primary: decode_bounds(&pair, BOUNDS_FIELD_PRIMARY)?,
        secondary: decode_bounds(&pair, BOUNDS_FIELD_SECONDARY)?,
    })
}

fn decode_bounds(message: &ProtoMessage, field_number: u32) -> Option<FormulaBounds> {
    let bounds = message
        .field(field_number)
        .and_then(|field| field.value.as_bytes())
        .and_then(|bytes| ProtoMessage::decode(bytes).ok())?;
    Some(FormulaBounds {
        first: bounds.field(1).and_then(|field| field.value.as_varint())?,
        second: bounds.field(2).and_then(|field| field.value.as_varint())?,
        third: bounds.field(3).and_then(|field| field.value.as_varint())?,
        fourth: bounds.field(4).and_then(|field| field.value.as_varint())?,
    })
}

fn decode_auxiliary_entry_payload(
    message: &ProtoMessage,
) -> Option<FormulaAuxiliaryEntryPayload> {
    let payload = message
        .field(AUX_ENTRY_FIELD_PAYLOAD)
        .and_then(|field| field.value.as_bytes())
        .and_then(|bytes| ProtoMessage::decode(bytes).ok())?;
    Some(FormulaAuxiliaryEntryPayload {
        field1: payload
            .field(AUX_PAYLOAD_FIELD_1)
            .and_then(|field| field.value.as_varint())?,
        field2: payload
            .field(AUX_PAYLOAD_FIELD_2)
            .and_then(|field| field.value.as_varint())?,
    })
}

fn referenced_auxiliary_ids(object: &IwaObject, auxiliary_ids: &HashSet<u64>) -> Vec<u64> {
    let Some(object_id) = object.identifier else {
        return Vec::new();
    };
    let mut referenced = Vec::new();
    for start in 0..object.payload.len() {
        let mut cursor = start;
        let Ok(value) = read_varint(&object.payload, &mut cursor) else {
            continue;
        };
        if value != object_id && auxiliary_ids.contains(&value) && !referenced.contains(&value) {
            referenced.push(value);
        }
    }
    referenced
}
