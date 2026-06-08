use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::CellValue;
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
    /// This currently rewrites the first visible 4-column table from the
    /// bundled personal-budget fixture. It does not yet synthesize the entire
    /// Numbers document graph from scratch.
    pub fn encode_scaffold_package(&self) -> Result<Vec<u8>, Error> {
        let Some(table) = self.tables.first() else {
            return Err(Error::InvalidIwa("workbook must contain at least one table"));
        };
        let strings = collect_strings_starting_at(&table.rows, 10_000);
        let tile = encode_tile_archive_with_root(&table.rows, &strings, 904_525)?;
        let datalist = encode_string_datalist_archive_with_root(&strings, 904_498)?;

        let scaffold = Document::from_bytes(PERSONAL_BUDGET_SCAFFOLD.to_vec())?;
        let package = scaffold.package();
        let mut writer = PackageWriter::new();
        let document_identifier = generated_document_identifier();

        for entry in package.entries() {
            let bytes = match entry.path.as_str() {
                SUMMARY_TABLE_TILE_PATH => tile.as_slice(),
                SUMMARY_TABLE_DATALIST_PATH => datalist.as_slice(),
                "Metadata/DocumentIdentifier" => document_identifier.as_bytes(),
                _ => package.entry_bytes(&entry.path)?,
            };
            writer.add_entry(entry.path.clone(), bytes.to_vec());
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

fn encode_tile_archive(
    rows: &[Vec<CellValue>],
    strings: &BTreeMap<String, u32>,
) -> Result<Vec<u8>, Error> {
    encode_tile_archive_with_root(rows, strings, 1)
}

fn encode_tile_archive_with_root(
    rows: &[Vec<CellValue>],
    strings: &BTreeMap<String, u32>,
    root_object_id: u64,
) -> Result<Vec<u8>, Error> {
    let body = encode_length_delimited_stream(
        rows.iter()
            .enumerate()
            .map(|(row_index, row)| encode_row_message(row_index, row, strings))
            .collect::<Result<Vec<_>, Error>>()?,
    )?;

    let descriptor = IwaArchiveDescriptor {
        root_object_id: Some(root_object_id),
        kind_hint: Some(6002),
        body_hint: Some(u64::try_from(body.len()).map_err(|_| Error::InvalidIwa("body length overflow"))?),
        object_references: Vec::new(),
    };
    let header = IwaPacket::new(descriptor.encode_message()?.encode()?);
    IwaArchive::encode(header, body)
}

fn encode_string_datalist_archive(strings: &BTreeMap<String, u32>) -> Result<Vec<u8>, Error> {
    encode_string_datalist_archive_with_root(strings, 1)
}

fn encode_string_datalist_archive_with_root(
    strings: &BTreeMap<String, u32>,
    root_object_id: u64,
) -> Result<Vec<u8>, Error> {
    let mut messages = Vec::new();
    messages.push(ProtoMessage::new(vec![
        ProtoField::varint(
            1,
            u64::try_from(strings.len()).map_err(|_| Error::InvalidIwa("datalist size overflow"))?,
        ),
        ProtoField::varint(2, 1),
    ]));

    for (value, key) in strings {
        messages.push(ProtoMessage::new(vec![
            ProtoField::varint(1, u64::from(*key)),
            ProtoField::varint(2, 1),
            ProtoField::string(3, value),
        ]));
    }

    let body = encode_length_delimited_stream(messages)?;
    let descriptor = IwaArchiveDescriptor {
        root_object_id: Some(root_object_id),
        kind_hint: Some(6005),
        body_hint: Some(u64::try_from(body.len()).map_err(|_| Error::InvalidIwa("body length overflow"))?),
        object_references: Vec::new(),
    };
    let header = IwaPacket::new(descriptor.encode_message()?.encode()?);
    IwaArchive::encode(header, body)
}

fn encode_length_delimited_stream(messages: Vec<ProtoMessage>) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    for message in messages {
        ProtoField::message(1, message)?.encode_into(&mut body)?;
    }
    Ok(body)
}

fn encode_row_message(
    row_index: usize,
    row: &[CellValue],
    strings: &BTreeMap<String, u32>,
) -> Result<ProtoMessage, Error> {
    let mut storage = Vec::new();
    let mut offsets = Vec::with_capacity(row.len());

    for cell in row {
        if matches!(cell, CellValue::Empty) {
            offsets.push(0xffffu16);
            continue;
        }

        let offset = u16::try_from(storage.len())
            .map_err(|_| Error::InvalidIwa("cell storage offset overflow"))?;
        offsets.push(offset);
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
            u64::try_from(row.len()).map_err(|_| Error::InvalidIwa("column count overflow"))?,
        ),
        ProtoField::varint(
            5,
            u64::try_from(row.iter().filter(|cell| !matches!(cell, CellValue::Empty)).count())
                .map_err(|_| Error::InvalidIwa("cell count overflow"))?,
        ),
        ProtoField::bytes(6, storage),
        ProtoField::bytes(7, offset_bytes),
    ]))
}

fn encode_cell_record(
    cell: &CellValue,
    strings: &BTreeMap<String, u32>,
) -> Result<Vec<u8>, Error> {
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

fn generated_document_identifier() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0u128, |duration| duration.as_nanos());
    let value = format!("{nanos:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &value[0..8],
        &value[8..12],
        &value[12..16],
        &value[16..20],
        &value[20..32]
    )
    .to_uppercase()
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
    fn scaffold_package_rewrites_summary_table() -> Result<(), Error> {
        let mut workbook = Workbook::new();
        let mut table = WritableTable::new("Summary");
        table.push_row(vec![
            CellValue::Text("Category".to_owned()),
            CellValue::Text("Budget".to_owned()),
            CellValue::Text("Actual".to_owned()),
            CellValue::Text("Difference".to_owned()),
        ]);
        table.push_row(vec![
            CellValue::Text("Consulting".to_owned()),
            CellValue::Number(1000.0),
            CellValue::Number(850.0),
            CellValue::Number(150.0),
        ]);
        workbook.add_table(table);

        let package_bytes = workbook.encode_scaffold_package()?;
        let spreadsheet = crate::numbers::Document::from_bytes(package_bytes)?.spreadsheet()?;
        let rewritten_table = spreadsheet
            .tables()
            .into_iter()
            .find(|table| {
                table.rows().iter().any(|row| {
                    row.cells.iter().any(|cell| {
                        *cell == CellValue::Text("Consulting".to_owned())
                    })
                })
            })
            .ok_or(Error::InvalidIwa("rewritten scaffold table not found"))?;

        assert_eq!(
            rewritten_table.rows()[0].cells[0],
            CellValue::Text("Category".to_owned())
        );
        assert_eq!(
            rewritten_table.rows()[1].cells,
            vec![
                CellValue::Text("Consulting".to_owned()),
                CellValue::Number(1000.0),
                CellValue::Number(850.0),
                CellValue::Number(150.0),
            ]
        );

        Ok(())
    }
}
