//! Structural decoder for sheet-level drawable objects (message type 5021).
//!
//! Type 5021 objects are referenced from `Sheet.field 2` as non-table
//! sheet-level objects and anchor the 5020-5030 drawing/chart object cluster.
//! The nested chart/drawing payload semantics are still unmapped, so this
//! decoder retains the high-confidence top-level payloads as raw bytes.

use crate::iwa::IwaObject;
use crate::protobuf::ProtoMessage;
use crate::Error;

pub(crate) const SHEET_DRAWABLE_TYPE: u64 = 5021;

const FIELD_DRAWABLE_INFO: u32 = 1;
const FIELD_DRAWABLE_PAYLOAD: u32 = 10_000;

/// A structurally decoded sheet-level drawable object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetDrawable {
    object_id: u64,
    info_payload: Vec<u8>,
    payload: Vec<u8>,
}

impl SheetDrawable {
    pub(crate) fn from_object(object: &IwaObject) -> Option<Self> {
        if object.message_type != Some(SHEET_DRAWABLE_TYPE) {
            return None;
        }

        let object_id = object.identifier?;
        let message = ProtoMessage::decode(&object.payload).ok()?;
        Some(Self {
            object_id,
            info_payload: message
                .field(FIELD_DRAWABLE_INFO)
                .and_then(|field| field.value.as_bytes())
                .map(<[u8]>::to_vec)?,
            payload: message
                .field(FIELD_DRAWABLE_PAYLOAD)
                .and_then(|field| field.value.as_bytes())
                .map(<[u8]>::to_vec)?,
        })
    }

    /// Object identifier of this drawable within the package.
    pub fn object_id(&self) -> u64 {
        self.object_id
    }

    /// Raw top-level field-1 payload.
    pub fn info_payload(&self) -> &[u8] {
        &self.info_payload
    }

    /// Decodes the raw field-1 payload as a nested protobuf message.
    pub fn info_message(&self) -> Result<ProtoMessage, Error> {
        ProtoMessage::decode(&self.info_payload)
    }

    /// Raw top-level field-10000 payload.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Decodes the raw field-10000 payload as a nested protobuf message.
    pub fn payload_message(&self) -> Result<ProtoMessage, Error> {
        ProtoMessage::decode(&self.payload)
    }
}
