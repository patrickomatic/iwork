# Apple iWork Rust Crate

`iwork` is a Rust crate for opening Apple iWork packages (`.numbers`, `.pages`, and `.key`), inspecting a small set of stable metadata, and writing the original package bytes back out unchanged.

## What This Crate Does

- opens iWork packages as ZIP containers
- exposes package entries and raw entry bytes
- reads `Metadata/Properties.plist`
- reads Numbers table cell values — text, numbers (decimal128), and dates
- inspects `Index/DocumentStylesheet.iwa` for simple keyword signals
- preserves the original bytes on write for round-trip workflows

## Current Guarantees

- `Document::open` accepts any supported iWork package
- `numbers::Document`, `pages::Document`, and `keynote::Document` enforce the expected file extension
- `write` is lossless for the current implementation because it writes the original package bytes back out unchanged
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

Extract semantic text from a Pages document:

```rust
use iwork::pages;

fn main() -> Result<(), iwork::Error> {
    let document = pages::Document::open("examples/pages/term_paper.pages")?;
    let semantic = document.semantic_document()?;

    println!("title: {:?}", semantic.title());
    println!("headings: {:?}", semantic.headings());
    println!("first fragments: {:?}", &semantic.text_fragments()[..3.min(semantic.text_fragments().len())]);
    Ok(())
}
```

## Numbers Parsing Notes

The Numbers reader currently follows a two-stage model:

- `Spreadsheet::table_archives()` exposes the raw `Index/Tables/*.iwa` archives
- `Spreadsheet::tables()` resolves those archives into decoded rows and [`CellValue`](src/numbers/table.rs) values

The current parser relies on these reverse-engineered format details:

- string values are looked up through `DataList*.iwa` archives
- numeric and date values are stored inline in each tile row's field-6 cell buffer
- field 7 is the authoritative uint16 offset table for locating cell records
- date values are `f64` seconds since the Cocoa epoch (`2001-01-01T00:00:00Z`)
- decimal values may be stored as IEEE 754-2008 decimal128 and are converted to `f64`

The test suite covers both low-level decoder branches and fixture-backed examples for:

- text header rows from `personal_budget.numbers` and `pivot_table.numbers`
- decimal128 numeric values from `my_stocks.numbers`
- Cocoa-epoch date cells from the Numbers fixtures
- row decoding behavior around column counts, sentinels, and truncated records

## Pages Semantic Parsing Notes

The Pages semantic layer is currently best-effort rather than fully structural.
It scans `Index/Document.iwa` for high-confidence user-facing text and returns:

- an optional title when a strong title candidate is present
- normalized headings such as `Prologue`, `Subheading`, or `Chapter 1`
- ordered text fragments that preserve recoverable document prose

This is enough to extract stable content from the current `term_paper.pages` and
`modern_novel.pages` fixtures, but it does not yet model Pages paragraphs,
object graphs, or text runs precisely.

App-specific entry points reject mismatched extensions:

```rust
use iwork::numbers;

fn main() -> Result<(), iwork::Error> {
    let document = numbers::Document::open("examples/numbers/personal_budget.numbers")?;
    document.write("/tmp/personal_budget-copy.numbers")?;
    Ok(())
}
```

## Format Notes

The codebase depends on a small set of reverse-engineered format assumptions. Those notes live in [docs/file-format.md](docs/file-format.md), with the most important details also documented directly in `src/package.rs` and `src/plist.rs`.

## Repository Layout

- `src/`: library implementation and the small inspection CLI
- `examples/`: sample `.numbers`, `.pages`, and `.key` fixtures
- `tests/`: fixture-driven integration coverage
- `docs/`: format notes and implementation-oriented documentation

## License

MIT. See [LICENSE](LICENSE).
