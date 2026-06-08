use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    Document, DocumentKind, Error, IwaArchive, Package, count_keywords, keynote, numbers, pages,
};

const PERSONAL_BUDGET_EXAMPLE: &str = "examples/numbers/personal_budget.numbers";
const MODERN_NOVEL_EXAMPLE: &str = "examples/pages/modern_novel.pages";
const BASIC_WHITE_EXAMPLE: &str = "examples/keynote/basic_white.key";

#[test]
fn counts_keyword_hits_case_insensitively() {
    let counts = count_keywords(b"Bold bold BOLD underline", &["bold", "underline"]);
    assert_eq!(counts["bold"], 3);
    assert_eq!(counts["underline"], 1);
}

#[test]
fn parses_a_fixture_archive() -> Result<(), Error> {
    let package = Package::open(PERSONAL_BUDGET_EXAMPLE)?;
    let properties = package.properties()?;

    assert_eq!(properties.file_format_version.as_deref(), Some("14.4.1"));
    assert_eq!(properties.is_multi_page, Some(true));
    assert!(
        package
            .entries()
            .iter()
            .any(|entry| entry.path == "Index/DocumentStylesheet.iwa")
    );

    Ok(())
}

#[test]
fn app_specific_entry_points_share_the_core_package_reader() -> Result<(), Error> {
    let iwork_doc = Document::open(PERSONAL_BUDGET_EXAMPLE)?;
    let numbers_doc = numbers::Document::open(PERSONAL_BUDGET_EXAMPLE)?;
    let pages_doc = pages::Document::open(MODERN_NOVEL_EXAMPLE)?;
    let keynote_doc = keynote::Document::open(BASIC_WHITE_EXAMPLE)?;

    assert_eq!(
        iwork_doc.inspect(PERSONAL_BUDGET_EXAMPLE)?.kind,
        DocumentKind::Numbers
    );
    assert_eq!(
        numbers_doc.inspect(PERSONAL_BUDGET_EXAMPLE)?.kind,
        DocumentKind::Numbers
    );
    assert_eq!(
        pages_doc.inspect(MODERN_NOVEL_EXAMPLE)?.kind,
        DocumentKind::Pages
    );
    assert_eq!(
        keynote_doc.inspect(BASIC_WHITE_EXAMPLE)?.kind,
        DocumentKind::Keynote
    );

    Ok(())
}

#[test]
fn app_specific_entry_points_reject_the_wrong_extension() {
    let numbers_error = numbers::Document::open(MODERN_NOVEL_EXAMPLE).unwrap_err();
    assert!(matches!(numbers_error, Error::UnsupportedDocumentType(_)));

    let pages_error = pages::Document::open(PERSONAL_BUDGET_EXAMPLE).unwrap_err();
    assert!(matches!(pages_error, Error::UnsupportedDocumentType(_)));

    let keynote_error = keynote::Document::open(MODERN_NOVEL_EXAMPLE).unwrap_err();
    assert!(matches!(keynote_error, Error::UnsupportedDocumentType(_)));
}

#[test]
fn generic_document_write_round_trips_all_fixture_formats() -> Result<(), Error> {
    for path in [
        PERSONAL_BUDGET_EXAMPLE,
        MODERN_NOVEL_EXAMPLE,
        BASIC_WHITE_EXAMPLE,
    ] {
        let document = Document::open(path)?;
        let output_path = unique_output_path(path);
        document.write(&output_path)?;

        let original = std::fs::read(path)?;
        let round_trip = std::fs::read(&output_path)?;
        assert_eq!(round_trip, original, "{path} should round-trip exactly");

        std::fs::remove_file(output_path)?;
    }

    Ok(())
}

#[test]
fn app_specific_document_writers_preserve_fixture_bytes() -> Result<(), Error> {
    let numbers_output = unique_output_path(PERSONAL_BUDGET_EXAMPLE);
    numbers::Document::open(PERSONAL_BUDGET_EXAMPLE)?.write(&numbers_output)?;
    assert_eq!(
        std::fs::read(&numbers_output)?,
        std::fs::read(PERSONAL_BUDGET_EXAMPLE)?
    );
    std::fs::remove_file(numbers_output)?;

    let pages_output = unique_output_path(MODERN_NOVEL_EXAMPLE);
    pages::Document::open(MODERN_NOVEL_EXAMPLE)?.write(&pages_output)?;
    assert_eq!(
        std::fs::read(&pages_output)?,
        std::fs::read(MODERN_NOVEL_EXAMPLE)?
    );
    std::fs::remove_file(pages_output)?;

    let keynote_output = unique_output_path(BASIC_WHITE_EXAMPLE);
    keynote::Document::open(BASIC_WHITE_EXAMPLE)?.write(&keynote_output)?;
    assert_eq!(
        std::fs::read(&keynote_output)?,
        std::fs::read(BASIC_WHITE_EXAMPLE)?
    );
    std::fs::remove_file(keynote_output)?;

    Ok(())
}

