use std::collections::HashMap;

use crate::iwa::IwaArchive;
use crate::protobuf::{ProtoMessage, ProtoValue, read_varint};
use crate::{Error, Package, StylesheetCatalog};
use super::table::{Table, decode_string_datalist};

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";
const DOCUMENT_METADATA_ENTRY: &str = "Index/DocumentMetadata.iwa";
const METADATA_ENTRY: &str = "Index/Metadata.iwa";
const STYLESHEET_ENTRY: &str = "Index/DocumentStylesheet.iwa";
const TABLE_PREFIX: &str = "Index/Tables/";

#[derive(Debug, Clone)]
pub struct Spreadsheet {
    document: IwaArchive,
    document_metadata: IwaArchive,
    metadata: IwaArchive,
    stylesheet: IwaArchive,
    table_archives: Vec<TableArchive>,
}

impl Spreadsheet {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let document = IwaArchive::decode(package.entry_bytes(DOCUMENT_ENTRY)?)?;
        let document_metadata = IwaArchive::decode(package.entry_bytes(DOCUMENT_METADATA_ENTRY)?)?;
        let metadata = IwaArchive::decode(package.entry_bytes(METADATA_ENTRY)?)?;
        let stylesheet = IwaArchive::decode(package.entry_bytes(STYLESHEET_ENTRY)?)?;

