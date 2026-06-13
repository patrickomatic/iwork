use std::collections::HashMap;

use crate::iwa::IwaArchive;
use crate::protobuf::read_varint;

/// A decoded table from a Numbers tile archive.
#[derive(Debug, Clone)]
pub struct Table {
    rows: Vec<TableRow>,
}

impl Table {
    /// Parse all rows from a tile archive, resolving string, rich-text, and
    /// formatted (currency / percentage) cells against their respective DataList maps.
    pub(crate) fn from_tile(
        tile: &IwaArchive,
        strings: &HashMap<u32, String>,
        rich_texts: &HashMap<u32, String>,
        formats: &HashMap<u32, CellFormat>,
    ) -> Self {
        let rows = decode_rows(tile, strings, rich_texts, formats);
        Self { rows }
    }

    /// Builds a table from already-decoded rows (e.g. merged across tiles).
    pub(crate) fn from_rows(rows: Vec<TableRow>) -> Self {
        Self { rows }
    }

    pub fn rows(&self) -> &[TableRow] {
        &self.rows
    }

    /// Consumes the table, returning its rows.
    pub(crate) fn into_rows(self) -> Vec<TableRow> {
        self.rows
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
    /// A checkbox / boolean cell (Numbers cell type 6).
    Bool(bool),
    /// A monetary value with an optional ISO 4217 currency code (e.g. `"USD"`).
    /// The `value` is the raw amount (e.g. `1234.0` for `$1,234.00`).
    Currency { value: f64, code: Option<String> },
    /// Seconds since the Cocoa epoch (January 1, 2001 UTC).
    Date(f64),
    /// A span of time, in seconds (Numbers cell type 7).
    Duration(f64),
    /// A cell holding a formula error, e.g. `=1/0` (Numbers cell type 8). The
    /// cell carries no recoverable value, only the error state.
    Error,
    /// A formula cell whose expression is not decoded yet, carrying the cached
    /// result value stored in the tile.
    Formula(Box<CellValue>),
    Number(f64),
    /// A percentage value stored as a decimal fraction (e.g. `0.33` for `33%`).
    Percentage(f64),
    Text(String),
}

impl CellValue {
    /// Returns the boolean value for checkbox cells.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Formula(value) => value.as_bool(),
            _ => None,
        }
    }

    /// Returns the duration, in seconds, for duration cells.
    pub fn as_duration_seconds(&self) -> Option<f64> {
        match self {
            Self::Duration(s) => Some(*s),
            Self::Formula(value) => value.as_duration_seconds(),
            _ => None,
        }
    }

    /// Returns the Cocoa-epoch seconds for date cells.
    pub fn as_date_seconds(&self) -> Option<f64> {
        match self {
            Self::Date(s) => Some(*s),
            Self::Formula(value) => value.as_date_seconds(),
            _ => None,
        }
    }

    /// Returns the amount for currency cells (the raw `f64`, not formatted with symbol).
    pub fn as_currency(&self) -> Option<f64> {
        match self {
            Self::Currency { value, .. } => Some(*value),
            Self::Formula(value) => value.as_currency(),
            _ => None,
        }
    }

    /// Returns the decimal fraction for percentage cells (e.g. `0.33` for 33%).
    pub fn as_percentage(&self) -> Option<f64> {
        match self {
            Self::Percentage(p) => Some(*p),
            Self::Formula(value) => value.as_percentage(),
            _ => None,
        }
    }

    /// Returns the numeric value for number cells.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            Self::Formula(value) => value.as_number(),
            _ => None,
        }
    }

    /// Returns the UTF-8 string slice for text cells.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Formula(value) => value.as_text(),
            _ => None,
        }
    }

    /// Returns the cached result for formula cells.
    pub fn formula_result(&self) -> Option<&CellValue> {
        if let Self::Formula(value) = self {
            Some(value)
        } else {
            None
        }
    }
}