#[test]
fn iwa_archives_decode_snappy_chunks_and_headers() -> Result<(), Error> {
    let document = Document::open(PERSONAL_BUDGET_EXAMPLE)?;
    let archive = IwaArchive::decode(document.package().entry_bytes("Index/Document.iwa")?)?;

    assert_eq!(archive.chunks().len(), 1);
    assert!(!archive.body().is_empty());
    assert_eq!(archive.descriptor().root_object_id, Some(1));
    assert!(!archive.descriptor().object_references.is_empty());

    let first_message = archive.header().decode_message()?;
    assert_eq!(
        first_message
            .field(1)
            .and_then(|field| field.value.as_varint()),
        Some(1)
    );

    let nested_field = first_message
        .field(2)
        .ok_or(Error::InvalidIwa("missing nested archive info"))?;
    let nested = nested_field
        .value
        .as_message()?
        .ok_or(Error::InvalidIwa("expected nested archive info"))?;
    assert_eq!(
        nested.field(1).and_then(|field| field.value.as_varint()),
        Some(1)
    );

    Ok(())
}

#[test]
fn numbers_spreadsheet_exposes_core_archives() -> Result<(), Error> {
    let document = numbers::Document::open(PERSONAL_BUDGET_EXAMPLE)?;
    let spreadsheet = document.spreadsheet()?;

    assert!(spreadsheet.document().header().decode_message().is_ok());
    assert!(
        spreadsheet
            .document_metadata()
            .header()
            .decode_message()
            .is_ok()
    );
    assert!(spreadsheet.metadata().header().decode_message().is_ok());
    assert!(spreadsheet.stylesheet().chunks().len() > 1);
    assert!(!spreadsheet.stylesheet().body().is_empty());
    assert!(
        spreadsheet
            .stylesheet()
            .descriptor()
            .object_references
            .iter()
            .any(|reference| reference.object_id.is_some())
    );
    assert!(
        spreadsheet
            .table_archives()
            .iter()
            .any(|archive| archive.path().ends_with("Tile.iwa"))
    );

    let catalog = spreadsheet.stylesheet_catalog();
    assert!(catalog.referenced_object_ids.len() > 20);
    assert!(!catalog.records.is_empty());
    assert!(
        catalog
            .identifiers
            .iter()
            .any(|identifier| identifier.contains("character-style-hyperlink"))
    );
    assert!(
        catalog
            .font_names
            .iter()
            .any(|font| font.contains("HelveticaNeue-Bold"))
    );
    assert!(
        catalog
            .style_names
            .iter()
            .any(|name| name == "Bold" || name == "Italic")
    );
    assert!(
        catalog
            .records
            .iter()
            .any(|record| record.name.contains("character-style-hyperlink"))
    );
    assert!(catalog.attribute_hints.iter().any(|hint| {
        hint.font_name
            .as_deref()
            .is_some_and(|font| font.contains("HelveticaNeue"))
    }));
    assert!(catalog.attribute_hints.iter().any(|hint| {
        hint.font_size.is_some_and(|size| {
            (size.0 - 10.0).abs() < f32::EPSILON
                || (size.0 - 12.0).abs() < f32::EPSILON
                || (size.0 - 14.0).abs() < f32::EPSILON
                || (size.0 - 16.0).abs() < f32::EPSILON
        })
    }));

    Ok(())
}

#[test]
fn pages_document_model_exposes_core_archives() -> Result<(), Error> {
    let document = pages::Document::open(MODERN_NOVEL_EXAMPLE)?;
    let model = document.document_model()?;

    assert!(model.document().header().decode_message().is_ok());
    assert!(model.document_metadata().header().decode_message().is_ok());
    assert!(model.metadata().header().decode_message().is_ok());
    assert!(model.stylesheet().header().decode_message().is_ok());
    assert!(!model.index_archives().is_empty());
    assert!(!model.stylesheet_catalog().referenced_object_ids.is_empty());
    assert!(!model.stylesheet_catalog().font_names.is_empty());

    Ok(())
}

