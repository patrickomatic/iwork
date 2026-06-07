use std::path::Path;

use iwork::{Document, DocumentKind, Error, keynote, numbers, pages};

const NUMBERS_EXAMPLES: &[&str] = &[
    "examples/numbers/my_stocks.numbers",
    "examples/numbers/personal_budget.numbers",
    "examples/numbers/pivot_table.numbers",
    "examples/numbers/table_and_charts.numbers",
];

const PAGES_EXAMPLES: &[&str] = &[
    "examples/pages/modern_novel.pages",
    "examples/pages/term_paper.pages",
];

const KEYNOTE_EXAMPLES: &[&str] = &[
    "examples/keynote/basic_white.key",
    "examples/keynote/blueprint.key",
    "examples/keynote/parchment.key",
];

#[test]
fn every_example_opens_and_exposes_core_metadata() -> Result<(), Error> {
    for path in NUMBERS_EXAMPLES
        .iter()
        .chain(PAGES_EXAMPLES.iter())
        .chain(KEYNOTE_EXAMPLES.iter())
    {
        let package = Document::open(path)?;
        let report = package.inspect((*path).to_owned())?;

        assert!(report.entry_count > 0, "{path} should not be empty");
        assert!(report.iwa_count > 0, "{path} should contain iwa payloads");
        assert!(
            report.properties.document_uuid.is_some(),
            "{path} should expose a document uuid"
        );
        assert!(
            report.properties.file_format_version.is_some(),
            "{path} should expose a file format version"
        );
    }

    Ok(())
}

#[test]
fn stylesheet_fixture_signal_is_present() -> Result<(), Error> {
    for path in NUMBERS_EXAMPLES
        .iter()
        .chain(PAGES_EXAMPLES.iter())
        .chain(KEYNOTE_EXAMPLES.iter())
    {
        let package = Document::open(path)?;
        let stylesheet = package
            .package()
            .entry_bytes("Index/DocumentStylesheet.iwa")?;

        assert!(
            !stylesheet.is_empty(),
            "{path} should include a stylesheet payload"
        );
    }

    Ok(())
}

#[test]
fn examples_are_classified_by_extension() -> Result<(), Error> {
    for path in NUMBERS_EXAMPLES {
        let kind = Document::open(path)?.inspect((*path).to_owned())?.kind;
        assert_eq!(kind, DocumentKind::Numbers);
        assert_eq!(
            Path::new(path).extension().and_then(|value| value.to_str()),
            Some("numbers")
        );
        assert!(numbers::Document::open(path).is_ok());
    }

    for path in PAGES_EXAMPLES {
        let kind = Document::open(path)?.inspect((*path).to_owned())?.kind;
        assert_eq!(kind, DocumentKind::Pages);
        assert!(pages::Document::open(path).is_ok());
    }

    for path in KEYNOTE_EXAMPLES {
        let kind = Document::open(path)?.inspect((*path).to_owned())?.kind;
        assert_eq!(kind, DocumentKind::Keynote);
        assert!(keynote::Document::open(path).is_ok());
    }

    Ok(())
}