/// Format kind values found in the cell format `DataList` (field 6 → field 1 of each entry).
const FORMAT_KIND_CURRENCY: u64 = 257;
const FORMAT_KIND_PERCENTAGE: u64 = 258;

/// Decoded cell format from the format `DataList` (DataStore field 22).
#[derive(Debug, Clone)]
pub(crate) enum CellFormat {
    Currency { code: Option<String> },
    Percentage,
    Other,
}

/// Parse a cell-format `DataList` archive into a key → `CellFormat` map.
///
/// Each entry has `{field 1: key, field 6: {field 1: format_kind, field 3: currency_code}}`.
/// Only `format_kind` 257 (currency) and 258 (percentage) produce typed variants;
/// everything else maps to [`CellFormat::Other`].
pub(crate) fn decode_cell_format_datalist(archive: &IwaArchive) -> HashMap<u32, CellFormat> {
    let body = archive.body();
    let mut cursor = archive.leading_object_references_len();
    let mut map = HashMap::new();

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
        let Ok(entry) = crate::protobuf::ProtoMessage::decode(chunk) else { continue };
        let Some(key_v) = entry.field(1).and_then(|f| f.value.as_varint()) else { continue };
        let Ok(key) = u32::try_from(key_v) else { continue };

        let format = entry
            .field(6)
            .and_then(|f| f.value.as_bytes())
            .and_then(|b| crate::protobuf::ProtoMessage::decode(b).ok())
            .map(|fmt| {
                let kind = fmt.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);
                match kind {
                    FORMAT_KIND_CURRENCY => {
                        let code = fmt
                            .field(3)
                            .and_then(|f| f.value.as_bytes())
                            .and_then(|b| std::str::from_utf8(b).ok())
                            .map(str::to_owned);
                        CellFormat::Currency { code }
                    }
                    FORMAT_KIND_PERCENTAGE => CellFormat::Percentage,
                    _ => CellFormat::Other,
                }
            })
            .unwrap_or(CellFormat::Other);

        map.insert(key, format);
    }
    map
}

/// Rich-text field 9 in a `DataList` entry: contains a reference to a type-`6218` object.
const RICH_TEXT_ENTRY_REF_FIELD: u32 = 9;

/// Parse a rich-text `DataList` archive into a key → plain-string map.
///
/// Chain: `DataList` entry `{field 1: key, field 9: {field 1: 6218_id}}` →
/// `6218` object `{field 1: {field 1: 2001_id}}` → `2001` object `{field 3: utf8}`.
/// All three object types are co-resident in the same `.iwa` archive file.
pub(crate) fn decode_rich_text_datalist(archive: &IwaArchive) -> HashMap<u32, String> {
    let objects = archive.objects();
    // Build id → payload map for secondary objects (6218 and 2001 wrappers).
    let secondary: HashMap<u64, &[u8]> = objects
        .iter()
        .skip(1)
        .filter_map(|obj| Some((obj.identifier?, obj.payload.as_slice())))
        .collect();

    // The root object is the DataList; parse its payload for entries.
    let Some(root_payload) = objects.first().map(|o| o.payload.as_slice()) else {
        return HashMap::new();
    };
    let Ok(root_msg) = crate::protobuf::ProtoMessage::decode(root_payload) else {
        return HashMap::new();
    };

    let mut map = HashMap::new();
    for entry_field in root_msg.fields_by_number(3) {
        let Some(entry_bytes) = entry_field.value.as_bytes() else {
            continue;
        };
        let Ok(entry) = crate::protobuf::ProtoMessage::decode(entry_bytes) else {
            continue;
        };
        let Some(key_v) = entry.field(1).and_then(|f| f.value.as_varint()) else {
            continue;
        };
        let Ok(key) = u32::try_from(key_v) else {
            continue;
        };
        // field 9 → {field 1: 6218_object_id}
        let Some(wrapper_id) = entry
            .field(RICH_TEXT_ENTRY_REF_FIELD)
            .and_then(|f| f.value.as_bytes())
            .and_then(|b| crate::protobuf::ProtoMessage::decode(b).ok())
            .and_then(|ref_msg| ref_msg.field(1).and_then(|f| f.value.as_varint()))
        else {
            continue;
        };
        // 6218 object: field 1 → {field 1: 2001_object_id}
        let Some(storage_id) = secondary
            .get(&wrapper_id)
            .and_then(|b| crate::protobuf::ProtoMessage::decode(b).ok())
            .as_ref()
            .and_then(|m| m.field(1).and_then(|f| f.value.as_bytes()))
            .and_then(|b| crate::protobuf::ProtoMessage::decode(b).ok())
            .as_ref()
            .and_then(|m| m.field(1).and_then(|f| f.value.as_varint()))
        else {
            continue;
        };
        // 2001 object: field 3 = UTF-8 plain text
        if let Some(text) = secondary
            .get(&storage_id)
            .and_then(|b| crate::protobuf::ProtoMessage::decode(b).ok())
            .as_ref()
            .and_then(|m| m.field(3).and_then(|f| f.value.as_bytes()))
            .and_then(|b| std::str::from_utf8(b).ok())
        {
            map.insert(key, text.to_owned());
        }
    }
    map
}