#[test]
fn keynote_presentation_exposes_core_archives() -> Result<(), Error> {
    let document = keynote::Document::open(BASIC_WHITE_EXAMPLE)?;
    let presentation = document.presentation()?;

    assert!(presentation.document().header().decode_message().is_ok());
    assert!(presentation.metadata().header().decode_message().is_ok());
    assert!(presentation.stylesheet().header().decode_message().is_ok());
    assert!(!presentation.index_archives().is_empty());
    assert!(
        !presentation
            .stylesheet_catalog()
            .referenced_object_ids
            .is_empty()
    );
    assert!(!presentation.stylesheet_catalog().font_names.is_empty());

    Ok(())
}

#[test]
fn stylesheet_payload_bold_and_italic_are_structural() -> Result<(), Error> {
    // term_paper.pages has Charter-Bold and Charter-Italic styles which carry field 1/2
    // explicitly in the payload — verify they surface as structural attributes, not heuristics.
    let package = Package::open("examples/pages/term_paper.pages")?;
    let bytes = package.entry_bytes("Index/DocumentStylesheet.iwa")?;
    let archive = IwaArchive::decode(bytes)?;
    let catalog = crate::stylesheet::StylesheetCatalog::from_archive(&archive);

    // At least some records should have bold=Some(true) from the payload (field 1 = 1),
    // not solely from name inference.
    let has_structural_bold = catalog
        .attribute_hints
        .iter()
        .any(|hint| hint.bold == Some(true));
    assert!(has_structural_bold, "expected at least one payload with field 1 = 1 (bold)");

    // At least some records should have italic=Some(true) from the payload (field 2 = 1).
    let has_structural_italic = catalog
        .attribute_hints
        .iter()
        .any(|hint| hint.italic == Some(true));
    assert!(has_structural_italic, "expected at least one payload with field 2 = 1 (italic)");

    Ok(())
}

#[test]
fn numbers_table_parses_text_cells() -> Result<(), Error> {
    let doc = numbers::Document::open(PERSONAL_BUDGET_EXAMPLE)?;
    let spreadsheet = doc.spreadsheet()?;
    let tables = spreadsheet.tables();

    let text_cells: Vec<_> = tables
        .iter()
        .flat_map(|t| t.rows())
        .flat_map(|r| r.cells.iter())
        .filter_map(|c| c.as_text())
        .collect();

    assert!(!text_cells.is_empty(), "expected at least one text cell");
    Ok(())
}

#[test]
fn numbers_table_parses_date_cells() -> Result<(), Error> {
    let doc = numbers::Document::open(PERSONAL_BUDGET_EXAMPLE)?;
    let spreadsheet = doc.spreadsheet()?;
    let tables = spreadsheet.tables();

    let date_cells: Vec<_> = tables
        .iter()
        .flat_map(|t| t.rows())
        .flat_map(|r| r.cells.iter())
        .filter_map(|c| c.as_date_seconds())
        .collect();

    assert!(!date_cells.is_empty(), "expected at least one date cell");
    assert!(
        date_cells.iter().all(|&s| s > 0.0),
        "date seconds should be positive (Cocoa epoch)",
    );
    Ok(())
}

#[test]
fn my_stocks_formula_cells_decode_to_numbers() -> Result<(), Error> {
    const MY_STOCKS: &str = "examples/numbers/my_stocks.numbers";
    let doc = numbers::Document::open(MY_STOCKS)?;
    let tables = doc.spreadsheet()?.tables();

    // Dump all tables and rows for debugging.
    for (ti, t) in tables.iter().enumerate() {
        for row in t.rows() {
            if row.index > 0 {
                println!("table[{ti}] row={}: {:?}", row.index, row.cells);
            }
        }
    }

    // Find the table with data rows (skip trivially-empty tiles).
    let data_table = tables.iter().find(|t| {
        t.rows().iter().any(|r| r.index > 0 && r.cells.iter().any(|c| c != &crate::numbers::CellValue::Empty))
    });
    let table = data_table.expect("should find a table with data");

    // Row 1 (first data row) should have at least one Number cell decoded from the
    // formula DataList. Before this change every formula cell returned Empty.
    let data_rows: Vec<_> = table.rows().iter().filter(|r| r.index > 0).collect();
    assert!(!data_rows.is_empty(), "should have data rows");
    let has_number = data_rows[0].cells.iter().any(|c| matches!(c, crate::numbers::CellValue::Number(_)));
    assert!(has_number, "first data row should contain at least one Number cell");

    Ok(())
}

