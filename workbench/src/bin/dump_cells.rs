//! Raw wide-cell record dumper for Numbers tiles.
//!
//! The cell-storage buffer (a tile row's protobuf field 6) is an opaque byte
//! blob that `protorev` cannot see into, so this example decodes it directly to
//! test the wide-cell layout hypothesis: that the **type byte** at `rec[1]`
//! selects the cell's value kind, while the u32 **flags** at `rec[8..12]` drive
//! an ordered walk of trailing optional fields (value, then format/style/formula
//! references). For each non-empty cell it prints `type` and `flags` plus the
//! leading payload bytes so the type-byte ↔ flags correlation is visible, then a
//! `type byte -> flag masks` summary.
//!
//! Protobuf wire decoding is delegated to `ProtoMessage`; only the opaque
//! field-6 record bytes are interpreted by hand.
//!
//! Usage: `cargo run --example dump_cells -- <file.numbers> [--limit N]`

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use iwork::numbers::{self, CellValue};
use iwork::iwa::IwaArchive;
use iwork::protobuf::ProtoMessage;

fn main() -> Result<(), iwork::Error> {
    let mut path = "examples/numbers/personal_budget.numbers".to_owned();
    let mut limit = usize::MAX;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--limit" => limit = args.next().and_then(|v| v.parse().ok()).unwrap_or(limit),
            other => other.clone_into(&mut path),
        }
    }

    let document = numbers::Document::open(&path)?;
    let spreadsheet = document.spreadsheet()?;

    // type byte -> distinct flag masks seen, with a count each.
    let mut type_to_flags: BTreeMap<u8, BTreeMap<u32, usize>> = BTreeMap::new();
    let mut printed = 0usize;

    for model in spreadsheet.table_models() {
        println!(
            "\n== table {:?} ({}x{}) tiles={} ==",
            model.name(),
            model.row_count(),
            model.column_count(),
            model.tile_ids().len()
        );

        // Resolve the decoded first-column text per row so each dumped record can
        // be labelled (the fixtures use column 0 as a row label).
        let decoded = spreadsheet.table(&model);
        let labels: HashMap<u64, String> = decoded
            .rows()
            .iter()
            .filter_map(|row| {
                row.cells
                    .iter()
                    .find_map(CellValue::as_text)
                    .map(|text| (row.index, text.to_owned()))
            })
            .collect();

        for tile_id in model.tile_ids() {
            let Some(archive) = spreadsheet
                .table_archives()
                .iter()
                .map(numbers::TableArchive::archive)
                .find(|a| a.descriptor().root_object_id == Some(*tile_id))
            else {
                continue;
            };
            for (row_index, storage, offsets) in tile_rows(archive) {
                for (col, &off) in offsets.iter().enumerate() {
                    let Some(rec) = (off != 0xffff).then(|| storage.get(off as usize..)).flatten()
                    else {
                        continue;
                    };
                    if rec.len() < 12 {
                        continue;
                    }
                    let (version, type_byte) = (rec[0], rec[1]);
                    let flags = u32::from_le_bytes([rec[8], rec[9], rec[10], rec[11]]);
                    *type_to_flags
                        .entry(type_byte)
                        .or_default()
                        .entry(flags)
                        .or_default() += 1;
                    if printed < limit {
                        let tail = &rec[12..rec.len().min(12 + 24)];
                        let label = labels.get(&row_index).map_or("", String::as_str);
                        println!(
                            "  r{row_index} c{col} [{label}]: ver={version:#04x} \
                             type={type_byte:2} flags={flags:#010x} tail={}",
                            hex(tail)
                        );
                        printed += 1;
                    }
                }
            }
        }
    }

    println!("\n== type byte -> flag masks (count) ==");
    for (type_byte, masks) in &type_to_flags {
        print!("  type {type_byte:2}:");
        for (mask, count) in masks {
            print!(" {mask:#010x}(x{count})");
        }
        println!();
    }

    Ok(())
}

/// Decodes a tile body and yields `(row_index, storage, offsets)` per row, where
/// `storage` is the field-6 cell buffer and `offsets` the field-7 u16 offset array.
fn tile_rows(archive: &IwaArchive) -> Vec<(u64, Vec<u8>, Vec<u16>)> {
    let Ok(body) = ProtoMessage::decode(archive.body()) else {
        return Vec::new();
    };
    body.fields_by_number(5)
        .filter_map(|field| {
            let row = field.value.as_message().ok().flatten()?;
            let row_index = row.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);
            let storage = row
                .field(6)
                .and_then(|f| f.value.as_bytes())
                .unwrap_or_default()
                .to_vec();
            let offsets = row
                .field(7)
                .and_then(|f| f.value.as_bytes())
                .unwrap_or_default()
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            Some((row_index, storage, offsets))
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x} ");
        out
    })
}