/// Parse all string entries from a `DataList` archive body into a key → string map.
pub(crate) fn decode_string_datalist(archive: &IwaArchive) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let body = archive.body();
    let mut cursor = archive.leading_object_references_len();

    while cursor < body.len() {
        let Ok(tag) = read_varint(body, &mut cursor) else {
            break;
        };
        let wire_type = tag & 0x07;
        if wire_type != 2 {
            match wire_type {
                0 => {
                    let _ = read_varint(body, &mut cursor);
                }
                1 => {
                    cursor = cursor.saturating_add(8);
                }
                5 => {
                    cursor = cursor.saturating_add(4);
                }
                _ => break,
            }
            continue;
        }
        let Ok(lv) = read_varint(body, &mut cursor) else {
            break;
        };
        let Ok(len) = usize::try_from(lv) else { break };
        let Some(chunk) = body.get(cursor..cursor + len) else {
            break;
        };
        cursor += len;
        let Ok(msg) = crate::protobuf::ProtoMessage::decode(chunk) else {
            continue;
        };
        let Some(key_v) = msg.field(1).and_then(|f| f.value.as_varint()) else {
            continue;
        };
        let Ok(key) = u32::try_from(key_v) else {
            continue;
        };
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

fn decode_rows(
    tile: &IwaArchive,
    strings: &HashMap<u32, String>,
    rich_texts: &HashMap<u32, String>,
    formats: &HashMap<u32, CellFormat>,
) -> Vec<TableRow> {
    let body = tile.body();
    let mut cursor = tile.leading_object_references_len();
    let mut rows = Vec::new();

    while cursor < body.len() {
        let Ok(tag) = read_varint(body, &mut cursor) else {
            break;
        };
        let wire_type = tag & 0x07;
        if wire_type != 2 {
            match wire_type {
                0 => {
                    let _ = read_varint(body, &mut cursor);
                }
                1 => {
                    cursor = cursor.saturating_add(8);
                }
                5 => {
                    cursor = cursor.saturating_add(4);
                }
                _ => break,
            }
            continue;
        }
        let Ok(lv) = read_varint(body, &mut cursor) else {
            break;
        };
        let Ok(len) = usize::try_from(lv) else { break };
        let Some(chunk) = body.get(cursor..cursor + len) else {
            break;
        };
        cursor += len;
        let Ok(msg) = crate::protobuf::ProtoMessage::decode(chunk) else {
            continue;
        };
        if msg.fields().is_empty() {
            continue;
        }

        let row_index = msg.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);
        let cells = decode_cells(&msg, strings, rich_texts, formats);
        rows.push(TableRow {
            index: row_index,
            cells,
        });
    }

    rows
}