#[test]
fn investigate_unknown_cell_types_in_my_stocks() -> Result<(), Error> {
    const MY_STOCKS: &str = "examples/numbers/my_stocks.numbers";

    let doc = numbers::Document::open(MY_STOCKS)?;
    let spreadsheet = doc.spreadsheet()?;

    // List all table archives (names + leading-object-ref count).
    println!("\n=== TABLE ARCHIVES ===");
    for archive in spreadsheet.table_archives() {
        let a = archive.archive();
        let refs = a.leading_object_references();
        println!("  {} body_len={} refs={refs:?}", archive.path(), a.body().len());
    }

    // For each Tile, print the row message's ALL fields (not just f4/f6/f7).
    println!("\n=== TILE ROW FIELDS (Tile-1139365 only, first 4 rows) ===");
    for archive in spreadsheet.table_archives() {
        if !archive.path().contains("Tile-1139365") {
            continue;
        }

        let tile = archive.archive();
        let body = tile.body();
        let mut cursor = tile.leading_object_references_len();
        let mut row_count = 0;

        while cursor < body.len() && row_count < 4 {
            let Ok(tag) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            let wire_type = tag & 0x07;
            if wire_type != 2 {
                match wire_type {
                    0 => { let _ = crate::protobuf::read_varint(body, &mut cursor); }
                    1 => { cursor = cursor.saturating_add(8); }
                    5 => { cursor = cursor.saturating_add(4); }
                    _ => break,
                }
                continue;
            }
            let Ok(lv) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            let Ok(len) = usize::try_from(lv) else { break };
            let Some(chunk) = body.get(cursor..cursor + len) else { break };
            cursor += len;

            let Ok(msg) = crate::protobuf::ProtoMessage::decode(chunk) else { continue };
            if msg.fields().is_empty() { continue; }

            let row_index = msg.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);
            println!("\n  row={row_index} fields present: {:?}", msg.fields().iter().map(|f| f.number).collect::<Vec<_>>());
            for field in msg.fields() {
                match &field.value {
                    crate::protobuf::ProtoValue::Varint(v) => println!("    field {} = varint {v}", field.number),
                    crate::protobuf::ProtoValue::LengthDelimited(b) => println!("    field {} = bytes[{}]", field.number, b.len()),
                    crate::protobuf::ProtoValue::Fixed32(v) => println!("    field {} = fixed32 0x{v:08x}", field.number),
                    crate::protobuf::ProtoValue::Fixed64(v) => println!("    field {} = fixed64 0x{v:016x}", field.number),
                }
            }
            row_count += 1;
        }
    }

    // Dump field 3 column metadata + full cell records per column for Tile-1139365.
    println!("\n=== TILE-1139365 CELL RECORDS PER COLUMN ===");
    for archive in spreadsheet.table_archives() {
        if !archive.path().contains("Tile-1139365") { continue; }
        let tile = archive.archive();
        let body = tile.body();
        let mut cursor = tile.leading_object_references_len();
        let mut row_count = 0;

        while cursor < body.len() && row_count < 5 {
            let Ok(tag) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            let wire_type = tag & 0x07;
            if wire_type != 2 {
                match wire_type {
                    0 => { let _ = crate::protobuf::read_varint(body, &mut cursor); }
                    1 => { cursor = cursor.saturating_add(8); }
                    5 => { cursor = cursor.saturating_add(4); }
                    _ => break,
                }
                continue;
            }
            let Ok(lv) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            let Ok(len) = usize::try_from(lv) else { break };
            let Some(chunk) = body.get(cursor..cursor + len) else { break };
            cursor += len;
            let Ok(msg) = crate::protobuf::ProtoMessage::decode(chunk) else { continue };
            if msg.fields().is_empty() { continue; }
            let row_index = msg.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);

            let f3 = msg.field(3).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
            let f4 = msg.field(4).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
            let f6 = msg.field(6).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
            let f7 = msg.field(7).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
            let ncols = f4.len() / 2;
            println!("\n  row={row_index} f3_len={} f6_len={} f7_len={}", f3.len(), f6.len(), f7.len());
            // Dump field 3 column metadata (12 bytes each)
            for col in 0..ncols {
                let off = f4.chunks_exact(2).nth(col).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0xffff);
                if off == 0xffff { continue; }
                let meta = f3.get(col * 12..(col + 1) * 12).unwrap_or(&[]);
                let cell = f6.get(off as usize..off as usize + 12).unwrap_or(&[]);
                let meta_hex: Vec<String> = meta.iter().map(|b| format!("{b:02x}")).collect();
                let cell_hex: Vec<String> = cell.iter().map(|b| format!("{b:02x}")).collect();
                println!("    col{col:02} off={off:3}  meta=[{}]  cell=[{}]",
                    meta_hex.join(" "), cell_hex.join(" "));
            }
            // Dump inline area start
            let inline_start = f7.chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .filter(|&v| v != 0xffff)
                .map(|v| v as usize)
                .max()
                .map_or(f6.len(), |last| last + 12);
            let inline_bytes = f6.get(inline_start..).unwrap_or(&[]);
            let inline_hex: Vec<String> = inline_bytes.iter().take(60).map(|b| format!("{b:02x}")).collect();
            println!("    inline_start={inline_start}  bytes=[{}]", inline_hex.join(" "));
            row_count += 1;
        }
    }

    // Print the object_references from the Tile-1139365 descriptor.
    println!("\n=== TILE-1139365 DESCRIPTOR OBJECT REFERENCES ===");
    for archive in spreadsheet.table_archives() {
        if !archive.path().contains("Tile-1139365") {
            continue;
        }
        let desc = archive.archive().descriptor();
        println!("  root_object_id={:?}", desc.root_object_id);
        for (i, r) in desc.object_references.iter().enumerate() {
            println!("  ref[{i}] object_id={:?} kind={:?}", r.object_id, r.kind_hint);
        }
    }

    // Scan raw Metadata.iwa body for varint sequences containing target IDs.
    // All 6 target IDs share bytes [0xC5, 0x45] as their varint suffix (3-byte varints).
    // First bytes: 1139359→0x9F, 1139365→0xA5, 1139370→0xAA, 1139377→0xB1, 1139386→0xBA, 1139393→0xC1.
    println!("\n=== METADATA.IWA RAW ID SEQUENCE SCAN ===");
    {
        let archive = spreadsheet.metadata();
        let body = archive.body();
        let first_bytes: std::collections::HashMap<u8, u64> = [
            (0x9Fu8, 1139359u64), (0xA5, 1139365), (0xAA, 1139370),
            (0xB1, 1139377), (0xBA, 1139386), (0xC1, 1139393),
        ].into_iter().collect();
        // Find all positions where a target ID varint appears.
        let mut id_positions: Vec<(usize, u64)> = Vec::new();
        for i in 0..body.len().saturating_sub(2) {
            if body[i+1] == 0xC5 && body[i+2] == 0x45 {
                if let Some(&id) = first_bytes.get(&body[i]) {
                    id_positions.push((i, id));
                }
            }
        }
        // Print IDs that appear within 20 bytes of another target ID.
        let mut printed = std::collections::HashSet::new();
        for (i, &(pos_a, id_a)) in id_positions.iter().enumerate() {
            for &(pos_b, id_b) in &id_positions[i+1..] {
                if pos_b - pos_a <= 30 && id_a != id_b {
                    let group_key = (pos_a / 100) * 100;
                    if !printed.contains(&group_key) {
                        printed.insert(group_key);
                        // Print a 60-byte window around this cluster.
                        let start = pos_a.saturating_sub(4);
                        let end = (pos_b + 3).min(body.len());
                        let hex: Vec<String> = body[start..end].iter().map(|b| format!("{b:02x}")).collect();
                        println!("  pos={pos_a}: [{id_a}] and [{id_b}] nearby: {}", hex.join(" "));
                    }
                    break;
                }
                if pos_b > pos_a + 30 { break; }
            }
        }
        // Print all positions regardless.
        println!("  All ID positions ({} total):", id_positions.len());
        for (pos, id) in &id_positions {
            let start = pos.saturating_sub(6);
            let end = (pos + 10).min(body.len());
            let hex: Vec<String> = body[start..end].iter().map(|b| format!("{b:02x}")).collect();
            println!("    pos={pos} id={id}: [{}]", hex.join(" "));
        }
        // Print 200-byte hex dump around second cluster to see full context.
        if body.len() > 25100 {
            let start = 24900usize;
            let end = (start + 400).min(body.len());
            println!("  Hex dump [{start}..{end}]:");
            for chunk_start in (start..end).step_by(16) {
                let chunk_end = (chunk_start + 16).min(end);
                let hex: Vec<String> = body[chunk_start..chunk_end].iter().map(|b| format!("{b:02x}")).collect();
                println!("    {chunk_start:5}: {}", hex.join(" "));
            }
        }
    }

    // Decode Metadata.iwa body to find messages grouping Tile IDs with DataList IDs.
    println!("\n=== METADATA.IWA MESSAGES WITH TARGET IDs ===");
    {
        let target_ids: std::collections::HashSet<u64> = [1139359u64, 1139365, 1139370, 1139377, 1139386, 1139393].into_iter().collect();
        let archive = spreadsheet.metadata();
        let body = archive.body();
        let mut cursor = archive.leading_object_references_len();
        let mut msg_count = 0u32;
        while cursor < body.len() {
            let Ok(tag) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            let wire_type = tag & 0x07;
            if wire_type != 2 {
                match wire_type {
                    0 => { let _ = crate::protobuf::read_varint(body, &mut cursor); }
                    1 => { cursor = cursor.saturating_add(8); }
                    5 => { cursor = cursor.saturating_add(4); }
                    _ => break,
                }
                continue;
            }
            let Ok(lv) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            let Ok(len) = usize::try_from(lv) else { break };
            let Some(chunk) = body.get(cursor..cursor + len) else { break };
            cursor += len;
            let Ok(msg) = crate::protobuf::ProtoMessage::decode(chunk) else { continue };
            // Collect all varint values in this message that are target IDs.
            let hits: Vec<u64> = msg.fields().iter()
                .filter_map(|f| f.value.as_varint())
                .filter(|v| target_ids.contains(v))
                .collect();
            if hits.len() >= 2 {
                println!("  msg#{msg_count}: multiple hits {hits:?}, fields: {:?}",
                    msg.fields().iter().map(|f| (f.number, match &f.value {
                        crate::protobuf::ProtoValue::Varint(v) => format!("v{v}"),
                        crate::protobuf::ProtoValue::LengthDelimited(b) => format!("b{}", b.len()),
                        crate::protobuf::ProtoValue::Fixed32(v) => format!("f32:{v:08x}"),
                        crate::protobuf::ProtoValue::Fixed64(v) => format!("f64:{v:016x}"),
                    })).collect::<Vec<_>>()
                );
            }
            msg_count += 1;
        }
    }

    // Scan Document.iwa, Metadata.iwa, DocumentMetadata.iwa for Tile/DataList cross-references.
    println!("\n=== DOCUMENT ARCHIVES SCANNING FOR IDs ===");
    let target_ids: std::collections::HashSet<u64> = [1139359u64, 1139365, 1139370, 1139377, 1139386, 1139393].into_iter().collect();
    let doc_archives = [
        ("document", spreadsheet.document()),
        ("metadata", spreadsheet.metadata()),
        ("document_metadata", spreadsheet.document_metadata()),
    ];
    for (name, archive) in &doc_archives {
        let mut found_ids: Vec<u64> = Vec::new();
        // Scan all varint values in the body for target IDs.
        let body = archive.body();
        let mut cursor = 0usize;
        while cursor < body.len() {
            let Ok(v) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            if target_ids.contains(&v) { found_ids.push(v); }
        }
        found_ids.sort();
        found_ids.dedup();
        if !found_ids.is_empty() {
            println!("  {name}: found target IDs {found_ids:?}");
        } else {
            println!("  {name}: no target IDs found");
        }
    }

    // Check leading object references of formula DataLists — should point to owning Tile.
    println!("\n=== FORMULA DATALIST LEADING REFS ===");
    for archive in spreadsheet.table_archives() {
        if !archive.path().contains("DataList") { continue; }
        let body = archive.archive().body();
        if body.len() <= 4 { continue; }
        let mut probe = 0usize;
        let Ok(t1) = crate::protobuf::read_varint(body, &mut probe) else { continue };
        if (t1 >> 3) != 1 || (t1 & 7) != 0 { continue; }
        let Ok(v1) = crate::protobuf::read_varint(body, &mut probe) else { continue };
        if v1 != 3 { continue; }
        let name = archive.path().trim_start_matches("Index/Tables/");
        let refs = archive.archive().leading_object_references();
        let desc_refs: Vec<_> = archive.archive().descriptor().object_references.iter()
            .filter_map(|r| r.object_id).collect();
        println!("  {name}: leading_refs={refs:?} desc_refs={desc_refs:?}");
    }

    // Scan ALL formula-type DataLists for key→f64 entries.
    // The f64 is at: entry.field5 → field1 (any occurrence) → field4 (fixed64).
    println!("\n=== FORMULA DATALIST KEY→F64 (all DataLists with type=3) ===");
    for archive in spreadsheet.table_archives() {
        if !archive.path().contains("DataList") { continue; }
        let body = archive.archive().body();
        if body.len() <= 4 { continue; }
        // Decode the first two fields of the DataList metadata message.
        let mut probe = 0usize;
        let Ok(t1) = crate::protobuf::read_varint(body, &mut probe) else { continue };
        if (t1 >> 3) != 1 || (t1 & 7) != 0 { continue; }  // must be field1 varint
        let Ok(v1) = crate::protobuf::read_varint(body, &mut probe) else { continue };
        let Ok(t2) = crate::protobuf::read_varint(body, &mut probe) else { continue };
        let v2 = if (t2 & 7) == 0 {
            crate::protobuf::read_varint(body, &mut probe).unwrap_or(0)
        } else { 0 };
        let name = archive.path().trim_start_matches("Index/Tables/");
        println!("  {name}: field1={v1} field2={v2}");
        if v1 != 3 { continue; }

        let name = archive.path().trim_start_matches("Index/Tables/");
        let mut entries: Vec<(u64, f64)> = Vec::new();

        let mut cursor = 0usize;
        while cursor < body.len() {
            let Ok(tag) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            let wire_type = tag & 0x07;
            let field_num = tag >> 3;
            if wire_type != 2 {
                match wire_type {
                    0 => { let _ = crate::protobuf::read_varint(body, &mut cursor); }
                    1 => { cursor = cursor.saturating_add(8); }
                    5 => { cursor = cursor.saturating_add(4); }
                    _ => break,
                }
                continue;
            }
            let Ok(lv) = crate::protobuf::read_varint(body, &mut cursor) else { break };
            let Ok(len) = usize::try_from(lv) else { break };
            let Some(chunk) = body.get(cursor..cursor + len) else { break };
            cursor += len;
            if field_num != 3 { continue; }

            let Ok(entry) = crate::protobuf::ProtoMessage::decode(chunk) else { continue };
            let key = entry.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);

            // Path: entry.field5 → field1(outer,single) → fields_by_number(1)(inner) → field4(fixed64).
            // For multi-sub-item entries, field5 may have multiple field1s at the outer level.
            let Some(f5_bytes) = entry.field(5).and_then(|f| f.value.as_bytes()) else { continue };
            let Ok(f5_msg) = crate::protobuf::ProtoMessage::decode(f5_bytes) else { continue };
            let find_fixed64 = |msg: &crate::protobuf::ProtoMessage| -> Option<f64> {
                msg.fields().iter().find_map(|f| {
                    if f.number == 4 {
                        if let crate::protobuf::ProtoValue::Fixed64(v) = f.value {
                            return Some(f64::from_bits(v));
                        }
                    }
                    None
                })
            };
            // Try: f5_msg.field1(outer).fields_by_number(1)(inner) → field4
            let val = f5_msg.fields_by_number(1).find_map(|outer_f1| {
                let outer_bytes = outer_f1.value.as_bytes()?;
                let outer_msg = crate::protobuf::ProtoMessage::decode(outer_bytes).ok()?;
                // Check direct field4 first, then recurse into inner field1s.
                find_fixed64(&outer_msg).or_else(|| {
                    outer_msg.fields_by_number(1).find_map(|inner_f1| {
                        let inner_bytes = inner_f1.value.as_bytes()?;
                        let inner_msg = crate::protobuf::ProtoMessage::decode(inner_bytes).ok()?;
                        find_fixed64(&inner_msg)
                    })
                })
            });
            if let Some(v) = val {
                entries.push((key, v));
            }
        }
        if !entries.is_empty() {
            println!("  {name}: {:?}", entries);
        }
    }

    Ok(())
}

