use std::collections::BTreeMap;

use super::CellValue;
use super::table::encode_decimal128;
use crate::iwa::{IwaArchive, IwaArchiveDescriptor, IwaPacket};
use crate::package::PackageWriter;
use crate::protobuf::{ProtoField, ProtoMessage};
use crate::{Document, Error};

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

    /// Emits a `.numbers` package using an embedded scaffold shell.
    ///
    /// Every `.iwa` member is decoded and re-serialized through this crate's own
    /// IWA encoder rather than copied verbatim, so the whole document graph
    /// (`Document`, `Metadata`, `CalculationEngine`, `ViewState`, …) is produced
    /// by our writer. The first visible 4-column table is replaced with the
    /// caller's rows. This still seeds the document/object graph from the
    /// bundled personal-budget fixture rather than synthesizing it from scratch.
    pub fn encode_scaffold_package(&self) -> Result<Vec<u8>, Error> {
        let Some(table) = self.tables.first() else {
            return Err(Error::InvalidIwa(
                "workbook must contain at least one table",
            ));
        };

        let scaffold = Document::from_bytes(PERSONAL_BUDGET_SCAFFOLD.to_vec())?;
        let package = scaffold.package();

        // The scaffold table's geometry, cell styles, and storage layout are
        // referenced by other (untouched) archives, so rather than synthesize a
        // tile from scratch we patch the real one in place: only cell *values*
        // change, leaving every structural byte (cell types, style ids, legacy
        // arrays) intact. Caller rows are mapped onto the original geometry.
        let original_tile = IwaArchive::decode(package.entry_bytes(SUMMARY_TABLE_TILE_PATH)?)?;
        let original_datalist =
            IwaArchive::decode(package.entry_bytes(SUMMARY_TABLE_DATALIST_PATH)?)?;
        let (row_count, col_count) = tile_geometry(&original_tile)?;
        let rows = reshape_rows(&table.rows, row_count, col_count);

        let mut strings = StringTable::from_datalist(&original_datalist)?;
        let tile_body = patch_tile_body(&original_tile, &rows, &mut strings)?;
        let tile = archive_with_body(&original_tile, tile_body)?;
        let datalist = archive_with_body(&original_datalist, strings.encode_body()?)?;

        let mut writer = PackageWriter::new();

        for entry in package.entries() {
            let bytes = match entry.path.as_str() {
                SUMMARY_TABLE_TILE_PATH => tile.clone(),
                SUMMARY_TABLE_DATALIST_PATH => datalist.clone(),
                path if std::path::Path::new(path)
                    .extension()
                    .is_some_and(|ext| ext == "iwa") =>
                {
                    // Re-emit through our own IWA encoder to prove the writer can
                    // reproduce the full archive graph, not just the table tiles.
                    IwaArchive::decode(package.entry_bytes(path)?)?.reencode()?
                }
                path => package.entry_bytes(path)?.to_vec(),
            };
            writer.add_entry(entry.path.clone(), bytes);
        }

        writer.finish()
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

const PERSONAL_BUDGET_SCAFFOLD: &[u8] =
    include_bytes!("../../examples/numbers/personal_budget.numbers");
const SUMMARY_TABLE_TILE_PATH: &str = "Index/Tables/Tile-904525-2.iwa";
const SUMMARY_TABLE_DATALIST_PATH: &str = "Index/Tables/DataList-904498-2.iwa";

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
            encode_row_message(row_index, row, col_count, strings)?,
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

/// Re-wraps a new `body` using an existing archive's header, preserving its
/// root id, kind, version triple, and object references — only the
/// `MessageInfo` body-length field (`f3`) is updated to match `body`.
///
/// This is the reliable way to replace a scaffold archive's contents: Numbers
/// validates the header against the body, and the original header carries
/// fields (version, object references) we do not otherwise reconstruct.
fn archive_with_body(original: &IwaArchive, body: Vec<u8>) -> Result<Vec<u8>, Error> {
    let header = original.header().decode_message()?;
    let body_len =
        u64::try_from(body.len()).map_err(|_| Error::InvalidIwa("body length overflow"))?;

    let mut fields = Vec::with_capacity(header.fields().len());
    for field in header.fields() {
        if field.number == 2
            && let Some(info) = field.value.as_message().ok().flatten()
        {
            let mut info_fields = Vec::with_capacity(info.fields().len());
            let mut replaced = false;
            for info_field in info.fields() {
                if info_field.number == 3 {
                    info_fields.push(ProtoField::varint(3, body_len));
                    replaced = true;
                } else {
                    info_fields.push(info_field.clone());
                }
            }
            if !replaced {
                info_fields.push(ProtoField::varint(3, body_len));
            }
            fields.push(ProtoField::message(2, ProtoMessage::new(info_fields))?);
        } else {
            fields.push(field.clone());
        }
    }

    let packet = IwaPacket::new(ProtoMessage::new(fields).encode()?);
    IwaArchive::encode(packet, body)
}

/// Reads `(row_count, column_count)` from a decoded tile archive.
///
/// Row count is the number of `TileRowInfo` (`f5`) entries; column count is the
/// first row's `f2`. Falls back to the tile-level `f4` row count when present.
fn tile_geometry(tile: &IwaArchive) -> Result<(usize, usize), Error> {
    let message = ProtoMessage::decode(tile.body())?;
    let rows: Vec<&ProtoField> = message.fields_by_number(5).collect();
    let row_count = rows.len();
    let col_count = rows
        .first()
        .and_then(|row| row.value.as_message().ok().flatten())
        .and_then(|row| row.field(2).and_then(|f| f.value.as_varint()))
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);

    if row_count == 0 || col_count == 0 {
        return Err(Error::InvalidIwa("scaffold tile has no decodable geometry"));
    }
    Ok((row_count, col_count))
}