/// Wide-cell record layout constants (see [`decode_cell_record`]).
const WIDE_CELL_VERSION: u8 = 0x05;
const CELL_TYPE_NUMBER: u8 = 2;
const CELL_TYPE_TEXT: u8 = 3;
const CELL_TYPE_DATE: u8 = 5;
const CELL_TYPE_BOOL: u8 = 6;
const CELL_TYPE_DURATION: u8 = 7;
const CELL_TYPE_ERROR: u8 = 8;
/// A second decimal128 numeric cell type, distinct from [`CELL_TYPE_NUMBER`].
/// Observed on currency cells (`more_types` fixture) and on formatted numeric
/// columns (`my_stocks`); what exactly separates it from the plain number type
/// is not pinned down (it is *not* "formula result" — a number-returning formula
/// is type 2, and a literal currency value is type 10). Both carry a decimal128
/// value, so both decode to [`CellValue::Number`].
const CELL_TYPE_NUMBER_ALT: u8 = 10;
/// Rich-text cell (type 9): the cell record carries a u32 key (flag `0x10`)
/// into the rich-text `DataList` referenced by `DataStore.field 17`. Each
/// `DataList` entry chains through a `6218` wrapper object to a `2001` TSWP storage
/// whose field 3 is the plain UTF-8 string.
const CELL_TYPE_RICH_TEXT: u8 = 9;

/// Decode a row's cells from the current (`field 6` + `field 7`) wide-cell encoding.
///
/// Field 7 is the u16 cell-offset array (`0xffff` = empty column); field 6 is the
/// cell-storage buffer. Each record begins with version byte `0x05`, a **type
/// byte** at offset 1, and a u32 LE flags bitmask at bytes 8-11. The type byte
/// selects the cell's value kind (number / text / date / bool); the flag bits
/// (low-to-high) then place the value and any format/style references as trailing
/// fields starting at byte 12, value fields first. Field 4 (legacy `_pre_bnc`,
/// fixed 12-byte stride) is intentionally ignored.
fn decode_cells(
    msg: &crate::protobuf::ProtoMessage,
    strings: &HashMap<u32, String>,
    rich_texts: &HashMap<u32, String>,
    formats: &HashMap<u32, CellFormat>,
) -> Vec<CellValue> {
    let f6 = msg.field(6).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
    let f7 = msg.field(7).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);

    let slots: Vec<u16> = f7
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();

    // Decode up to and including the last column that has a real cell record.
    // field(2) is a non-empty-record count (not a column-range limit), so using
    // it as a take() bound silently drops cells when there are leading-sentinel
    // columns (e.g. a table with an empty column 0). rposition gives the
    // correct upper bound regardless of where sentinels appear.
    let col_count = slots
        .iter()
        .rposition(|&v| v != 0xffff)
        .map_or(0, |pos| pos + 1);

    slots
        .iter()
        .take(col_count)
        .map(|&off| {
            if off == 0xffff {
                return CellValue::Empty;
            }
            f6.get(off as usize..)
                .map_or(CellValue::Empty, |rec| {
                    decode_cell_record(rec, strings, rich_texts, formats)
                })
        })
        .collect()
}

/// Decode a single wide-cell record (see [`decode_cells`] for the layout).
///
/// The value kind is taken from the **type byte** (`rec[1]`), not from the flag
/// bits: booleans and numbers both store an 8-byte double, so only the type byte
/// tells them apart. The flags still locate the value within the trailing field
/// region — for numbers they distinguish a 16-byte decimal128 (`0x1`) from an
/// 8-byte IEEE double (`0x2`).
/// Flag-bit mask for the cell style key (first trailing u32 after the value).
const FLAG_CELL_STYLE: u32 = 0x1000;
/// Flag-bit mask observed on cached formula result cells.
const FLAG_FORMULA_RESULT: u32 = 0x0200;
/// Flag-bit mask for the numfmt key (second trailing u32 after the value).
/// Bits 13, 14, and 16 are each used by different cell types but all place
/// a single numfmt key at the same position in the trailing region.
const FLAG_NUMFMT: u32 = 0x2000 | 0x4000 | 0x0001_0000;

