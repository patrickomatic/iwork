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