#[test]
fn investigate_datalist_1139369() -> Result<(), Error> {
    use crate::protobuf::{ProtoMessage, ProtoValue, read_varint};
    const MY_STOCKS: &str = "examples/numbers/my_stocks.numbers";
    let doc = numbers::Document::open(MY_STOCKS)?;
    let spreadsheet = doc.spreadsheet()?;

    let describe = |v: &ProtoValue| -> String {
        match v {
            ProtoValue::Varint(x) => format!("varint {x}"),
            ProtoValue::Fixed32(x) => format!("f32 {} (0x{x:08x})", f32::from_bits(*x)),
            ProtoValue::Fixed64(x) => format!("f64 {} (0x{x:016x})", f64::from_bits(*x)),
            ProtoValue::LengthDelimited(b) => {
                let hex: Vec<String> = b.iter().take(24).map(|x| format!("{x:02x}")).collect();
                format!("bytes[{}] = {}", b.len(), hex.join(" "))
            }
        }
    };

    for target in ["DataList-1139369", "DataList-1139359"] {
        let Some(archive) = spreadsheet
            .table_archives()
            .iter()
            .find(|a| a.path().contains(target))
        else { continue };
        let body = archive.archive().body();
        println!("\n=== {target} (body_len={}) ===", body.len());

        // Top-level fields of the DataList message.
        let mut probe = 0usize;
        let t1 = read_varint(body, &mut probe).unwrap_or(0);
        let v1 = read_varint(body, &mut probe).unwrap_or(0);
        let t2 = read_varint(body, &mut probe).unwrap_or(0);
        let v2 = read_varint(body, &mut probe).unwrap_or(0);
        println!("  listType(field1)={v1} field2={v2}  (tags {t1:#x},{t2:#x})");

        // Walk field-3 entries and dump each entry's sub-fields.
        let mut cursor = 0usize;
        let mut shown = 0;
        while cursor < body.len() && shown < 8 {
            let Ok(tag) = read_varint(body, &mut cursor) else { break };
            let wire = tag & 7;
            let field = tag >> 3;
            if wire != 2 {
                match wire {
                    0 => { let _ = read_varint(body, &mut cursor); }
                    1 => cursor += 8,
                    5 => cursor += 4,
                    _ => break,
                }
                continue;
            }
            let Ok(lv) = read_varint(body, &mut cursor) else { break };
            let Ok(len) = usize::try_from(lv) else { break };
            let Some(chunk) = body.get(cursor..cursor + len) else { break };
            cursor += len;
            if field != 3 { continue; }
            let Ok(entry) = ProtoMessage::decode(chunk) else { continue };
            let key = entry.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);
            println!("  entry key={key} fields={:?}", entry.fields().iter().map(|f| f.number).collect::<Vec<_>>());
            for f in entry.fields() {
                println!("    f{} = {}", f.number, describe(&f.value));
                // Recurse one level into length-delimited sub-messages.
                if let ProtoValue::LengthDelimited(b) = &f.value {
                    if let Ok(sub) = ProtoMessage::decode(b) {
                        for sf in sub.fields() {
                            println!("        .f{} = {}", sf.number, describe(&sf.value));
                        }
                    }
                }
            }
            shown += 1;
        }
    }
    Ok(())
}

