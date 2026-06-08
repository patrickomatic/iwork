use std::collections::HashMap;

use crate::iwa::IwaArchive;
use crate::protobuf::read_varint;

/// A decoded table from a Numbers tile archive.
#[derive(Debug, Clone)]
pub struct Table {
    rows: Vec<TableRow>,
}

impl Table {
    /// Parse all rows from a tile archive, resolving string cells against the provided `DataList`s.
    pub(crate) fn from_tile(tile: &IwaArchive, strings: &HashMap<u32, String>) -> Self {
        let rows = decode_rows(tile, strings);
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

fn decode_rows(tile: &IwaArchive, strings: &HashMap<u32, String>) -> Vec<TableRow> {
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
        let cells = decode_cells(&msg, strings);
        rows.push(TableRow { index: row_index, cells });
    }

    rows
}

/// Decode a row's cells from the current (`field 6` + `field 7`) wide-cell encoding.
///
/// Field 7 is the u16 cell-offset array (`0xffff` = empty column); field 6 is the
/// cell-storage buffer. Each record begins with version byte `0x05`, a type byte,
/// and a u32 LE flags bitmask at bytes 8-11. The low flag bits select which value
/// field follows (in bit order) at byte 12: bit 0 = decimal128 number, bit 1 = IEEE
/// double, bit 2 = date (seconds since the Cocoa epoch), bit 3 = string `DataList`
/// key. Field 4 (legacy `_pre_bnc`, fixed 12-byte stride) is intentionally ignored.
fn decode_cells(msg: &crate::protobuf::ProtoMessage, strings: &HashMap<u32, String>) -> Vec<CellValue> {
    let f6 = msg.field(6).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
    let f7 = msg.field(7).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);

    let slots: Vec<u16> = f7
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();

    // The row's cell count (field 2) bounds how many offset slots are real columns;
    // fall back to the count of leading non-sentinel slots if it is missing or bogus.
    let col_count = msg
        .field(2)
        .and_then(|f| f.value.as_varint())
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n <= slots.len())
        .unwrap_or_else(|| slots.iter().take_while(|&&v| v != 0xffff).count());

    slots
        .iter()
        .take(col_count)
        .map(|&off| {
            if off == 0xffff {
                return CellValue::Empty;
            }
            f6.get(off as usize..)
                .map_or(CellValue::Empty, |rec| decode_cell_record(rec, strings))
        })
        .collect()
}

/// Decode a single wide-cell record (see [`decode_cells`] for the layout).
fn decode_cell_record(rec: &[u8], strings: &HashMap<u32, String>) -> CellValue {
    if rec.len() < 12 || rec[0] != 0x05 {
        return CellValue::Empty;
    }
    let flags = u32::from_le_bytes([rec[8], rec[9], rec[10], rec[11]]);
    let body = &rec[12..];

    if flags & 0x1 != 0 {
        body.get(..16).map_or(CellValue::Empty, |b| CellValue::Number(decode_decimal128(b)))
    } else if flags & 0x2 != 0 {
        body.get(..8).map_or(CellValue::Empty, |b| {
            CellValue::Number(f64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
        })
    } else if flags & 0x4 != 0 {
        body.get(..8).map_or(CellValue::Empty, |b| {
            CellValue::Date(f64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
        })
    } else if flags & 0x8 != 0 {
        let key = body.get(..4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap_or([0; 4])));
        strings.get(&key).cloned().map_or(CellValue::Empty, CellValue::Text)
    } else {
        CellValue::Empty
    }
}

/// Decode a 16-byte IEEE 754-2008 decimal128 value (as stored by Numbers) to `f64`.
///
/// The trailing two bytes hold the sign bit and biased exponent; bytes 0-13 plus the
/// low bit of byte 14 form the coefficient. This mirrors the well-known
/// `numbers-parser` decode and is exact enough for display use.
fn decode_decimal128(b: &[u8]) -> f64 {
    debug_assert!(b.len() >= 16);
    let exp = ((i32::from(b[15] & 0x7f) << 7) | i32::from(b[14] >> 1)) - 0x1820;
    let mut mantissa = f64::from(b[14] & 1);
    for &byte in b[..14].iter().rev() {
        mantissa = mantissa * 256.0 + f64::from(byte);
    }
    if b[15] & 0x80 != 0 {
        mantissa = -mantissa;
    }
    mantissa * 10f64.powi(exp)
}
