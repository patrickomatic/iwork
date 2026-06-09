//! `TableModel` (message type 6001) — a Numbers table's name and geometry.
//!
//! The field layout below was recovered structurally and cross-validated: for
//! every table across all fixtures, field 6 and field 7 equal the row and column
//! counts independently recovered by the tile decoder, and field 8 holds the
//! table name shown in Numbers. See `docs/file-format.md` for the grounding.

use crate::iwa::{IwaArchive, IwaObject};
use crate::protobuf::ProtoMessage;

/// The message type identifier of a `TableModel` object.
pub(crate) const TABLE_MODEL_TYPE: u64 = 6001;

const FIELD_UUID: u32 = 1;
const FIELD_ROW_COUNT: u32 = 6;
const FIELD_COLUMN_COUNT: u32 = 7;
const FIELD_NAME: u32 = 8;
const FIELD_HEADER_ROWS: u32 = 9;
const FIELD_HEADER_COLUMNS: u32 = 10;

/// A decoded Numbers table model: its name and grid geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableModel {
    id: u64,
    uuid: Option<String>,
    name: Option<String>,
    row_count: u32,
    column_count: u32,
    header_row_count: u32,
    header_column_count: u32,
}

impl TableModel {
    /// Decodes every `TableModel` object stored in an archive.
    pub(crate) fn collect(archive: &IwaArchive) -> Vec<Self> {
        archive
            .objects()
            .iter()
            .filter_map(Self::from_object)
            .collect()
    }

    /// Decodes a single object if it is a `TableModel`, otherwise returns `None`.
    fn from_object(object: &IwaObject) -> Option<Self> {
        if object.message_type != Some(TABLE_MODEL_TYPE) {
            return None;
        }
        let id = object.identifier?;
        let message = ProtoMessage::decode(&object.payload).ok()?;

        let string_field = |number: u32| {
            message
                .field(number)
                .and_then(|field| field.value.as_bytes())
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .map(str::to_owned)
        };
        let count_field = |number: u32| {
            message
                .field(number)
                .and_then(|field| field.value.as_varint())
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(0)
        };

        Some(Self {
            id,
            uuid: string_field(FIELD_UUID),
            name: string_field(FIELD_NAME),
            row_count: count_field(FIELD_ROW_COUNT),
            column_count: count_field(FIELD_COLUMN_COUNT),
            header_row_count: count_field(FIELD_HEADER_ROWS),
            header_column_count: count_field(FIELD_HEADER_COLUMNS),
        })
    }

    /// The model object's identifier within the package.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The table's stable UUID string, if present.
    pub fn uuid(&self) -> Option<&str> {
        self.uuid.as_deref()
    }

    /// The table's display name as shown in Numbers, if present.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Total number of rows in the table grid (including header rows).
    pub fn row_count(&self) -> u32 {
        self.row_count
    }

    /// Total number of columns in the table grid (including header columns).
    pub fn column_count(&self) -> u32 {
        self.column_count
    }

    /// Number of header rows banding the top of the table.
    ///
    /// Grounded less strongly than the row/column counts: it is the small
    /// non-negative varint in field 9 and is always `<= row_count`, but has not
    /// been cross-validated against the header storage buckets.
    pub fn header_row_count(&self) -> u32 {
        self.header_row_count
    }

    /// Number of header columns banding the left of the table. See
    /// [`TableModel::header_row_count`] for the confidence caveat.
    pub fn header_column_count(&self) -> u32 {
        self.header_column_count
    }
}
