use std::collections::BTreeMap;

use super::CellValue;
use crate::Error;
use crate::iwa::{IwaArchive, IwaArchiveDescriptor, IwaPacket};
use crate::protobuf::{ProtoField, ProtoMessage};

#[derive(Debug, Clone, Default)]
pub struct Workbook {
    tables: Vec<WritableTable>,
}

impl Workbook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_table(&mut self, table: WritableTable) -> &mut Self {
        self.tables.push(table);
        self
    }

    pub fn tables(&self) -> &[WritableTable] {
        &self.tables
    }

    pub fn encode_table_archives(&self) -> Result<Vec<EncodedTableArchive>, Error> {
        self.tables
            .iter()
            .enumerate()
            .map(|(index, table)| table.encode_archives(index))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct WritableTable {
    name: String,
    rows: Vec<Vec<CellValue>>,
}

impl WritableTable {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rows: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn rows(&self) -> &[Vec<CellValue>] {
        &self.rows
    }

    pub fn push_row(&mut self, row: Vec<CellValue>) -> &mut Self {
        self.rows.push(row);
        self
    }

    fn encode_archives(&self, table_index: usize) -> Result<EncodedTableArchive, Error> {
        let strings = collect_strings(&self.rows);
        let tile = encode_tile_archive(&self.rows, &strings)?;
        let datalist = encode_string_datalist_archive(&strings)?;

        Ok(EncodedTableArchive {
            name: self.name.clone(),
            tile_path: format!("Index/Tables/Tile-{}-1.iwa", table_index + 1),
            tile,
            datalist_path: format!("Index/Tables/DataList-{}-1.iwa", table_index + 1),
            datalist,
        })
    }
}

#[derive(Debug, Clone)]
pub struct EncodedTableArchive {
    pub name: String,
    pub tile_path: String,
    pub tile: Vec<u8>,
    pub datalist_path: String,
    pub datalist: Vec<u8>,
}

fn collect_strings(rows: &[Vec<CellValue>]) -> BTreeMap<String, u32> {
    collect_strings_starting_at(rows, 1)
}

fn collect_strings_starting_at(rows: &[Vec<CellValue>], first_key: u32) -> BTreeMap<String, u32> {
    let mut map = BTreeMap::new();
    let mut next_key = first_key;

    for row in rows {
        for cell in row {
            if let CellValue::Text(value) = cell
                && !map.contains_key(value)
            {
                map.insert(value.clone(), next_key);
                next_key += 1;
            }
        }
    }

    map
}

/// Number of `u16` offset slots in a `TileRowInfo` offset array (`f4`/`f7`).
/// Real Numbers tiles always store 255 slots (510 bytes), padded with `0xffff`.
const TILE_OFFSET_SLOTS: usize = 255;
/// Constant `TileRowInfo.f5` cell-storage marker observed across real tiles.
const TILE_ROW_STORAGE_VERSION: u64 = 5;
/// `MessageInfo.version` triple (`f2`) carried by every real archive header.
const ARCHIVE_VERSION: [u8; 3] = [1, 0, 5];

fn encode_tile_archive(
    rows: &[Vec<CellValue>],
    strings: &BTreeMap<String, u32>,
) -> Result<Vec<u8>, Error> {
    let col_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    encode_tile_archive_with_root(rows, col_count, strings, 1)
}

fn encode_tile_archive_with_root(
    rows: &[Vec<CellValue>],
    col_count: usize,
    strings: &BTreeMap<String, u32>,
    root_object_id: u64,
) -> Result<Vec<u8>, Error> {
    let body = encode_tile_body(rows, col_count, strings)?;
    let header = synthesize_header(root_object_id, 6002, body.len())?;
    IwaArchive::encode(header, body)
}

/// Encodes a `TileArchive` body as a single Tile message.
///
/// The Tile message carries `f4` = row count and a repeated `f5` of
/// [`encode_row_message`] entries, matching the structure real Numbers tiles
/// use (rather than a bare stream of row messages, which Numbers rejects).
fn encode_tile_body(
    rows: &[Vec<CellValue>],
    col_count: usize,
    strings: &BTreeMap<String, u32>,
) -> Result<Vec<u8>, Error> {
    let row_count =
        u64::try_from(rows.len()).map_err(|_| Error::InvalidIwa("row count overflow"))?;

    let mut fields = vec![
        ProtoField::varint(1, 0),
        ProtoField::varint(2, 0),
        ProtoField::varint(3, 0),
        ProtoField::varint(4, row_count),
    ];
    for (row_index, row) in rows.iter().enumerate() {
        fields.push(ProtoField::message(
            5,
            &encode_row_message(row_index, row, col_count, strings)?,
        )?);
    }
    fields.push(ProtoField::varint(6, 5));
    fields.push(ProtoField::varint(7, 1));

    ProtoMessage::new(fields).encode()
}

/// Builds an archive header packet for a freshly synthesized body.
fn synthesize_header(root_object_id: u64, kind: u64, body_len: usize) -> Result<IwaPacket, Error> {
    let descriptor = IwaArchiveDescriptor {
        root_object_id: Some(root_object_id),
        kind_hint: Some(kind),
        message_version: Some(ARCHIVE_VERSION.to_vec()),
        body_hint: Some(
            u64::try_from(body_len).map_err(|_| Error::InvalidIwa("body length overflow"))?,
        ),
        object_references: Vec::new(),
    };
    Ok(IwaPacket::new(descriptor.encode_message()?.encode()?))
}

fn encode_string_datalist_archive(strings: &BTreeMap<String, u32>) -> Result<Vec<u8>, Error> {
    encode_string_datalist_archive_with_root(strings, 1)
}

fn encode_string_datalist_archive_with_root(
    strings: &BTreeMap<String, u32>,
    root_object_id: u64,
) -> Result<Vec<u8>, Error> {
    let body = encode_string_datalist_body(strings)?;
    let header = synthesize_header(root_object_id, 6005, body.len())?;
    IwaArchive::encode(header, body)
}

/// Encodes a string `TableDataList` body as a single message: `f1` = list type
/// (1 = string), `f2` = next free key, and a repeated `f3` of `ListEntry`
/// `{ f1 = key, f2 = refcount, f3 = string }`.
fn encode_string_datalist_body(strings: &BTreeMap<String, u32>) -> Result<Vec<u8>, Error> {
    let next_id = strings
        .values()
        .copied()
        .max()
        .map_or(1, |max| u64::from(max) + 1);

    let mut fields = vec![ProtoField::varint(1, 1), ProtoField::varint(2, next_id)];
    for (value, key) in strings {
        fields.push(ProtoField::message(
            3,
            &ProtoMessage::new(vec![
                ProtoField::varint(1, u64::from(*key)),
                ProtoField::varint(2, 1),
                ProtoField::string(3, value),
            ]),
        )?);
    }

    ProtoMessage::new(fields).encode()
}

/// Encodes one `TileRowInfo`: row index (`f1`), column count (`f2`), the wide
/// cell-storage buffer (`f6`) and its `u16` offset array (`f7`, `0xffff` = empty).
///
/// The row is laid out across exactly `col_count` columns; cells beyond the
/// row's length (or empty cells) are recorded as empty offset slots.
fn encode_row_message(
    row_index: usize,
    row: &[CellValue],
    col_count: usize,
    strings: &BTreeMap<String, u32>,
) -> Result<ProtoMessage, Error> {
    let mut storage = Vec::new();
    let mut offsets = vec![0xffff_u16; TILE_OFFSET_SLOTS];

    for (column, slot) in offsets.iter_mut().take(col_count).enumerate() {
        let Some(cell) = row.get(column) else {
            continue;
        };
        if matches!(cell, CellValue::Empty) {
            continue;
        }

        *slot = u16::try_from(storage.len())
            .map_err(|_| Error::InvalidIwa("cell storage offset overflow"))?;
        storage.extend_from_slice(&encode_cell_record(cell, strings)?);
    }

    let mut offset_bytes = Vec::with_capacity(offsets.len() * 2);
    for offset in offsets {
        offset_bytes.extend_from_slice(&offset.to_le_bytes());
    }

    Ok(ProtoMessage::new(vec![
        ProtoField::varint(
            1,
            u64::try_from(row_index).map_err(|_| Error::InvalidIwa("row index overflow"))?,
        ),
        ProtoField::varint(
            2,
            u64::try_from(col_count).map_err(|_| Error::InvalidIwa("column count overflow"))?,
        ),
        ProtoField::varint(5, TILE_ROW_STORAGE_VERSION),
        ProtoField::bytes(6, storage),
        ProtoField::bytes(7, offset_bytes),
    ]))
}

fn encode_cell_record(cell: &CellValue, strings: &BTreeMap<String, u32>) -> Result<Vec<u8>, Error> {
    let (flags, payload) = match cell {
        CellValue::Empty => return Ok(Vec::new()),
        CellValue::Number(value) => (0x2u32, value.to_le_bytes().to_vec()),
        CellValue::Date(value) => (0x4u32, value.to_le_bytes().to_vec()),
        CellValue::Text(value) => {
            let key = strings
                .get(value)
                .ok_or(Error::InvalidIwa("missing string datalist key"))?;
            (0x8u32, key.to_le_bytes().to_vec())
        }
    };

    let mut record = vec![0x05, 0, 0, 0, 0, 0, 0, 0];
    record.extend_from_slice(&flags.to_le_bytes());
    record.extend_from_slice(&payload);
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numbers::table::{Table, decode_string_datalist};

    #[test]
    fn encoded_archives_round_trip_scalar_cells() -> Result<(), Error> {
        let mut workbook = Workbook::new();
        let mut table = WritableTable::new("Budget");
        table.push_row(vec![
            CellValue::Text("Category".to_owned()),
            CellValue::Text("Amount".to_owned()),
            CellValue::Text("When".to_owned()),
        ]);
        table.push_row(vec![
            CellValue::Text("Utilities".to_owned()),
            CellValue::Number(42.5),
            CellValue::Date(625_881_600.0),
        ]);
        workbook.add_table(table);

        let archives = workbook.encode_table_archives()?;
        assert_eq!(archives.len(), 1);

        let datalist_archive = IwaArchive::decode(&archives[0].datalist)?;
        let strings = decode_string_datalist(&datalist_archive);
        assert_eq!(strings.get(&1).map(String::as_str), Some("Category"));
        assert!(strings.values().any(|value| value == "Utilities"));

        let tile_archive = IwaArchive::decode(&archives[0].tile)?;
        let decoded = Table::from_tile(&tile_archive, &strings);
        assert_eq!(decoded.rows().len(), 2);
        assert_eq!(
            decoded.rows()[1].cells,
            vec![
                CellValue::Text("Utilities".to_owned()),
                CellValue::Number(42.5),
                CellValue::Date(625_881_600.0),
            ]
        );

        Ok(())
    }
}
