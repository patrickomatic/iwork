# Apple iWork Rust Crate

`iwork` is a Rust crate for opening Apple iWork packages (`.numbers`, `.pages`, and `.key`), inspecting a small set of stable metadata, and extracting semantic content from supported document types.

## What This Crate Does

**Numbers** (`.numbers`)
- sheet names and table membership (`Sheet`, `TableModel`)
- cell values: text, rich text, numbers (decimal128), dates, booleans, durations,
  errors, cached formula results with formula ids, currency, percentages
- table geometry (row/column counts, header rows/columns)
- write support: build minimal `.numbers` files from scalar cell data

**Pages** (`.pages`)
- document template name
- all text fragments from the body (TSWP storage, paragraphs split and cleaned)
- image alt-text (media descriptions)

**Keynote** (`.key`)
- presentation theme name
- per-slide: title, text fragments, image alt-text, template vs. content distinction

**All formats**
- document UUID and file-format version from `Metadata/Properties.plist`
- document kind detection from file extension

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

List a Numbers document's tables with their names, geometry, and cells:

```rust
use iwork::numbers;

fn main() -> Result<(), iwork::Error> {
    let document = numbers::Document::open("examples/numbers/my_stocks.numbers")?;

    for (model, table) in document.spreadsheet()?.decoded_tables() {
        println!(
            "{} — {}x{}, {} rows decoded",
            model.name().unwrap_or("(unnamed)"),
            model.row_count(),
            model.column_count(),
            table.rows().len(),
        );
    }

    Ok(())
}
```

List a Numbers document's sheets and the table models they contain:

```rust
use iwork::numbers;

fn main() -> Result<(), iwork::Error> {
    let spreadsheet = numbers::Document::open("examples/numbers/my_stocks.numbers")?
        .spreadsheet()?;

    for sheet in spreadsheet.sheets() {
        println!(
            "{}: {:?}",
            sheet.name().unwrap_or("(unnamed)"),
            sheet.table_model_ids()
        );
    }

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
                    _ => {}
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

The Numbers API surface:

- `Spreadsheet::sheets()` — sheet names and which tables each sheet contains
- `Spreadsheet::table_models()` — authoritative table list with name and geometry (row/column counts, header counts)
- `Spreadsheet::decoded_tables()` — one `(TableModel, Table)` per real table; cells resolved per-table so string keys never collide across tables
- `Spreadsheet::formula_records()` — formula record objects; field 2 matches formula ids stored on some cached formula-result cells
- `Spreadsheet::header_storage_bucket()` — row- and column-indexed header storage referenced by each table model

**Writing Numbers files:**

- `numbers::Workbook` and `numbers::WritableTable` assemble table rows from scalar cell values
- `Workbook::to_numbers_bytes()` and `Workbook::save_numbers()` produce a `.numbers` package

The generated package round-trips through this crate. Full Apple Numbers
compatibility still requires the complete document object graph, view state,
styles, and calculation-engine references.

Example tools help map those missing structures without turning fixture content
into parser heuristics. They decode the IWA/Snappy framing and hand the raw
protobuf payloads to the [`protorev`](https://github.com/patrickomatic/protorev) workbench rather than
re-implementing wire decoding:

```bash
# IWA framing + object stream overview, and a structural diff between packages
cargo run -p iwork-workbench --bin dump_iwa_graph -- path/to/document.numbers
cargo run -p iwork-workbench --bin diff_iwa_graph -- left.numbers right.numbers
cargo run -p iwork-workbench --bin inspect_numbers -- path/to/document.numbers

# Schema-infer one object type across packages (protorev schema/explain/values/diff/experiments)
cargo run -p iwork-workbench --bin iwa_corpus -- schema       6001 examples/numbers/*.numbers
cargo run -p iwork-workbench --bin iwa_corpus -- diff         6001 before.numbers after.numbers
cargo run -p iwork-workbench --bin iwa_corpus -- experiments  7    layout_investigation.protorev

# Explore the cross-object reference graph
cargo run -p iwork-workbench --bin iwa_refs -- edges 6001 examples/numbers/personal_budget.numbers
```

`dump_iwa_graph`/`diff_iwa_graph` work at the IWA framing and object-graph
level (entries, descriptor refs, object-type counts) with a `protorev` corpus
shape per archive. `iwa_corpus` gathers every object of one message type and
runs `protorev`'s confidence-gated schema inference, per-field explanations,
value sampling, and corpus diffing on it. `iwa_refs` recovers the cross-object
reference graph structurally (which object types reference which). Together they
let reverse engineering focus on controlled one-edit deltas, such as adding one
table, renaming one sheet, or changing one cell style.

**Known date encoding:** date cell values are `f64` seconds since the Cocoa epoch (`2001-01-01T00:00:00Z`). Decimal values may use IEEE 754-2008 decimal128 and are converted to `f64`.

## Pages Parsing Notes

What is decoded today:

- `Body::template_name()` — iWork template identifier (e.g. `"04B_Term_Paper"`)
- `Body::text_fragments()` — all paragraph text in document order; paragraphs split on `\n`, control-character block markers and object-replacement characters filtered
- `Body::media_descriptions()` — alt-text for each image

Known gaps:

- `Body::title()` returns `None` — paragraph-style title classification not yet implemented
- `Body::headings()` returns empty — heading style classification not yet implemented
- page/section structure and inline tables are not yet modeled

## Keynote Parsing Notes

What is decoded today:

- `Presentation::theme_name()` — theme name (e.g. `"Blueprint"`)
- `Slide::is_template()` — distinguishes layout masters from real slides
- `Slide::title()` — slide title from the title placeholder
- `Slide::text_fragments()` — all text on the slide in archive order
- `Slide::media_descriptions()` — alt-text for each image on the slide

Known gaps:

- `Slide::layout_name()` returns `None` — not yet located in the format
- `Slide::speaker_notes()` not yet implemented (pattern confirmed, ready to implement)
- slide ordering is by archive path, not a full slide-graph traversal
- animations, transitions, and exact object placement are not modeled

## Format Notes

The codebase depends on a small set of reverse-engineered format assumptions. Those notes live in [docs/file-format.md](docs/file-format.md), with the most important details also documented directly in `src/package.rs` and `src/plist.rs`.

## Repository Layout

- `src/`: library implementation and the small inspection CLI
- `examples/`: sample `.numbers`, `.pages`, and `.key` fixtures
- `tests/`: fixture-driven integration coverage
- `docs/`: format notes and implementation-oriented documentation

## License

MIT. See [LICENSE](LICENSE).
