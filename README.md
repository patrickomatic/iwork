# Apple iWork Rust Crate

`iwork` is a Rust crate for opening Apple iWork packages (`.numbers`, `.pages`, and `.key`), inspecting a small set of stable metadata, and extracting semantic content from supported document types.

## What This Crate Does

- opens iWork packages as ZIP containers
- exposes package entries and raw entry bytes
- can write stored ZIP packages and encode protobuf / IWA payloads
- reads `Metadata/Properties.plist`
- reads Numbers table cell values — text, numbers (decimal128), and dates
- can build Numbers table archives from scratch for scalar cell data
- inspects `Index/DocumentStylesheet.iwa` for simple keyword signals
- extracts UTF-8 string fields from Pages documents and Keynote decks

## Current Guarantees

- `Document::open` accepts any supported iWork package
- `numbers::Document`, `pages::Document`, and `keynote::Document` enforce the expected file extension
- fixture coverage exists for Numbers, Pages, and Keynote examples in `examples/`

## Usage

Add the crate to your project, then open a document and inspect it:

```rust
use iwork::Document;

fn main() -> Result<(), iwork::Error> {
    let document = Document::open("examples/numbers/personal_budget.numbers")?;
    let report = document.inspect("personal_budget.numbers")?;

    println!("kind: {}", report.kind.as_str());
    println!(
        "file format version: {}",
        report.properties.file_format_version.as_deref().unwrap_or("<unknown>")
    );

    Ok(())
}
```

You can also work at the package level:

```rust
use iwork::Document;

fn main() -> Result<(), iwork::Error> {
    let document = Document::open("examples/pages/modern_novel.pages")?;
    let stylesheet = document
        .package()
        .entry_bytes("Index/DocumentStylesheet.iwa")?;

    println!("stylesheet bytes: {}", stylesheet.len());
    Ok(())
}
```

Read cell values from a Numbers spreadsheet:

```rust
use iwork::numbers::{self, CellValue};

fn main() -> Result<(), iwork::Error> {
    let document = numbers::Document::open("examples/numbers/my_stocks.numbers")?;

    for table in document.spreadsheet()?.tables() {
        for row in table.rows() {
            for cell in &row.cells {
                match cell {
                    CellValue::Text(s) => println!("text:   {s}"),
                    CellValue::Number(n) => println!("number: {n}"),
                    CellValue::Date(secs) => println!("date:   {secs} s since 2001-01-01"),
                    CellValue::Empty => {}
                }
            }
        }
    }

    Ok(())
}
```

Extract UTF-8 string fields from a Pages document:

```rust
use iwork::pages;

fn main() -> Result<(), iwork::Error> {
    let document = pages::Document::open("examples/pages/term_paper.pages")?;
    let content = document.document()?;

    println!("title: {:?}", content.title());     // None until title fields are structurally decoded
    println!("headings: {:?}", content.headings()); // empty until heading fields are structurally decoded
    println!("first fragments: {:?}", &content.text_fragments()[..3.min(content.text_fragments().len())]);
    Ok(())
}
```

Extract UTF-8 string fields from a Keynote deck:

```rust
use iwork::keynote;

fn main() -> Result<(), iwork::Error> {
    let document = keynote::Document::open("examples/keynote/blueprint.key")?;
    let presentation = document.presentation()?;

    for slide in presentation.slides() {
        println!("{:?} {:?}", slide.layout_name(), slide.title());
        println!("text: {:?}", slide.text_fragments());
        println!("media: {:?}", slide.media_descriptions());
    }

    Ok(())
}
```

## Numbers Parsing Notes

The Numbers reader currently follows a two-stage model:

- `Spreadsheet::table_archives()` exposes the raw `Index/Tables/*.iwa` archives
- `Spreadsheet::tables()` resolves those archives into decoded rows and [`CellValue`](src/numbers/table.rs) values

The write-side Numbers support can create minimal crate-readable `.numbers`
packages:

- `numbers::Workbook` and `numbers::WritableTable` let you assemble table rows from scratch
- `Workbook::encode_table_archives()` emits fresh `Tile` and string `DataList` archives for scalar cells
- `Workbook::to_numbers_bytes()` and `Workbook::save_numbers()` wrap those archives in a ZIP package with minimal metadata and core IWA members

The generated package is currently intended for round-tripping through this
crate. Full Apple Numbers compatibility still requires a complete document
object graph, stylesheet links, table references, view state, and calculation
metadata.

The current parser relies on these reverse-engineered format details:

- string values are looked up through `DataList*.iwa` archives
- numeric and date values are stored inline in each tile row's field-6 cell buffer
- field 7 is the authoritative uint16 offset table for locating cell records
- date values are `f64` seconds since the Cocoa epoch (`2001-01-01T00:00:00Z`)
- decimal values may be stored as IEEE 754-2008 decimal128 and are converted to `f64`

The test suite covers both low-level decoder branches and fixture-backed structural checks for:

- text cells, multi-text rows, and grouped text rows
- finite numeric values decoded through the current cell-storage layout
- Cocoa-epoch date cells from the Numbers fixtures
- row decoding behavior around column counts, sentinels, and truncated records

## Pages String Field Parsing Notes

The Pages reader currently decodes `Index/Document.iwa` as IWA/protobuf data and
walks length-delimited fields to return valid UTF-8 string values. It returns:

- `None` for title until the title object field is structurally decoded
- an empty heading list until heading/paragraph style fields are structurally decoded
- ordered UTF-8 string fields recovered from the document archive

This avoids treating fixture prose as format knowledge. The parser does not scan
raw printable byte runs and does not classify strings by matching words from the
example documents.

Known gaps today:

- title, heading, paragraph, and text-run object fields are not yet mapped
- returned strings may include non-document metadata fields because the schema is
  not fully decoded yet
- this is read-only semantic extraction, not a structured Pages editing model

## Keynote String Field Parsing Notes

The Keynote reader works at the slide-archive level. It decodes `Slide*.iwa` and
`TemplateSlide*.iwa` entries as IWA/protobuf data and walks length-delimited
fields to return valid UTF-8 string values:

- `None` for layout names and titles until those object fields are structurally decoded
- ordered UTF-8 string fields recovered from each slide archive
- an empty media description list until media/alt-text fields are structurally decoded

This avoids treating fixture slide text, placeholder words, or image descriptions
as format knowledge.

Known gaps today:

- slide ordering is inferred from archive paths rather than a fully decoded slide graph
- template slides and live slides are both surfaced when they contain UTF-8 fields
- presenter notes, animations, and exact object placement are not yet modeled

## Format Notes

The codebase depends on a small set of reverse-engineered format assumptions. Those notes live in [docs/file-format.md](docs/file-format.md), with the most important details also documented directly in `src/package.rs` and `src/plist.rs`.

## Repository Layout

- `src/`: library implementation and the small inspection CLI
- `examples/`: sample `.numbers`, `.pages`, and `.key` fixtures
- `tests/`: fixture-driven integration coverage
- `docs/`: format notes and implementation-oriented documentation

## License

MIT. See [LICENSE](LICENSE).
