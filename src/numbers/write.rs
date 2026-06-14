use std::collections::BTreeMap;
use std::path::Path;

use super::CellValue;
use crate::iwa::{IwaArchive, IwaArchiveDescriptor, IwaObjectReference, IwaPacket};
use crate::protobuf::{ProtoField, ProtoMessage};
use crate::package::PackageWriter;
use crate::Error;

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

    /// Serializes this workbook as a minimal `.numbers` package.
    ///
    /// The generated package contains the metadata, core IWA members, stylesheet
    /// archive, and table archives needed for this crate to re-open and decode
    /// the workbook. This is intentionally a minimal package writer, not yet a
    /// complete Apple Numbers document graph.
    pub fn to_numbers_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut writer = PackageWriter::new();
        writer
            .add_entry("Metadata/Properties.plist", properties_plist())
            .add_entry("Metadata/DocumentIdentifier", document_identifier())
            .add_entry(
                "Metadata/BuildVersionHistory.plist",
                build_version_history_plist(),
            )
            .add_entry("Index/Document.iwa", encode_document_archive()?)
            .add_entry(
                "Index/DocumentMetadata.iwa",
                encode_document_metadata_archive()?,
            )
            .add_entry("Index/Metadata.iwa", encode_metadata_archive()?)
            .add_entry(
                "Index/ObjectContainer.iwa",
                encode_object_container_archive()?,
            )
            .add_entry(
                "Index/CalculationEngine.iwa",
                encode_calculation_engine_archive()?,
            )
            .add_entry("Index/ViewState.iwa", encode_view_state_archive()?)
            .add_entry(
                "Index/AnnotationAuthorStorage.iwa",
                encode_empty_archive(80, 11009)?,
            )
            .add_entry(
                "Index/DocumentStylesheet.iwa",
                encode_document_stylesheet_archive()?,
            );

        for archive in self.encode_table_archives()? {
            writer
                .add_entry(archive.datalist_path, archive.datalist)
                .add_entry(archive.tile_path, archive.tile);
        }

        writer.finish()
    }

    /// Writes this workbook as a minimal `.numbers` package.
    pub fn save_numbers(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        std::fs::write(path, self.to_numbers_bytes()?)?;
        Ok(())
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
            match cell.as_text() {
                Some(value) if !map.contains_key(value) => {
                    map.insert(value.to_owned(), next_key);
                    next_key += 1;
                }
                Some(_) | None => {}
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
const FLAG_FORMULA_RESULT: u32 = 0x0200;
/// `MessageInfo.version` triple (`f2`) carried by every real archive header.
const ARCHIVE_VERSION: [u8; 3] = [1, 0, 5];

const DOCUMENT_ROOT_ID: u64 = 1;
const METADATA_ROOT_ID: u64 = 2;
const OBJECT_CONTAINER_ROOT_ID: u64 = 61;
const DOCUMENT_METADATA_ROOT_ID: u64 = 71;
const ANNOTATION_AUTHOR_STORAGE_ROOT_ID: u64 = 80;
const CALCULATION_ENGINE_ROOT_ID: u64 = 1_000;
const VIEW_STATE_ROOT_ID: u64 = 1_001;
const STYLESHEET_ROOT_ID: u64 = 1_002;

fn properties_plist() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>documentUUID</key>
  <string>iwork-rs-generated-document</string>
  <key>fileFormatVersion</key>
  <string>14.4.1</string>
  <key>isMultiPage</key>
  <false/>
  <key>revision</key>
  <string>1</string>
  <key>stableDocumentUUID</key>
  <string>iwork-rs-generated-document</string>
  <key>versionUUID</key>
  <string>iwork-rs-generated-version</string>
</dict>
</plist>
"#
    .to_vec()
}

fn document_identifier() -> Vec<u8> {
    b"iwork-rs-generated-document".to_vec()
}

fn build_version_history_plist() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
  <dict>
    <key>app</key>
    <string>iwork-rs</string>
    <key>version</key>
    <string>0.1.0</string>
  </dict>
</array>
</plist>
"#
    .to_vec()
}

