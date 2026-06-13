//! Structural decoder for formula records (message type 4008).
//!
//! Formula-result cells in table tiles carry a local formula id. Type-4008
//! objects in `Index/CalculationEngine.iwa` carry the matching id in field 2,
//! giving the reader a stable join point before the expression payload itself
//! is fully decoded.

use crate::iwa::{IwaArchive, IwaObject};
use crate::protobuf::ProtoMessage;

pub(crate) const FORMULA_RECORD_TYPE: u64 = 4008;

const FIELD_FORMULA_ID: u32 = 2;
const FIELD_FORMULA_KIND: u32 = 3;

/// A decoded formula record from `Index/CalculationEngine.iwa`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaRecord {
    object_id: u64,
    formula_id: u32,
    formula_kind: u64,
}

impl FormulaRecord {
    pub(crate) fn collect(archive: &IwaArchive) -> Vec<Self> {
        archive
            .objects()
            .iter()
            .filter_map(Self::from_object)
            .collect()
    }

    fn from_object(object: &IwaObject) -> Option<Self> {
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
}