fn decode_cell_record(
    rec: &[u8],
    strings: &HashMap<u32, String>,
    rich_texts: &HashMap<u32, String>,
    formats: &HashMap<u32, CellFormat>,
) -> CellValue {
    if rec.len() < 12 || rec[0] != WIDE_CELL_VERSION {
        return CellValue::Empty;
    }
    let cell_type = rec[1];
    let flags = u32::from_le_bytes([rec[8], rec[9], rec[10], rec[11]]);
    let body = &rec[12..];

    let value = match cell_type {
        CELL_TYPE_NUMBER | CELL_TYPE_NUMBER_ALT => {
            let (value, value_len) = if flags & 0x1 != 0 {
                (body.get(..16).map(decode_decimal128), 16)
            } else {
                (read_f64(body), 8)
            };
            let Some(v) = value else {
                return CellValue::Empty;
            };
            // Trailing u32s: cell_style_key (bit 12), then numfmt_key (bits 13/14/16).
            let after_value = &body[value_len.min(body.len())..];
            let numfmt_key = if flags & FLAG_CELL_STYLE != 0 && flags & FLAG_NUMFMT != 0 {
                after_value
                    .get(4..8)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            } else if flags & FLAG_NUMFMT != 0 {
                after_value
                    .get(..4)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            } else {
                None
            };
            match numfmt_key.and_then(|k| formats.get(&k)) {
                Some(CellFormat::Percentage) => CellValue::Percentage(v),
                Some(CellFormat::Currency { code }) => CellValue::Currency {
                    value: v,
                    code: code.clone(),
                },
                _ => CellValue::Number(v),
            }
        }
        CELL_TYPE_BOOL => read_f64(body).map_or(CellValue::Empty, |v| CellValue::Bool(v != 0.0)),
        CELL_TYPE_DATE => read_f64(body).map_or(CellValue::Empty, CellValue::Date),
        CELL_TYPE_DURATION => read_f64(body).map_or(CellValue::Empty, CellValue::Duration),
        // An error cell carries no value field (only an error-kind id behind a
        // higher flag bit), so the cell type byte is the whole signal.
        CELL_TYPE_ERROR => CellValue::Error,
        CELL_TYPE_TEXT => body
            .get(..4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .and_then(|key| strings.get(&key).cloned())
            .map_or(CellValue::Empty, CellValue::Text),
        CELL_TYPE_RICH_TEXT => body
            .get(..4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .and_then(|key| rich_texts.get(&key).cloned())
            .map_or(CellValue::Empty, CellValue::Text),
        _ => CellValue::Empty,
    };

    if flags & FLAG_FORMULA_RESULT != 0 && !matches!(value, CellValue::Empty) {
        CellValue::Formula(Box::new(value))
    } else {
        value
    }
}

/// Reads a little-endian `f64` from the start of `body`, if 8 bytes are present.
fn read_f64(body: &[u8]) -> Option<f64> {
    body.get(..8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProtoMessage;

    #[test]
    fn decode_cell_record_decodes_each_type_byte() {
        let mut strings = HashMap::new();
        strings.insert(7, "Utilities".to_owned());
        let empty_rich: HashMap<u32, String> = HashMap::new();

        let number = decode_cell_record(&double_record(CELL_TYPE_NUMBER, 0x2, 42.5), &strings, &empty_rich, &HashMap::new());
        assert_eq!(number, CellValue::Number(42.5));

        let formula = decode_cell_record(&double_record(CELL_TYPE_NUMBER, 0x202, 42.5), &strings, &empty_rich, &HashMap::new());
        assert_eq!(formula.formula_result(), Some(&CellValue::Number(42.5)));
        assert_eq!(formula.as_number(), Some(42.5));

        let decimal =
            decode_cell_record(&decimal_record(CELL_TYPE_NUMBER, 1234), &strings, &empty_rich, &HashMap::new());
        assert_eq!(decimal, CellValue::Number(1234.0));

        let date_seconds = 625_881_600.0_f64;
        let date = decode_cell_record(&double_record(CELL_TYPE_DATE, 0x4, date_seconds), &strings, &empty_rich, &HashMap::new());
        assert_eq!(date, CellValue::Date(date_seconds));

        let bool_true = decode_cell_record(&double_record(CELL_TYPE_BOOL, 0x2, 1.0), &strings, &empty_rich, &HashMap::new());
        assert_eq!(bool_true, CellValue::Bool(true));
        let bool_false = decode_cell_record(&double_record(CELL_TYPE_BOOL, 0x2, 0.0), &strings, &empty_rich, &HashMap::new());
        assert_eq!(bool_false, CellValue::Bool(false));

        // 2h30m == 9000s, stored as an 8-byte double like a bool.
        let duration = decode_cell_record(&double_record(CELL_TYPE_DURATION, 0x2, 9000.0), &strings, &empty_rich, &HashMap::new());
        assert_eq!(duration, CellValue::Duration(9000.0));

        // Error cells carry no value field, just the type byte.
        let error = decode_cell_record(&double_record_bytes(CELL_TYPE_ERROR, 0x800, &[1, 0, 0, 0]), &strings, &empty_rich, &HashMap::new());
        assert_eq!(error, CellValue::Error);

        // Rich text (type 9) resolves through the 6218→2001 chain.
        let mut rich_map: HashMap<u32, String> = HashMap::new();
        rich_map.insert(1, "hello rich".to_owned());
        let rich = decode_cell_record(&text_record_typed(CELL_TYPE_RICH_TEXT, 1), &strings, &rich_map, &HashMap::new());
        assert_eq!(rich, CellValue::Text("hello rich".to_owned()));

        // Rich text key not in map → Empty.
        let rich_miss = decode_cell_record(&text_record_typed(CELL_TYPE_RICH_TEXT, 99), &strings, &rich_map, &HashMap::new());
        assert_eq!(rich_miss, CellValue::Empty);

        let text = decode_cell_record(&text_record(7), &strings, &empty_rich, &HashMap::new());
        assert_eq!(text, CellValue::Text("Utilities".to_owned()));
    }

    #[test]
    fn decode_cell_record_returns_empty_for_unknown_or_truncated_payloads() {
        let strings = HashMap::new();
        let rich: HashMap<u32, String> = HashMap::new();

        assert_eq!(decode_cell_record(&[], &strings, &rich, &HashMap::new()), CellValue::Empty);
        // Right version but an unrecognized type byte.
        assert_eq!(decode_cell_record(&[0x05; 12], &strings, &rich, &HashMap::new()), CellValue::Empty);
        // A number cell whose double payload is truncated.
        assert_eq!(
            decode_cell_record(&double_record_bytes(CELL_TYPE_NUMBER, 0x2, &[1, 2, 3]), &strings, &rich, &HashMap::new()),
            CellValue::Empty
        );
        // A text cell whose key is truncated.
        assert_eq!(
            decode_cell_record(&double_record_bytes(CELL_TYPE_TEXT, 0x8, &3u16.to_le_bytes()), &strings, &rich, &HashMap::new()),
            CellValue::Empty
        );
    }

    #[test]
    fn decode_cells_includes_all_columns_up_to_last_non_sentinel() {
        let mut strings = HashMap::new();
        strings.insert(1, "Groceries".to_owned());

        let first = double_record(CELL_TYPE_NUMBER, 0x2, 10.0);
        let second = text_record(1);
        let third = double_record(CELL_TYPE_DATE, 0x4, 1000.0);

        let mut storage = Vec::new();
        storage.extend_from_slice(&first);
        storage.extend_from_slice(&second);
        storage.extend_from_slice(&[0; 4]);
        storage.extend_from_slice(&third);

        // field(2) = 2 here, but the value cell is at index 2 — all three
        // columns must be decoded (leading-empty-column scenario from more_types).
        let empty_rich: HashMap<u32, String> = HashMap::new();
        let msg = row_message(Some(2), &storage, &[0, 20, 40]);
        assert_eq!(
            decode_cells(&msg, &strings, &empty_rich, &HashMap::new()),
            vec![
                CellValue::Number(10.0),
                CellValue::Text("Groceries".to_owned()),
                CellValue::Date(1000.0),
            ]
        );

        // A sentinel in the middle still yields an Empty slot; the valid cell
        // after it is included because it is the last non-sentinel.
        let msg2 = row_message(None, &storage, &[0, 20, 0xffff, 40]);
        assert_eq!(
            decode_cells(&msg2, &strings, &empty_rich, &HashMap::new()),
            vec![
                CellValue::Number(10.0),
                CellValue::Text("Groceries".to_owned()),
                CellValue::Empty,
                CellValue::Date(1000.0),
            ]
        );
    }

    /// Builds a wide-cell record of `cell_type` whose value is an 8-byte double.
    fn double_record(cell_type: u8, flags: u32, value: f64) -> Vec<u8> {
        double_record_bytes(cell_type, flags, &value.to_le_bytes())
    }

    /// Builds a wide-cell record with an arbitrary (possibly short) payload.
    fn double_record_bytes(cell_type: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
        let mut record = vec![0x05, cell_type, 0, 0, 0, 0, 0, 0];
        record.extend_from_slice(&flags.to_le_bytes());
        record.extend_from_slice(payload);
        record
    }

    /// Builds a number record carrying an integer-valued decimal128 (flag `0x1`).
    fn decimal_record(cell_type: u8, value: u64) -> Vec<u8> {
        let mut d128 = [0u8; 16];
        d128[..8].copy_from_slice(&value.to_le_bytes());
        d128[14] = 0x40; // low exponent bit
        d128[15] = 0x30; // high exponent bits -> bias 0x1820, i.e. 10^0
        double_record_bytes(cell_type, 0x1, &d128)
    }

    /// Builds a text record whose u32 key (flag `0x8`) is `key`.
    fn text_record(key: u32) -> Vec<u8> {
        text_record_typed(CELL_TYPE_TEXT, key)
    }

    /// Builds a record of `cell_type` carrying a u32 key (flag `0x8`).
    fn text_record_typed(cell_type: u8, key: u32) -> Vec<u8> {
        double_record_bytes(cell_type, 0x8, &key.to_le_bytes())
    }

    fn row_message(column_count: Option<u64>, storage: &[u8], offsets: &[u16]) -> ProtoMessage {
        let mut bytes = Vec::new();
        if let Some(count) = column_count {
            push_varint_field(&mut bytes, 2, count);
        }
        push_bytes_field(&mut bytes, 6, storage);

        let mut offset_bytes = Vec::with_capacity(offsets.len() * 2);
        for offset in offsets {
            offset_bytes.extend_from_slice(&offset.to_le_bytes());
        }
        push_bytes_field(&mut bytes, 7, &offset_bytes);

        ProtoMessage::decode(&bytes).expect("synthetic row protobuf should decode")
    }

    fn push_varint_field(out: &mut Vec<u8>, number: u32, value: u64) {
        push_varint(out, u64::from(number << 3));
        push_varint(out, value);
    }

    fn push_bytes_field(out: &mut Vec<u8>, number: u32, value: &[u8]) {
        push_varint(out, u64::from((number << 3) | 2));
        push_varint(out, value.len() as u64);
        out.extend_from_slice(value);
    }

    fn push_varint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }
}