fn encode_empty_archive(root_object_id: u64, kind: u64) -> Result<Vec<u8>, Error> {
    let header = synthesize_header(root_object_id, kind, 0)?;
    IwaArchive::encode(header, Vec::new())
}

fn encode_document_archive() -> Result<Vec<u8>, Error> {
    let body = encode_document_body()?;
    let header = synthesize_header_with_references(
        DOCUMENT_ROOT_ID,
        1,
        body.len(),
        vec![IwaObjectReference {
            object_id: Some(METADATA_ROOT_ID),
            kind_hint: Some(3),
            state_hint: Some(0),
        }],
    )?;
    IwaArchive::encode(header, body)
}

fn encode_document_body() -> Result<Vec<u8>, Error> {
    let body_message = ProtoMessage::new(vec![
        ProtoField::bytes(1, object_reference(CALCULATION_ENGINE_ROOT_ID)?),
        ProtoField::bytes(4, object_reference(CALCULATION_ENGINE_ROOT_ID)?),
        ProtoField::bytes(5, object_reference(VIEW_STATE_ROOT_ID)?),
        ProtoField::bytes(6, object_reference(STYLESHEET_ROOT_ID)?),
        ProtoField::bytes(9, b"Application/Blank/Traditional".to_vec()),
        ProtoField::bytes(12, object_reference(ANNOTATION_AUTHOR_STORAGE_ROOT_ID)?),
    ]);

    ProtoMessage::new(vec![
        ProtoField::message(
            1,
            &ProtoMessage::new(vec![ProtoField::varint(1, CALCULATION_ENGINE_ROOT_ID)]),
        )?,
        ProtoField::message(
            1,
            &ProtoMessage::new(vec![ProtoField::varint(1, VIEW_STATE_ROOT_ID)]),
        )?,
        ProtoField::message(
            1,
            &ProtoMessage::new(vec![ProtoField::varint(1, STYLESHEET_ROOT_ID)]),
        )?,
        ProtoField::message(4, &body_message)?,
    ])
    .encode()
}

fn encode_document_metadata_archive() -> Result<Vec<u8>, Error> {
    let body = ProtoMessage::new(vec![
        ProtoField::varint(1, 0),
        ProtoField::bytes(3, Vec::new()),
    ])
    .encode()?;
    let header = synthesize_header_with_references(
        DOCUMENT_METADATA_ROOT_ID,
        11011,
        body.len(),
        vec![IwaObjectReference {
            object_id: Some(DOCUMENT_ROOT_ID),
            kind_hint: Some(3),
            state_hint: Some(1),
        }],
    )?;
    IwaArchive::encode(header, body)
}

fn encode_metadata_archive() -> Result<Vec<u8>, Error> {
    let body = ProtoMessage::new(vec![
        ProtoField::varint(1, 0),
        ProtoField::bytes(2, object_reference(DOCUMENT_ROOT_ID)?),
        ProtoField::bytes(3, object_reference(DOCUMENT_METADATA_ROOT_ID)?),
        ProtoField::bytes(4, object_reference(OBJECT_CONTAINER_ROOT_ID)?),
    ])
    .encode()?;
    let header = synthesize_header_with_references(
        METADATA_ROOT_ID,
        11006,
        body.len(),
        vec![IwaObjectReference {
            object_id: Some(DOCUMENT_ROOT_ID),
            kind_hint: Some(1),
            state_hint: Some(1),
        }],
    )?;
    IwaArchive::encode(header, body)
}

fn encode_object_container_archive() -> Result<Vec<u8>, Error> {
    let body = ProtoMessage::new(vec![
        ProtoField::varint(1, 0),
        ProtoField::bytes(2, object_reference(DOCUMENT_ROOT_ID)?),
    ])
    .encode()?;
    let header = synthesize_header(OBJECT_CONTAINER_ROOT_ID, 11008, body.len())?;
    IwaArchive::encode(header, body)
}