        let mut table_archives = package
            .entries()
            .iter()
            .filter(|entry| entry.path.starts_with(TABLE_PREFIX))
            .map(|entry| {
                Ok(TableArchive {
                    path: entry.path.clone(),
                    archive: IwaArchive::decode(package.entry_bytes(&entry.path)?)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        table_archives.sort_by(|left, right| left.path.cmp(&right.path));

        Ok(Self {
            document,
            document_metadata,
            metadata,
            stylesheet,
            table_archives,
        })
    }

    pub fn document(&self) -> &IwaArchive {
        &self.document
    }

    pub fn document_metadata(&self) -> &IwaArchive {
        &self.document_metadata
    }

    pub fn metadata(&self) -> &IwaArchive {
        &self.metadata
    }

    pub fn stylesheet(&self) -> &IwaArchive {
        &self.stylesheet
    }

    pub fn stylesheet_catalog(&self) -> StylesheetCatalog {
        StylesheetCatalog::from_archive(&self.stylesheet)
    }

    pub fn table_archives(&self) -> &[TableArchive] {
        &self.table_archives
    }

    pub fn tables(&self) -> Vec<Table> {
        let strings: HashMap<u32, String> = self
            .table_archives
            .iter()
            .filter(|a| a.path.contains("DataList"))
            .flat_map(|a| decode_string_datalist(&a.archive))
            .collect();

        // Build formula DataList maps (type=3), sorted by max_key ascending.
        let mut formula_lists: Vec<(u32, HashMap<u32, f64>)> = self
            .table_archives
            .iter()
            .filter(|a| a.path.contains("DataList"))
            .filter_map(|a| decode_formula_datalist(&a.archive))
            .collect();
        formula_lists.sort_by_key(|(max_key, _)| *max_key);

        self.table_archives
            .iter()
            .filter(|a| a.path.contains("Tile"))
            .map(|a| {
                let formula = match_formula_datalist(&a.archive, &formula_lists);
                Table::from_tile(&a.archive, &strings, formula)
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct TableArchive {
    path: String,
    archive: IwaArchive,
}

impl TableArchive {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn archive(&self) -> &IwaArchive {
        &self.archive
    }
}

/// Parse a formula DataList (type=3) archive and return (max_key, key→f64 map).
fn decode_formula_datalist(archive: &IwaArchive) -> Option<(u32, HashMap<u32, f64>)> {
    let body = archive.body();
    if body.len() < 4 {
        return None;
    }
    let mut probe = 0usize;
    let t1 = read_varint(body, &mut probe).ok()?;
    if (t1 >> 3) != 1 || (t1 & 7) != 0 {
        return None;
    }
    let v1 = read_varint(body, &mut probe).ok()?;
    if v1 != 3 {
        return None;
    }
    let t2 = read_varint(body, &mut probe).ok()?;
    let max_key = if (t2 & 7) == 0 {
        u32::try_from(read_varint(body, &mut probe).ok()?).ok()?
    } else {
        return None;
    };

    let mut map = HashMap::new();
    let mut cursor = 0usize;
    while cursor < body.len() {
        let Ok(tag) = read_varint(body, &mut cursor) else { break };
        let wire_type = tag & 0x07;
        let field_num = tag >> 3;
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
        if field_num != 3 {
            continue;
        }
        let Ok(entry) = ProtoMessage::decode(chunk) else { continue };
        let key_v = entry.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);
        let Ok(key) = u32::try_from(key_v) else { continue };
        let Some(f5_bytes) = entry.field(5).and_then(|f| f.value.as_bytes()) else { continue };
        let Ok(f5_msg) = ProtoMessage::decode(f5_bytes) else { continue };
        let val = f5_msg.fields_by_number(1).find_map(|outer| {
            let ob = outer.value.as_bytes()?;
            let om = ProtoMessage::decode(ob).ok()?;
            find_fixed64_f64(&om).or_else(|| {
                om.fields_by_number(1).find_map(|inner| {
                    let ib = inner.value.as_bytes()?;
                    let im = ProtoMessage::decode(ib).ok()?;
                    find_fixed64_f64(&im)
                })
            })
        });
        if let Some(v) = val {
            map.insert(key, v);
        }
    }
    #[cfg(test)]
    eprintln!("decode_formula_datalist: max_key={max_key} entries={}", map.len());
    Some((max_key, map))
}

fn find_fixed64_f64(msg: &ProtoMessage) -> Option<f64> {
    msg.fields().iter().find_map(|f| {
        if f.number == 4 {
            if let ProtoValue::Fixed64(v) = f.value {
                return Some(f64::from_bits(v));
            }
        }
        None
    })
}

/// Scan a Tile's cell records to find the maximum bytes8-11 value that varies
/// across rows within the same column (formula result DataList key).
///
/// `upper_bound` is the largest key present in any formula `DataList`; non-formula
/// columns store unrelated values in bytes 8-11 (e.g. style/format indices that
/// can read as large numbers), so any candidate above `upper_bound` cannot be a
/// formula key and is rejected.
fn scan_max_formula_key(tile: &IwaArchive, upper_bound: u32) -> u32 {
    let body = tile.body();
    let mut cursor = tile.leading_object_references_len();
    let mut col_first: HashMap<usize, u32> = HashMap::new();
    let mut max_varying: u32 = 0;

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
        let Ok(msg) = ProtoMessage::decode(chunk) else { continue };
        if msg.fields().is_empty() {
            continue;
        }
        let f4 = msg.field(4).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
        let f6 = msg.field(6).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);

        for (col_idx, b) in f4.chunks_exact(2).enumerate() {
            let off = u16::from_le_bytes([b[0], b[1]]) as usize;
            if off == 0xffff {
                continue;
            }
            let Some(rec) = f6.get(off..off + 12) else { continue };
            let key = u32::from_le_bytes([rec[8], rec[9], rec[10], rec[11]]);
            if key == 0 || key > upper_bound {
                continue;
            }
            match col_first.get(&col_idx) {
                None => {
                    col_first.insert(col_idx, key);
                }
                Some(&first) if first != key => {
                    max_varying = max_varying.max(key).max(first);
                }
                _ => {}
            }
        }
    }
    #[cfg(test)]
    eprintln!("scan_max_formula_key: max_varying={max_varying}");
    max_varying
}

/// Return the formula DataList map that covers this Tile's formula keys.
/// `formula_lists` must be sorted by max_key ascending.
fn match_formula_datalist<'a>(
    tile: &IwaArchive,
    formula_lists: &'a [(u32, HashMap<u32, f64>)],
) -> &'a HashMap<u32, f64> {
    static EMPTY: std::sync::OnceLock<HashMap<u32, f64>> = std::sync::OnceLock::new();
    let empty = EMPTY.get_or_init(HashMap::new);
    // `formula_lists` is sorted by max_key ascending, so the last entry's max_key
    // is the largest formula key in the whole document — the upper bound on what
    // a Tile's cell records can legitimately reference.
    let Some((upper_bound, _)) = formula_lists.last() else {
        return empty;
    };
    let max_key = scan_max_formula_key(tile, *upper_bound);
    if max_key == 0 {
        return empty;
    }
    formula_lists
        .iter()
        .find(|(mk, _)| *mk >= max_key)
        .map_or(empty, |(_, map)| map)
}
