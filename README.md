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
