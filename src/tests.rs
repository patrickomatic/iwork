use crate::{
    Document, DocumentKind, Error, IwaArchive, Package, PackageSupport, PackageWriter, ProtoField,
    ProtoMessage, count_keywords, keynote, numbers, pages,
};

const PERSONAL_BUDGET_EXAMPLE: &str = "examples/numbers/personal_budget.numbers";
const ATTENDANCE_EXAMPLE: &str = "examples/numbers/attendance.numbers";
const MODERN_NOVEL_EXAMPLE: &str = "examples/pages/modern_novel.pages";
const BASIC_WHITE_EXAMPLE: &str = "examples/keynote/basic_white.key";
const DEFLATED_ZIP_ENTRY: &[u8] = &[
    0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00, 0x08, 0x00, 0x86, 0x90, 0xc8, 0x5c, 0x5a, 0x15,
    0xfa, 0x42, 0x26, 0x00, 0x00, 0x00, 0xf0, 0x0a, 0x00, 0x00, 0x08, 0x00, 0x1c, 0x00, 0x66, 0x69,
    0x6c, 0x65, 0x2e, 0x74, 0x78, 0x74, 0x55, 0x54, 0x09, 0x00, 0x03, 0x5c, 0x3c, 0x27, 0x6a, 0x5c,
    0x3c, 0x27, 0x6a, 0x75, 0x78, 0x0b, 0x00, 0x01, 0x04, 0xf5, 0x01, 0x00, 0x00, 0x04, 0x00, 0x00,
    0x00, 0x00, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x48, 0x49, 0x4d, 0xcb, 0x49, 0x2c, 0x49, 0xe5,
    0xca, 0x18, 0xe5, 0x8d, 0xf2, 0x46, 0x79, 0xa3, 0xbc, 0x51, 0xde, 0x28, 0x6f, 0x94, 0x37, 0xca,
    0x1b, 0xe5, 0x8d, 0xf2, 0x86, 0x15, 0x0f, 0x00, 0x50, 0x4b, 0x01, 0x02, 0x1e, 0x03, 0x14, 0x00,
    0x00, 0x00, 0x08, 0x00, 0x86, 0x90, 0xc8, 0x5c, 0x5a, 0x15, 0xfa, 0x42, 0x26, 0x00, 0x00, 0x00,
    0xf0, 0x0a, 0x00, 0x00, 0x08, 0x00, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0xa4, 0x81, 0x00, 0x00, 0x00, 0x00, 0x66, 0x69, 0x6c, 0x65, 0x2e, 0x74, 0x78, 0x74, 0x55, 0x54,
    0x05, 0x00, 0x03, 0x5c, 0x3c, 0x27, 0x6a, 0x75, 0x78, 0x0b, 0x00, 0x01, 0x04, 0xf5, 0x01, 0x00,
    0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x01, 0x00, 0x4e, 0x00, 0x00, 0x00, 0x68, 0x00, 0x00, 0x00, 0x00, 0x00,
];

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

    assert_eq!(
        package.support(),
        PackageSupport::SupportedDirectIndexEntries
    );
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
fn reads_deflated_zip_entries() -> Result<(), Error> {
    let package = Package::from_bytes(DEFLATED_ZIP_ENTRY.to_vec())?;
    let entry = package.entry_bytes("file.txt")?;

    assert_eq!(entry.len(), 2_800);
    assert!(entry.starts_with(b"hello deflate\nhello deflate\n"));
    assert!(entry.ends_with(b"hello deflate\n"));

    Ok(())
}

#[test]
fn package_writer_round_trips_stored_entries() -> Result<(), Error> {
    let mut writer = PackageWriter::new();
    writer
        .add_entry("Metadata/DocumentIdentifier", b"doc-123".to_vec())
        .add_entry("Index/Metadata.iwa", b"bytes".to_vec());

    let package = Package::from_bytes(writer.finish()?)?;
    assert_eq!(
        package.entry_bytes("Metadata/DocumentIdentifier")?,
        b"doc-123"
    );
    assert_eq!(package.entry_bytes("Index/Metadata.iwa")?, b"bytes");

    Ok(())
}

