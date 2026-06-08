use std::collections::HashMap;

use crate::iwa::IwaArchive;
use crate::protobuf::read_varint;

/// A decoded table from a Numbers tile archive.
#[derive(Debug, Clone)]
pub struct Table {
    rows: Vec<TableRow>,
}

impl Table {
    /// Parse all tile archives from a tile archive, using the provided string and formula `DataList`s.
    pub(crate) fn from_tile(
        tile: &IwaArchive,
        strings: &HashMap<u32, String>,
        formula: &HashMap<u32, f64>,
    ) -> Self {
        let rows = decode_rows(tile, strings, formula);
        Self { rows }
    }

    pub fn rows(&self) -> &[TableRow] {
        &self.rows
    }
}

/// A single row within a table.
#[derive(Debug, Clone)]
pub struct TableRow {
    pub index: u64,
    pub cells: Vec<CellValue>,
}

/// The typed value of a single cell.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    /// Seconds since the Cocoa epoch (January 1, 2001 UTC).
    Date(f64),
    Number(f64),
    Text(String),
}

impl CellValue {
    pub fn as_date_seconds(&self) -> Option<f64> {
        if let Self::Date(s) = self { Some(*s) } else { None }
    }

    pub fn as_number(&self) -> Option<f64> {
        if let Self::Number(n) = self { Some(*n) } else { None }
    }

    pub fn as_text(&self) -> Option<&str> {
        if let Self::Text(s) = self { Some(s) } else { None }
    }
}

/// Parse all string entries from a `DataList` archive body into a key → string map.
pub(crate) fn decode_string_datalist(archive: &IwaArchive) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let body = archive.body();
    let mut cursor = archive.leading_object_references_len();

    while cursor < body.len() {
        let Ok(tag) = read_varint(body, &mut cursor) else { break };
        let wire_type = tag & 0x07;
        if wire_type != 2 {
            match wire_type {
                0 => { let _ = read_varint(body, &mut cursor); }
                1 => { cursor = cursor.saturating_add(8); }
                5 => { cursor = cursor.saturating_add(4); }
                _ => break,
            }
            continue;
        }
        let Ok(lv) = read_varint(body, &mut cursor) else { break };
        let Ok(len) = usize::try_from(lv) else { break };
        let Some(chunk) = body.get(cursor..cursor + len) else { break };
        cursor += len;
        let Ok(msg) = crate::protobuf::ProtoMessage::decode(chunk) else { continue };
        let Some(key_v) = msg.field(1).and_then(|f| f.value.as_varint()) else { continue };
        let Ok(key) = u32::try_from(key_v) else { continue };
        if let Some(s) = msg
            .field(3)
            .and_then(|f| f.value.as_bytes())
            .and_then(|b| std::str::from_utf8(b).ok())
        {
            map.insert(key, s.to_owned());
        }
    }

    map
}

fn decode_rows(tile: &IwaArchive, strings: &HashMap<u32, String>, formula: &HashMap<u32, f64>) -> Vec<TableRow> {
    let body = tile.body();
    let mut cursor = tile.leading_object_references_len();
    let mut rows = Vec::new();

    while cursor < body.len() {
        let Ok(tag) = read_varint(body, &mut cursor) else { break };
        let wire_type = tag & 0x07;
        if wire_type != 2 {
            match wire_type {
                0 => { let _ = read_varint(body, &mut cursor); }
                1 => { cursor = cursor.saturating_add(8); }
                5 => { cursor = cursor.saturating_add(4); }
                _ => break,
            }
            continue;
        }
        let Ok(lv) = read_varint(body, &mut cursor) else { break };
        let Ok(len) = usize::try_from(lv) else { break };
        let Some(chunk) = body.get(cursor..cursor + len) else { break };
        cursor += len;
        let Ok(msg) = crate::protobuf::ProtoMessage::decode(chunk) else { continue };
        if msg.fields().is_empty() { continue }

        let row_index = msg.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);
        let cells = decode_cells(&msg, strings, formula);
        rows.push(TableRow { index: row_index, cells });
    }

    rows
}

fn decode_cells(msg: &crate::protobuf::ProtoMessage, strings: &HashMap<u32, String>, formula: &HashMap<u32, f64>) -> Vec<CellValue> {
    let f4 = msg.field(4).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
    let f6 = msg.field(6).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
    let f7 = msg.field(7).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);

    // Collect non-sentinel f4 column offsets (byte offsets into f6 for cell records).
    let col_offsets: Vec<usize> = f4
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|&v| v != 0xffff)
        .map(|v| v as usize)
        .collect();

    // The inline numeric value area starts immediately after the last style record
    // (max of f7 non-sentinel offsets + 12 bytes).
    let inline_area_start = f7
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .filter(|&v| v != 0xffff)
        .map(|v| v as usize)
        .max()
        .map_or(f6.len(), |last| last + 12);

    let mut cells = Vec::with_capacity(col_offsets.len());
    let mut inline_idx = 0usize;

    for off in col_offsets {
        let Some(rec) = f6.get(off..off + 12) else {
            cells.push(CellValue::Empty);
            continue;
        };

        let type_byte = rec[0];
        let sub_type = rec[2];

        let value = match type_byte {
            0x03 => {
                // Plain text: u32 DataList key at bytes 4-7
                let key = u32::from_le_bytes(rec[4..8].try_into().unwrap_or([0; 4]));
                strings.get(&key).cloned().map_or(CellValue::Empty, CellValue::Text)
            }

            0x00 if sub_type == 0x00 => {
                // Date cell: f64 LE at bytes 0-7.
                // Cocoa dates (seconds since Jan 1 2001) for years 2001-2128 are in [1, 4e9].
                // If the value is outside that range it is Pattern D (formula cell): bytes 8-11
                // hold the formula DataList key.
                let secs = f64::from_le_bytes(rec[..8].try_into().unwrap_or([0; 8]));
                if secs > 1.0 && secs < 4_000_000_000.0 {
                    CellValue::Date(secs)
                } else {
                    let key = u32::from_le_bytes(rec[8..12].try_into().unwrap_or([0; 4]));
                    formula.get(&key).copied().map_or(CellValue::Empty, CellValue::Number)
                }
            }

            0x00 if sub_type == 0x80 => {
                // Number: u32 in the inline area, one per numeric cell in order
                let pos = inline_area_start + inline_idx * 12;
                inline_idx += 1;
                if let Some(raw) = f6.get(pos..pos + 4) {
                    let n = u32::from_le_bytes(raw.try_into().unwrap_or([0; 4]));
                    CellValue::Number(f64::from(n))
                } else {
                    CellValue::Empty
                }
            }

            // Pattern A: byte0 is a non-zero type that varies per row (formula cells),
            // bytes 1-3 are zero, bytes 8-11 hold the formula DataList key.
            _ if rec[1] == 0 && rec[2] == 0 && rec[3] == 0 => {
                let key = u32::from_le_bytes(rec[8..12].try_into().unwrap_or([0; 4]));
                formula.get(&key).copied().map_or(CellValue::Empty, CellValue::Number)
            }

            _ => CellValue::Empty,
        };

        cells.push(value);
    }

    cells
}
