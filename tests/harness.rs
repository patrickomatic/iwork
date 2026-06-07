use std::path::Path;

use iwork::Document;

const EXAMPLES: &[&str] = &[
    "examples/my_stocks.numbers",
    "examples/personal_budget.numbers",
    "examples/pivot_table.numbers",
    "examples/table_and_charts.numbers",
];

#[test]
fn every_example_opens_and_exposes_core_metadata() {
    for path in EXAMPLES {
        let package = Document::open(path).unwrap_or_else(|error| {
            panic!("failed to open {path}: {error}");
        });
        let report = package.inspect((*path).to_owned()).unwrap_or_else(|error| {
            panic!("failed to inspect {path}: {error}");
        });

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
}

#[test]
fn stylesheet_fixture_signal_is_present() {
    for path in EXAMPLES {
        let package = Document::open(path).unwrap();
        let stylesheet = package
            .package()
            .entry_bytes("Index/DocumentStylesheet.iwa")
            .unwrap();

        assert!(
            stylesheet
                .windows("Italic".len())
                .any(|window| window == b"Italic"),
            "{path} should include italic markers"
        );
        assert!(
            stylesheet
                .windows("Strikethrough".len())
                .any(|window| window == b"Strikethrough"),
            "{path} should include strikethrough markers"
        );
    }
}

#[test]
fn all_examples_use_the_numbers_extension() {
    for path in EXAMPLES {
        assert_eq!(
            Path::new(path).extension().and_then(|value| value.to_str()),
            Some("numbers")
        );
    }
}