#[test]
fn iwa_encoder_reproduces_every_fixture_archive() -> Result<(), Error> {
    let document = Document::open(PERSONAL_BUDGET_EXAMPLE)?;
    let package = document.package();

    let mut checked = 0;
    for entry in package.entries() {
        if !entry.path.ends_with(".iwa") {
            continue;
        }

        let original = IwaArchive::decode(package.entry_bytes(&entry.path)?)?;
        let reencoded = IwaArchive::decode(&original.reencode()?)?;

        assert_eq!(
            reencoded.header().bytes(),
            original.header().bytes(),
            "header packet differs for {}",
            entry.path
        );
        assert_eq!(
            reencoded.body(),
            original.body(),
            "body differs for {}",
            entry.path
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "expected at least one .iwa archive in the fixture"
    );
    Ok(())
}

#[test]
fn protobuf_messages_round_trip_through_encoder() -> Result<(), Error> {
    let nested = ProtoMessage::new(vec![ProtoField::varint(1, 99)]);
    let message = ProtoMessage::new(vec![
        ProtoField::varint(1, 7),
        ProtoField::fixed32(2, 1234),
        ProtoField::fixed64(3, 5678),
        ProtoField::string(4, "hello"),
        ProtoField::message(5, &nested)?,
    ]);

    let decoded = ProtoMessage::decode(&message.encode()?)?;
    assert_eq!(
        decoded.field(1).and_then(|field| field.value.as_varint()),
        Some(7)
    );
    assert_eq!(
        decoded.field(4).and_then(|field| field.value.as_bytes()),
        Some(&b"hello"[..])
    );
    assert_eq!(
        decoded
            .field(5)
            .and_then(|field| field.value.as_message().ok())
            .flatten()
            .and_then(|nested| nested.field(1).and_then(|field| field.value.as_varint())),
        Some(99)
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
        iwork_doc.inspect(PERSONAL_BUDGET_EXAMPLE)?.support,
        PackageSupport::SupportedDirectIndexEntries
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
fn iwa_archive_decodes_full_object_stream() -> Result<(), Error> {
    let document = Document::open(PERSONAL_BUDGET_EXAMPLE)?;
    let package = document.package();

    // Index/Document.iwa is a composite archive: a Document root followed by
    // sheet/table objects, each framed by its own ArchiveInfo.
    let composite = IwaArchive::decode(package.entry_bytes("Index/Document.iwa")?)?;
    let objects = composite.objects();
    assert!(
        objects.len() > 1,
        "composite archive should expose more than its root object"
    );

    let root = &objects[0];
    assert_eq!(root.identifier, Some(1));
    assert_eq!(root.message_type, Some(1));
    assert_eq!(
        root.message_type.and_then(numbers::message_type_name),
        Some("Document")
    );
    // The root payload is the first `body_hint` bytes of the archive body.
    assert_eq!(
        Some(root.payload.len() as u64),
        composite.descriptor().body_hint
    );

    // Every object carries a positive identifier and a known-or-unknown type;
    // the trailing objects must decode without truncating the stream.
    assert!(objects.iter().all(|object| object.identifier.is_some()));

    // A leaf Tile archive is a single object whose payload is the whole body.
    let tile = IwaArchive::decode(package.entry_bytes("Index/Tables/Tile.iwa")?)?;
    let tile_objects = tile.objects();
    assert_eq!(tile_objects.len(), 1);
    assert_eq!(tile_objects[0].message_type, Some(6002));
    assert_eq!(tile_objects[0].payload, tile.body());

    // CalculationEngine.iwa contains multi-message objects (ArchiveInfo with a
    // repeated message_infos). The walk must skip the sum of every message
    // length, otherwise it desynchronizes and drops later objects — including
    // the second TableModel (type 6001) that personal_budget stores there.
    let calc = IwaArchive::decode(package.entry_bytes("Index/CalculationEngine.iwa")?)?;
    let table_models = calc
        .objects()
        .iter()
        .filter(|object| object.message_type == Some(6001))
        .count();
    assert_eq!(table_models, 2, "both TableModels must survive the walk");

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
fn pages_document_decodes_structural_utf8_fields() -> Result<(), Error> {
    let modern = pages::Document::open(MODERN_NOVEL_EXAMPLE)?.document()?;
    assert_eq!(modern.title(), None);
    assert!(modern.headings().is_empty());
    assert!(!modern.text_fragments().is_empty());

    let term_paper = pages::Document::open("examples/pages/term_paper.pages")?.document()?;
    assert_eq!(term_paper.title(), None);
    assert!(term_paper.headings().is_empty());
    assert!(!term_paper.text_fragments().is_empty());

    Ok(())
}

#[test]
fn keynote_presentation_decodes_structural_utf8_fields() -> Result<(), Error> {
    let basic = keynote::Document::open(BASIC_WHITE_EXAMPLE)?.presentation()?;
    assert!(basic.slides().iter().any(|slide| slide.is_template()));
    assert!(basic.slides().iter().any(|slide| slide.title().is_none()));
    assert!(
        basic
            .slides()
            .iter()
            .any(|slide| !slide.text_fragments().is_empty())
    );

    let blueprint = keynote::Document::open("examples/keynote/blueprint.key")?.presentation()?;
    assert!(
        blueprint
            .slides()
            .iter()
            .any(|slide| !slide.text_fragments().is_empty())
    );
    assert!(
        blueprint
            .slides()
            .iter()
            .any(|slide| !slide.media_descriptions().is_empty()),
        "blueprint.key should have slides with media descriptions"
    );

    let parchment = keynote::Document::open("examples/keynote/parchment.key")?.presentation()?;
    assert!(
        parchment
            .slides()
            .iter()
            .any(|slide| !slide.text_fragments().is_empty())
    );

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
    assert!(
        has_structural_bold,
        "expected at least one payload with field 1 = 1 (bold)"
    );

    // At least some records should have italic=Some(true) from the payload (field 2 = 1).
    let has_structural_italic = catalog
        .attribute_hints
        .iter()
        .any(|hint| hint.italic == Some(true));
    assert!(
        has_structural_italic,
        "expected at least one payload with field 2 = 1 (italic)"
    );

    // Records should be driven by the type-401 name registry, not raw byte heuristics.
    // personal_budget.numbers has 466 named styles; each should have a name.
    let budget_package = Package::open(PERSONAL_BUDGET_EXAMPLE)?;
    let budget_bytes = budget_package.entry_bytes("Index/DocumentStylesheet.iwa")?;
    let budget_archive = IwaArchive::decode(budget_bytes)?;
    let budget_catalog = crate::stylesheet::StylesheetCatalog::from_archive(&budget_archive);
    assert!(
        budget_catalog.records.len() >= 400,
        "expected ≥400 named style records, got {}",
        budget_catalog.records.len()
    );
    // All records must have a non-empty name.
    assert!(
        budget_catalog.records.iter().all(|r| !r.name.is_empty()),
        "every record should have a name from the type-401 registry"
    );
    // Several records should carry font sizes decoded from field 11.3.
    let with_font_size = budget_catalog
        .records
        .iter()
        .filter(|r| r.attributes.font_size.is_some())
        .count();
    assert!(
        with_font_size >= 100,
        "expected ≥100 records with a decoded font size, got {with_font_size}"
    );
    // Several records should carry colors decoded from field 11.7.
    let with_color = budget_catalog
        .records
        .iter()
        .filter(|r| r.attributes.color.is_some())
        .count();
    assert!(
        with_color >= 50,
        "expected ≥50 records with a decoded text color, got {with_color}"
    );

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
fn numbers_table_parses_mixed_scalar_cells() -> Result<(), Error> {
    const MY_STOCKS: &str = "examples/numbers/my_stocks.numbers";
    let doc = numbers::Document::open(MY_STOCKS)?;
    let tables = doc.spreadsheet()?.tables();

    let has_text_and_number_row = tables.iter().flat_map(|t| t.rows()).any(|r| {
        r.cells.iter().any(|c| c.as_text().is_some())
            && r.cells
                .iter()
                .filter_map(|c| c.as_number())
                .any(f64::is_finite)
    });
    assert!(
        has_text_and_number_row,
        "expected at least one row with text and numeric cells"
    );

    let has_date = tables
        .iter()
        .flat_map(|t| t.rows())
        .any(|r| r.cells.iter().any(|c| c.as_date_seconds().is_some()));
    assert!(has_date, "time-series table should contain Date cells");

    Ok(())
}

#[test]
fn more_types_decodes_bool_duration_and_error_cells() -> Result<(), Error> {
    use crate::numbers::CellValue;

    const MORE_TYPES: &str = "examples/numbers/more_types.numbers";
    let decoded = numbers::Document::open(MORE_TYPES)?
        .spreadsheet()?
        .decoded_tables();
    let cells: Vec<&CellValue> = decoded
        .iter()
        .flat_map(|(_, table)| table.rows())
        .flat_map(|row| row.cells.iter())
        .collect();

    // Boolean checkbox cells (type 6) — previously mis-decoded as numbers.
    assert!(
        cells.iter().any(|c| c.as_bool().is_some()),
        "expected a checkbox / boolean cell"
    );
    // Error cell (type 8), e.g. =1/0.
    assert!(
        cells.iter().any(|c| matches!(c, CellValue::Error)),
        "expected a formula-error cell"
    );
    // Duration cell (type 7): 2h30m is stored as 9000 seconds.
    let duration = cells.iter().find_map(|c| c.as_duration_seconds());
    assert!(
        duration.is_some_and(|seconds| (seconds - 9000.0).abs() < 1.0),
        "2h30m should decode to ~9000s, got {duration:?}"
    );

    // Rich-text cell (type 9): resolved through DataList → 6218 → 2001 chain.
    let has_rich = cells
        .iter()
        .any(|c| matches!(c, CellValue::Text(s) if s.contains("rich text")));
    assert!(has_rich, "expected a rich-text cell containing 'rich text'");

    // Formula result cells retain their formula marker while preserving cached
    // result access through the normal typed accessors.
    let formula_results: Vec<&CellValue> = cells
        .iter()
        .filter_map(|cell| cell.formula_result())
        .collect();
    assert!(
        formula_results.len() >= 2,
        "expected cached formula result cells, got {formula_results:?}"
    );
    assert!(
        formula_results.iter().any(|cell| cell.as_number().is_some()),
        "expected a formula cell with a cached numeric result"
    );
    assert!(
        formula_results.iter().any(|cell| cell.as_text().is_some()),
        "expected a formula cell with a cached text result"
    );

    // Currency cell: decoded as CellValue::Currency with "USD" code.
    let currency = cells.iter().find_map(|c| {
        if let CellValue::Currency { value, code } = c {
            Some((*value, code.clone()))
        } else {
            None
        }
    });
    let (cur_val, cur_code) = currency.expect("expected a currency cell");
    assert!(
        cur_val.is_finite(),
        "currency value should be a finite number, got {cur_val}"
    );
    assert_eq!(cur_code.as_deref(), Some("USD"), "expected USD currency code");

    // Percentage cell: decoded as CellValue::Percentage with decimal fraction.
    let pct = cells.iter().find_map(|c| c.as_percentage());
    assert!(
        pct.is_some_and(|p| (p - 0.33).abs() < 0.01),
        "expected a percentage cell ~0.33, got {pct:?}"
    );

    Ok(())
}

#[test]
fn personal_budget_preserves_multi_text_rows() -> Result<(), Error> {
    let tables = numbers::Document::open(PERSONAL_BUDGET_EXAMPLE)?
        .spreadsheet()?
        .tables();

    let has_multi_text_row = tables
        .iter()
        .flat_map(|table| table.rows())
        .any(|row| row.cells.iter().filter_map(|cell| cell.as_text()).count() >= 3);
    assert!(
        has_multi_text_row,
        "expected a row with multiple text cells"
    );

    Ok(())
}

#[test]
fn pivot_table_preserves_grouped_text_rows() -> Result<(), Error> {
    const PIVOT_TABLE: &str = "examples/numbers/pivot_table.numbers";
    let tables = numbers::Document::open(PIVOT_TABLE)?
        .spreadsheet()?
        .tables();

    let text_row_count = tables
        .iter()
        .flat_map(|table| table.rows())
        .filter(|row| row.cells.iter().filter_map(|cell| cell.as_text()).count() >= 2)
        .count();
    assert!(
        text_row_count >= 2,
        "expected multiple rows with grouped text cells"
    );

    Ok(())
}

#[test]
fn decoded_tables_link_cells_to_models_and_scope_strings() -> Result<(), Error> {
    let spreadsheet = numbers::Document::open(PERSONAL_BUDGET_EXAMPLE)?.spreadsheet()?;
    let decoded = spreadsheet.decoded_tables();

    // Each model-driven table decodes exactly as many rows as the model declares.
    for (model, table) in &decoded {
        let rows = u32::try_from(table.rows().len()).unwrap_or(0);
        assert_eq!(
            rows,
            model.row_count(),
            "table {:?} decoded {rows} rows but the model declares {}",
            model.name(),
            model.row_count(),
        );
    }

    // String cells resolve through each table's own DataList, so the two tables
    // carry their own distinct text rather than a collided global pool.
    let text_of = |name: &str| -> Vec<String> {
        decoded
            .iter()
            .find(|(model, _)| model.name() == Some(name))
            .map(|(_, table)| {
                table
                    .rows()
                    .iter()
                    .flat_map(|row| &row.cells)
                    .filter_map(|cell| cell.as_text().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };

    let transactions = text_of("Transactions");
    let summary = text_of("Summary by Category");
    assert!(
        transactions.iter().any(|text| text == "Groceries"),
        "Transactions should contain its own category cell"
    );
    assert!(
        summary.iter().any(|text| text == "Budget"),
        "Summary should contain its own header cell"
    );
    assert!(
        !summary.iter().any(|text| text == "Groceries"),
        "Summary must not borrow the Transactions string pool"
    );

    Ok(())
}

#[test]
fn multi_tile_table_merges_rows_with_absolute_indices() -> Result<(), Error> {
    // The attendance table is tall enough to span more than one 256-row tile,
    // exercising the merge that single-tile fixtures cannot.
    let spreadsheet = numbers::Document::open(ATTENDANCE_EXAMPLE)?.spreadsheet()?;
    let (model, table) = spreadsheet
        .decoded_tables()
        .into_iter()
        .find(|(model, _)| model.name() == Some("Attendance"))
        .ok_or(Error::InvalidIwa("missing Attendance table"))?;

    assert!(
        model.tile_ids().len() >= 2,
        "fixture must span multiple tiles to test the merge (tiles: {})",
        model.tile_ids().len()
    );
    assert!(
        model.row_count() > 256,
        "fixture must exceed one tile's worth of rows"
    );

    let indices: Vec<u64> = table.rows().iter().map(|row| row.index).collect();
    assert_eq!(
        u32::try_from(indices.len()).unwrap_or(0),
        model.row_count(),
        "every declared row should decode"
    );
    // Rows must be absolute and contiguous across tile boundaries — the old bug
    // reset each tile's indices to 0, so 256 would be missing and 0 would repeat.
    for (position, index) in indices.iter().enumerate() {
        let expected = u64::try_from(position).unwrap_or(u64::MAX);
        assert_eq!(*index, expected, "row {position} index reset (per-tile bug)");
    }

    Ok(())
}

#[test]
fn numbers_table_models_expose_names_and_geometry() -> Result<(), Error> {
    let models = numbers::Document::open("examples/numbers/my_stocks.numbers")?
        .spreadsheet()?
        .table_models();

    // my_stocks has three named tables with distinct geometry.
    let mut named: Vec<(String, u32, u32)> = models
        .iter()
        .filter_map(|model| {
            model
                .name()
                .map(|name| (name.to_owned(), model.row_count(), model.column_count()))
        })
        .collect();
    named.sort();

    assert_eq!(
        named,
        vec![
            ("30-Day History Table".to_owned(), 32, 3),
            ("My Portfolio".to_owned(), 5, 13),
            ("Overview".to_owned(), 3, 3),
        ]
    );

    Ok(())
}

#[test]
fn numbers_sheets_expose_names_and_table_membership() -> Result<(), Error> {
    let spreadsheet =
        numbers::Document::open("examples/numbers/my_stocks.numbers")?.spreadsheet()?;
    let sheets = spreadsheet.sheets();
    let models = spreadsheet.table_models();

    let mut sheet_names: Vec<_> = sheets
        .iter()
        .filter_map(|sheet| sheet.name().map(str::to_owned))
        .collect();
    sheet_names.sort();
    assert_eq!(
        sheet_names,
        vec!["30-Day History".to_owned(), "Portfolio".to_owned()]
    );

    let table_name_for_id = |id: u64| {
        models
            .iter()
            .find(|model| model.id() == id)
            .and_then(numbers::TableModel::name)
            .map(str::to_owned)
    };

    let tables_for_sheet = |name: &str| -> Vec<String> {
        sheets
            .iter()
            .find(|sheet| sheet.name() == Some(name))
            .map(|sheet| {
                sheet
                    .table_model_ids()
                    .iter()
                    .filter_map(|id| table_name_for_id(*id))
                    .collect()
            })
            .unwrap_or_default()
    };

    assert_eq!(
        tables_for_sheet("30-Day History"),
        vec!["30-Day History Table".to_owned()]
    );
    assert_eq!(
        tables_for_sheet("Portfolio"),
        vec!["My Portfolio".to_owned(), "Overview".to_owned()]
    );
    assert!(
        sheets
            .iter()
            .all(|sheet| !sheet.table_info_ids().is_empty()),
        "each decoded sheet should retain its TableInfo references"
    );

    Ok(())
}

#[test]
fn table_models_reference_header_storage_buckets() -> Result<(), Error> {
    const NUMBERS_EXAMPLES: &[&str] = &[
        "examples/numbers/attendance.numbers",
        "examples/numbers/more_types.numbers",
        "examples/numbers/my_stocks.numbers",
        "examples/numbers/personal_budget.numbers",
        "examples/numbers/pivot_table.numbers",
        "examples/numbers/table_and_charts.numbers",
    ];

    let mut non_empty_buckets = 0usize;
    let mut saw_tall_table_last_index = false;

    for path in NUMBERS_EXAMPLES {
        let spreadsheet = numbers::Document::open(path)?.spreadsheet()?;
        for model in spreadsheet.table_models() {
            let row_bucket_id = model
                .row_header_storage_bucket_id()
                .ok_or(Error::InvalidIwa("missing row header storage bucket id"))?;
            let column_bucket_id = model
                .column_header_storage_bucket_id()
                .ok_or(Error::InvalidIwa("missing column header storage bucket id"))?;
            assert_eq!(
                model.header_storage_bucket_ids(),
                &[row_bucket_id, column_bucket_id],
                "{path} table {:?} should expose row then column HeaderStorageBucket ids",
                model.name(),
            );

            for id in model.header_storage_bucket_ids() {
                let bucket = spreadsheet
                    .header_storage_bucket(*id)
                    .ok_or(Error::InvalidIwa("missing header storage bucket"))?;
                assert_eq!(bucket.id(), *id);
                if !bucket.entries().is_empty() {
                    non_empty_buckets += 1;
                }
                assert!(
                    bucket
                        .entries()
                        .windows(2)
                        .all(|pair| pair[0].index() < pair[1].index()),
                    "{path} table {:?} bucket {id} entries should be ordered by index",
                    model.name(),
                );
                saw_tall_table_last_index |= bucket
                    .entries()
                    .iter()
                    .any(|entry| entry.index() == u64::from(model.row_count().saturating_sub(1)));
            }

            let row_bucket = spreadsheet
                .header_storage_bucket(row_bucket_id)
                .ok_or(Error::InvalidIwa("missing row header storage bucket"))?;
            assert!(
                row_bucket
                    .entries()
                    .iter()
                    .all(|entry| entry.index() < u64::from(model.row_count())),
                "{path} table {:?} row bucket entries should index the row axis",
                model.name(),
            );

            let column_bucket = spreadsheet
                .header_storage_bucket(column_bucket_id)
                .ok_or(Error::InvalidIwa("missing column header storage bucket"))?;
            assert!(
                column_bucket
                    .entries()
                    .iter()
                    .all(|entry| entry.index() < u64::from(model.column_count())),
                "{path} table {:?} column bucket entries should index the column axis",
                model.name(),
            );
        }
    }

    assert!(
        non_empty_buckets > 0,
        "fixtures should include populated HeaderStorageBucket entries"
    );
    assert!(
        saw_tall_table_last_index,
        "at least one header bucket should span the final row index of a tall table"
    );

    Ok(())
}

#[test]
fn table_model_geometry_matches_decoded_tile_dimensions() -> Result<(), Error> {
    // The "Summary by Category" table is the only model in personal_budget; its
    // declared geometry must match the dimensions the tile decoder recovers.
    let spreadsheet = numbers::Document::open(PERSONAL_BUDGET_EXAMPLE)?.spreadsheet()?;
    let models = spreadsheet.table_models();

    // personal_budget has two tables; both models must be recovered (the second
    // lives past a multi-message object in CalculationEngine.iwa).
    let mut named: Vec<(String, u32, u32)> = models
        .iter()
        .filter_map(|model| {
            model
                .name()
                .map(|name| (name.to_owned(), model.row_count(), model.column_count()))
        })
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![
            ("Summary by Category".to_owned(), 11, 4),
            ("Transactions".to_owned(), 27, 4),
        ]
    );

    let summary = models
        .iter()
        .find(|model| model.name() == Some("Summary by Category"))
        .ok_or(Error::InvalidIwa("missing Summary by Category table model"))?;
    assert_eq!(summary.row_count(), 11);
    assert_eq!(summary.column_count(), 4);
    assert!(summary.header_row_count() <= summary.row_count());
    assert!(summary.uuid().is_some());

    let matches_a_tile = spreadsheet.tables().iter().any(|table| {
        let rows = u32::try_from(table.rows().len()).unwrap_or(0);
        let cols = u32::try_from(
            table
                .rows()
                .iter()
                .map(|row| row.cells.len())
                .max()
                .unwrap_or(0),
        )
        .unwrap_or(0);
        rows == summary.row_count() && cols == summary.column_count()
    });
    assert!(
        matches_a_tile,
        "a decoded tile should match the model's row/column counts"
    );

    Ok(())
}