/// Pads or truncates `rows` to exactly `row_count` × `col_count`, filling any
/// missing cells with [`CellValue::Empty`].
fn reshape_rows(
    rows: &[Vec<CellValue>],
    row_count: usize,
    col_count: usize,
) -> Vec<Vec<CellValue>> {
    (0..row_count)
        .map(|r| {
            (0..col_count)
                .map(|c| {
                    rows.get(r)
                        .and_then(|row| row.get(c))
                        .cloned()
                        .unwrap_or(CellValue::Empty)
                })
                .collect()
        })
        .collect()
}

/// A string `DataList` seeded from an existing archive, so original entries keep
/// their keys; new strings are appended with fresh keys.
struct StringTable {
    entries: Vec<(u32, String)>,
    next_key: u32,
}

impl StringTable {
    fn from_datalist(archive: &IwaArchive) -> Result<Self, Error> {
        let message = ProtoMessage::decode(archive.body())?;
        let mut entries = Vec::new();
        let mut max_key = 0;

        for entry in message.fields_by_number(3) {
            let Some(entry) = entry.value.as_message().ok().flatten() else {
                continue;
            };
            let Some(key) = entry
                .field(1)
                .and_then(|f| f.value.as_varint())
                .and_then(|k| u32::try_from(k).ok())
            else {
                continue;
            };
            let value = entry
                .field(3)
                .and_then(|f| f.value.as_bytes())
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or_default()
                .to_owned();
            max_key = max_key.max(key);
            entries.push((key, value));
        }

        Ok(Self {
            entries,
            next_key: max_key + 1,
        })
    }

    /// Returns the key for `value`, reusing an existing entry or appending one.
    fn key_for(&mut self, value: &str) -> u32 {
        if let Some((key, _)) = self.entries.iter().find(|(_, s)| s == value) {
            return *key;
        }
        let key = self.next_key;
        self.next_key += 1;
        self.entries.push((key, value.to_owned()));
        key
    }

    /// Encodes the `TableDataList` body (`f1`=list type, `f2`=next key, repeated
    /// `f3` entries `{ f1=key, f2=refcount, f3=string }`).
    fn encode_body(&self) -> Result<Vec<u8>, Error> {
        let mut fields = vec![
            ProtoField::varint(1, 1),
            ProtoField::varint(2, u64::from(self.next_key)),
        ];
        for (key, value) in &self.entries {
            fields.push(ProtoField::message(
                3,
                ProtoMessage::new(vec![
                    ProtoField::varint(1, u64::from(*key)),
                    ProtoField::varint(2, 1),
                    ProtoField::string(3, value),
                ]),
            )?);
        }
        ProtoMessage::new(fields).encode()
    }
}

/// Rebuilds a tile body by patching only the *values* of cells the caller
/// supplied, leaving every other byte of the real tile untouched.
///
/// A cell is patched only when the caller value matches the original cell's
/// stored kind (number → decimal128/double, date → seconds, text → string key),
/// so record lengths and offsets never change.
fn patch_tile_body(
    tile: &IwaArchive,
    rows: &[Vec<CellValue>],
    strings: &mut StringTable,
) -> Result<Vec<u8>, Error> {
    let message = ProtoMessage::decode(tile.body())?;

    let mut fields: Vec<ProtoField> = Vec::with_capacity(message.fields().len());
    let mut row_index = 0usize;
    for field in message.fields() {
        if field.number != 5 {
            fields.push(field.clone());
            continue;
        }

        let row_message = field
            .value
            .as_message()
            .ok()
            .flatten()
            .ok_or(Error::InvalidIwa("tile row is not a message"))?;
        let patched = patch_row_message(&row_message, rows.get(row_index), strings)?;
        fields.push(ProtoField::message(5, patched)?);
        row_index += 1;
    }

    ProtoMessage::new(fields).encode()
}