fn encode_calculation_engine_archive() -> Result<Vec<u8>, Error> {
    let body =
        ProtoMessage::new(vec![ProtoField::varint(1, 0), ProtoField::varint(5, 0)]).encode()?;
    let header = synthesize_header_with_references(
        CALCULATION_ENGINE_ROOT_ID,
        4000,
        body.len(),
        vec![IwaObjectReference {
            object_id: Some(DOCUMENT_ROOT_ID),
            kind_hint: Some(1),
            state_hint: Some(0),
        }],
    )?;
    IwaArchive::encode(header, body)
}

fn encode_view_state_archive() -> Result<Vec<u8>, Error> {
    let body = ProtoMessage::new(vec![
        ProtoField::varint(1, 0),
        ProtoField::bytes(2, object_reference(DOCUMENT_ROOT_ID)?),
    ])
    .encode()?;
    let header = synthesize_header(VIEW_STATE_ROOT_ID, 11012, body.len())?;
    IwaArchive::encode(header, body)
}

fn encode_document_stylesheet_archive() -> Result<Vec<u8>, Error> {
    let body = ProtoMessage::new(vec![
        ProtoField::message(
            2,
            &ProtoMessage::new(vec![
                ProtoField::string(1, "Normal"),
                ProtoField::bytes(2, object_reference(STYLESHEET_ROOT_ID)?),
            ]),
        )?,
        ProtoField::message(
            2,
            &ProtoMessage::new(vec![
                ProtoField::string(1, "Bold"),
                ProtoField::bytes(2, object_reference(STYLESHEET_ROOT_ID + 1)?),
                ProtoField::message(11, &ProtoMessage::new(vec![ProtoField::varint(1, 1)]))?,
            ]),
        )?,
    ])
    .encode()?;
    let header = synthesize_header_with_references(
        STYLESHEET_ROOT_ID,
        2,
        body.len(),
        vec![IwaObjectReference {
            object_id: Some(DOCUMENT_ROOT_ID),
            kind_hint: Some(1),
            state_hint: Some(0),
        }],
    )?;
    IwaArchive::encode(header, body)
}

fn object_reference(object_id: u64) -> Result<Vec<u8>, Error> {
    ProtoMessage::new(vec![ProtoField::varint(1, object_id)]).encode()
}

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
    synthesize_header_with_references(root_object_id, kind, body_len, Vec::new())
}

fn synthesize_header_with_references(
    root_object_id: u64,
    kind: u64,
    body_len: usize,
    object_references: Vec<IwaObjectReference>,
) -> Result<IwaPacket, Error> {
    let descriptor = IwaArchiveDescriptor {
        root_object_id: Some(root_object_id),
        kind_hint: Some(kind),
        message_version: Some(ARCHIVE_VERSION.to_vec()),
        body_hint: Some(
            u64::try_from(body_len).map_err(|_| Error::InvalidIwa("body length overflow"))?,
        ),
        object_references,
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
    let mut legacy_columns = Vec::new();

    for (column, slot) in offsets.iter_mut().take(col_count).enumerate() {
        let Some(cell) = row.get(column) else {
            legacy_columns.extend_from_slice(&legacy_column_record(
                column,
                0xffff,
                &CellValue::Empty,
            )?);
            continue;
        };
        if matches!(cell, CellValue::Empty) {
            legacy_columns.extend_from_slice(&legacy_column_record(column, 0xffff, cell)?);
            continue;
        }

        *slot = u16::try_from(storage.len())
            .map_err(|_| Error::InvalidIwa("cell storage offset overflow"))?;
        storage.extend_from_slice(&encode_cell_record(cell, strings)?);
        legacy_columns.extend_from_slice(&legacy_column_record(column, *slot, cell)?);
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
        ProtoField::bytes(3, legacy_columns),
        ProtoField::bytes(4, offset_bytes.clone()),
        ProtoField::varint(5, TILE_ROW_STORAGE_VERSION),
        ProtoField::bytes(6, storage),
        ProtoField::bytes(7, offset_bytes),
    ]))
}

