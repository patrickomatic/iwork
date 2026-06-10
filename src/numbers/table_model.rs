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

/// `TableModel.field 4` holds the `DataStore` (the table's storage references).
const FIELD_DATA_STORE: u32 = 4;
/// `DataStore.field 3` holds the `TileStorage` (the table's data tiles).
const STORE_FIELD_TILES: u32 = 3;
/// `DataStore.field 4` references the `DataList` of cell strings (validated:
/// across every fixture its entries are the table's text cells, distinct from
/// the format store).
const STORE_FIELD_STRINGS: u32 = 4;
/// `TileStorage.field 1` is the repeated list of `(tile index, tile reference)`.
const TILES_FIELD_ENTRIES: u32 = 1;
/// `TileStorage.field 2` is the tile size: the number of rows each tile spans.
const TILES_FIELD_SIZE: u32 = 2;
const TILE_ENTRY_INDEX: u32 = 1;
const TILE_ENTRY_REFERENCE: u32 = 2;
/// An object reference message stores the referenced identifier in field 1.
const REFERENCE_FIELD_ID: u32 = 1;
/// Rows per tile when the `TileStorage` does not state its tile size. Numbers
/// uses 256-row tiles; a table taller than that is split across several.
const DEFAULT_TILE_SIZE: u32 = 256;

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
    tile_ids: Vec<u64>,
    /// Absolute starting row of each tile in `tile_ids` (tile index × tile size).
    tile_row_offsets: Vec<u32>,
    string_data_list_id: Option<u64>,
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

        let data_store = message
            .field(FIELD_DATA_STORE)
            .and_then(|field| field.value.as_bytes())
            .and_then(|bytes| ProtoMessage::decode(bytes).ok());
        let tiles = data_store.as_ref().map(decode_tiles).unwrap_or_default();

        Some(Self {
            id,
            uuid: string_field(FIELD_UUID),
            name: string_field(FIELD_NAME),
            row_count: count_field(FIELD_ROW_COUNT),
            column_count: count_field(FIELD_COLUMN_COUNT),
            header_row_count: count_field(FIELD_HEADER_ROWS),
            header_column_count: count_field(FIELD_HEADER_COLUMNS),
            tile_ids: tiles.iter().map(|(_, id)| *id).collect(),
            tile_row_offsets: tiles.iter().map(|(offset, _)| *offset).collect(),
            string_data_list_id: data_store.as_ref().and_then(decode_string_data_list_id),
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

    /// Identifiers of the `Tile` objects holding this table's cell data, in
    /// tile-index order. Each identifier matches a `Index/Tables/Tile*.iwa`
    /// archive's root object.
    pub fn tile_ids(&self) -> &[u64] {
        &self.tile_ids
    }

    /// Absolute starting row of each tile in [`TableModel::tile_ids`] (parallel
    /// slice). Adding a tile's offset to a within-tile row index yields the row's
    /// absolute position, which matters once a table spans more than one tile.
    pub(crate) fn tile_row_offsets(&self) -> &[u32] {
        &self.tile_row_offsets
    }

    /// Identifier of the `DataList` object that resolves this table's string
    /// cells. Scoping string lookups to this list keeps per-table string keys
    /// from colliding across tables.
    pub(crate) fn string_data_list_id(&self) -> Option<u64> {
        self.string_data_list_id
    }
}

/// Extracts the table's tiles from a `DataStore` as `(row_offset, tile_id)` pairs
/// in tile order, where `row_offset` is the tile's absolute starting row in the
/// table grid (tile index × tile size).
fn decode_tiles(data_store: &ProtoMessage) -> Vec<(u32, u64)> {
    let Some(tile_storage) = data_store
        .field(STORE_FIELD_TILES)
        .and_then(|field| field.value.as_bytes())
        .and_then(|bytes| ProtoMessage::decode(bytes).ok())
    else {
        return Vec::new();
    };

    let tile_size = tile_storage
        .field(TILES_FIELD_SIZE)
        .and_then(|field| field.value.as_varint())
        .and_then(|size| u32::try_from(size).ok())
        .filter(|size| *size > 0)
        .unwrap_or(DEFAULT_TILE_SIZE);

    let mut tiles: Vec<(u32, u32, u64)> = tile_storage
        .fields_by_number(TILES_FIELD_ENTRIES)
        .filter_map(|entry| {
            let entry = entry
                .value
                .as_bytes()
                .and_then(|bytes| ProtoMessage::decode(bytes).ok())?;
            let index = entry
                .field(TILE_ENTRY_INDEX)
                .and_then(|field| field.value.as_varint())
                .and_then(|index| u32::try_from(index).ok())
                .unwrap_or(0);
            let tile_id = entry
                .field(TILE_ENTRY_REFERENCE)
                .and_then(|field| field.value.as_bytes())
                .and_then(|bytes| ProtoMessage::decode(bytes).ok())
                .and_then(|reference| {
                    reference
                        .field(REFERENCE_FIELD_ID)
                        .and_then(|field| field.value.as_varint())
                })?;
            Some((index, index.saturating_mul(tile_size), tile_id))
        })
        .collect();

    tiles.sort_by_key(|(index, _, _)| *index);
    tiles
        .into_iter()
        .map(|(_, row_offset, tile_id)| (row_offset, tile_id))
        .collect()
}

/// Extracts the cell-string `DataList` identifier from a `DataStore`.
fn decode_string_data_list_id(data_store: &ProtoMessage) -> Option<u64> {
    data_store
        .field(STORE_FIELD_STRINGS)
        .and_then(|field| field.value.as_bytes())
        .and_then(|bytes| ProtoMessage::decode(bytes).ok())
        .and_then(|reference| {
            reference
                .field(REFERENCE_FIELD_ID)
                .and_then(|field| field.value.as_varint())
        })
}