#[test]
fn investigate_tile_f6_gap() -> Result<(), Error> {
    use crate::protobuf::{ProtoMessage, read_varint};
    const MY_STOCKS: &str = "examples/numbers/my_stocks.numbers";
    let doc = numbers::Document::open(MY_STOCKS)?;
    let spreadsheet = doc.spreadsheet()?;

    // For tile 1139370 (the large time-series table) dump, for the first few data
    // rows, every 8-byte window in f6 that decodes to a "reasonable" finite f64.
    // A value column shows up as the SAME offset producing finite f64s every row.
    for tile_name in ["Tile-1139365", "Tile-1139370"] {
        let Some(archive) = spreadsheet.table_archives().iter().find(|a| a.path().contains(tile_name))
        else { continue };
        let tile = archive.archive();
        let body = tile.body();
        let mut cursor = tile.leading_object_references_len();
        let mut shown = 0;
        println!("\n=== {tile_name}: finite-f64 offsets in f6 per row ===");
        while cursor < body.len() && shown < 4 {
            let Ok(tag) = read_varint(body, &mut cursor) else { break };
            if tag & 7 != 2 {
                match tag & 7 { 0 => { let _ = read_varint(body, &mut cursor); }, 1 => cursor += 8, 5 => cursor += 4, _ => break }
                continue;
            }
            let Ok(lv) = read_varint(body, &mut cursor) else { break };
            let Ok(len) = usize::try_from(lv) else { break };
            let Some(chunk) = body.get(cursor..cursor + len) else { break };
            cursor += len;
            let Ok(msg) = ProtoMessage::decode(chunk) else { continue };
            if msg.fields().is_empty() { continue; }
            let row = msg.field(1).and_then(|f| f.value.as_varint()).unwrap_or(0);
            if row == 0 { continue; }
            let f6 = msg.field(6).and_then(|f| f.value.as_bytes()).unwrap_or(&[]);
            let mut hits = Vec::new();
            for off in 0..f6.len().saturating_sub(8) {
                let v = f64::from_le_bytes(f6[off..off + 8].try_into().unwrap_or([0; 8]));
                if v.is_finite() && v != 0.0 && v.abs() > 1e-6 && v.abs() < 1e12 {
                    hits.push((off, v));
                }
            }
            println!("  row={row} f6_len={} finite_f64_hits={}", f6.len(), hits.len());
            for (off, v) in hits.iter().take(12) {
                println!("    off={off:3} -> {v}");
            }
            shown += 1;
        }
    }
    Ok(())
}

fn unique_output_path(input_path: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let extension = Path::new(input_path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("tmp");
    std::env::temp_dir().join(format!("iwork-roundtrip-{timestamp}.{extension}"))
}