fn legacy_column_record(column: usize, offset: u16, cell: &CellValue) -> Result<[u8; 12], Error> {
    let mut record = [0u8; 12];
    let column = u16::try_from(column).map_err(|_| Error::InvalidIwa("column index overflow"))?;
    record[0..2].copy_from_slice(&column.to_le_bytes());
    record[2..4].copy_from_slice(&offset.to_le_bytes());
    record[4] = match cell {
        CellValue::Empty | CellValue::Error => 0,
        CellValue::Number(_) | CellValue::Bool(_) | CellValue::Duration(_)
        | CellValue::Percentage(_) | CellValue::Currency { .. } => 2,
        CellValue::Date(_) => 4,
        CellValue::Formula { result, .. } => {
            return legacy_column_record(usize::from(column), offset, result);
        }
        CellValue::Text(_) => 8,
    };
    Ok(record)
}

fn encode_cell_record(cell: &CellValue, strings: &BTreeMap<String, u32>) -> Result<Vec<u8>, Error> {
    // Each arm is (wide-cell type byte, value flag, value bytes). The type byte at
    // record[1] is what the reader keys off; the flag locates the value at byte 12.
    let (cell_type, flags, payload) = match cell {
        CellValue::Empty => return Ok(Vec::new()),
        CellValue::Number(value) | CellValue::Percentage(value) | CellValue::Currency { value, .. } => {
            (2u8, 0x2u32, value.to_le_bytes().to_vec())
        }
        CellValue::Bool(value) => (6u8, 0x2u32, f64::from(u8::from(*value)).to_le_bytes().to_vec()),
        CellValue::Date(value) => (5u8, 0x4u32, value.to_le_bytes().to_vec()),
        CellValue::Duration(value) => (7u8, 0x2u32, value.to_le_bytes().to_vec()),
        // An error cell has no value field; the type byte alone round-trips it.
        CellValue::Error => (8u8, 0x0u32, Vec::new()),
        CellValue::Text(value) => {
            let key = strings
                .get(value)
                .ok_or(Error::InvalidIwa("missing string datalist key"))?;
            (3u8, 0x8u32, key.to_le_bytes().to_vec())
        }
        CellValue::Formula { result, formula_id } => {
            let mut record = encode_cell_record(result, strings)?;
            if record.len() >= 12 {
                let flags = u32::from_le_bytes([record[8], record[9], record[10], record[11]])
                    | FLAG_FORMULA_RESULT;
                record[8..12].copy_from_slice(&flags.to_le_bytes());
                if let Some(id) = formula_id {
                    record.extend_from_slice(&id.to_le_bytes());
                }
            }
            return Ok(record);
        }
    };

    let mut record = vec![0x05, cell_type, 0, 0, 0, 0, 0, 0];
    record.extend_from_slice(&flags.to_le_bytes());
    record.extend_from_slice(&payload);
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numbers::table::{Table, decode_string_datalist};
    use crate::{DocumentKind, PackageSupport, numbers};

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
        table.push_row(vec![
            CellValue::Text("Formula".to_owned()),
            CellValue::Formula {
                result: Box::new(CellValue::Number(7.0)),
                formula_id: Some(17),
            },
            CellValue::Formula {
                result: Box::new(CellValue::Text("cached".to_owned())),
                formula_id: Some(18),
            },
        ]);
        workbook.add_table(table);

        let archives = workbook.encode_table_archives()?;
        assert_eq!(archives.len(), 1);

        let datalist_archive = IwaArchive::decode(&archives[0].datalist)?;
        let strings = decode_string_datalist(&datalist_archive);
        assert_eq!(strings.get(&1).map(String::as_str), Some("Category"));
        assert!(strings.values().any(|value| value == "Utilities"));

        let tile_archive = IwaArchive::decode(&archives[0].tile)?;
        let tile_body = ProtoMessage::decode(tile_archive.body())?;
        let first_row = tile_body
            .field(5)
            .and_then(|field| field.value.as_message().ok())
            .flatten()
            .ok_or(Error::InvalidIwa("missing generated row message"))?;
        assert!(
            first_row
                .field(3)
                .and_then(|f| f.value.as_bytes())
                .is_some()
        );
        assert_eq!(
            first_row.field(4).and_then(|f| f.value.as_bytes()),
            first_row.field(7).and_then(|f| f.value.as_bytes())
        );

        let decoded = Table::from_tile(&tile_archive, &strings, &std::collections::HashMap::new(), &std::collections::HashMap::new());
        assert_eq!(decoded.rows().len(), 3);
        assert_eq!(
            decoded.rows()[1].cells,
            vec![
                CellValue::Text("Utilities".to_owned()),
                CellValue::Number(42.5),
                CellValue::Date(625_881_600.0),
            ]
        );
        assert_eq!(
            decoded.rows()[2].cells,
            vec![
                CellValue::Text("Formula".to_owned()),
                CellValue::Formula {
                    result: Box::new(CellValue::Number(7.0)),
                    formula_id: Some(17),
                },
                CellValue::Formula {
                    result: Box::new(CellValue::Text("cached".to_owned())),
                    formula_id: Some(18),
                },
            ]
        );

        Ok(())
    }

    #[test]
    fn workbook_serializes_to_readable_numbers_package() -> Result<(), Error> {
        let workbook = sample_workbook();
        let bytes = workbook.to_numbers_bytes()?;
        let document = numbers::Document::from_bytes(bytes)?;
        let report = document.inspect("generated.numbers")?;

        assert_eq!(report.kind, DocumentKind::Numbers);
        assert_eq!(report.support, PackageSupport::SupportedDirectIndexEntries);
        assert_eq!(
            report.properties.document_uuid.as_deref(),
            Some("iwork-rs-generated-document")
        );
        assert!(
            document
                .package()
                .entries()
                .iter()
                .any(|entry| entry.path == "Index/ObjectContainer.iwa")
        );
        assert!(
            document
                .package()
                .entries()
                .iter()
                .any(|entry| entry.path == "Index/CalculationEngine.iwa")
        );
        assert!(
            document
                .package()
                .entries()
                .iter()
                .any(|entry| entry.path == "Index/ViewState.iwa")
        );

        let spreadsheet = document.spreadsheet()?;
        let tables = spreadsheet.tables();
        assert_eq!(tables.len(), 1);
        assert_eq!(
            tables[0].rows()[1].cells,
            vec![
                CellValue::Text("Utilities".to_owned()),
                CellValue::Number(42.5),
                CellValue::Date(625_881_600.0),
            ]
        );

        Ok(())
    }

    #[test]
    fn workbook_saves_to_readable_numbers_file() -> Result<(), Error> {
        let path =
            std::env::temp_dir().join(format!("iwork-generated-{}.numbers", std::process::id()));
        let workbook = sample_workbook();

        workbook.save_numbers(&path)?;
        let document = numbers::Document::open(&path)?;
        let tables = document.spreadsheet()?.tables();
        assert_eq!(tables.len(), 1);
        assert_eq!(
            tables[0].rows()[0].cells,
            vec![
                CellValue::Text("Category".to_owned()),
                CellValue::Text("Amount".to_owned()),
                CellValue::Text("When".to_owned()),
            ]
        );

        std::fs::remove_file(path)?;
        Ok(())
    }

    fn sample_workbook() -> Workbook {
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
        workbook
    }
}