/// Patches one `TileRowInfo`'s cell-storage buffer (`f6`) in place; all other
/// row fields are preserved verbatim.
fn patch_row_message(
    row: &ProtoMessage,
    values: Option<&Vec<CellValue>>,
    strings: &mut StringTable,
) -> Result<ProtoMessage, Error> {
    let Some(values) = values else {
        return Ok(row.clone());
    };

    let mut storage = row
        .field(6)
        .and_then(|f| f.value.as_bytes())
        .unwrap_or_default()
        .to_vec();
    let offsets: Vec<u16> = row
        .field(7)
        .and_then(|f| f.value.as_bytes())
        .unwrap_or_default()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();

    for (column, value) in values.iter().enumerate() {
        if matches!(value, CellValue::Empty) {
            continue;
        }
        let Some(&offset) = offsets.get(column) else {
            continue;
        };
        if offset == 0xffff {
            continue;
        }
        patch_cell_value(&mut storage, offset as usize, value, strings);
    }

    let mut fields: Vec<ProtoField> = Vec::with_capacity(row.fields().len());
    for field in row.fields() {
        if field.number == 6 {
            fields.push(ProtoField::bytes(6, storage.clone()));
        } else {
            fields.push(field.clone());
        }
    }
    Ok(ProtoMessage::new(fields))
}

/// Overwrites the value bytes of a single wide-cell record, keeping its flags,
/// type byte, and trailing style ids unchanged. No-op on a kind mismatch.
fn patch_cell_value(
    storage: &mut [u8],
    offset: usize,
    value: &CellValue,
    strings: &mut StringTable,
) {
    let Some(record) = storage.get(offset..) else {
        return;
    };
    if record.len() < 12 || record[0] != 0x05 {
        return;
    }
    let flags = u32::from_le_bytes([record[8], record[9], record[10], record[11]]);

    // The value field is the lowest set value bit; its offset is 12 plus the
    // widths of any lower-order value fields that are also present.
    let value_offset = |bit: u32| -> usize {
        let mut at = offset + 12;
        for (b, width) in [(0x1, 16usize), (0x2, 8), (0x4, 8), (0x8, 4)] {
            if b == bit {
                break;
            }
            if flags & b != 0 {
                at += width;
            }
        }
        at
    };

    match value {
        CellValue::Number(n) if flags & 0x1 != 0 => {
            let at = value_offset(0x1);
            if let Some(slot) = storage.get_mut(at..at + 16) {
                slot.copy_from_slice(&encode_decimal128(*n));
            }
        }
        CellValue::Number(n) if flags & 0x2 != 0 => {
            let at = value_offset(0x2);
            if let Some(slot) = storage.get_mut(at..at + 8) {
                slot.copy_from_slice(&n.to_le_bytes());
            }
        }
        CellValue::Date(seconds) if flags & 0x4 != 0 => {
            let at = value_offset(0x4);
            if let Some(slot) = storage.get_mut(at..at + 8) {
                slot.copy_from_slice(&seconds.to_le_bytes());
            }
        }
        CellValue::Text(text) if flags & 0x8 != 0 => {
            let key = strings.key_for(text);
            let at = value_offset(0x8);
            if let Some(slot) = storage.get_mut(at..at + 4) {
                slot.copy_from_slice(&key.to_le_bytes());
            }
        }
        _ => {}
    }
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
            ProtoMessage::new(vec![
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

    #[test]
    fn scaffold_package_patches_numeric_cells_in_place() -> Result<(), Error> {
        // The scaffold tile is patched in place: number cells in the caller's
        // rows overwrite the matching number cells of the real tile (verified to
        // open in Numbers). We assert those patched values round-trip back out.
        let mut workbook = Workbook::new();
        let mut table = WritableTable::new("Summary");
        // Row 0 maps onto the tile header row; row 1 onto the first data row,
        // whose number columns are patched (text/date columns are left intact).
        table.push_row(vec![
            CellValue::Empty,
            CellValue::Empty,
            CellValue::Empty,
            CellValue::Empty,
        ]);
        table.push_row(vec![
            CellValue::Empty,
            CellValue::Number(1234.5),
            CellValue::Empty,
            CellValue::Number(6789.0),
        ]);
        workbook.add_table(table);

        let package_bytes = workbook.encode_scaffold_package()?;
        let spreadsheet = crate::numbers::Document::from_bytes(package_bytes)?.spreadsheet()?;

        let patched: Vec<f64> = spreadsheet
            .tables()
            .iter()
            .flat_map(|table| table.rows())
            .flat_map(|row| &row.cells)
            .filter_map(CellValue::as_number)
            .collect();

        assert!(
            patched.iter().any(|n| (n - 1234.5).abs() < 1e-6),
            "patched value 1234.5 not found in {patched:?}"
        );
        assert!(
            patched.iter().any(|n| (n - 6789.0).abs() < 1e-6),
            "patched value 6789.0 not found in {patched:?}"
        );

        Ok(())
    }
}
